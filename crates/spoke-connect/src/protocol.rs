//! Connect protocol constants (protocol v1).
//!
//! Normative semantics: `.mstar/specs/spoke-connect.md` in the SPOKE repo.

/// Connect protocol version exchanged in `ConnectHello` (not the data `schema_version`).
/// Protocol version **1** is current.
pub const PROTOCOL_VERSION: u64 = 1;

/// Signature algorithm name (spec §Signature canonicalization):
/// RFC 8785 JCS over the signed object, Ed25519 via the libp2p identity keypair,
/// raw signature bytes encoded base64url (no padding).
pub const HELLO_SIGNATURE_ALGORITHM: &str = "spoke-connect-hello-jcs-v1";

/// Request-response protocol name for the authenticated hello exchange.
pub const HELLO_PROTOCOL: &str = "/spoke/connect/hello/1.0.0";

/// Request-response protocol name for op invocation (Task 2 wires it).
pub const INVOKE_PROTOCOL: &str = "/spoke/connect/invoke/1.0.0";

/// Transport-level acknowledgement for an accepted hello.
///
/// The hello content travels as the request; the response is a minimal ack.
/// A rejected hello is answered by closing the stream (the request-response
/// channel is dropped without a response) — there is no hello error envelope
/// in protocol v1.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HelloAck {
    /// Whether the peer's hello passed the allowlist / signature / nonce gates.
    pub accepted: bool,
}

impl HelloAck {
    /// Success acknowledgement.
    #[must_use]
    pub fn accepted() -> Self {
        Self { accepted: true }
    }
}
