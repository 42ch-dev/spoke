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
//! first-binding surface for foreign-language bindings; the sync vs async
//! facade decision and the landed Swift skeleton are recorded in the crate
//! README ("Binding facade").

/// Connect protocol version exchanged in `ConnectHello` (not the data
/// `schema_version`). Protocol version **1** is current.
pub const PROTOCOL_VERSION: u64 = 1;

mod allowlist;
mod capability_token;
mod correlate;
mod dispatch;
mod error;
#[cfg(test)]
pub(crate) mod golden;
mod hello_crypto;
mod nonce;
mod peer_id;
mod sequence;

pub use allowlist::is_allowlisted;
pub use capability_token::{
    issue_capability_token, verify_capability_token, CapabilityClaims, CapabilityTokenProof,
    CLOCK_SKEW_SECONDS, TOKEN_VERSION,
};
pub use correlate::{check_response_correlation, Correlation};
pub use dispatch::{
    dispatch_allowed, required_capability, token_authorizes_op, CAPABILITY_L2_COMPUTABLE,
    CAPABILITY_SPOKE_BASELINE,
};
pub use error::{CoreError, CoreInvokeError};
pub use hello_crypto::{sign_hello_ed25519, verify_hello_ed25519};
pub use nonce::NonceStore;
pub use peer_id::{derive_peer_id_from_ed25519_pubkey, ed25519_pubkey_from_peer_id};
pub use sequence::{InboundSequence, OutboundSequence, MAX_SEQUENCE};
