//! Authenticated hello: signing and verification per `spoke-connect-hello-jcs-v1`.
//!
//! Normative rules (`.mstar/specs/spoke-connect.md`):
//! - Signed object = exactly `{protocol_version, peer_id, nonce, host}`.
//! - Canonicalize with RFC 8785 JCS (via `serde_jcs`), sign with the libp2p
//!   identity keypair (Ed25519), encode raw signature bytes base64url (no padding).
//! - Verification uses the public key bound to the **noise-authenticated**
//!   remote peer (libp2p identify carries it); the claimed `peer_id` in the
//!   hello must equal that authenticated peer id.
//! - Top-level hello `extensions` and the `signature` field are not part of
//!   the signed object.

use crate::error::ConnectError;
use crate::protocol::PROTOCOL_VERSION;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use libp2p::identity::{Keypair, PublicKey};
use libp2p::PeerId;
use spoke_schemas::connect::connect_hello::HostCapabilityManifest;
use spoke_schemas::connect::ConnectHello;

/// The exact signed object for `spoke-connect-hello-jcs-v1`.
///
/// Not a wire envelope — an internal canonicalization helper. Serializing it
/// with JCS produces the byte sequence covered by the hello signature.
#[derive(Debug, serde::Serialize)]
struct SignedHello<'a> {
    protocol_version: u64,
    peer_id: &'a str,
    nonce: &'a str,
    host: &'a HostCapabilityManifest,
}

/// Generate a single-use hello nonce: 128 bits of CSPRNG entropy, base64url
/// encoded (22 characters, above the wire floor of 16).
pub(crate) fn generate_nonce() -> Result<String, ConnectError> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|e| ConnectError::Transport(format!("CSPRNG failure: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn protocol_version() -> std::num::NonZeroU64 {
    std::num::NonZeroU64::new(PROTOCOL_VERSION).expect("PROTOCOL_VERSION is a non-zero constant")
}

/// Sign a fresh hello for `identity` carrying `manifest` under `nonce`.
///
/// `peer_id` on the wire is `identity.public().to_peer_id().to_string()`
/// (libp2p PeerId base58btc multihash form).
pub(crate) fn sign_hello(
    identity: &Keypair,
    nonce: &str,
    manifest: &HostCapabilityManifest,
) -> Result<ConnectHello, ConnectError> {
    let peer_id = identity.public().to_peer_id();
    let signed = SignedHello {
        protocol_version: PROTOCOL_VERSION,
        peer_id: &peer_id.to_string(),
        nonce,
        host: manifest,
    };
    let bytes = serde_jcs::to_vec(&signed)
        .map_err(|e| ConnectError::Transport(format!("JCS canonicalization failed: {e}")))?;
    let raw_signature = identity
        .sign(&bytes)
        .map_err(|e| ConnectError::Transport(format!("hello signing failed: {e}")))?;

    Ok(ConnectHello {
        protocol_version: protocol_version(),
        peer_id: peer_id.to_string().parse().expect("peer id string parses"),
        nonce: nonce.parse().map_err(|e| {
            ConnectError::Config(format!(
                "nonce {nonce:?} does not meet wire constraints: {e}"
            ))
        })?,
        host: manifest.clone(),
        signature: URL_SAFE_NO_PAD
            .encode(raw_signature)
            .parse()
            .expect("signature parses"),
        extensions: Default::default(),
    })
}

/// Verify a received hello.
///
/// Checks, in order:
/// 1. `protocol_version` equals [`PROTOCOL_VERSION`] (`HandshakeFailed`).
/// 2. `public_key` derives `expected_peer_id` — the noise-authenticated
///    remote peer. A key that does not derive the authenticated peer id
///    cannot attest that peer's identity, so the hello is rejected
///    (`HandshakeFailed`).
/// 3. The claimed `peer_id` parses and equals `expected_peer_id`
///    (`HandshakeFailed`).
/// 4. The signature verifies against `public_key` (`InvalidHelloSignature`).
///
/// Allowlist and nonce gates are applied by [`crate::gate`] around this check.
pub(crate) fn verify_hello(
    public_key: &PublicKey,
    expected_peer_id: &PeerId,
    hello: &ConnectHello,
) -> Result<(), ConnectError> {
    if hello.protocol_version.get() != PROTOCOL_VERSION {
        return Err(ConnectError::HandshakeFailed {
            reason: format!(
                "unsupported protocol_version {} (expected {PROTOCOL_VERSION})",
                hello.protocol_version
            ),
        });
    }

    // The verify key must be *bound* to the noise-authenticated peer id: the
    // hello signature attests that peer's identity key, so a key deriving a
    // different peer id must not be usable here.
    if public_key.to_peer_id() != *expected_peer_id {
        return Err(ConnectError::HandshakeFailed {
            reason: format!(
                "identify public key derives peer id {} instead of the noise-authenticated peer {expected_peer_id}",
                public_key.to_peer_id()
            ),
        });
    }

    let claimed_peer_id: PeerId =
        hello
            .peer_id
            .parse()
            .map_err(|_| ConnectError::HandshakeFailed {
                reason: format!("hello peer_id is not a valid PeerId: {:?}", hello.peer_id),
            })?;
    if &claimed_peer_id != expected_peer_id {
        return Err(ConnectError::HandshakeFailed {
            reason: format!(
                "hello peer_id {claimed_peer_id} does not match noise-authenticated peer {expected_peer_id}"
            ),
        });
    }

    let signed = SignedHello {
        protocol_version: hello.protocol_version.get(),
        peer_id: hello.peer_id.as_str(),
        nonce: hello.nonce.as_str(),
        host: &hello.host,
    };
    let bytes = serde_jcs::to_vec(&signed)
        .map_err(|e| ConnectError::Transport(format!("JCS canonicalization failed: {e}")))?;
    let signature = URL_SAFE_NO_PAD
        .decode(hello.signature.as_str())
        .map_err(|_| ConnectError::InvalidHelloSignature)?;

    if !public_key.verify(&bytes, &signature) {
        return Err(ConnectError::InvalidHelloSignature);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // Deterministic test vectors: the fixed nonce strings in these tests are
    // reproducible fixtures, not production CSPRNG output (production nonces
    // come from `generate_nonce`).
    use super::*;
    use libp2p::identity::Keypair;

    fn manifest(host_id: &str) -> HostCapabilityManifest {
        HostCapabilityManifest {
            authority: None,
            capabilities: vec!["spoke-baseline".into()],
            extensions: Default::default(),
            host_id: host_id.parse().expect("host id parses"),
            namespaces: Vec::new(),
            roles: vec!["data-store".into()],
            schema_version: std::num::NonZeroU64::new(1).expect("non-zero"),
        }
    }

    #[test]
    fn nonce_meets_wire_floor_and_is_unique() {
        let a = generate_nonce().expect("nonce");
        let b = generate_nonce().expect("nonce");
        assert!(a.len() >= 16, "nonce below wire minLength: {}", a.len());
        assert_ne!(a, b);
    }

    #[test]
    fn short_caller_supplied_nonce_is_an_error_not_a_panic() {
        // The wire floor is 16 characters; a caller-supplied nonce below it
        // must surface as an error from the public signing API.
        let keypair = Keypair::generate_ed25519();
        let err = sign_hello(&keypair, "short", &manifest("host-a")).expect_err("short nonce");
        assert!(matches!(err, ConnectError::Config(_)));
    }

    #[test]
    fn sign_then_verify_round_trip() {
        let keypair = Keypair::generate_ed25519();
        let peer_id = keypair.public().to_peer_id();
        let hello =
            sign_hello(&keypair, "round-trip-nonce-123", &manifest("host-a")).expect("sign hello");

        assert_eq!(hello.protocol_version.get(), PROTOCOL_VERSION);
        assert_eq!(hello.peer_id.as_str(), peer_id.to_string());
        verify_hello(&keypair.public(), &peer_id, &hello).expect("verify hello");
    }

    #[test]
    fn tampered_host_fails_verification() {
        let keypair = Keypair::generate_ed25519();
        let peer_id = keypair.public().to_peer_id();
        let mut hello =
            sign_hello(&keypair, "tamper-nonce-12345", &manifest("host-a")).expect("sign hello");
        hello.host.roles.push("checker".into());

        let err = verify_hello(&keypair.public(), &peer_id, &hello).expect_err("tampered host");
        assert!(matches!(err, ConnectError::InvalidHelloSignature));
    }

    #[test]
    fn verify_key_must_derive_the_expected_peer_id() {
        // A key that does not derive the noise-authenticated peer id is
        // rejected at the binding check, before any signature work: the hello
        // signature must attest the authenticated peer's own identity key.
        let signer = Keypair::generate_ed25519();
        let other = Keypair::generate_ed25519();
        let hello =
            sign_hello(&signer, "wrong-key-nonce-12", &manifest("host-a")).expect("sign hello");

        let err = verify_hello(&other.public(), &signer.public().to_peer_id(), &hello)
            .expect_err("unbound verify key");
        assert!(
            matches!(err, ConnectError::HandshakeFailed { .. }),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn claimed_peer_id_mismatch_fails_before_signature_check() {
        let keypair = Keypair::generate_ed25519();
        let other = Keypair::generate_ed25519();
        let hello =
            sign_hello(&keypair, "mismatch-nonce-123", &manifest("host-a")).expect("sign hello");

        // Verify with the signer's key but expect the *other* peer id: the
        // claimed peer id (from the hello) then disagrees with the expected one.
        let err = verify_hello(&keypair.public(), &other.public().to_peer_id(), &hello)
            .expect_err("peer id mismatch");
        assert!(matches!(err, ConnectError::HandshakeFailed { .. }));
    }

    #[test]
    fn unsupported_protocol_version_rejected() {
        let keypair = Keypair::generate_ed25519();
        let peer_id = keypair.public().to_peer_id();
        let mut hello =
            sign_hello(&keypair, "version-nonce-1234", &manifest("host-a")).expect("sign hello");
        hello.protocol_version = std::num::NonZeroU64::new(2).expect("non-zero");

        let err =
            verify_hello(&keypair.public(), &peer_id, &hello).expect_err("unsupported version");
        assert!(matches!(err, ConnectError::HandshakeFailed { .. }));
    }

    #[test]
    fn malformed_signature_rejected() {
        let keypair = Keypair::generate_ed25519();
        let peer_id = keypair.public().to_peer_id();
        let mut hello =
            sign_hello(&keypair, "bad-sig-nonce-123", &manifest("host-a")).expect("sign hello");
        hello.signature = "%%%not-base64url%%%"
            .parse()
            .expect("string parses as opaque");

        let err =
            verify_hello(&keypair.public(), &peer_id, &hello).expect_err("malformed signature");
        assert!(matches!(err, ConnectError::InvalidHelloSignature));
    }

    #[test]
    fn signed_fields_are_jcs_deterministic() {
        // Same fields, two keypairs → identical canonical bytes must sign and
        // verify independently (JCS determinism across signers).
        let a = Keypair::generate_ed25519();
        let b = Keypair::generate_ed25519();
        let manifest_a = manifest("host-x");
        let hello_a = sign_hello(&a, "jcs-nonce-1234567", &manifest_a).expect("sign a");
        let hello_b = sign_hello(&b, "jcs-nonce-1234567", &manifest_a).expect("sign b");

        // HostCapabilityManifest has no PartialEq derive; compare wire shape.
        assert_eq!(
            serde_json::to_value(&hello_a.host).expect("serialize a"),
            serde_json::to_value(&hello_b.host).expect("serialize b"),
        );
        assert_eq!(hello_a.nonce, hello_b.nonce);
        assert_ne!(hello_a.peer_id, hello_b.peer_id);

        verify_hello(&a.public(), &a.public().to_peer_id(), &hello_a).expect("a verifies");
        verify_hello(&b.public(), &b.public().to_peer_id(), &hello_b).expect("b verifies");
    }
}
