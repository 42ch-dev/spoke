//! Integration test: Rust verify path accepts the TS-minted capability-token
//! golden vector (`fixtures/capability-token-ts-golden.json`, Task 1).
//!
//! Pinned outputs are transcribed from the committed fixture — never regenerated
//! by running TS or re-minting from Rust (sync-gate invariant).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use spoke_connect::core::{
    ed25519_pubkey_from_peer_id, verify_capability_token, CapabilityClaims,
    CapabilityTokenProof, CoreError, TOKEN_VERSION,
};

const FIXTURE: &str = include_str!("fixtures/capability-token-ts-golden.json");
const NOW: u64 = 1_000_000_000;

/// Strip top-level `provenance` (metadata only) and parse the wire proof.
fn load_ts_golden_proof() -> CapabilityTokenProof {
    let mut value: serde_json::Value =
        serde_json::from_str(FIXTURE).expect("fixture parses as JSON");
    value
        .as_object_mut()
        .expect("fixture is an object")
        .remove("provenance");
    serde_json::from_value(value).expect("wire proof parses as CapabilityTokenProof")
}

fn assert_claims_match_expected(claims: &CapabilityClaims) {
    assert_eq!(
        claims.iss,
        "12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf"
    );
    assert_eq!(
        claims.sub,
        "12D3KooWRawPbxPtP1eZaJpumGnyWX2DcUyd3RQnydr3eAto4Az7"
    );
    assert_eq!(
        claims.aud,
        "12D3KooWB8sCGZCrwr79HtabLAn95qyPQx6RYHXjEbiD6QKou7ww"
    );
    assert_eq!(
        claims.capabilities,
        vec!["spoke-baseline".to_string(), "l2-computable".to_string()]
    );
    assert_eq!(claims.exp, 1_000_003_600);
    assert_eq!(claims.iat, Some(1_000_000_000));
    assert_eq!(claims.jti.as_deref(), Some("golden-jti-001"));
}

/// Reconstruct JCS signing bytes from `claims` and assert the pinned signature
/// verifies byte-for-byte (same projection Rust uses at issue time).
fn assert_jcs_and_signature_match(proof: &CapabilityTokenProof) {
    let jcs_bytes = serde_jcs::to_vec(&proof.claims).expect("claims canonicalize with JCS");

    let issuer_pubkey =
        ed25519_pubkey_from_peer_id(&proof.claims.iss).expect("iss is Ed25519 peer id");
    let verifying_key =
        VerifyingKey::from_bytes(&issuer_pubkey).expect("valid Ed25519 public key");

    let raw_sig = URL_SAFE_NO_PAD
        .decode(proof.sig.as_str())
        .expect("sig is valid base64url");
    assert_eq!(raw_sig.len(), 64, "Ed25519 signature is 64 bytes");
    assert_eq!(
        URL_SAFE_NO_PAD.encode(&raw_sig),
        proof.sig,
        "signature is canonical base64url"
    );
    let signature = Signature::from_slice(&raw_sig).expect("signature is 64 bytes");
    verifying_key
        .verify(&jcs_bytes, &signature)
        .expect("JCS bytes verify against pinned signature");
}

#[test]
fn raw_fixture_rejects_provenance_without_strip() {
    assert!(
        serde_json::from_str::<CapabilityTokenProof>(FIXTURE).is_err(),
        "deny_unknown_fields rejects provenance on strict parse"
    );
}

#[test]
fn ts_minted_golden_vector_verifies_end_to_end() {
    let proof = load_ts_golden_proof();
    assert_eq!(proof.v, TOKEN_VERSION);
    assert_claims_match_expected(&proof.claims);
    assert_jcs_and_signature_match(&proof);

    let trusted = vec![proof.claims.iss.clone()];
    let granted = verify_capability_token(
        &proof,
        &trusted,
        &proof.claims.aud,
        &proof.claims.sub,
        NOW,
    )
    .expect("TS-minted golden vector verifies");
    assert_eq!(
        granted,
        vec!["spoke-baseline".to_string(), "l2-computable".to_string()]
    );
}

#[test]
fn flipped_claims_byte_rejects_with_token_invalid() {
    let mut proof = load_ts_golden_proof();
    // Flip one byte of the signed claims (keep `sig` fixed) → signature mismatch.
    proof.claims.exp ^= 1;

    let trusted = vec![proof.claims.iss.clone()];
    let err = verify_capability_token(
        &proof,
        &trusted,
        &proof.claims.aud,
        &proof.claims.sub,
        NOW,
    )
    .expect_err("tampered claims must not verify");
    assert!(
        matches!(err, CoreError::TokenInvalid(ref msg) if msg == "signature does not verify"),
        "expected signature-mismatch TokenInvalid, got {err:?}"
    );
}
