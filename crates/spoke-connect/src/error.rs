//! Connect error types.
//!
//! Local / transport / configuration failures surface as [`ConnectError`].
//! Remote **application** failures (op invocation) surface as
//! [`InvokeError::Wire`] carrying the wire `ErrorEnvelope` — they are never
//! encoded in [`ConnectError`].

use libp2p::PeerId;
use spoke_schemas::connect::connect_invoke_response::ErrorEnvelope;

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

/// Errors from op invocation over a [`crate::PeerSession`].
///
/// Remote application failures surface as [`InvokeError::Wire`] carrying the
/// wire `ErrorEnvelope` (the codegen-inline
/// `spoke_schemas::connect::connect_invoke_response::ErrorEnvelope` — the
/// exact type inside the `ConnectInvokeResponse` error branch). Local /
/// transport / session failures use the other variants; they are never
/// encoded as wire envelopes.
#[derive(Debug, thiserror::Error)]
pub enum InvokeError {
    /// Remote application / op failure — the wire `ErrorEnvelope` echoed in
    /// the `ConnectInvokeResponse` error branch. No parallel error DTO.
    #[error("remote error: {0:?}")]
    Wire(ErrorEnvelope),

    /// Transport-level failure (timeout, dial, I/O on the invoke stream).
    #[error("transport: {0}")]
    Transport(String),

    /// The session is no longer usable (node stopped, connection closed).
    #[error("session closed")]
    SessionClosed,

    /// The session's outbound sequence space (2⁵³−1) is exhausted; the
    /// session has been closed and must be reopened.
    #[error("sequence space exhausted — reopen session")]
    SequenceExhausted,

    /// The response did not echo the request's `session_id` / `sequence` /
    /// `request_id`.
    #[error("request/response mismatch")]
    CorrelationMismatch,
}
