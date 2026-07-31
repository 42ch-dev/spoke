# spoke-connect

Reference spike for the SPOKE Connect wire family (`.mstar/specs/spoke-connect.md`):
an embeddable Rust library that maps the connect envelopes onto rust-libp2p and
demonstrates the `noise-peerid` authenticated hello handshake.

## Status

- **Reference spike** — a transport demonstration, not a published SDK.
- **Workspace-private** (`publish = false`); library only, no daemon or binary target.
- Consumes wire types exclusively from `spoke-schemas` generated modules
  (`ConnectHello`, `HostCapabilityManifest`, …) — no parallel hand-written envelopes.

## Transport composition

| Layer | Choice |
|-------|--------|
| Security | `noise` (authenticated encryption; remote `PeerId` is noise-authenticated) |
| Multiplexing | `yamux` |
| Messaging | `request-response` — hello exchange now, op invocation next |
| Peer info | `identify` (carries the remote public key used to verify hello signatures) |

libp2p is pinned to a single version (`=0.56.0`) with a minimal feature set:
`noise`, `yamux`, `tcp`, `tokio`, `identify`, `request-response`, `json`,
`macros`, `ed25519`. QUIC, relay, DHT, and gossipsub are not enabled.

## Discovery

- **Default: explicit peering** — nodes are configured with static listen
  addresses and dial each other directly.
- **mDNS** is an optional, non-default cargo feature (`mdns`) for same-LAN
  development convenience only. It is not production discovery and is not
  required for tests.

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

## Protocol names

| Protocol | Name |
|----------|------|
| Hello exchange | `/spoke/connect/hello/1.0.0` |
| Op invocation | `/spoke/connect/invoke/1.0.0` |

## Usage

```rust
use libp2p::identity::Keypair;
use spoke_connect::{ConnectConfig, SpokeConnectNode, parse_multiaddr};
use spoke_schemas::data::HostCapabilityManifest;

let identity = Keypair::generate_ed25519();
let peer_id = identity.public().to_peer_id();
let manifest = HostCapabilityManifest { /* host_id, roles, capabilities, schema_version, … */ };

let config = ConnectConfig {
    identity,
    peer_allowlist: vec![peer_id],
    listen_addrs: vec![parse_multiaddr("/ip4/127.0.0.1/tcp/0").expect("addr")],
    local_manifest: manifest,
    handshake_timeout: None,
};

let node = SpokeConnectNode::start(config).await.expect("start");
println!("listening on {:?} as {}", node.listen_addrs(), node.local_peer_id());
node.shutdown().await.expect("shutdown");
```

## Testing

```shell
cargo test -p spoke-connect          # unit tests (default features; mDNS off)
cargo test --workspace               # no regression across the workspace
cargo tree -p spoke-connect --depth 1  # dependency surface check
```

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
