# spoke-connect

Reference spike for the SPOKE Connect wire family (`.mstar/specs/spoke-connect.md`):
an embeddable Rust library that maps the connect envelopes onto rust-libp2p and
demonstrates the `noise-peerid` authenticated hello handshake, per-session
ordering, and op invocation.

## Status

- **Reference spike** — a workspace-private library crate (`publish = false`)
  with a library-only target; consumers integrate it as a Cargo path
  dependency.
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
`macros`, `ed25519`.

## Discovery

**Explicit peering** is the discovery mechanism: nodes are configured with
static listen addresses and dial each other directly. LAN discovery (mDNS) is
planned for a future discovery iteration.

## Authenticated hello (`spoke-connect-hello-jcs-v1`)

1. Both sides of a connection exchange a signed `ConnectHello`.
2. The signed object is exactly `{protocol_version, peer_id, nonce, host}`
   (top-level `extensions` and `signature` are excluded).
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
6. Rejection closes the hello stream; protocol v1 acknowledges accepted
   hellos with an ack response.

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

Session ids are **per-side** in protocol v1: each node records its own opaque
id for the pairing (there is no session-announce message), and the response
echo correlates by peer.

The accept path answers inbound invokes through the `invoke_handler`
configuration hook (`(op, payload) -> Result<payload, ErrorEnvelope>`). The
hook runs **synchronously on the node's network event loop**: it must return
promptly and must not block on I/O. Panics are contained — the invoke is
answered with an `internal_error` wire envelope and the node keeps running.
The hook is spike-scoped — the op dispatcher is adapter-owned in products;
without a handler, inbound invokes receive an `op_unsupported` error
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
the `connect` / `invoke` futures) — no sleep-based synchronization.

## Normative reference

- `.mstar/specs/spoke-connect.md` — envelope field tables, JCS rules, nonce /
  replay, ordering, auth model, discovery boundary.
- `schemas/connect/` — the JSON Schema SSOT for the six connect envelopes.
