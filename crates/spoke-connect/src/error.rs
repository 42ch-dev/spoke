//! Connect error types.
//!
//! Local / transport / configuration failures surface as [`ConnectError`].
//! Remote **application** failures (op invocation) surface as
//! [`InvokeError::Wire`] carrying the wire `ErrorEnvelope` — they are never
//! encoded in [`ConnectError`].
//!
//! The pure session core reports [`crate::core::CoreError`] /
//! [`crate::core::CoreInvokeError`]; the transport maps them here at the
//! boundary ([`map_core_error`] / [`map_core_invoke_error`]).

use crate::core::{CoreError, CoreInvokeError};
use libp2p::PeerId;
use spoke_schemas::connect::connect_invoke_response::ErrorEnvelope;

/// Map a pure core error to the transport-facing [`ConnectError`].
///
/// Identity variants keep their meaning; `InvalidNonce` (a signing-time
/// caller error) maps to `Config`, and `Crypto` / `Jcs` (cryptographic or
/// canonicalization failures on the wire path) map to `Transport`.
/// `TokenInvalid` maps to `AuthFailed` — on the wire it becomes an
/// `auth_failed` error envelope (see `crate::node`).
pub(crate) fn map_core_error(err: CoreError) -> ConnectError {
    match err {
        CoreError::InvalidHelloSignature => ConnectError::InvalidHelloSignature,
        CoreError::NonceReplay => ConnectError::NonceReplay,
        CoreError::HandshakeFailed { reason } => ConnectError::HandshakeFailed { reason },
        CoreError::InvalidNonce(reason) => ConnectError::Config(reason),
        CoreError::Crypto(reason) => ConnectError::Transport(reason),
        CoreError::Jcs(reason) => ConnectError::Transport(reason),
        CoreError::TokenInvalid(reason) => ConnectError::AuthFailed(reason),
    }
}

/// Map a pure core invoke error to the transport-facing [`InvokeError`].
///
/// `SequenceExhausted` and `CorrelationMismatch` keep their identity.
/// `InboundSequenceMismatch` never surfaces on the outbound invoke path —
/// the inbound path maps it to a wire `invalid_sequence` envelope instead
/// (see `crate::node`).
pub(crate) fn map_core_invoke_error(err: CoreInvokeError) -> InvokeError {
    match err {
        CoreInvokeError::SequenceExhausted => InvokeError::SequenceExhausted,
        CoreInvokeError::InboundSequenceMismatch { .. } => {
            InvokeError::Transport("inbound sequence mismatch".into())
        }
        CoreInvokeError::CorrelationMismatch => InvokeError::CorrelationMismatch,
    }
}

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

    /// A capability-token proof failed validation. On the wire this becomes
    /// an `auth_failed` error envelope (open vocabulary string — no schema
    /// change); the session remains unauthorized for invokes.
    #[error("capability token invalid: {0}")]
    AuthFailed(String),

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
