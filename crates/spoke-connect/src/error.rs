//! Connect error types.
//!
//! Local / transport / configuration failures surface as [`ConnectError`].
//! Remote **application** failures (op invocation) surface as
//! `InvokeError::Wire(ErrorEnvelope)` in Task 2 — they are never encoded in
//! [`ConnectError`].

use libp2p::PeerId;

/// Errors from node lifecycle, transport, and the authenticated hello handshake.
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    /// The remote peer's noise-authenticated `PeerId` is not on the allowlist.
    #[error("peer not in allowlist: {peer_id}")]
    NotAllowlisted { peer_id: PeerId },

    /// The hello signature did not verify against the peer's public key.
    #[error("hello signature invalid")]
    InvalidHelloSignature,

    /// The `(peer_id, nonce)` pair was already accepted in this process.
    #[error("hello nonce replayed")]
    NonceReplay,

    /// Handshake-level failure (protocol version, claimed peer id mismatch, …).
    #[error("handshake failed: {reason}")]
    HandshakeFailed { reason: String },

    /// Transport-level failure (dial, listen, shutdown, I/O).
    #[error("transport: {0}")]
    Transport(String),

    /// Invalid [`crate::ConnectConfig`].
    #[error("invalid config: {0}")]
    Config(String),

    /// An asynchronous operation did not complete within its deadline.
    #[error("timed out waiting for {0}")]
    Timeout(String),
}
