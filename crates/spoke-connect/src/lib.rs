//! SPOKE Connect reference spike — authenticated cross-process connectivity
//! for the connect wire family.
//!
//! `spoke-connect` demonstrates the reference stack mapping of the connect
//! envelopes (`.mstar/specs/spoke-connect.md`) onto rust-libp2p: **noise**
//! authenticated transport + **yamux** multiplexing + **request-response**
//! for the signed hello exchange and op invocation + **identify** for peer
//! metadata. Discovery defaults to **explicit peering**; mDNS is a
//! non-default cargo feature (`mdns`) for same-LAN development only.
//!
//! The crate is a workspace-private spike (`publish = false`): a library
//! that embedders bind against, not a daemon and not a published SDK. All
//! wire types come from `spoke-schemas` generated modules — no parallel
//! hand-written envelopes.
//!
//! # Hello handshake
//!
//! Both sides of a connection exchange a signed [`ConnectHello`]
//! (`spoke-connect-hello-jcs-v1`: RFC 8785 JCS over
//! `{protocol_version, peer_id, nonce, host}`, Ed25519 via the libp2p
//! identity keypair, base64url signature). A hello is accepted only when the
//! noise-authenticated peer is allowlisted and the signature verifies, and
//! each `(peer_id, nonce)` pair is single-use. Rejection closes the stream —
//! protocol v1 has no hello error envelope.
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
//! The accept path answers inbound invokes through the configured
//! [`ConnectConfig::invoke_handler`] hook (spike-scoped; the dispatcher is
//! adapter-owned in products).
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

pub mod config;
pub mod error;
pub mod gate;
pub mod hello;
pub mod node;
pub mod protocol;
pub mod session;

pub use config::{ConnectConfig, DEFAULT_HANDSHAKE_TIMEOUT};
pub use error::{ConnectError, InvokeError};
pub use gate::{gate_hello, is_allowlisted, NonceStore};
pub use hello::{generate_nonce, sign_hello, verify_hello};
pub use node::{parse_multiaddr, SpokeConnectNode};
pub use protocol::{
    HelloAck, HELLO_PROTOCOL, HELLO_SIGNATURE_ALGORITHM, INVOKE_PROTOCOL, MAX_SEQUENCE,
    PROTOCOL_VERSION,
};
pub use session::{InvokeSuccess, PeerSession};
