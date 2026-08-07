# spoke-connect

Reference library for the SPOKE Connect wire family (`../../.mstar/specs/spoke-connect.md`):
an embeddable Rust library that maps the connect envelopes onto rust-libp2p and
demonstrates the `noise-peerid` authenticated hello handshake, the
`capability-token` step-up auth method, per-session ordering, and op
invocation.

## Status

- **Reference library** — published on crates.io as `spoke-connect`; consumers
  depend on it via `cargo add spoke-connect`.
- Consumes wire types exclusively from `spoke-schemas` generated modules
  (`ConnectHello`, `HostCapabilityManifest`, `ConnectInvokeRequest`,
  `ConnectInvokeResponse`, …) — every connect wire type comes from codegen.

## Transport composition

| Layer | Choice |
|-------|--------|
| Security | `noise` (authenticated encryption; remote `PeerId` is noise-authenticated) |
| Multiplexing | `yamux` |
| Messaging | `request-response` — hello exchange and op invocation |
| Peer info | `identify` (carries the remote public key used to verify hello signatures) |

libp2p is pinned to a single version (`=0.56.0`) with a minimal feature set:
`noise`, `yamux`, `tcp`, `tokio`, `identify`, `request-response`, `json`,
`macros`, `ed25519`.

## Discovery

**Explicit peering** is the discovery mechanism: nodes are configured with
static listen addresses and dial each other directly. Dials are
single-flight — a second `connect` while a dial is pending is rejected with
a `connect is already in progress` error.

Session admission stays fully gated by the allowlist and signed hello: the
connect wire carries only the six envelope families; discovery is
transport-side (see the [spoke-connect spec §Discovery
boundary](../../.mstar/specs/spoke-connect.md)).

## Authenticated hello (`spoke-connect-hello-jcs-v1`)

1. Both sides of a connection exchange a signed `ConnectHello`.
2. The signed object is **role-aware**: the **initiator** signs exactly
   `{protocol_version, peer_id, nonce, host}`; the **responder** signs
   exactly `{protocol_version, peer_id, nonce, host, peer_nonce}` where
   `peer_nonce` is the initiator's nonce (dial binding). Top-level
   `extensions` and `signature` are excluded in both roles.
3. The object is canonicalized with **RFC 8785 JCS** — implemented by
   [`serde_jcs`](https://crates.io/crates/serde_jcs) (0.2.0): generic over
   serde `Serialize`, so the generated `spoke-schemas` manifest serializes
   directly with no `serde_json::Value` round-trip; small dependency tree
   (`ryu-js`, `serde`, `serde_json`); MIT/Apache-2.0.
4. The bytes are signed with the libp2p identity keypair (Ed25519); the raw
   signature is encoded **base64url, no padding**.
5. The receiver accepts only when: protocol version is 1, the claimed
   `peer_id` equals the noise-authenticated remote peer, the identify public
   key derives that peer id, the peer is on the configured allowlist (empty
   allowlist rejects all peers — fail-closed), the signature verifies against
   the peer's public key, and the `(peer_id, nonce)` pair is new (in-memory
   single-use set, process lifetime).
6. **Dial binding (fail-closed):** the responder hello carries `peer_nonce` =
   the initiator's nonce, binding the responder to the current dial. An
   initiator that supplied its nonce verifies the responder hello over the
   5-field object and rejects a `peer_nonce` mismatch — a replayed responder
   hello captured on an earlier dial cannot re-enter a fresh dial across a
   process restart (the in-memory nonce store resets). An initiator that
   expects a responder also rejects a hello WITHOUT `peer_nonce`: an old or
   mixed-version responder that omits the field must not bypass the binding.
7. Rejection closes the hello stream; protocol v1 acknowledges accepted
   hellos with an ack response.

`peer_id` on the wire is `PeerId::to_string()` — the libp2p PeerId base58btc
multihash form.

## Capability-token auth (`capability-token`)

The `capability-token` method (normative, [spoke-connect spec §Method —
capability-token](../../.mstar/specs/spoke-connect.md)) is a **step-up /
mid-session capability grant** on top of the `noise-peerid` hello identity:
a trusted issuer signs a short claim set (`iss` / `sub` / `aud` /
`capabilities` / `exp`, optional `iat` / `jti`) over RFC 8785 JCS with
Ed25519, and the proof rides the `ConnectAuthChallenge` /
`ConnectAuthResponse` exchange and optionally the `ConnectInvokeRequest.auth`
blob.

Configure the method on `ConnectConfig`:

- `trusted_issuers: Vec<String>` — issuer `peer_id`s whose signed tokens this
  node accepts. **Empty list ⇒ the method is disabled**; the node offers challenges and
  accepts proofs only when the list is non-empty (fail closed).
- `require_capability_token: bool` (default `false`) — when `true` (and
  `trusted_issuers` is non-empty), every new session must complete the
  challenge before invokes are accepted. Default keeps the
  `noise-peerid`-only behavior.
- `capability_token_provider: Option<Arc<CapabilityTokenProvider>>` — the
  hook that answers inbound challenges. Called with the challenger's
  `peer_id` (the token's `aud`) and returns the wire `proof` object
  (`{ v, claims, sig }`); the proof must be issued by a trusted issuer with
  `sub` = this node's `peer_id`. The hook may hold a token cache or mint on
  demand.

Challenge / response flow: after both hellos are accepted, a node with an
active token policy sends `ConnectAuthChallenge { method:
"capability-token", challenge_id, challenge }` (fresh `challenge_id`, random
nonce ≥ 16 chars, bound in a single-use pending slot) and the peer answers
through its provider with `ConnectAuthResponse { challenge_id, method,
proof }`. A valid response marks the session token-authorized and completes
the pending `connect`; a missing provider, provider error, or unknown method
drops the exchange — the session stays unauthorized for invokes and the
pending `connect` resolves only at the handshake timeout (fail closed).
The session grant is held for the session lifetime; per-invoke `auth`
revalidates expiry on later invokes. Products needing mid-session expiry
enforcement implement their own re-challenge flow or attach a per-invoke
`auth`.

Per-invoke `auth`: `ConnectInvokeRequest.auth` optionally carries the same
proof object. When present, the receiver validates it on **every** invoke
(expiry re-checked; same issuer / subject / audience rules), independent of
the challenge flag. Once the challenge policy is active, invokes attach
`auth`; a session without a validated grant is rejected with an
`auth_failed` wire envelope.

Error codes ride the existing open-string wire vocabulary (`ErrorEnvelope`
`code` — no new envelope fields): `auth_failed` for missing or invalid
tokens and unknown auth methods; `op_unsupported` when the token is valid
but the op's required capability is absent from the grant (the same code as
the dispatch-deny and no-handler paths); `internal_error` for handler
panics. Implementations MAY distinguish failure reasons in `message` /
optional `details`.

Reference tests: `cargo test -p spoke-connect` covers the issue→verify
round-trip, expiry / issuer / subject / audience rejection, the
challenge / response flow, and the invoke token gate (unit + two-node
integration).

## Sessions and op invocation

`SpokeConnectNode::connect(addr)` dials an explicit address and returns a
`PeerSession` once **both** hellos of the connection are confirmed (the
remote acknowledged ours and we accepted theirs). Each session:

- owns a per-direction **outbound** `sequence` counter starting at **0**
  (each peer numbers its own requests from 0; exhaustion
  closes the session with `InvokeError::SequenceExhausted`);
- assigns the next sequence **atomically** per `invoke` (concurrent invokes
  are allowed) and generates a UUID v4 `request_id`; the outbound counter is
  the mutex-guarded core `OutboundSequence` — concurrent `invoke` calls
  receive distinct sequences and `next_sequence` observation is synchronous;
- requires the response to **echo** `session_id`, `sequence`, and
  `request_id` — any mismatch fails with `InvokeError::CorrelationMismatch`;
- enforces **inbound** sequence monotonicity on the accept path: the
  receiver tracks the next expected inbound sequence per session (starts at
  0) and answers a replayed or out-of-order sequence with an
  `invalid_sequence` wire envelope — handler side effects run only for
  accepted sequences;
- returns `InvokeSuccess { sequence, request_id, payload }` on success;
  remote application failures arrive as `InvokeError::Wire(ErrorEnvelope)`;
  transport / session failures use the other `InvokeError` variants.

Session ids are **per-side** in protocol v1: each node records its own opaque
id for the pairing (there is no session-announce message), and the response
echo correlates by peer.

The accept path answers inbound invokes through the configured dispatch
hook: `invoke_handler_v2` when set (`(peer, op, payload) -> Result<payload,
ErrorEnvelope>` — the first argument is the **noise-authenticated session
peer id**, never a payload-carried `peer_id` claim), else the legacy
`invoke_handler` (`(op, payload)`). The hook runs **synchronously on the
node's network event loop**: it must return promptly and must not block on
I/O. Panics are contained — the invoke is answered with an `internal_error`
wire envelope and the node keeps running. The hook is spike-scoped — the op
dispatcher is adapter-owned in products; inbound invokes are answered by the
registered handler; unhandled ops receive an `op_unsupported` error
envelope.

The wire imposes no payload size limit; the spike inherits libp2p's
request-response JSON codec default of a **1 MiB maximum request size** as a
transport backstop (configurable in a later iteration). Size bounding and
flow control are transport/adapter-owned (see the spoke-connect spec
§Hard boundaries).

Node startup listens on the configured addresses; this spike settles one
resolved address per configured listener (a single-listener ceiling —
multi-listener settlement with `ListenerId` tracking is a later iteration).

Wire note: codegen inlines `$ref` types. `ConnectHello.host` is the file-local
`spoke_schemas::connect::connect_hello::HostCapabilityManifest`
(field-identical to `data::HostCapabilityManifest` but a distinct generated
type), and the invoke error branch carries the inline
`spoke_schemas::connect::connect_invoke_response::ErrorEnvelope`.
`ConnectConfig.local_manifest`, `PeerSession::remote_manifest`, and
`InvokeError::Wire` use exactly those wire types — zero conversion.

## Protocol names

| Protocol | Name |
|----------|------|
| Hello exchange | `/spoke/connect/hello/1.0.0` |
| Auth exchange | `/spoke/connect/auth/1.0.0` |
| Op invocation | `/spoke/connect/invoke/1.0.0` |

## Usage

The compiled example [`examples/two_node_usage.rs`](examples/two_node_usage.rs)
(`cargo run -p spoke-connect --example two_node_usage`) runs the full happy
path: node B dials node A's resolved listen address with A's `PeerId` on B's
allowlist, both sides exchange their manifests through the signed hello, and
B invokes the `check` op on A's handler:

```rust
use libp2p::identity::Keypair;
use spoke_connect::{parse_multiaddr, ConnectConfig, SpokeConnectNode};
use spoke_schemas::connect::connect_hello::HostCapabilityManifest;
use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::Arc;

let key_a = Keypair::generate_ed25519();
let key_b = Keypair::generate_ed25519();
let peer_a = key_a.public().to_peer_id();
let peer_b = key_b.public().to_peer_id();

let manifest = |host_id: &str, role: &str| HostCapabilityManifest {
    authority: None,
    capabilities: vec!["spoke-baseline".into()],
    extensions: Default::default(),
    host_id: host_id.parse().expect("host id"),
    namespaces: Vec::new(),
    roles: vec![role.into()],
    schema_version: NonZeroU64::new(1).expect("non-zero"),
};

// Node A answers inbound invokes; node B dials A.
let node_a = SpokeConnectNode::start(ConnectConfig {
    identity: key_a,
    peer_allowlist: vec![peer_b],          // the remote peer's PeerId
    listen_addrs: vec![parse_multiaddr("/ip4/127.0.0.1/tcp/0").expect("addr")],
    local_manifest: manifest("host-a", "checker"),
    handshake_timeout: None,
    invoke_handler: Some(Arc::new(|op: &str, _payload: serde_json::Value| {
        assert_eq!(op, "check");
        Ok(serde_json::json!({ "findings": [], "extensions": {} }))
    })),
    invoke_handler_v2: None,               // additive session-peer-aware hook; optional
    op_capability_requirements: HashMap::new(),
    trusted_issuers: vec![],               // capability-token auth disabled by default
    require_capability_token: false,
    capability_token_provider: None,
})
.await
.expect("start a");

let node_b = SpokeConnectNode::start(ConnectConfig {
    identity: key_b,
    peer_allowlist: vec![peer_a],
    listen_addrs: vec![parse_multiaddr("/ip4/127.0.0.1/tcp/0").expect("addr")],
    local_manifest: manifest("host-b", "input-source"),
    handshake_timeout: None,
    invoke_handler: None,
    invoke_handler_v2: None,
    op_capability_requirements: HashMap::new(),
    trusted_issuers: vec![],
    require_capability_token: false,
    capability_token_provider: None,
})
.await
.expect("start b");

// Dial the remote's resolved address; the session is ready once both
// hellos are confirmed.
let session = node_b
    .connect(node_a.listen_addrs()[0].clone())
    .await
    .expect("session");
let success = session
    .invoke("check", serde_json::json!({ "scope": { "scope_id": "s1" } }))
    .await
    .expect("invoke");
println!("sequence {} -> {:?}", success.sequence, success.payload);

node_a.shutdown().await.expect("shutdown a");
node_b.shutdown().await.expect("shutdown b");
```

`HostCapabilityManifest` here is the codegen-*inline* type
(`spoke_schemas::connect::connect_hello::HostCapabilityManifest`) — the exact
field type of `ConnectHello.host` and `ConnectConfig.local_manifest` (see
[Sessions and op invocation](#sessions-and-op-invocation)).

## Binding facade

The session core (`src/core/`) is the pure, synchronous, language-portable
layer of this crate: it owns the session rules — `peer_id` derivation, hello
sign/verify, allowlist, nonce single-use, sequence allocation/advance,
response correlation, dispatch gate — with `libp2p`, `tokio`, and I/O kept
transport-side. It operates on plain `String` peer ids, byte-oriented
keys, `spoke-schemas` connect types, and opaque `serde_json` payloads; the
transport layer converts `libp2p::PeerId` ↔ `String` at the boundary and
calls into the core.

This section records the binding facade decision for the spec's
**native-bindings** embedding path (shared core bindings,
`../../.mstar/specs/spoke-connect.md` §Embedding model):
what stays synchronous vs asynchronous on the FFI boundary, which surface the
first binding exposes, and which languages are targeted. **Swift (SPM
`SpokeConnect`)** and **Kotlin (GitHub Packages Maven)** are landed alongside
C#, Go, and Python: the exported surface below ships through uniffi behind the
optional `ffi` feature as a `cdylib`, foreign bindings generate from that
library, and golden-parity smokes assert parity from each host side. The
binding exposes the sync core surface; async node lifecycle stays Rust-side.

### Sync vs async boundary

| Concern | Sync or async | FFI implication |
|---------|---------------|-----------------|
| `peer_id` derive, hello sign/verify, allowlist, nonce, sequence allocate/advance, correlation, dispatch gate | **Sync** — pure, fast | First binding surface: exported as synchronous FFI functions / objects; safe on any foreign caller thread, no tokio |
| Node start, listen, shutdown | **Async** (tokio today) | Do not block foreign UI threads on the swarm loop; the landed binding is core-only (host-language transport + Rust core); a uniffi async/foreign-runtime bridge for a thin `SpokeConnectNode` is a deferred option |
| `connect(addr)` / session establishment | **Async** | Transport-owned, same as the node surface |
| `invoke` wait for response | **Async** today | Foreign side uses an async binding or a callback/channel; the core only assigns the sequence and checks correlation on bytes already received |
| `invoke_handler` | Sync **on the event loop** today | Product handlers must return promptly and must not block on I/O; before exposing handlers over FFI, dispatch moves off the loop (`spawn_blocking`) |

### Exported surface (landed)

Eight functions and three objects are exported; uniffi renames them to Swift
camelCase with `Data` keys:

| Rust (FFI) | Swift | Behavior |
|------------|-------|----------|
| `derive_peer_id_from_ed25519_pubkey(pubkey: Vec<u8>) -> Result<String, CoreError>` | `derivePeerIdFromEd25519Pubkey(pubkey: Data) throws -> String` | Wire `peer_id` for a 32-byte Ed25519 public key |
| `sign_hello_ed25519(secret: Vec<u8>, nonce: String, host_json: String) -> Result<String, CoreError>` | `signHelloEd25519(secret: Data, nonce: String, hostJson: String) throws -> String` | JCS-canonicalized, Ed25519-signed hello; returns the `ConnectHello` envelope as JSON |
| `verify_hello_ed25519(public_key: Vec<u8>, expected_peer_id: String, hello_json: String) -> Result<(), CoreError>` | `verifyHelloEd25519(publicKey: Data, expectedPeerId: String, helloJson: String) throws` | Signature and peer-id binding checks |
| `is_allowlisted(allowlist: Vec<String>, peer_id: String) -> bool` | `isAllowlisted(allowlist: [String], peerId: String) -> Bool` | Fail-closed allowlist check |
| `check_response_correlation(expected_…, actual_…) -> Result<(), CoreInvokeError>` | `checkResponseCorrelation(expectedSessionId:expectedSequence:expectedRequestId:actual…:) throws` | Echo check on `session_id` / `sequence` / `request_id`, flattened to primitives |
| `dispatch_allowed(op: String, negotiated_capabilities: Vec<String>) -> bool` | `dispatchAllowed(op: String, negotiatedCapabilities: [String]) -> Bool` | Op capability ⊆ negotiated capabilities (core-op table), fails closed |
| `required_capability(op: String) -> Option<String>` | `requiredCapability(op: String) -> String?` | Capability required by the core-op table; `nil` for product-defined ops |
| `protocol_version() -> u64` | `protocolVersion() -> UInt64` | Connect protocol version (1) |

| Rust (FFI) | Swift | Methods |
|------------|-------|---------|
| `NonceStore` | `NonceStore()` | `checkAndRecord(peerId: String, nonce: String) -> Bool` — single-use `(peer_id, nonce)` gate |
| `OutboundSequence` | `OutboundSequence()` | `allocate() throws -> UInt64` — outbound sequence from 0, no wrap (2⁵³−1) |
| `InboundSequence` | `InboundSequence()` | `advance(sequence: Int64) throws -> UInt64` — strict next-expected inbound check |

Errors map variant-for-variant onto two uniffi enums:

- `CoreError` → Swift `CoreError`: `InvalidHelloSignature`, `NonceReplay`,
  `HandshakeFailed(reason: String)`, `InvalidNonce(message: String)`,
  `Crypto(message: String)`, `Jcs(message: String)`,
  `TokenInvalid(message: String)`.
- `CoreInvokeError` → Swift `CoreInvokeError`: `SequenceExhausted`,
  `InboundSequenceMismatch(expected: UInt64, actual: Int64)`,
  `CorrelationMismatch`.

Keys cross the boundary as raw bytes (validated to exactly 32 bytes inside
the wrapper), peer ids as strings, and the host manifest / hello envelope as
JSON strings deserialized with `serde_json` inside Rust — `Multiaddr`,
swarm, libp2p, and generated schema types stay Rust-side.

### Additive remote-adapter surface (sync `RemoteAdapterFFI` / `MultiPeerRouterFFI`)

With the `remote-adapter` feature enabled alongside `ffi`, the cdylib exports
a synchronous remote-adapter facade over the same encapsulated
`RemoteAdapter` lifecycle as the Rust and TypeScript clients: dial through a
foreign-implemented message transport, then call BaselinePorts-shaped methods
that return JSON strings or `FfiError`; the same surface exposes the
multi-peer router (`MultiPeerRouterFFI`) over registered adapter handles. The
binding implements a synchronous callback `Transport`; Rust bridges it to the
async adapter seam via a process-wide tokio runtime (`block_on` /
`spawn_blocking` — foreign `recv` blocks on the binding thread pool, not on
async workers).

Build the metadata-bearing cdylib with both features:

```shell
cargo build -p spoke-connect --features ffi,remote-adapter
```

| Rust (FFI) | Swift | Behavior |
|------------|-------|----------|
| `connect_remote_adapter_ffi(transport, local_seed, local_manifest_json, remote_pubkey, allowlist, invoke_timeout_ms) -> Result<RemoteAdapterFFI, FfiError>` | `connectRemoteAdapterFfi(transport:localSeed:localManifestJson:remotePubkey:allowlist:invokeTimeoutMs:) throws -> RemoteAdapterFfi` | Dial + signed-hello handshake over the callback `Transport`; returns a live adapter handle |

| Rust (FFI) | Swift | Methods |
|------------|-------|---------|
| `RemoteAdapterFFI` | `RemoteAdapterFfi` | `state() -> String`; `session_id()` / `remote_peer_id()` / `remote_manifest()` (optional strings); BaselinePorts proxies — `get_host_capability_manifest()`, `get_knowledge_entry(entry_id)`, `put_knowledge_entry(entry_json, expected_base_revision)`, `get_relation(relation_id)`, `put_relation(relation_json, expected_base_revision)`, `list_knowledge_entries(scope_json)`, `list_timeline_events(scope_json)`, `put_findings(findings_json)`, `list_rules(rule_refs)`, `list_peer_host_capability_manifests()` (each `Result<String, FfiError>` JSON payload); `close()` |

| Rust (FFI) | Swift | Behavior |
|------------|-------|----------|
| `new_multi_peer_router_ffi() -> MultiPeerRouterFFI` | `newMultiPeerRouterFfi() -> MultiPeerRouterFfi` | Empty router over the same runtime; register established adapter handles |

| Rust (FFI) | Swift | Methods |
|------------|-------|---------|
| `MultiPeerRouterFFI` | `MultiPeerRouterFfi` | `register_peer(adapter) -> Result<String, FfiError>` (accepts an established `RemoteAdapterFFI`, returns the remote `peer_id`); `unregister_peer(peer_id)`; `list_peers() -> Vec<String>`; the BaselinePorts proxies and both `HostManifestPort` views (each `Result<String, FfiError>` JSON payload) |

Callback transport (foreign binding implements; Rust calls synchronously):

| Rust (FFI) | Swift | Methods |
|------------|-------|---------|
| `Transport` (callback interface) | `Transport` | `send(envelope) -> Result<(), TransportError>`; `recv() -> Result<Vec<u8>, TransportError>` (blocks until one envelope or close); `close() -> Result<(), TransportError>` (idempotent) |

Loopback test helpers (in-memory pair for binding smokes — no network):

| Rust (FFI) | Swift | Behavior |
|------------|-------|----------|
| `loopback_transport_pair() -> LoopbackTransportPair` | `loopbackTransportPair() -> LoopbackTransportPair` | Back-to-back in-memory connection |
| `LoopbackTransportPair` | `LoopbackTransportPair` | `client()` / `server()` → `LoopbackTransport` ends |
| `LoopbackTransport` | `LoopbackTransport` | Same `send` / `recv` / `close` surface as the callback `Transport` |

Additional error enums (additive — core enums above stay unchanged):

- `TransportError` → Swift `TransportError`: `Closed`, `Io(message: String)`.
- `FfiError` → Swift `FfiError`: `Dial(kind: String, message: String)` for
  constructor / dial failures (`config` / `handshake` / `timeout` kinds);
  `Rejected(code: String, message: String, kind: String?, wire_code: String?)`
  for invoke-path `SpokeResult::Reject` passthrough (application codes
  preserved; `INTERNAL_ERROR` rows carry `kind`; dispatch deny and unknown wire
  codes carry `wire_code`).

### Native-binding scope (first language)

The landed binding surface is **core + remote adapter**: the sync session-core
table above plus the additive `RemoteAdapterFFI` / `MultiPeerRouterFFI` /
callback `Transport` / `FfiError` surface when `remote-adapter` is enabled. Host languages can
either implement their own transport against the wire contract using the
core-only helpers, or supply a callback `Transport` and dial through
`connect_remote_adapter_ffi` for the full encapsulated adapter. The **core +
async node** option — additionally bridging a thin `SpokeConnectNode`
lifecycle over the uniffi async/foreign-runtime mechanism — is deferred:
node start/listen/shutdown and `connect(addr)` stay Rust-side today.

### Target-language matrix

> Priority per product direction (2026-08-02): **C#, Go, Python, Swift, Kotlin**. **C#**, **Go**, **Python**, **Swift (SPM)**, and **Kotlin (Maven)** are **landed** on their publish channels (see binding READMEs under `bindings/`). The order below records embedding path and channel for each language.

| Language | Embedding path | Publish channel | Priority | Rationale |
|----------|----------------|-----------------|----------|-----------|
| C# | Native binding (uniffi) | **GitHub Packages NuGet** (`42ch.Spoke.Connect`) | First target — **landed** | Desktop/server hosts; generated binding + net8.0 golden-parity smoke via a vendored `uniffi-bindgen-cs` fork retargeted to uniffi 0.32 (fork dropped when a bindgen-cs tag targets 0.32+); the community pipeline's latest tag (v0.11.0+v0.31.0) targets uniffi 0.31 and cannot read the 0.32 cdylib metadata |
| Go | Native binding (uniffi) | **Go modules** (`go get …/bindings/go@vX.Y.Z`) | Second — **landed** | Server/CLI hosts; community `uniffi-bindgen-go` pipeline |
| Python | Native binding (uniffi) | **PyPI** (`pip install spoke-connect`) | Third — **landed** | Platform wheels + golden-parity smoke via first-party uniffi 0.32 bindgen; `publish-pypi` on `release.yml` |
| **Swift (iOS / macOS)** | Native binding (uniffi) | **SPM git** (root `Package.swift` + `vX.Y.Z` tags) | Fourth — **landed** | Product `SpokeConnect`; macOS golden-parity smoke; [`bindings/swift/README.md`](bindings/swift/README.md) |
| **Kotlin (Android)** | Native binding (uniffi) | **GitHub Packages Maven** (`dev.42ch:spoke-connect`) | Fifth — **landed** | Same sync core surface; `publish-maven` on `release.yml`; [`bindings/kotlin/README.md`](bindings/kotlin/README.md) |
| TypeScript (browser / Node) | **Language-native client** (TypeScript, direct) | **npm** (`@42ch/spoke-connect`) | Parallel track | The TypeScript route decision lives with the TS identity proof |

### Binding checklist

- [x] 1. Stable sync core API list (the exported surface above) with string
     peer ids and byte-oriented keys.
- [x] 2. Error code mapping table (`CoreError` / `CoreInvokeError` →
     foreign enums).
- [x] 3. Native-binding choice for a second language — core-only is landed for
     Swift and Kotlin; the core + async-node option stays open for a later iteration.
- [x] 4. Golden hello vector — JCS bytes + signature + `peer_id` for a known
     Ed25519 keypair — shared across every language: a single SSOT at
     [`tests/fixtures/golden-hello.json`](tests/fixtures/golden-hello.json)
     carries the seed / nonce / manifest inputs and the pinned output bytes
     (pubkey, peer id, JCS hex, signature). The Rust golden tests, the
     TypeScript client, and every binding smoke load from the SSOT or a
     registered byte-identical copy; the sync gate
     [`tooling/connect/golden-vector-sync.mjs`](../../../tooling/connect/golden-vector-sync.mjs)
     exits non-zero on any drift.
- [x] 5. `Multiaddr` / swarm types stay Rust-side at the FFI boundary —
     satisfied by the landed surface.

### Golden hello vector (shared SSOT)

The cross-language golden hello vector — Ed25519 seed, derived pubkey +
peer id, golden nonce, host manifest, and the pinned RFC 8785 JCS bytes +
libp2p-captured signature — has a single source of truth at
[`tests/fixtures/golden-hello.json`](tests/fixtures/golden-hello.json).
The vector is transcribed from committed libp2p-captured constants (never
regenerated by running the code under test) and carries both the inputs and
the independently captured output bytes. The same file also carries the
**responder golden** (`responder` block): the same key pair / manifest signed
over the 5-field object incl. `peer_nonce` = the initiator golden nonce,
pinned when the dial-binding mechanism landed. The crate's golden-vector
tests, the TypeScript client (`packages/spoke-connect-ts`), and all five
binding smokes (C# / Go / Python / Swift / Kotlin) load from the SSOT or a
registered byte-identical copy; `tooling/connect/golden-vector-sync.mjs`
verifies byte-equality across every copy and exits non-zero on drift.

### Swift smoke (macOS)

The macOS-local smoke (`bindings/swift/Smoke/main.swift`) derives the golden
peer id from the golden Ed25519 seed, signs and verifies the golden hello
(asserting base64url signature parity with the Rust core), and exercises the
rest of the exported surface — allowlist, sequences, nonce store, dispatch
gate, correlation, protocol version — with the mapped error cases. Golden
fixtures load from the registered byte-identical copy of the shared SSOT
(`Smoke/fixtures/golden-hello.json`). Every check prints `PASS`.

> **Swift smoke: maintainer macOS** — CI exercises the Rust export surface
> (`cargo build` / `cargo test -p spoke-connect --features ffi`) on ubuntu;
> the Swift toolchain and this smoke stay macOS-local.

Run from the repository root (exact working forms; generated bindings and the
xcframework are committed under `bindings/swift/` — regenerate when the FFI
surface changes; see [`bindings/swift/README.md`](bindings/swift/README.md)):

```bash
# 1. Build the cdylib that carries the exported-surface metadata.
cargo build -p spoke-connect --features ffi

# 2. Regenerate the Swift bindings from the cdylib.
cargo run -p spoke-connect --features bindgen-cli --bin uniffi-bindgen -- \
  generate --library target/debug/libspoke_connect.dylib \
  --language swift --out-dir crates/spoke-connect/bindings/swift/generated

# 3. Point the dylib install name at @rpath (cargo bakes in the absolute
#    deps-dir path, which would pin the smoke to one machine).
install_name_tool -id @rpath/libspoke_connect.dylib target/debug/libspoke_connect.dylib

# 4. Compile the smoke (Swift 5 language mode keeps top-level code simple;
#    `-fmodule-map-file` supplies the module map to the Clang importer).
swiftc -Xcc -fmodule-map-file="$PWD/crates/spoke-connect/bindings/swift/generated/spoke_connectFFI.modulemap" \
  -L target/debug -lspoke_connect \
  -Xlinker -rpath -Xlinker "$PWD/target/debug" \
  -swift-version 5 \
  -o crates/spoke-connect/bindings/swift/Smoke/smoke \
  crates/spoke-connect/bindings/swift/Smoke/main.swift \
  crates/spoke-connect/bindings/swift/generated/spoke_connect.swift

# 5. Run it — every line must print PASS.
./crates/spoke-connect/bindings/swift/Smoke/smoke
```

Local env quirk: if `cargo` fails with `error: the option Z is only accepted
on the nightly compiler` — a nightly-only `-Z` flag set in
`~/.cargo/config.toml` `[unstable] rustflags` — run the cargo steps with the
**nightly toolchain** (`cargo +nightly …`) so the flag is honored; CI builds
on stable.

## Testing

```shell
cargo test -p spoke-connect          # unit + two-node integration tests (default features)
cargo test --workspace               # no regression across the workspace
cargo tree -p spoke-connect --depth 1  # dependency surface check
```

The two-node integration test (`tests/two_node_exchange.rs`) covers: hello
exchange with manifests round-tripping both directions (role reversal),
`check` invokes with `sequence == 0` / `sequence == 1` and `request_id` echo,
rejection of a non-allowlisted third node, the `InvokeError::Wire`
remote-failure path, a panicking handler contained with an `internal_error`
envelope, duplicate dials completing with the existing session, deterministic
dial timeout and retry, unreachable-address dial failure, and reconnection
after the peer shuts down. All waits are bounded event waits (timeouts on
the `connect` / `invoke` futures).

## Normative reference

- `../../.mstar/specs/spoke-connect.md` — envelope field tables, JCS rules, nonce /
  replay, ordering, auth model, discovery boundary.
- `schemas/connect/` — the JSON Schema SSOT for the six connect envelopes.
