//! SPOKE Connect reference library — authenticated cross-process connectivity
//! for the connect wire family.
//!
//! `spoke-connect` demonstrates the reference stack mapping of the connect
//! envelopes (`.mstar/specs/spoke-connect.md`) onto rust-libp2p: **noise**
//! authenticated transport + **yamux** multiplexing + **request-response**
//! for the signed hello exchange and op invocation + **identify** for peer
//! metadata. Discovery is **explicit peering**: nodes are configured with
//! static listen addresses and dial each other directly.
//!
//! The crate is published on crates.io as `spoke-connect`; embedders depend on
//! it via `cargo add spoke-connect`.
//! All wire types come from `spoke-schemas` generated modules — no parallel
//! hand-written envelopes.
//!
//! # Hello handshake
//!
//! Both sides of a connection exchange a signed
//! [`spoke_schemas::connect::ConnectHello`] (`spoke-connect-hello-jcs-v1`:
//! RFC 8785 JCS over `{protocol_version, peer_id, nonce, host}`, Ed25519 via
//! the libp2p identity keypair, base64url signature). A hello is accepted
//! only when the noise-authenticated peer is allowlisted, the identify
//! public key derives that peer id, the signature verifies, and each
//! `(peer_id, nonce)` pair is single-use. Protocol v1 acknowledges accepted
//! hellos with an ack response; a rejected hello is answered by closing the
//! stream.
//!
//! # Sessions and op invocation
//!
//! [`SpokeConnectNode::connect`] dials an explicit address and returns a
//! [`PeerSession`] once both hellos of the connection are confirmed. Each
//! session owns a per-direction **outbound** sequence counter starting at 0
//! (never wraps — exhaustion closes the session). [`PeerSession::invoke`]
//! sends a `ConnectInvokeRequest` over `/spoke/connect/invoke/1.0.0`,
//! assigns the next sequence atomically (concurrent invokes are allowed),
//! generates a UUID v4 `request_id`, and waits for the correlated
//! `ConnectInvokeResponse`. The response MUST echo `session_id`, `sequence`,
//! and `request_id`; any mismatch fails with `InvokeError::CorrelationMismatch`.
//! Remote application failures arrive as `InvokeError::Wire(ErrorEnvelope)`;
//! transport / session failures use the other `InvokeError` variants.
//!
//! The accept path answers inbound invokes through the configured dispatch
//! hook: [`ConnectConfig::invoke_handler_v2`] when set (it additionally
//! receives the noise-authenticated session peer id), else the legacy
//! [`ConnectConfig::invoke_handler`]. The hook runs **synchronously on
//! the node's network event loop**: it must return promptly and must not
//! block on I/O. Panics are contained — the invoke is answered with an
//! `internal_error` wire envelope and the node keeps running.
//! (simplify: dispatch off-loop, e.g. `spawn_blocking`, when handler latency
//! matters.) The hook is spike-scoped; the dispatcher is adapter-owned in
//! products.
//!
//! # Generated wire types
//!
//! Codegen inlines `$ref` types, so `ConnectHello.host` is the file-local
//! `spoke_schemas::connect::connect_hello::HostCapabilityManifest`
//! (field-identical to `data::HostCapabilityManifest` but a distinct
//! generated type) and the `ConnectInvokeResponse` error branch carries the
//! inline `spoke_schemas::connect::connect_invoke_response::ErrorEnvelope`.
//! `ConnectConfig.local_manifest`, `PeerSession::remote_manifest`, and
//! `InvokeError::Wire` use exactly those wire types — zero conversion.
//!
//! # Public surface
//!
//! The locked facade is [`ConnectConfig`], [`ConnectError`], [`InvokeError`],
//! [`SpokeConnectNode`], [`PeerSession`], and [`InvokeSuccess`], plus
//! [`parse_multiaddr`] (test/example convenience) and the spike dispatch
//! hooks [`ConnectConfig::invoke_handler`] /
//! [`ConnectConfig::invoke_handler_v2`]. Transport internals — hello and
//! gate modules, protocol constants, the transport `HelloAck`, and session
//! plumbing — are crate-private.
//!
//! # Capability-token auth
//!
//! The `capability-token` method (normative, `.mstar/specs/spoke-connect.md`
//! §Method — capability-token) is a step-up / delegated capability grant on
//! top of the `noise-peerid` hello identity: a trusted issuer signs a short
//! claim set (`iss`/`sub`/`aud`/`capabilities`/`exp`, optional `iat`/`jti`)
//! over RFC 8785 JCS with Ed25519; the proof rides the
//! `ConnectAuthChallenge`/`ConnectAuthResponse` exchange and optionally the
//! `ConnectInvokeRequest.auth` blob. Configure [`ConnectConfig`] with
//! `trusted_issuers` (empty ⇒ method disabled) and
//! `require_capability_token` (challenge every session); a node answers
//! challenges through [`ConnectConfig::capability_token_provider`].
//! [`PeerSession::invoke_with_auth`] attaches a per-invoke proof;
//! [`PeerSession::capability_token_ok`] reports the challenge state.
//!
//! # Pure session core
//!
//! [`core`] holds the pure, language-portable session rules (peer id
//! derivation, hello sign/verify over raw Ed25519 keys, nonce store,
//! allowlist, sequence counters, correlation, dispatch gate, capability
//! token issue/verify). It has no libp2p or tokio dependencies; the
//! transport converts `libp2p::PeerId` ↔ `String` at the boundary and
//! delegates to it.

pub mod core;

#[cfg(feature = "remote-adapter")]
pub mod remote;

#[cfg(any(test, feature = "ffi-smoke-host"))]
pub mod test_support;

#[cfg(feature = "ffi")]
pub mod ffi;

// uniffi scaffolding for the `ffi` feature: registers the crate's metadata
// and buffer support so `uniffi-bindgen generate --library` can read the
// exported surface from the cdylib. Compiled only when `--features ffi`.
#[cfg(feature = "ffi")]
uniffi::setup_scaffolding!();

mod config;
mod error;
mod gate;
mod hello;
mod node;
mod protocol;
mod runtime;
mod session;

pub use config::{
    CapabilityTokenProvider, ConnectConfig, InvokeHandler, InvokeHandlerV2,
    DEFAULT_HANDSHAKE_TIMEOUT,
};
pub use error::{ConnectError, InvokeError};
pub use node::{parse_multiaddr, SpokeConnectNode};
pub use runtime::InvokeSuccess;
pub use session::PeerSession;
