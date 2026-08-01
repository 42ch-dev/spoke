//! Pure session core — the language-portable session rules of the connect
//! wire family (`.mstar/specs/spoke-connect.md` §Session-core state machine).
//!
//! This module has **no** transport or runtime dependencies: no `libp2p`,
//! `tokio`, `Multiaddr`, swarm, `request-response` protocol types, or any
//! I/O — it operates on `spoke-schemas` connect types, plain `String` peer
//! ids, and `serde_json` opaque payloads only. The transport layer converts
//! `libp2p::PeerId` ↔ `String` at the boundary and calls into this module.
//!
//! Everything here is synchronous and pure, which is what makes it the
//! first-binding surface for future foreign-language bindings; the sync vs
//! async facade decision is recorded in the crate README ("Binding facade
//! (no uniffi yet)").

/// Connect protocol version exchanged in `ConnectHello` (not the data
/// `schema_version`). Protocol version **1** is current.
pub const PROTOCOL_VERSION: u64 = 1;

mod allowlist;
mod correlate;
mod dispatch;
mod error;
mod hello_crypto;
mod nonce;
mod peer_id;
mod sequence;

pub use allowlist::is_allowlisted;
pub use correlate::{check_response_correlation, Correlation};
pub use dispatch::{
    dispatch_allowed, required_capability, CAPABILITY_L2_COMPUTABLE, CAPABILITY_SPOKE_BASELINE,
};
pub use error::{CoreError, CoreInvokeError};
pub use hello_crypto::{sign_hello_ed25519, verify_hello_ed25519};
pub use nonce::NonceStore;
pub use peer_id::derive_peer_id_from_ed25519_pubkey;
pub use sequence::{InboundSequence, OutboundSequence, MAX_SEQUENCE};
