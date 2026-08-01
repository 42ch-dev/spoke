//! Pure session-core error types.
//!
//! [`CoreError`] covers hello-gate / identity failures; [`CoreInvokeError`]
//! covers the invoke path (sequence, correlation). Both are pure — no
//! `libp2p::PeerId` or transport types — and map to [`crate::ConnectError`] /
//! [`crate::InvokeError`] at the transport boundary.

/// Errors from identity derivation, hello verification, and the accept gate.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CoreError {
    /// The hello signature did not verify against the peer's public key
    /// (or the signature is not valid base64url / not 64 bytes).
    #[error("hello signature invalid")]
    InvalidHelloSignature,

    /// The `(peer_id, nonce)` pair was already accepted.
    #[error("hello nonce replayed")]
    NonceReplay,

    /// Handshake-level failure (protocol version, peer id binding, …).
    #[error("handshake failed: {reason}")]
    HandshakeFailed { reason: String },

    /// The hello nonce does not satisfy the wire constraints (minLength 16).
    #[error("invalid hello nonce: {0}")]
    InvalidNonce(String),

    /// Cryptography-level failure (invalid key bytes, base64 decoding, …).
    #[error("crypto: {0}")]
    Crypto(String),

    /// RFC 8785 JCS canonicalization of the signed object failed.
    #[error("JCS canonicalization failed: {0}")]
    Jcs(String),
}

/// Errors from op invocation over an established session (pure core rules).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CoreInvokeError {
    /// The session's outbound sequence space (2⁵³−1) is exhausted; the
    /// session must be closed and reopened — sequences never wrap.
    #[error("sequence space exhausted — reopen session")]
    SequenceExhausted,

    /// An inbound invoke `sequence` is not the next expected one (replay or
    /// out-of-order); the invoke must not be dispatched.
    #[error("inbound sequence {actual} is not the next expected {expected}")]
    InboundSequenceMismatch { expected: u64, actual: i64 },

    /// A response did not echo the request's `session_id` / `sequence` /
    /// `request_id`.
    #[error("request/response mismatch")]
    CorrelationMismatch,
}
