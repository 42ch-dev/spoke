//! Connect protocol constants (protocol v1).
//!
//! Normative semantics: `.mstar/specs/spoke-connect.md` in the SPOKE repo.
//!
//! Crate-private: protocol names, version, and transport acknowledgement
//! shapes are internal composition facts (documented in the crate README),
//! not part of the locked public facade.
//!
//! The protocol version and the sequence ceiling are owned by the pure
//! session core (`crate::core`).

/// Maximum outbound invoke sequence per session: 2⁵³−1, the JSON-safe wire
/// maximum for `ConnectInvokeRequest.sequence` (core-owned rule).
pub(crate) use crate::core::MAX_SEQUENCE;

/// Request-response protocol name for the authenticated hello exchange.
pub(crate) const HELLO_PROTOCOL: &str = "/spoke/connect/hello/1.0.0";

/// Request-response protocol name for op invocation.
pub(crate) const INVOKE_PROTOCOL: &str = "/spoke/connect/invoke/1.0.0";

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
