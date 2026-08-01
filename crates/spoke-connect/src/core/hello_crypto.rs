//! Hello signing and verification over **raw** Ed25519 key bytes
//! (`spoke-connect-hello-jcs-v1`).
//!
//! Normative rules (`.mstar/specs/spoke-connect.md` §Signature
//! canonicalization, §Identity binding):
//! - Signed object = exactly `{protocol_version, peer_id, nonce, host}`.
//! - Canonicalize with RFC 8785 JCS (`serde_jcs`) → UTF-8 bytes.
//! - Sign with the Ed25519 private key whose public key derives `peer_id`;
//!   the raw 64-byte signature is encoded base64url without padding.
//! - Top-level hello `extensions` and the `signature` field are **not** part
//!   of the signed object.
//!
//! No `libp2p::identity::Keypair` here — callers pass 32-byte raw key
//! material and `String` peer ids (the transport converts at the boundary).

use crate::core::error::CoreError;
use crate::core::peer_id::derive_peer_id_from_ed25519_pubkey;
use crate::core::PROTOCOL_VERSION;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
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

/// Canonicalize the signed hello object (`{protocol_version, peer_id, nonce,
/// host}`) with RFC 8785 JCS. The result is the byte sequence the hello
/// signature covers.
pub(crate) fn canonical_hello_bytes(
    peer_id: &str,
    nonce: &str,
    host: &HostCapabilityManifest,
) -> Result<Vec<u8>, CoreError> {
    let signed = SignedHello {
        protocol_version: PROTOCOL_VERSION,
        peer_id,
        nonce,
        host,
    };
    serde_jcs::to_vec(&signed).map_err(|e| CoreError::Jcs(e.to_string()))
}

/// Sign a hello with a raw Ed25519 secret key (32-byte seed), producing the
/// full `ConnectHello` wire envelope.
///
/// `peer_id` is derived from the public key of `secret`; the hello
/// `protocol_version` is the core protocol version. The `nonce` must meet
/// the wire floor (minLength 16) — enforced by the generated
/// `ConnectHelloNonce` type.
pub fn sign_hello_ed25519(
    secret: &[u8; 32],
    nonce: &str,
    manifest: &HostCapabilityManifest,
) -> Result<ConnectHello, CoreError> {
    let signing_key = SigningKey::from_bytes(secret);
    let peer_id = derive_peer_id_from_ed25519_pubkey(&signing_key.verifying_key().to_bytes());
    let bytes = canonical_hello_bytes(&peer_id, nonce, manifest)?;
    let signature: Signature = signing_key.sign(&bytes);

    Ok(ConnectHello {
        protocol_version: std::num::NonZeroU64::new(PROTOCOL_VERSION)
            .expect("PROTOCOL_VERSION is a non-zero constant"),
        // A 32-byte public key always derives a well-formed peer id string
        // (base58btc, non-empty), so these parses are infallible; the nonce
        // parse is fallible (wire floor).
        peer_id: peer_id.parse().expect("derived peer id parses"),
        nonce: nonce.parse().map_err(
            |e: spoke_schemas::connect::connect_hello::error::ConversionError| {
                CoreError::InvalidNonce(e.to_string())
            },
        )?,
        host: manifest.clone(),
        signature: URL_SAFE_NO_PAD
            .encode(signature.to_bytes())
            .parse()
            .expect("base64url signature parses"),
        extensions: Default::default(),
    })
}

/// Verify a received hello against a raw Ed25519 public key (32 bytes).
///
/// Checks, in order:
/// 1. `protocol_version` equals the core protocol version.
/// 2. The verify key derives `expected_peer_id` — the authenticated remote
///    peer. A key that derives a different peer id cannot attest that
///    peer's identity.
/// 3. The claimed `hello.peer_id` equals `expected_peer_id`.
/// 4. The signature verifies over the JCS-canonicalized signed object.
///
/// Allowlist and nonce gates are separate core checks (see `allowlist` and
/// `nonce` modules).
pub fn verify_hello_ed25519(
    public_key: &[u8; 32],
    expected_peer_id: &str,
    hello: &ConnectHello,
) -> Result<(), CoreError> {
    if hello.protocol_version.get() != PROTOCOL_VERSION {
        return Err(CoreError::HandshakeFailed {
            reason: format!(
                "unsupported protocol_version {} (expected {PROTOCOL_VERSION})",
                hello.protocol_version
            ),
        });
    }

    let verifying_key = VerifyingKey::from_bytes(public_key)
        .map_err(|e| CoreError::Crypto(format!("invalid Ed25519 public key: {e}")))?;
    let derived_peer_id = derive_peer_id_from_ed25519_pubkey(public_key);
    if derived_peer_id != expected_peer_id {
        return Err(CoreError::HandshakeFailed {
            reason: format!(
                "public key derives peer id {derived_peer_id} instead of the authenticated peer {expected_peer_id}"
            ),
        });
    }
    if hello.peer_id.as_str() != expected_peer_id {
        return Err(CoreError::HandshakeFailed {
            reason: format!(
                "hello peer_id {} does not match authenticated peer {expected_peer_id}",
                hello.peer_id.as_str()
            ),
        });
    }

    // The claimed peer id was just checked to equal `expected_peer_id`, so
    // canonicalizing over `expected_peer_id` reproduces the signer's bytes.
    let bytes = canonical_hello_bytes(expected_peer_id, hello.nonce.as_str(), &hello.host)?;
    let signature = URL_SAFE_NO_PAD
        .decode(hello.signature.as_str())
        .map_err(|_| CoreError::InvalidHelloSignature)?;
    let signature =
        Signature::from_slice(&signature).map_err(|_| CoreError::InvalidHelloSignature)?;
    if verifying_key.verify(&bytes, &signature).is_err() {
        return Err(CoreError::InvalidHelloSignature);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // Deterministic test vectors: the fixed nonce strings in these tests are
    // reproducible fixtures, not production CSPRNG output.
    use super::*;

    /// Golden key pair: seed bytes 1..=32, captured from the same seed fed
    /// to libp2p `Keypair::ed25519_from_bytes` (libp2p-identity 0.2.14 /
    /// libp2p 0.56).
    const GOLDEN_SEED: [u8; 32] = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e,
        0x1f, 0x20,
    ];
    const GOLDEN_PUBKEY: [u8; 32] = [
        0x79, 0xb5, 0x56, 0x2e, 0x8f, 0xe6, 0x54, 0xf9, 0x40, 0x78, 0xb1, 0x12, 0xe8, 0xa9, 0x8b,
        0xa7, 0x90, 0x1f, 0x85, 0x3a, 0xe6, 0x95, 0xbe, 0xd7, 0xe0, 0xe3, 0x91, 0x0b, 0xad, 0x04,
        0x96, 0x64,
    ];
    const GOLDEN_PEER_ID: &str = "12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf";
    const GOLDEN_NONCE: &str = "golden-nonce-000000000001";
    /// RFC 8785 JCS bytes of the signed object, captured from the libp2p
    /// hello path (`serde_jcs` over `{protocol_version, peer_id, nonce,
    /// host}`) before the transport cutover.
    const GOLDEN_JCS_HEX: &str = "7b22686f7374223a7b226361706162696c6974696573223a5b2273706f6b652d626173656c696e65225d2c22657874656e73696f6e73223a7b7d2c22686f73745f6964223a22676f6c64656e2d686f7374222c226e616d65737061636573223a5b5d2c22726f6c6573223a5b22646174612d73746f7265225d2c22736368656d615f76657273696f6e223a317d2c226e6f6e6365223a22676f6c64656e2d6e6f6e63652d303030303030303030303031222c22706565725f6964223a22313244334b6f6f574a315473696a48374835463734686641443558697368517a337378726d41745659333747744e643943715966222c2270726f746f636f6c5f76657273696f6e223a317d";
    /// base64url (no padding) of the raw 64-byte Ed25519 signature over
    /// `GOLDEN_JCS_HEX`, captured from libp2p `Keypair::sign` before the
    /// transport cutover.
    const GOLDEN_SIGNATURE: &str =
        "yWu5Dl0jcKPWGyFDWJ1K8PbgoGcxerFSXSxiCu6Sdh8cqwH667TuAZJwgbuRHJFWehVaJtn5ox2vuYRO8IcMCg";

    /// Golden manifest — `authority` is `None`. JCS serialization **omits**
    /// absent optional fields: the canonical bytes in `GOLDEN_JCS_HEX`
    /// contain no `"authority"` member at all (the generated
    /// `HostCapabilityManifest` carries `skip_serializing_if =
    /// "Option::is_none"` on `authority`, the only non-required manifest
    /// field). Path A porters must not emit `"authority": null` — an
    /// explicit null serializes as a `null` member, which canonicalizes to
    /// different bytes (and therefore a different signature) than the
    /// omitted form.
    fn golden_manifest() -> HostCapabilityManifest {
        HostCapabilityManifest {
            authority: None,
            capabilities: vec!["spoke-baseline".into()],
            extensions: Default::default(),
            host_id: "golden-host".parse().expect("host id parses"),
            namespaces: Vec::new(),
            roles: vec!["data-store".into()],
            schema_version: std::num::NonZeroU64::new(1).expect("non-zero"),
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn golden_signature_matches_libp2p_captured_vector() {
        let hello = sign_hello_ed25519(&GOLDEN_SEED, GOLDEN_NONCE, &golden_manifest())
            .expect("sign golden hello");
        assert_eq!(hello.peer_id.as_str(), GOLDEN_PEER_ID);
        assert_eq!(hello.signature.as_str(), GOLDEN_SIGNATURE);
    }

    #[test]
    fn golden_canonical_bytes_match_libp2p_captured_vector() {
        let bytes =
            canonical_hello_bytes(GOLDEN_PEER_ID, GOLDEN_NONCE, &golden_manifest()).expect("jcs");
        assert_eq!(hex(&bytes), GOLDEN_JCS_HEX);
    }

    #[test]
    fn golden_hello_verifies_with_raw_pubkey() {
        let hello = sign_hello_ed25519(&GOLDEN_SEED, GOLDEN_NONCE, &golden_manifest())
            .expect("sign golden hello");
        verify_hello_ed25519(&GOLDEN_PUBKEY, GOLDEN_PEER_ID, &hello).expect("verify golden hello");
    }

    #[test]
    fn sign_then_verify_round_trip() {
        let secret = [7u8; 32];
        let public_key = SigningKey::from_bytes(&secret).verifying_key().to_bytes();
        let peer_id = derive_peer_id_from_ed25519_pubkey(&public_key);
        let hello = sign_hello_ed25519(&secret, "round-trip-nonce-123", &golden_manifest())
            .expect("sign hello");
        assert_eq!(hello.peer_id.as_str(), peer_id);
        verify_hello_ed25519(&public_key, &peer_id, &hello).expect("verify hello");
    }

    #[test]
    fn short_nonce_is_an_error_not_a_panic() {
        let secret = [8u8; 32];
        let err =
            sign_hello_ed25519(&secret, "short", &golden_manifest()).expect_err("short nonce");
        assert!(matches!(err, CoreError::InvalidNonce(_)));
    }

    #[test]
    fn tampered_host_fails_verification() {
        let mut hello = sign_hello_ed25519(&GOLDEN_SEED, GOLDEN_NONCE, &golden_manifest())
            .expect("sign golden hello");
        hello.host.roles.push("checker".into());
        let err = verify_hello_ed25519(&GOLDEN_PUBKEY, GOLDEN_PEER_ID, &hello)
            .expect_err("tampered host");
        assert!(matches!(err, CoreError::InvalidHelloSignature));
    }

    #[test]
    fn verify_key_must_derive_the_expected_peer_id() {
        let other_secret = [9u8; 32];
        let other_pubkey = SigningKey::from_bytes(&other_secret)
            .verifying_key()
            .to_bytes();
        let hello = sign_hello_ed25519(&GOLDEN_SEED, GOLDEN_NONCE, &golden_manifest())
            .expect("sign golden hello");
        let err = verify_hello_ed25519(&other_pubkey, GOLDEN_PEER_ID, &hello)
            .expect_err("unbound verify key");
        assert!(matches!(err, CoreError::HandshakeFailed { .. }));
    }

    #[test]
    fn claimed_peer_id_mismatch_fails_before_signature_check() {
        let hello = sign_hello_ed25519(&GOLDEN_SEED, GOLDEN_NONCE, &golden_manifest())
            .expect("sign golden hello");
        let other_secret = [10u8; 32];
        let other_peer = derive_peer_id_from_ed25519_pubkey(
            &SigningKey::from_bytes(&other_secret)
                .verifying_key()
                .to_bytes(),
        );
        let err = verify_hello_ed25519(&GOLDEN_PUBKEY, &other_peer, &hello)
            .expect_err("peer id mismatch");
        assert!(matches!(err, CoreError::HandshakeFailed { .. }));
    }

    #[test]
    fn unsupported_protocol_version_rejected() {
        let mut hello = sign_hello_ed25519(&GOLDEN_SEED, GOLDEN_NONCE, &golden_manifest())
            .expect("sign golden hello");
        hello.protocol_version = std::num::NonZeroU64::new(2).expect("non-zero");
        let err = verify_hello_ed25519(&GOLDEN_PUBKEY, GOLDEN_PEER_ID, &hello)
            .expect_err("unsupported version");
        assert!(matches!(err, CoreError::HandshakeFailed { .. }));
    }

    #[test]
    fn malformed_signature_rejected() {
        let mut hello = sign_hello_ed25519(&GOLDEN_SEED, GOLDEN_NONCE, &golden_manifest())
            .expect("sign golden hello");
        hello.signature = "%%%not-base64url%%%"
            .parse()
            .expect("string parses as opaque");
        let err = verify_hello_ed25519(&GOLDEN_PUBKEY, GOLDEN_PEER_ID, &hello)
            .expect_err("malformed signature");
        assert!(matches!(err, CoreError::InvalidHelloSignature));
    }

    #[test]
    fn signed_fields_are_jcs_deterministic_across_signer_keys() {
        // Same fields, two seeds → identical canonical bytes must sign and
        // verify independently (JCS determinism across signers).
        let secret_a = [11u8; 32];
        let secret_b = [12u8; 32];
        let hello_a =
            sign_hello_ed25519(&secret_a, "jcs-nonce-1234567", &golden_manifest()).expect("sign a");
        let hello_b =
            sign_hello_ed25519(&secret_b, "jcs-nonce-1234567", &golden_manifest()).expect("sign b");

        assert_eq!(
            serde_json::to_value(&hello_a.host).expect("serialize a"),
            serde_json::to_value(&hello_b.host).expect("serialize b"),
        );
        assert_eq!(hello_a.nonce, hello_b.nonce);
        assert_ne!(hello_a.peer_id, hello_b.peer_id);

        let pubkey_a = SigningKey::from_bytes(&secret_a).verifying_key().to_bytes();
        let pubkey_b = SigningKey::from_bytes(&secret_b).verifying_key().to_bytes();
        verify_hello_ed25519(&pubkey_a, hello_a.peer_id.as_str(), &hello_a).expect("a verifies");
        verify_hello_ed25519(&pubkey_b, hello_b.peer_id.as_str(), &hello_b).expect("b verifies");
    }
}
