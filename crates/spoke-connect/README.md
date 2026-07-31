# spoke-connect

Reference spike for the SPOKE Connect wire family (`.mstar/specs/spoke-connect.md`):
an embeddable Rust library that maps the connect envelopes onto rust-libp2p and
demonstrates the `noise-peerid` authenticated hello handshake, per-session
ordering, and op invocation.

## Status

- **Reference spike** — a transport demonstration, not a published SDK.
- **Workspace-private** (`publish = false`); library only, no daemon or binary target.
- Consumes wire types exclusively from `spoke-schemas` generated modules
  (`ConnectHello`, `HostCapabilityManifest`, `ConnectInvokeRequest`,
  `ConnectInvokeResponse`, …) — no parallel hand-written envelopes.

## Transport composition

| Layer | Choice |
|-------|--------|
| Security | `noise` (authenticated encryption; remote `PeerId` is noise-authenticated) |
| Multiplexing | `yamux` |
| Messaging | `request-response` — hello exchange and op invocation |
| Peer info | `identify` (carries the remote public key used to verify hello signatures) |

libp2p is pinned to a single version (`=0.56.0`) with a minimal feature set:
`noise`, `yamux`, `tcp`, `tokio`, `identify`, `request-response`, `json`,
`macros`, `ed25519`. QUIC, relay, DHT, and gossipsub are not enabled.

## Discovery

- **Default: explicit peering** — nodes are configured with static listen
  addresses and dial each other directly.
- **mDNS** is an optional, non-default cargo feature (`mdns`) for same-LAN
  development convenience only. In this spike the feature only enables the
  libp2p mDNS dependency; the mDNS behaviour wiring is a later discovery
  task. mDNS is not production discovery and is not required for tests.

## Authenticated hello (`spoke-connect-hello-jcs-v1`)

1. Both sides of a connection exchange a signed `ConnectHello`.
2. The signed object is exactly `{protocol_version, peer_id, nonce, host}`
   (top-level `extensions` and `signature` are excluded).
3. The object is canonicalized with **RFC 8785 JCS** — implemented by
   [`serde_jcs`](https://crates.io/crates/serde_jcs) (0.2.0): generic over
   serde `Serialize`, so the generated `spoke-schemas` manifest serializes
   directly with no `serde_json::Value` round-trip; small dependency tree
   (`ryu-js`, `serde`, `serde_json`); MIT/Apache-2.0; maintained.
4. The bytes are signed with the libp2p identity keypair (Ed25519); the raw
   signature is encoded **base64url, no padding**.
5. The receiver accepts only when: protocol version is 1, the claimed
   `peer_id` equals the noise-authenticated remote peer, the peer is on the
   configured allowlist (empty allowlist rejects all peers — fail-closed),
   the signature verifies against the peer's public key, and the
   `(peer_id, nonce)` pair is new (in-memory single-use set, process lifetime).
6. Rejection closes the hello stream — protocol v1 defines no hello error
   envelope.

`peer_id` on the wire is `PeerId::to_string()` — the libp2p PeerId base58btc
multihash form.

## Sessions and op invocation

`SpokeConnectNode::connect(addr)` dials an explicit address and returns a
`PeerSession` once **both** hellos of the connection are confirmed (the
remote acknowledged ours and we accepted theirs). Each session:

- owns a per-direction **outbound** `sequence` counter starting at **0**
  (each peer numbers its own requests; sequences never wrap — exhaustion
  closes the session with `InvokeError::SequenceExhausted`);
- assigns the next sequence **atomically** per `invoke` (concurrent invokes
  are allowed) and generates a UUID v4 `request_id`;
- requires the response to **echo** `session_id`, `sequence`, and
  `request_id` — any mismatch fails with `InvokeError::CorrelationMismatch`;
- returns `InvokeSuccess { sequence, request_id, payload }` on success;
  remote application failures arrive as `InvokeError::Wire(ErrorEnvelope)`;
  transport / session failures use the other `InvokeError` variants.

The accept path answers inbound invokes through the `invoke_handler`
configuration hook (`(op, payload) -> Result<payload, ErrorEnvelope>`).
This hook is spike-scoped — the op dispatcher is adapter-owned in products;
without a handler, inbound invokes receive an `op_unsupported` error envelope.
The wire imposes no payload size limit; size bounding and flow control are
transport/adapter-owned (see the spoke-connect spec §Hard boundaries).

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
| Op invocation | `/spoke/connect/invoke/1.0.0` |

## Usage

```rust
use libp2p::identity::Keypair;
use spoke_connect::{ConnectConfig, SpokeConnectNode, parse_multiaddr};
use spoke_schemas::connect::connect_hello::HostCapabilityManifest;

let identity = Keypair::generate_ed25519();
let peer_id = identity.public().to_peer_id();
let manifest = HostCapabilityManifest { /* host_id, roles, capabilities, schema_version, … */ };

let config = ConnectConfig {
    identity,
    peer_allowlist: vec![peer_id],
    listen_addrs: vec![parse_multiaddr("/ip4/127.0.0.1/tcp/0").expect("addr")],
    local_manifest: manifest,
    handshake_timeout: None,
    invoke_handler: None, // optional dispatcher hook for inbound invokes
};

let node = SpokeConnectNode::start(config).await.expect("start");
println!("listening on {:?} as {}", node.listen_addrs(), node.local_peer_id());

// Dial an explicit peer address; the session is ready once both hellos
// are confirmed.
let session = node
    .connect(node.listen_addrs()[0].clone())
    .await
    .expect("session");
let success = session
    .invoke("check", serde_json::json!({ "scope": { "scope_id": "s1" } }))
    .await
    .expect("invoke");
println!("sequence {} -> {:?}", success.sequence, success.payload);

node.shutdown().await.expect("shutdown");
```

`HostCapabilityManifest` here is the codegen-*inline* type
(`spoke_schemas::connect::connect_hello::HostCapabilityManifest`) — the exact
field type of `ConnectHello.host` and `ConnectConfig.local_manifest` (see
[Sessions and op invocation](#sessions-and-op-invocation)).

## Testing

```shell
cargo test -p spoke-connect          # unit + two-node integration tests (default features; mDNS off)
cargo test --workspace               # no regression across the workspace
cargo tree -p spoke-connect --depth 1  # dependency surface check
```

The two-node integration test (`tests/two_node_exchange.rs`) covers: hello
exchange with manifests round-tripping both ways, one `check` invoke with
`sequence == 0` and `request_id` echo, a second invoke with `sequence == 1`,
rejection of a non-allowlisted third node, and the
`InvokeError::Wire(ErrorEnvelope)` remote-failure path. All waits are bounded
event waits (timeouts on the `connect` / `invoke` futures) — no sleep-based
synchronization.

Local development note: if `~/.cargo/config.toml` sets `-Zno-embed-metadata`
(an unstable flag), prefix cargo commands with `RUSTFLAGS=""` because that
flag is incompatible with stable rustc:

```shell
RUSTFLAGS="" cargo test -p spoke-connect
```

CI does not need this workaround.

## Normative reference

- `.mstar/specs/spoke-connect.md` — envelope field tables, JCS rules, nonce /
  replay, ordering, auth model, discovery boundary.
- `schemas/connect/` — the JSON Schema SSOT for the six connect envelopes.
