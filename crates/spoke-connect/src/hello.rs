//! Authenticated hello: transport adapter for `spoke-connect-hello-jcs-v1`.
//!
//! The signing/verification rules themselves live in the pure session core
//! ([`crate::core::sign_hello_ed25519`] / [`crate::core::verify_hello_ed25519`]
//! over raw Ed25519 key bytes). This module is the **boundary adapter**: it
//! converts libp2p identity types to raw key material and maps core errors to
//! transport errors.
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

use crate::core;
use crate::error::{map_core_error, ConnectError};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use libp2p::identity::{Keypair, PublicKey};
use libp2p::PeerId;
use spoke_schemas::connect::connect_hello::HostCapabilityManifest;
use spoke_schemas::connect::ConnectHello;

/// Generate a single-use hello nonce: 128 bits of CSPRNG entropy, base64url
/// encoded (22 characters, above the wire floor of 16).
pub(crate) fn generate_nonce() -> Result<String, ConnectError> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|e| ConnectError::Transport(format!("CSPRNG failure: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// The raw 32-byte Ed25519 seed of `identity` (the core's signing input).
///
/// Connect pins Ed25519 identity (protocol v1), so a differently keyed
/// identity is a configuration error.
fn ed25519_seed(identity: &Keypair) -> Result<[u8; 32], ConnectError> {
    let pair = identity
        .clone()
        .try_into_ed25519()
        .map_err(|_| ConnectError::Config("connect identity must be an Ed25519 keypair".into()))?;
    Ok(pair
        .secret()
        .as_ref()
        .try_into()
        .expect("libp2p Ed25519 secret key is 32 bytes"))
}

/// Sign a fresh hello for `identity` carrying `manifest` under `nonce`.
///
/// `peer_id` on the wire is derived from the Ed25519 public key (libp2p
/// PeerId base58btc multihash form), matching
/// `identity.public().to_peer_id().to_string()`.
pub(crate) fn sign_hello(
    identity: &Keypair,
    nonce: &str,
    manifest: &HostCapabilityManifest,
) -> Result<ConnectHello, ConnectError> {
    let seed = ed25519_seed(identity)?;
    core::sign_hello_ed25519(&seed, nonce, manifest).map_err(map_core_error)
}

/// Verify a received hello.
///
/// Checks, in order (all delegated to the core rule):
/// 1. `protocol_version` equals the protocol version (`HandshakeFailed`).
/// 2. `public_key` derives `expected_peer_id` — the noise-authenticated
///    remote peer (`HandshakeFailed`).
/// 3. The claimed `peer_id` equals `expected_peer_id` (`HandshakeFailed`).
/// 4. The signature verifies against `public_key` (`InvalidHelloSignature`).
///
/// Allowlist and nonce gates are applied by [`crate::gate`] around this check.
pub(crate) fn verify_hello(
    public_key: &PublicKey,
    expected_peer_id: &PeerId,
    hello: &ConnectHello,
) -> Result<(), ConnectError> {
    // The core rule operates on raw Ed25519 keys; connect pins Ed25519
    // identity, so any other remote key type fails the handshake closed.
    let pubkey =
        public_key
            .clone()
            .try_into_ed25519()
            .map_err(|_| ConnectError::HandshakeFailed {
                reason: "remote public key is not Ed25519".into(),
            })?;
    core::verify_hello_ed25519(&pubkey.to_bytes(), &expected_peer_id.to_string(), hello)
        .map_err(map_core_error)
}

#[cfg(test)]
mod tests {
    // Deterministic test vectors: the fixed nonce strings in these tests are
    // reproducible fixtures, not production CSPRNG output (production nonces
    // come from `generate_nonce`).
    use super::*;
    use crate::core::PROTOCOL_VERSION;
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
