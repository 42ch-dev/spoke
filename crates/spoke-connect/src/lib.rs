//! SPOKE Connect reference spike — authenticated cross-process connectivity
//! for the connect wire family.
//!
//! `spoke-connect` demonstrates the reference stack mapping of the connect
//! envelopes (`.mstar/specs/spoke-connect.md`) onto rust-libp2p: **noise**
//! authenticated transport + **yamux** multiplexing + **request-response**
//! for the signed hello exchange (and op invocation) + **identify** for peer
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

pub mod config;
pub mod error;
pub mod gate;
pub mod hello;
pub mod node;
pub mod protocol;

pub use config::{ConnectConfig, DEFAULT_HANDSHAKE_TIMEOUT};
pub use error::ConnectError;
pub use gate::{gate_hello, is_allowlisted, NonceStore};
pub use hello::{generate_nonce, sign_hello, verify_hello};
pub use node::{parse_multiaddr, SpokeConnectNode};
pub use protocol::{HelloAck, HELLO_PROTOCOL, HELLO_SIGNATURE_ALGORITHM, PROTOCOL_VERSION};
