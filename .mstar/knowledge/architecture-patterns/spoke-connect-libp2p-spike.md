---
module: spoke-connect
date: 2026-07-31
problem_type: architecture_pattern
category: architecture-patterns
severity: high
applies_when: ["building a libp2p-based spoke-connect runtime", "binding spoke-connect to a foreign language via uniffi", "hardening a p2p handshake/allowlist implementation"]
tags: [spoke-connect, libp2p, rust, noise, identify, allowlist, pending-dial, session, uniffi-facade]
---

# spoke-connect Rust libp2p spike: transport, auth binding, and event-loop pitfalls

## Context

The `crates/spoke-connect` reference spike (workspace-private, `publish = false`) implements the `spoke-connect` wire family over rust-libp2p to prove two independent client processes can discover/connect, authenticate, exchange `HostCapabilityManifest`s, and invoke each other's ops in order. rust-libp2p was a first-time dependency for this repo; the spike surfaced several non-obvious integration patterns and two latent reliability root causes during QC.

## Guidance

### Transport composition (locked minimal feature set)

Pin a single rust-libp2p version (e.g. `=0.56.0`); enable only `noise`, `yamux`, `request-response`, `identify`, `macros`, `tokio`, `ed25519` (+ `tcp`, `json` as required by the composition). Avoid QUIC, relay, kad, gossipsub, tls, mdns unless a real behaviour is wired - a capability-named feature flag with no runtime behaviour must not ship (a no-op `mdns` feature was removed in QC).

### Identify key <-> noise PeerId binding (defense in depth)

libp2p `identify` supplies the remote's advertised public key. Do **not** trust it blindly: on `Identify::Received`, drop the payload if `info.public_key.to_peer_id() != peer_id` (the noise-authenticated id). Additionally assert in the hello verify path that the signing `public_key.to_peer_id() == expected_peer_id`. This closes a spoof path where a peer authenticated as allowlisted `A` over Noise advertises a different signing key `B` and signs the hello (claiming `peer_id=A`) with `B`.

### Allowlist at ConnectionEstablished (fail-closed before disclosure)

The noise `PeerId` is known the moment a connection establishes - apply the allowlist **there**: a non-allowlisted peer receives **no** signed hello (no manifest disclosure), is **not** buffered, and is disconnected. Do not defer the allowlist to `gate_hello` after `identify`/pending buffering, or non-allowlisted peers can fill pending buffers and receive signed manifests. Cap pending hellos per peer (e.g. 8) as a DoS backstop even for allowlisted peers waiting on identify.

### Pending-dial binding (avoid wrong-peer completion)

Bind an outbound `PendingConnect` to its own `ConnectionId` + expected `PeerId` (gate `ConnectionEstablished` by `endpoint.is_dialer()` and matching peer). A responder-side session created from an **unrelated** inbound handshake MUST NOT consume the outbound pending reply - otherwise `connect(B)` can return peer C's session. The event loop owns the dial deadline (deterministic sweep) so timeout/cancellation clears the entry; a caller-side timeout that only drops the oneshot receiver leaves a stuck entry that blocks all later connects. Duplicate-dial to an already-sessioned peer should complete with the existing session handle (and close the surplus connection) rather than hang to timeout.

### Session-ordered invoke correlation

Per-session atomic outbound `sequence` from 0 (`AtomicU64::fetch_add`); no wrap (exhaustion closes session or returns `SequenceExhausted`). Responses MUST echo `session_id`/`sequence`/`request_id`; mismatch -> `InvokeError::CorrelationMismatch`. Remote application failures arrive as `InvokeError::Wire(ErrorEnvelope)` (the codegen-inline wire type - zero conversion, no parallel error DTO); transport/session failures use the other `InvokeError` variants. Inbound `sequence < 0 || > MAX_SEQUENCE` is rejected with a wire `invalid_sequence` envelope before the handler runs.

### Event-loop + handler contract (uniffi-ready facade)

Keep the public API surface minimal and `Send + Sync`: `ConnectConfig`, `SpokeConnectNode` (start/shutdown/`local_manifest()`/accessors), `PeerSession`, `invoke -> Result<InvokeSuccess, InvokeError>`. Make implementation modules private; re-export only the locked facade (plus the ratified `invoke_handler` config hook). Break node<->session cycles by moving `LoopCommand`/`SessionHandle` into a private `runtime` module.

The `invoke_handler` hook runs **synchronously on the swarm event loop** - a blocking/panicking handler stalls or kills the node. Wrap the call in `std::panic::catch_unwind` (panic -> wire `internal_error` envelope, node survives) and propagate the event-loop task's `JoinError` through `shutdown()` (do not swallow it). Document the "handler must not block/panic" contract and mark the off-loop (`spawn_blocking`) upgrade path with `simplify:` for the uniffi iteration.

### Wire types: reuse, never redefine

All connect/host/error types come from `spoke-schemas` generated modules. Codegen inlines `$ref`s, so `ConnectHello.host` is a file-local `connect_hello::HostCapabilityManifest` (field-identical to but distinct from `data::HostCapabilityManifest`) - use exactly that inline type in `ConnectConfig.local_manifest` / `InvokeError::Wire` with zero conversion. Never hand-write parallel wire structs.

## Why This Matters

These patterns are the difference between a spike that "works on the happy path" and one that is safe to bind into a multi-language SDK. The identify-key binding and early allowlist are auth-correctness invariants; the pending-dial binding prevents misrouting sensitive application data to the wrong authenticated peer; the handler contract keeps a buggy adapter hook from silently killing the node. Capturing them here means the next iteration's uniffi bindings and TS connectivity story start from a hardened, documented core instead of rediscovering the same pitfalls.

## When to Apply

- Building any libp2p-based spoke-connect runtime (Rust core or a foreign-language SDK consuming it) - the transport composition, key binding, allowlist timing, and pending-dial binding are normative for the reference implementation.
- Hardening a p2p handshake/allowlist - the fail-closed-before-disclosure and per-peer pending-cap patterns generalize beyond SPOKE.
- Preparing a Rust crate for uniffi binding - the minimal `Send + Sync` facade, private internals, `catch_unwind` handler containment, and `shutdown()` error propagation are the preconditions for a stable FFI boundary.

## Examples

### Two latent root causes found in QC (reliability)

- **Stale dial-failure串扰**: a `ConnectionEstablished`/`OutboundFailure` event from a **previous** failed dial could fail the **next** pending dial (~50% repro). Fix: strict connection-scoped failure matching - only attribute a failure to a pending dial whose `ConnectionId`/peer matches.
- **macOS SO_REUSEPORT loopback collision**: repeat dials to `127.0.0.1/tcp/0` can collide on reused ports. Fix: `allocate_new_port()` fast path using the recorded listen address; serialize network tests on a documented shared lock for determinism.

### Facade privatization (excerpt)

```rust
// lib.rs - only the locked surface is public
pub use config::{ConnectConfig, InvokeHandler};
pub use node::SpokeConnectNode;
pub use session::PeerSession;
pub use error::{ConnectError, InvokeError, InvokeSuccess};
// runtime.rs (LoopCommand, SessionHandle, InvokeCorrelation) is crate-private
mod runtime;
mod node; mod session; mod gate; mod hello; mod protocol; mod config; mod error;
```
