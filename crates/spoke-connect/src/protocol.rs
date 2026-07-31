//! Connect protocol constants (protocol v1).
//!
//! Normative semantics: `.mstar/specs/spoke-connect.md` in the SPOKE repo.
//!
//! Crate-private: protocol names, version, and transport acknowledgement
//! shapes are internal composition facts (documented in the crate README),
//! not part of the locked public facade.

/// Connect protocol version exchanged in `ConnectHello` (not the data `schema_version`).
/// Protocol version **1** is current.
pub(crate) const PROTOCOL_VERSION: u64 = 1;

/// Request-response protocol name for the authenticated hello exchange.
pub(crate) const HELLO_PROTOCOL: &str = "/spoke/connect/hello/1.0.0";

/// Request-response protocol name for op invocation.
pub(crate) const INVOKE_PROTOCOL: &str = "/spoke/connect/invoke/1.0.0";

/// Maximum outbound invoke sequence per session: 2⁵³−1, the JSON-safe wire
/// maximum for `ConnectInvokeRequest.sequence`. Sessions never wrap — an
/// invoke past this value fails with `InvokeError::SequenceExhausted` and
/// closes the session (normative ordering rule, `.mstar/specs/spoke-connect.md`
/// §Ordering semantics).
pub(crate) const MAX_SEQUENCE: u64 = (1 << 53) - 1;

/// Transport-level acknowledgement for an accepted hello.
///
/// The hello content travels as the request; the response is a minimal ack.
/// A rejected hello is answered by closing the stream (the request-response
/// channel is dropped without a response) — there is no hello error envelope
/// in protocol v1. `HelloAck` is a rust-libp2p transport acknowledgement,
/// not a SPOKE wire envelope.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct HelloAck {
    /// Whether the peer's hello passed the allowlist / signature / nonce gates.
    pub(crate) accepted: bool,
}

impl HelloAck {
    /// Success acknowledgement.
    #[must_use]
    pub(crate) fn accepted() -> Self {
        Self { accepted: true }
    }
}
