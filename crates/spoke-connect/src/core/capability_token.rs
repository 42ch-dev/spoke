//! Capability-token issuance and validation over **raw** Ed25519 key bytes.
//!
//! Normative rules (`.mstar/specs/spoke-connect.md` §Method —
//! capability-token):
//! - Signed claims object = exactly `{iss, sub, aud, capabilities, exp}`
//!   plus optional `iat` / `jti`. Unknown claim keys **reject** (fail
//!   closed) so the JCS bytes stay intentional.
//! - Canonicalize with RFC 8785 JCS (`serde_jcs`) → UTF-8 bytes; sign with
//!   the **issuer** Ed25519 private key; the raw 64-byte signature is
//!   encoded base64url without padding. The signature covers **only**
//!   JCS(`claims`) — not the `{v, claims, sig}` wire wrapper.
//! - The issuer key MUST derive `claims.iss`; verification recovers the
//!   issuer public key from the `iss` peer id itself (Ed25519 peer ids use
//!   the identity multihash, so the mapping is an encoding inversion — see
//!   `peer_id::ed25519_pubkey_from_peer_id`).
//!
//! Validation is **offline**: signature + trusted-issuer list + subject /
//! audience / expiry + capability membership. No revocation list, no refresh
//! token, no issuance endpoint (see spec non-goals).
//!
//! No `libp2p` here — callers pass raw 32-byte key material and `String`
//! peer ids (the transport converts at the boundary), mirroring
//! `hello_crypto`.

use crate::core::error::CoreError;
use crate::core::peer_id::{derive_peer_id_from_ed25519_pubkey, ed25519_pubkey_from_peer_id};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Token format version carried in the wire wrapper `v` (protocol version 1
/// uses `1`).
pub const TOKEN_VERSION: u64 = 1;

/// Clock-skew allowance (seconds) applied to `iat` on both sides: a token
/// whose `iat` is up to 60s in the future is accepted.
pub const CLOCK_SKEW_SECONDS: u64 = 60;

/// The signed claims object of a capability token (normative claim set).
///
/// Serialized with RFC 8785 JCS to produce the byte sequence covered by the
/// token signature. Unknown claim keys are rejected at deserialization
/// (fail closed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityClaims {
    /// Issuer `peer_id` — the string form of the signing key's derived id.
    pub iss: String,
    /// Subject `peer_id` — who may present the token.
    pub sub: String,
    /// Audience — the verifying node's `peer_id`.
    pub aud: String,
    /// Capability names granted to `sub` (e.g. `spoke-baseline`).
    pub capabilities: Vec<String>,
    /// Expiry as Unix time seconds (UTC); reject when `now >= exp`.
    pub exp: u64,
    /// Issued-at Unix seconds; when present, reject when `iat` is beyond the
    /// clock-skew window ahead of `now`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iat: Option<u64>,
    /// Unique token id; when present, must be non-empty. Reserved for a
    /// future revocation design — not consulted by protocol version 1
    /// validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,
}

/// The wire `proof` wrapper for the capability-token method.
///
/// `sig` covers **only** JCS(`claims`) — the wrapper (`v`, `claims`, `sig`)
/// is not itself signed. Unknown wrapper keys are rejected at
/// deserialization (fail closed), matching the spec's malformed-proof rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityTokenProof {
    /// Token format version ([`TOKEN_VERSION`]).
    pub v: u64,
    /// The signed claims object.
    pub claims: CapabilityClaims,
    /// base64url (no padding) of the 64 raw Ed25519 signature bytes over
    /// JCS(`claims`).
    pub sig: String,
}

/// Issue a capability token: canonicalize `claims` with JCS, sign with the
/// issuer Ed25519 secret key (32-byte seed), and wrap the result.
///
/// The issuer's derived `peer_id` MUST equal `claims.iss` — the token must
/// be issued by the authority it names, or it cannot verify.
pub fn issue_capability_token(
    issuer_secret: &[u8; 32],
    claims: CapabilityClaims,
) -> Result<CapabilityTokenProof, CoreError> {
    let signing_key = SigningKey::from_bytes(issuer_secret);
    let derived_issuer =
        derive_peer_id_from_ed25519_pubkey(&signing_key.verifying_key().to_bytes());
    if derived_issuer != claims.iss {
        return Err(CoreError::TokenInvalid(format!(
            "issuer key derives peer id {derived_issuer}, not claims.iss {}",
            claims.iss
        )));
    }
    let bytes = serde_jcs::to_vec(&claims).map_err(|e| CoreError::Jcs(e.to_string()))?;
    let signature: Signature = signing_key.sign(&bytes);
    Ok(CapabilityTokenProof {
        v: TOKEN_VERSION,
        claims,
        sig: URL_SAFE_NO_PAD.encode(signature.to_bytes()),
    })
}

/// Validate a capability-token proof against this node's trust configuration
/// and the authenticated session peer.
///
/// Checks, in order (normative §Trust root and validation rules):
/// 1. `v` is the current token version.
/// 2. The signature verifies over JCS(`claims`) with the public key that
///    derives `claims.iss` (recovered from the issuer peer id).
/// 3. `claims.iss` is an exact-string member of `trusted_issuers` (empty
///    list ⇒ method disabled: every proof is rejected).
/// 4. `claims.sub` equals `session_peer_id` — the peer that passed the
///    `noise-peerid` hello. Tokens are not transferable across peers.
/// 5. `claims.aud` equals `this_peer_id` — the verifying node.
/// 6. `exp` is required; reject when `now >= exp`.
/// 7. `iat`, when present, must not be beyond the ±[`CLOCK_SKEW_SECONDS`]
///    window ahead of `now`.
/// 8. `jti`, when present, must be non-empty.
///
/// Unknown claims / wrapper keys and malformed shapes are rejected at
/// deserialization before this function runs (the caller deserializes the
/// opaque proof). Returns the validated grant (`claims.capabilities`) for
/// the dispatch gate — the token does **not** replace the session's
/// `negotiated_capabilities`.
pub fn verify_capability_token(
    proof: &CapabilityTokenProof,
    trusted_issuers: &[String],
    this_peer_id: &str,
    session_peer_id: &str,
    now: u64,
) -> Result<Vec<String>, CoreError> {
    if proof.v != TOKEN_VERSION {
        return Err(CoreError::TokenInvalid(format!(
            "unsupported token version {} (expected {TOKEN_VERSION})",
            proof.v
        )));
    }

    // The issuer public key is recovered from the issuer peer id: Ed25519
    // peer ids are identity multihashes, so the string carries the key.
    let Some(issuer_pubkey) = ed25519_pubkey_from_peer_id(&proof.claims.iss) else {
        return Err(CoreError::TokenInvalid(format!(
            "iss is not an Ed25519 peer id: {}",
            proof.claims.iss
        )));
    };
    let verifying_key = VerifyingKey::from_bytes(&issuer_pubkey)
        .map_err(|e| CoreError::Crypto(format!("invalid Ed25519 public key: {e}")))?;
    let bytes = serde_jcs::to_vec(&proof.claims).map_err(|e| CoreError::Jcs(e.to_string()))?;
    let signature = URL_SAFE_NO_PAD
        .decode(proof.sig.as_str())
        .map_err(|_| CoreError::TokenInvalid("signature is not valid base64url".into()))?;
    let signature = Signature::from_slice(&signature)
        .map_err(|_| CoreError::TokenInvalid("signature is not 64 bytes".into()))?;
    if verifying_key.verify(&bytes, &signature).is_err() {
        return Err(CoreError::TokenInvalid("signature does not verify".into()));
    }

    if !trusted_issuers
        .iter()
        .any(|issuer| issuer == &proof.claims.iss)
    {
        return Err(CoreError::TokenInvalid(format!(
            "issuer {} is not trusted",
            proof.claims.iss
        )));
    }
    if proof.claims.sub != session_peer_id {
        return Err(CoreError::TokenInvalid(format!(
            "subject {} does not match the session peer {session_peer_id}",
            proof.claims.sub
        )));
    }
    if proof.claims.aud != this_peer_id {
        return Err(CoreError::TokenInvalid(format!(
            "audience {} does not match this node {this_peer_id}",
            proof.claims.aud
        )));
    }
    if now >= proof.claims.exp {
        return Err(CoreError::TokenInvalid(format!(
            "token expired at {} (now {now})",
            proof.claims.exp
        )));
    }
    if let Some(iat) = proof.claims.iat {
        if iat > now.saturating_add(CLOCK_SKEW_SECONDS) {
            return Err(CoreError::TokenInvalid(format!(
                "token issued at {iat} is beyond the {CLOCK_SKEW_SECONDS}s clock-skew window ahead of now {now}"
            )));
        }
    }
    if let Some(jti) = &proof.claims.jti {
        if jti.is_empty() {
            return Err(CoreError::TokenInvalid("jti must be non-empty".into()));
        }
    }
    Ok(proof.claims.capabilities.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    /// Deterministic test keys: distinct 32-byte seeds per role.
    const ISSUER_SEED: [u8; 32] = [1u8; 32];
    const OTHER_ISSUER_SEED: [u8; 32] = [2u8; 32];

    /// A peer id for a raw Ed25519 seed (runtime-derived — no literals in
    /// crypto positions).
    fn peer_of(secret: &[u8; 32]) -> String {
        derive_peer_id_from_ed25519_pubkey(
            &SigningKey::from_bytes(secret).verifying_key().to_bytes(),
        )
    }

    fn claims(
        iss: &str,
        sub: &str,
        aud: &str,
        capabilities: &[&str],
        exp: u64,
    ) -> CapabilityClaims {
        CapabilityClaims {
            iss: iss.to_string(),
            sub: sub.to_string(),
            aud: aud.to_string(),
            capabilities: capabilities.iter().map(|c| (*c).to_string()).collect(),
            exp,
            iat: None,
            jti: None,
        }
    }

    fn trusted(issuers: &[&str]) -> Vec<String> {
        issuers.iter().map(|s| (*s).to_string()).collect()
    }

    /// A valid (issuer, subject, audience) triple for the happy path: the
    /// issuer is trusted, `sub` is the session peer, `aud` the verifying
    /// node, and the token is unexpired at `now`.
    fn happy_token(
        now: u64,
        capabilities: &[&str],
    ) -> (CapabilityTokenProof, Vec<String>, Vec<String>) {
        let issuer = peer_of(&ISSUER_SEED);
        let subject = peer_of(&[7u8; 32]);
        let audience = peer_of(&[8u8; 32]);
        let proof = issue_capability_token(
            &ISSUER_SEED,
            claims(&issuer, &subject, &audience, capabilities, now + 3600),
        )
        .expect("issue");
        (proof, trusted(&[issuer.as_str()]), vec![subject, audience])
    }

    #[test]
    fn issue_verify_round_trip_returns_the_grant() {
        let now = 1_000_000_000u64;
        let (proof, trusted, peers) = happy_token(now, &["spoke-baseline", "l2-computable"]);
        let granted = verify_capability_token(&proof, &trusted, &peers[1], &peers[0], now)
            .expect("valid token verifies");
        assert_eq!(granted, vec!["spoke-baseline", "l2-computable"]);
    }

    #[test]
    fn expired_token_is_rejected_at_the_expiry_boundary() {
        let now = 1_000_000_000u64;
        let issuer = peer_of(&ISSUER_SEED);
        let subject = peer_of(&[7u8; 32]);
        let audience = peer_of(&[8u8; 32]);
        let trusted = trusted(&[issuer.as_str()]);
        // exp == now is already expired (reject if now >= exp).
        let proof = issue_capability_token(
            &ISSUER_SEED,
            claims(&issuer, &subject, &audience, &["spoke-baseline"], now),
        )
        .expect("issue");
        let err = verify_capability_token(&proof, &trusted, &audience, &subject, now)
            .expect_err("expired at now");
        assert!(matches!(err, CoreError::TokenInvalid(_)));
        // And any time after exp.
        let err = verify_capability_token(&proof, &trusted, &audience, &subject, now + 1)
            .expect_err("expired after");
        assert!(matches!(err, CoreError::TokenInvalid(_)));
    }

    #[test]
    fn untrusted_issuer_is_rejected() {
        let now = 1_000_000_000u64;
        let (proof, _trusted, peers) = happy_token(now, &["spoke-baseline"]);
        let other = peer_of(&OTHER_ISSUER_SEED);
        let err = verify_capability_token(
            &proof,
            &trusted(&[other.as_str()]),
            &peers[1],
            &peers[0],
            now,
        )
        .expect_err("untrusted issuer");
        assert!(matches!(err, CoreError::TokenInvalid(_)));
    }

    #[test]
    fn empty_trusted_issuers_rejects_every_proof() {
        // Empty list ⇒ the capability-token method is disabled: proofs are
        // rejected (fail closed), even a perfectly valid one.
        let now = 1_000_000_000u64;
        let (proof, _trusted, peers) = happy_token(now, &["spoke-baseline"]);
        let err = verify_capability_token(&proof, &[], &peers[1], &peers[0], now)
            .expect_err("method disabled");
        assert!(matches!(err, CoreError::TokenInvalid(_)));
    }

    #[test]
    fn wrong_subject_is_rejected() {
        let now = 1_000_000_000u64;
        let (proof, trusted, peers) = happy_token(now, &["spoke-baseline"]);
        let other_peer = peer_of(&[9u8; 32]);
        let err = verify_capability_token(&proof, &trusted, &peers[1], &other_peer, now)
            .expect_err("wrong subject");
        assert!(matches!(err, CoreError::TokenInvalid(_)));
    }

    #[test]
    fn wrong_audience_is_rejected() {
        let now = 1_000_000_000u64;
        let (proof, trusted, peers) = happy_token(now, &["spoke-baseline"]);
        let other_peer = peer_of(&[10u8; 32]);
        let err = verify_capability_token(&proof, &trusted, &other_peer, &peers[0], now)
            .expect_err("wrong audience");
        assert!(matches!(err, CoreError::TokenInvalid(_)));
    }

    #[test]
    fn unknown_claims_and_wrapper_fields_are_rejected_at_deserialize() {
        let now = 1_000_000_000u64;
        let (proof, _trusted, _peers) = happy_token(now, &["spoke-baseline"]);
        let mut value = serde_json::to_value(&proof).expect("proof serializes");

        // Unknown claim key inside the signed object.
        value["claims"]["extra_claim"] = serde_json::json!("sneaky");
        assert!(
            serde_json::from_value::<CapabilityTokenProof>(value.clone()).is_err(),
            "unknown claim key must reject"
        );

        // Unknown wrapper key.
        let mut value = serde_json::to_value(&proof).expect("proof serializes");
        value["extra_wrapper"] = serde_json::json!(true);
        assert!(
            serde_json::from_value::<CapabilityTokenProof>(value).is_err(),
            "unknown wrapper key must reject"
        );

        // Missing required claim.
        let mut value = serde_json::to_value(&proof).expect("proof serializes");
        value["claims"]
            .as_object_mut()
            .expect("claims object")
            .remove("sub");
        assert!(
            serde_json::from_value::<CapabilityTokenProof>(value).is_err(),
            "missing required claim must reject"
        );
    }

    #[test]
    fn malformed_proof_shapes_are_rejected_at_deserialize() {
        // Non-object proof (the wire proof must be an OpaqueJson object).
        assert!(
            serde_json::from_value::<CapabilityTokenProof>(serde_json::json!("just-a-string"))
                .is_err()
        );
        // Wrong-typed claims (string where the claims object belongs).
        assert!(
            serde_json::from_value::<CapabilityTokenProof>(serde_json::json!({
                "v": 1, "claims": "not-an-object", "sig": "AA"
            }))
            .is_err()
        );
        // exp as a float is not a JSON integer.
        let value = serde_json::json!({
            "v": 1,
            "claims": {
                "iss": "x", "sub": "y", "aud": "z",
                "capabilities": [], "exp": 1e9
            },
            "sig": "AA"
        });
        assert!(serde_json::from_value::<CapabilityTokenProof>(value).is_err());
    }

    #[test]
    fn malformed_signature_is_rejected() {
        let now = 1_000_000_000u64;
        let (mut proof, trusted, peers) = happy_token(now, &["spoke-baseline"]);

        // Not valid base64url.
        proof.sig = "%%%not-base64url%%%".into();
        let err = verify_capability_token(&proof, &trusted, &peers[1], &peers[0], now)
            .expect_err("bad base64url");
        assert!(matches!(err, CoreError::TokenInvalid(_)));

        // Valid base64url but not 64 bytes.
        proof.sig = URL_SAFE_NO_PAD.encode([0u8; 1]);
        let err = verify_capability_token(&proof, &trusted, &peers[1], &peers[0], now)
            .expect_err("wrong length");
        assert!(matches!(err, CoreError::TokenInvalid(_)));
    }

    #[test]
    fn tampered_claims_fail_signature_verification() {
        let now = 1_000_000_000u64;
        let (mut proof, trusted, peers) = happy_token(now, &["spoke-baseline"]);
        // Re-signing is not possible without the issuer key; mutating a claim
        // after issuance must break the signature check.
        proof.claims.capabilities.push("l2-computable".into());
        let err = verify_capability_token(&proof, &trusted, &peers[1], &peers[0], now)
            .expect_err("tampered claims");
        assert!(matches!(err, CoreError::TokenInvalid(_)));
    }

    #[test]
    fn issuer_key_must_derive_claims_iss() {
        let now = 1_000_000_000u64;
        let issuer = peer_of(&ISSUER_SEED);
        let subject = peer_of(&[7u8; 32]);
        let audience = peer_of(&[8u8; 32]);
        // Sign with a different key than the named issuer: the issuance API
        // must refuse before any bytes are signed.
        let err = issue_capability_token(
            &OTHER_ISSUER_SEED,
            claims(
                &issuer,
                &subject,
                &audience,
                &["spoke-baseline"],
                now + 3600,
            ),
        )
        .expect_err("unbound issuer key");
        assert!(matches!(err, CoreError::TokenInvalid(_)));
    }

    #[test]
    fn wrong_token_version_is_rejected() {
        let now = 1_000_000_000u64;
        let (mut proof, trusted, peers) = happy_token(now, &["spoke-baseline"]);
        proof.v = 2;
        let err = verify_capability_token(&proof, &trusted, &peers[1], &peers[0], now)
            .expect_err("wrong version");
        assert!(matches!(err, CoreError::TokenInvalid(_)));
    }

    #[test]
    fn iat_within_clock_skew_is_accepted_and_beyond_is_rejected() {
        let now = 1_000_000_000u64;
        let issuer = peer_of(&ISSUER_SEED);
        let subject = peer_of(&[7u8; 32]);
        let audience = peer_of(&[8u8; 32]);
        let trusted = trusted(&[issuer.as_str()]);

        // iat exactly at the skew boundary is accepted.
        let mut grant = claims(
            &issuer,
            &subject,
            &audience,
            &["spoke-baseline"],
            now + 3600,
        );
        grant.iat = Some(now + CLOCK_SKEW_SECONDS);
        let proof = issue_capability_token(&ISSUER_SEED, grant).expect("issue");
        verify_capability_token(&proof, &trusted, &audience, &subject, now)
            .expect("iat at the skew boundary");

        // iat beyond the skew window is rejected (issuer clock too far ahead).
        let mut grant = claims(
            &issuer,
            &subject,
            &audience,
            &["spoke-baseline"],
            now + 3600,
        );
        grant.iat = Some(now + CLOCK_SKEW_SECONDS + 1);
        let proof = issue_capability_token(&ISSUER_SEED, grant).expect("issue");
        let err = verify_capability_token(&proof, &trusted, &audience, &subject, now)
            .expect_err("iat beyond skew");
        assert!(matches!(err, CoreError::TokenInvalid(_)));

        // iat in the past is always accepted.
        let mut grant = claims(
            &issuer,
            &subject,
            &audience,
            &["spoke-baseline"],
            now + 3600,
        );
        grant.iat = Some(now - 10);
        let proof = issue_capability_token(&ISSUER_SEED, grant).expect("issue");
        verify_capability_token(&proof, &trusted, &audience, &subject, now)
            .expect("iat in the past");
    }

    #[test]
    fn jti_must_be_non_empty_when_present() {
        let now = 1_000_000_000u64;
        let issuer = peer_of(&ISSUER_SEED);
        let subject = peer_of(&[7u8; 32]);
        let audience = peer_of(&[8u8; 32]);
        let trusted = trusted(&[issuer.as_str()]);

        let mut grant = claims(
            &issuer,
            &subject,
            &audience,
            &["spoke-baseline"],
            now + 3600,
        );
        grant.jti = Some("token-abc-123".into());
        let proof = issue_capability_token(&ISSUER_SEED, grant).expect("issue");
        verify_capability_token(&proof, &trusted, &audience, &subject, now)
            .expect("non-empty jti is accepted (reserved, not consulted)");

        let mut grant = claims(
            &issuer,
            &subject,
            &audience,
            &["spoke-baseline"],
            now + 3600,
        );
        grant.jti = Some(String::new());
        let proof = issue_capability_token(&ISSUER_SEED, grant).expect("issue");
        let err = verify_capability_token(&proof, &trusted, &audience, &subject, now)
            .expect_err("empty jti");
        assert!(matches!(err, CoreError::TokenInvalid(_)));
    }

    #[test]
    fn proof_serializes_to_the_wire_wrapper_shape() {
        let now = 1_000_000_000u64;
        let (proof, _trusted, _peers) = happy_token(now, &["spoke-baseline"]);
        let value = serde_json::to_value(&proof).expect("serialize");
        // Wire shape: object { v, claims { … }, sig } — exactly.
        let object = value.as_object().expect("wrapper is an object");
        assert_eq!(object.len(), 3, "wrapper has exactly v, claims, sig");
        assert_eq!(value["v"], serde_json::json!(1));
        assert!(value["claims"].is_object());
        assert!(value["sig"].as_str().is_some_and(|s| !s.is_empty()));
        // The claims object carries exactly the normative claim keys.
        let claims_object = value["claims"].as_object().expect("claims object");
        for key in claims_object.keys() {
            assert!(
                ["iss", "sub", "aud", "capabilities", "exp"].contains(&key.as_str()),
                "unexpected claim key {key}"
            );
        }
    }
}
