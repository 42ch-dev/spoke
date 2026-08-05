//! Envelope authentication for post-hello connect envelopes over **raw**
//! Ed25519 key bytes (`spoke-connect-session-jcs-v1`,
//! `spoke-connect-invoke-request-jcs-v1`, `spoke-connect-invoke-response-jcs-v1`).
//!
//! Same construction as hello (`spoke-connect-hello-jcs-v1`, see
//! [`super::hello_crypto`]): the signed object — exactly the keys of the
//! locked field tables (`.mstar/specs/spoke-connect.md` §Authenticated field
//! sets, frozen by the iteration contract §3) — is canonicalized with
//! RFC 8785 JCS (`serde_jcs`) → UTF-8 bytes, signed with the sender's
//! peer-identity Ed25519 private key (the same key used for hello), and the
//! raw 64-byte signature is encoded base64url without padding (exactly 86
//! characters).
//!
//! Signed-object rules (locked):
//! - `extensions` and the `signature` field are excluded from every signed
//!   object.
//! - `ConnectInvokeRequest.auth` is included in the signed object **when
//!   present** on the wire (including explicit `null`) and absent when
//!   absent — it is trust-affecting and MUST be bound.
//! - The `ConnectInvokeResponse` signed object mirrors the wire branch
//!   exactly (`payload` success branch XOR `error` branch) — the two
//!   branches are NEVER merged into one object with optional payload/error;
//!   the discriminator is implicit in which keys are present.
//!
//! ## Verify operates on the wire form (before typed deserialization)
//!
//! [`verify_session_auth`], [`verify_invoke_request_auth`], and
//! [`verify_invoke_response_auth`] take the **parsed wire envelope** as a
//! `serde_json::Value`, not the typed schema struct. This mirrors the TS
//! implementation, whose runtime whitelists run over the wire object before
//! any typed handling, and it is what makes the fail-closed kinds of
//! contract §8 reachable: the generated typed structs enforce the schema
//! (`signature` required and exactly 86 characters, `additionalProperties:
//! false`, oneOf branches), so a missing signature, an unknown wire key, or
//! a both/neither-branch response would be rejected **by deserialization**
//! with a non-kind serde error instead of the locked machine kind. The
//! wire-form verify implements the full 7-step procedure (presence →
//! canonical base64url round-trip → exact-keys signed-object construction →
//! JCS → Ed25519 verify with the peer's hello public key → session binding)
//! so every rejection carries its locked `details.kind`; the adapter runs
//! it on the inbound wire JSON **before** typed deserialization, the
//! dispatch gate, and the inbound sequence-counter advance
//! (auth-before-advance — contract §7 amendment).
//!
//! Fail-closed kinds (contract §8):
//! [`EnvelopeAuthError::Missing`] (`envelope_auth_missing`),
//! [`EnvelopeAuthError::Invalid`] (`envelope_auth_invalid`),
//! [`EnvelopeAuthError::SessionUnbound`] (`envelope_auth_session_unbound`)
//! — all map to the wire `ErrorEnvelope.code: "auth_failed"`
//! ([`EnvelopeAuthError::CODE`]).
//!
//! Key misuse on either side (a secret or public key that is not 32 bytes,
//! or a public key that is not a valid Ed25519 point) is a **local**
//! programming error — [`EnvelopeAuthError::Crypto`] on the verify path,
//! [`CoreError::Crypto`] on the sign path — mirroring TS `CoreError("crypto")`
//! and `verifyHelloEd25519`; the key is supplied by the adapter, never the
//! wire, so it carries no envelope-auth kind and is never encoded as
//! `auth_failed`.
//!
//! Crate-private (contract §9): consumed by the remote adapter and the core
//! tests; NOT exported through the `remote-adapter` feature public surface
//! or FFI.

use crate::core::error::CoreError;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequest;
use spoke_schemas::connect::connect_invoke_response::{
    ConnectInvokeResponse, ConnectInvokeResponseVariant0ExtensionsKey,
    ConnectInvokeResponseVariant1ExtensionsKey, ErrorEnvelope,
};
use spoke_schemas::connect::connect_session::ConnectSession;
use std::collections::HashMap;

/// Algorithm id: `ConnectSession` snapshot (`spoke-connect-session-jcs-v1`).
pub(crate) const ALGORITHM_SESSION_JCS_V1: &str = "spoke-connect-session-jcs-v1";
/// Algorithm id: `ConnectInvokeRequest` (`spoke-connect-invoke-request-jcs-v1`).
pub(crate) const ALGORITHM_INVOKE_REQUEST_JCS_V1: &str =
    "spoke-connect-invoke-request-jcs-v1";
/// Algorithm id: `ConnectInvokeResponse` (`spoke-connect-invoke-response-jcs-v1`).
pub(crate) const ALGORITHM_INVOKE_RESPONSE_JCS_V1: &str =
    "spoke-connect-invoke-response-jcs-v1";

/// Stable machine kind of an envelope-auth rejection (contract §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnvelopeAuthErrorKind {
    /// Missing `signature` on a v2 envelope (also the mixed-version
    /// unauthenticated-envelope case).
    Missing,
    /// Invalid signature / non-canonical encoding / field-set drift /
    /// invalid branch shape.
    Invalid,
    /// Session-binding mismatch (`session_id` not bound, or signer not the
    /// session peer).
    SessionUnbound,
}

impl EnvelopeAuthErrorKind {
    /// The wire `details.kind` string (contract §8).
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "envelope_auth_missing",
            Self::Invalid => "envelope_auth_invalid",
            Self::SessionUnbound => "envelope_auth_session_unbound",
        }
    }
}

/// Envelope-authentication failure.
///
/// Mirrors the TS `EnvelopeAuthError` class (a single error type with a
/// stable machine `kind`) plus the TS `CoreError("crypto")` local-key-misuse
/// case: the three wire rejections carry a machine kind and map to the wire
/// `ErrorEnvelope.code: "auth_failed"` (contract §8);
/// [`EnvelopeAuthError::Crypto`] is a **local** programming error
/// (adapter-supplied key bytes of the wrong length or an invalid Ed25519
/// point) that carries NO wire kind and is never encoded as `auth_failed`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum EnvelopeAuthError {
    /// Local key misuse — mirrors TS `CoreError("crypto")`. Not a wire
    /// rejection; carries no envelope-auth kind.
    #[error("crypto: {0}")]
    Crypto(String),
    /// Missing `signature` on the wire envelope (contract §8
    /// `envelope_auth_missing`).
    #[error("envelope authentication failed (envelope_auth_missing): {0}")]
    Missing(String),
    /// Invalid signature / non-canonical encoding / field-set drift /
    /// invalid branch shape (contract §8 `envelope_auth_invalid`).
    #[error("envelope authentication failed (envelope_auth_invalid): {0}")]
    Invalid(String),
    /// Session-binding mismatch (contract §8 `envelope_auth_session_unbound`).
    #[error("envelope authentication failed (envelope_auth_session_unbound): {0}")]
    SessionUnbound(String),
}

impl EnvelopeAuthError {
    /// Wire `ErrorEnvelope.code` for every envelope-auth wire rejection.
    pub(crate) const CODE: &'static str = "auth_failed";

    /// The stable machine kind for wire rejections; `None` for the local
    /// [`EnvelopeAuthError::Crypto`] case.
    #[must_use]
    pub(crate) const fn kind(&self) -> Option<EnvelopeAuthErrorKind> {
        match self {
            Self::Crypto(_) => None,
            Self::Missing(_) => Some(EnvelopeAuthErrorKind::Missing),
            Self::Invalid(_) => Some(EnvelopeAuthErrorKind::Invalid),
            Self::SessionUnbound(_) => Some(EnvelopeAuthErrorKind::SessionUnbound),
        }
    }
}

/// Wire `extensions` bag: product namespace → opaque JSON object
/// (structurally identical to the schema `ExtensionMap`; mirrors TS
/// `EnvelopeExtensions`). Keys are validated against the schema pattern
/// `^[a-z][a-z0-9_-]*$` when the typed wire envelope is built.
pub(crate) type EnvelopeExtensions =
    HashMap<String, serde_json::Map<String, serde_json::Value>>;

/// Input to [`authenticate_session`]: the session snapshot fields covered
/// by the signature (mirrors TS `SessionSignInput`). Serializing this with
/// JCS produces the exact bytes the session signature covers.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct SessionSignInput {
    pub(crate) session_id: String,
    pub(crate) initiator_peer_id: String,
    pub(crate) responder_peer_id: String,
    pub(crate) opened_at: chrono::DateTime<chrono::Utc>,
    pub(crate) negotiated_capabilities: Vec<String>,
    pub(crate) initial_sequence: u64,
}

/// Input to [`authenticate_invoke_request`]: the request fields covered by
/// the signature (`auth` optional — included in the signed object only when
/// present on the wire; mirrors TS `InvokeRequestSignInput`). Serializing
/// this with JCS produces the exact bytes the request signature covers.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct InvokeRequestSignInput {
    pub(crate) session_id: String,
    pub(crate) sequence: i64,
    pub(crate) request_id: String,
    pub(crate) op: String,
    pub(crate) payload: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) auth: Option<serde_json::Value>,
}

/// Input to [`authenticate_invoke_response`]: exactly one of the two
/// response branches (mirrors TS `InvokeResponseSignInput`).
#[derive(Debug, Clone)]
pub(crate) enum InvokeResponseSignInput {
    /// Success branch — signed object `{session_id, sequence, request_id,
    /// payload}`.
    Success {
        session_id: String,
        sequence: i64,
        request_id: String,
        payload: serde_json::Value,
    },
    /// Error branch — signed object `{session_id, sequence, request_id,
    /// error}`.
    Error {
        session_id: String,
        sequence: i64,
        request_id: String,
        error: ErrorEnvelope,
    },
}

/// The exact signed object for the invoke-response success branch.
#[derive(Debug, serde::Serialize)]
struct SignedInvokeResponseSuccess<'a> {
    session_id: &'a str,
    sequence: i64,
    request_id: &'a str,
    payload: &'a serde_json::Value,
}

/// The exact signed object for the invoke-response error branch.
#[derive(Debug, serde::Serialize)]
struct SignedInvokeResponseError<'a> {
    session_id: &'a str,
    sequence: i64,
    request_id: &'a str,
    error: &'a ErrorEnvelope,
}

// ── wire key sets (runtime whitelists, mirror the locked field tables) ─────

/// Allowed wire keys of a `ConnectSession` envelope.
const SESSION_WIRE_KEYS: &[&str] = &[
    "session_id",
    "initiator_peer_id",
    "responder_peer_id",
    "opened_at",
    "negotiated_capabilities",
    "initial_sequence",
    "signature",
    "extensions",
];

/// Signed fields of a `ConnectSession` (exactly these keys, no others).
const SESSION_SIGNED_KEYS: &[&str] = &[
    "session_id",
    "initiator_peer_id",
    "responder_peer_id",
    "opened_at",
    "negotiated_capabilities",
    "initial_sequence",
];

/// Allowed wire keys of a `ConnectInvokeRequest` envelope (`auth` optional).
const INVOKE_REQUEST_WIRE_KEYS: &[&str] = &[
    "session_id",
    "sequence",
    "request_id",
    "op",
    "payload",
    "auth",
    "signature",
    "extensions",
];

/// Signed fields of a `ConnectInvokeRequest` (`auth` conditional, checked
/// separately).
const INVOKE_REQUEST_SIGNED_KEYS: &[&str] =
    &["session_id", "sequence", "request_id", "op", "payload"];

/// Allowed wire keys of a `ConnectInvokeResponse` success branch.
const INVOKE_RESPONSE_SUCCESS_WIRE_KEYS: &[&str] = &[
    "session_id",
    "sequence",
    "request_id",
    "payload",
    "signature",
    "extensions",
];

/// Allowed wire keys of a `ConnectInvokeResponse` error branch.
const INVOKE_RESPONSE_ERROR_WIRE_KEYS: &[&str] = &[
    "session_id",
    "sequence",
    "request_id",
    "error",
    "signature",
    "extensions",
];

/// Signed fields common to both `ConnectInvokeResponse` branches (branch
/// key checked separately).
const INVOKE_RESPONSE_SIGNED_KEYS: &[&str] = &["session_id", "sequence", "request_id"];

// ── signed-object construction (exact keys, no others) ─────────────────────

/// Fail-closed wire-shape guard: every wire key must be in the allowed set
/// (unknown keys = field-set drift ⇒ `envelope_auth_invalid`) and every
/// signed field must be present (a missing signed field is a malformed
/// signed object ⇒ `envelope_auth_invalid`). Mirrors the TS
/// `assertWireKeys` runtime whitelist.
fn assert_wire_keys(
    wire: &serde_json::Value,
    allowed: &[&str],
    required: &[&str],
) -> Result<(), EnvelopeAuthError> {
    let Some(object) = wire.as_object() else {
        return Err(EnvelopeAuthError::Invalid(
            "signed envelope is not a JSON object".into(),
        ));
    };
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(EnvelopeAuthError::Invalid(format!(
                "unknown key {key} in signed envelope (field-set drift)"
            )));
        }
    }
    for key in required {
        if !object.contains_key(*key) {
            return Err(EnvelopeAuthError::Invalid(format!(
                "missing signed field {key}"
            )));
        }
    }
    Ok(())
}

/// The session signed object: exactly the six locked session fields.
fn session_signed_object(wire: &serde_json::Value) -> Result<serde_json::Value, EnvelopeAuthError> {
    assert_wire_keys(wire, SESSION_WIRE_KEYS, SESSION_SIGNED_KEYS)?;
    let object = wire.as_object().expect("object checked by assert_wire_keys");
    let mut signed = serde_json::Map::new();
    for key in SESSION_SIGNED_KEYS {
        signed.insert((*key).to_string(), object[*key].clone());
    }
    Ok(serde_json::Value::Object(signed))
}

/// The invoke-request signed object: five locked fields + `auth` when
/// present (including explicit `null`).
fn invoke_request_signed_object(
    wire: &serde_json::Value,
) -> Result<serde_json::Value, EnvelopeAuthError> {
    assert_wire_keys(wire, INVOKE_REQUEST_WIRE_KEYS, INVOKE_REQUEST_SIGNED_KEYS)?;
    let object = wire.as_object().expect("object checked by assert_wire_keys");
    let mut signed = serde_json::Map::new();
    for key in INVOKE_REQUEST_SIGNED_KEYS {
        signed.insert((*key).to_string(), object[*key].clone());
    }
    // `auth` is trust-affecting and MUST be bound: included in the signed
    // object when present on the wire (including explicit null), absent
    // when absent.
    if let Some(auth) = object.get("auth") {
        signed.insert("auth".to_string(), auth.clone());
    }
    Ok(serde_json::Value::Object(signed))
}

/// The invoke-response signed object: mirrors the wire branch exactly
/// (`payload` success branch XOR `error` branch, branch-specific key
/// whitelist). The branches are never merged.
fn invoke_response_signed_object(
    wire: &serde_json::Value,
) -> Result<serde_json::Value, EnvelopeAuthError> {
    let Some(object) = wire.as_object() else {
        return Err(EnvelopeAuthError::Invalid(
            "signed envelope is not a JSON object".into(),
        ));
    };
    let has_payload = object.contains_key("payload");
    let has_error = object.contains_key("error");
    if has_payload == has_error {
        return Err(EnvelopeAuthError::Invalid(
            "invoke response must carry exactly one of payload or error".into(),
        ));
    }
    let (allowed, branch_key) = if has_payload {
        (INVOKE_RESPONSE_SUCCESS_WIRE_KEYS, "payload")
    } else {
        (INVOKE_RESPONSE_ERROR_WIRE_KEYS, "error")
    };
    let mut required = INVOKE_RESPONSE_SIGNED_KEYS.to_vec();
    required.push(branch_key);
    assert_wire_keys(wire, allowed, &required)?;
    let mut signed = serde_json::Map::new();
    for key in INVOKE_RESPONSE_SIGNED_KEYS {
        signed.insert((*key).to_string(), object[*key].clone());
    }
    signed.insert(branch_key.to_string(), object[branch_key].clone());
    Ok(serde_json::Value::Object(signed))
}

// ── JCS + signature primitives ─────────────────────────────────────────────

/// Sign the JCS bytes of `signed_object` with a raw 32-byte Ed25519 secret
/// key → canonical base64url. Mirrors TS `signEnvelope`: a non-32-byte
/// secret is adapter-supplied misuse → `CoreError::Crypto`.
fn sign_envelope(
    secret: &[u8],
    signed_object: &impl serde::Serialize,
) -> Result<String, CoreError> {
    if secret.len() != 32 {
        return Err(CoreError::Crypto("secret must be 32 bytes".into()));
    }
    let signing_key = SigningKey::from_bytes(secret.try_into().expect("length checked above"));
    let bytes = serde_jcs::to_vec(signed_object).map_err(|e| CoreError::Jcs(e.to_string()))?;
    let signature: Signature = signing_key.sign(&bytes);
    Ok(URL_SAFE_NO_PAD.encode(signature.to_bytes()))
}

/// Verify steps 1–2 (locked order): the `signature` field must be present
/// and non-empty (`envelope_auth_missing`), then it must decode and
/// round-trip through `encode(decode(sig)) === sig` — the unique canonical
/// RFC 4648 base64url no-padding encoding, rejecting alternate encodings of
/// the final character's slack bits and padded input — and be exactly 64
/// bytes (`envelope_auth_invalid`). (Rust's `URL_SAFE_NO_PAD` strict decoder
/// already rejects non-zero slack bits at decode, so on this side the
/// round-trip is defense-in-depth — it preserves source-level parity with
/// TS, whose atob-based decoder is slack-lenient.)
fn require_signature(wire: &serde_json::Value) -> Result<[u8; 64], EnvelopeAuthError> {
    let signature_field = wire.get("signature");
    let Some(serde_json::Value::String(signature)) = signature_field else {
        return Err(EnvelopeAuthError::Missing(
            "envelope is missing a signature".into(),
        ));
    };
    if signature.is_empty() {
        return Err(EnvelopeAuthError::Missing(
            "envelope is missing a signature".into(),
        ));
    }
    let raw = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| EnvelopeAuthError::Invalid("signature is not valid base64url".into()))?;
    if URL_SAFE_NO_PAD.encode(&raw) != signature.as_str() {
        return Err(EnvelopeAuthError::Invalid(
            "signature is not canonical base64url (no padding)".into(),
        ));
    }
    raw.try_into().map_err(|_| {
        EnvelopeAuthError::Invalid(
            "signature is not 64 bytes (86-char base64url expected)".into(),
        )
    })
}

/// Verify steps 4–5 (locked order): JCS-canonicalize the signed object and
/// Ed25519-verify the decoded signature against the peer's hello public
/// key. A non-32-byte public key is adapter-supplied misuse →
/// [`EnvelopeAuthError::Crypto`] (mirrors TS `CoreError("crypto")` via
/// `verifyHelloEd25519`), not a wire rejection.
fn verify_canonical_signature(
    public_key: &[u8],
    signed_object: &serde_json::Value,
    signature: &[u8; 64],
) -> Result<(), EnvelopeAuthError> {
    if public_key.len() != 32 {
        return Err(EnvelopeAuthError::Crypto("public key must be 32 bytes".into()));
    }
    let verifying_key = VerifyingKey::from_bytes(public_key.try_into().expect("length checked above"))
        .map_err(|e| EnvelopeAuthError::Crypto(format!("invalid Ed25519 public key: {e}")))?;
    let bytes = serde_jcs::to_vec(signed_object).map_err(|e| {
        EnvelopeAuthError::Invalid(format!("signed object is not JSON-serializable: {e}"))
    })?;
    let signature = Signature::from_bytes(signature);
    if verifying_key.verify(&bytes, &signature).is_err() {
        return Err(EnvelopeAuthError::Invalid("signature does not verify".into()));
    }
    Ok(())
}

// ── ConnectSession ─────────────────────────────────────────────────────────

/// Sign a `ConnectSession` snapshot with a raw 32-byte Ed25519 secret key,
/// producing the full wire envelope (`spoke-connect-session-jcs-v1`). The
/// signed object covers exactly `{session_id, initiator_peer_id,
/// responder_peer_id, opened_at, negotiated_capabilities, initial_sequence}`;
/// `extensions` are carried on the wire envelope but never signed.
pub(crate) fn authenticate_session(
    secret: &[u8],
    session: &SessionSignInput,
    extensions: EnvelopeExtensions,
) -> Result<ConnectSession, CoreError> {
    let signature = sign_envelope(secret, session)?;
    Ok(ConnectSession {
        session_id: session
            .session_id
            .parse()
            .expect("non-empty session_id parses"),
        initiator_peer_id: session
            .initiator_peer_id
            .parse()
            .expect("non-empty initiator_peer_id parses"),
        responder_peer_id: session
            .responder_peer_id
            .parse()
            .expect("non-empty responder_peer_id parses"),
        opened_at: session.opened_at,
        negotiated_capabilities: session.negotiated_capabilities.clone(),
        initial_sequence: session.initial_sequence,
        signature: signature.parse().expect("86-char base64url signature parses"),
        extensions: extensions
            .into_iter()
            .map(|(key, value)| {
                (
                    key.parse()
                        .expect("extension key matches ^[a-z][a-z0-9_-]*$"),
                    value,
                )
            })
            .collect(),
    })
}

/// Verify a received `ConnectSession` against a raw Ed25519 public key (the
/// emitter's hello key), over the **wire form** (see module docs). Fail-closed
/// per contract §7; on success additionally asserts the snapshot's
/// `initiator_peer_id` / `responder_peer_id` match the authenticated hellos
/// of the current session (step 6 — mismatch is
/// `envelope_auth_session_unbound`, after the signature has verified).
pub(crate) fn verify_session_auth(
    public_key: &[u8],
    session: &serde_json::Value,
    expected_initiator_peer_id: &str,
    expected_responder_peer_id: &str,
) -> Result<(), EnvelopeAuthError> {
    let signature = require_signature(session)?;
    let signed_object = session_signed_object(session)?;
    verify_canonical_signature(public_key, &signed_object, &signature)?;
    // Binding fields are strings on every schema-valid envelope, but verify
    // runs on the raw wire form before typed deserialization: a key-holding
    // signer could craft a signed envelope with a non-string peer id, which
    // must fail closed (session_unbound — a non-string can never match the
    // authenticated hellos), never panic.
    let initiator_peer_id = session
        .get("initiator_peer_id")
        .and_then(serde_json::Value::as_str);
    let responder_peer_id = session
        .get("responder_peer_id")
        .and_then(serde_json::Value::as_str);
    if initiator_peer_id != Some(expected_initiator_peer_id)
        || responder_peer_id != Some(expected_responder_peer_id)
    {
        let initiator_peer_id = initiator_peer_id.unwrap_or("<non-string>");
        let responder_peer_id = responder_peer_id.unwrap_or("<non-string>");
        return Err(EnvelopeAuthError::SessionUnbound(format!(
            "session peer ids ({initiator_peer_id}, {responder_peer_id}) do not match the authenticated hellos ({expected_initiator_peer_id}, {expected_responder_peer_id})"
        )));
    }
    Ok(())
}

// ── ConnectInvokeRequest ───────────────────────────────────────────────────

/// Sign a `ConnectInvokeRequest` with a raw 32-byte Ed25519 secret key,
/// producing the full wire envelope (`spoke-connect-invoke-request-jcs-v1`).
/// The signed object covers `{session_id, sequence, request_id, op,
/// payload}` plus `auth` **only when present** on the input (conditional
/// inclusion — `auth` is trust-affecting and MUST be bound).
pub(crate) fn authenticate_invoke_request(
    secret: &[u8],
    request: &InvokeRequestSignInput,
    extensions: EnvelopeExtensions,
) -> Result<ConnectInvokeRequest, CoreError> {
    let signature = sign_envelope(secret, request)?;
    Ok(ConnectInvokeRequest {
        auth: request.auth.clone(),
        op: request.op.parse().expect("non-empty op parses"),
        payload: request.payload.clone(),
        request_id: request
            .request_id
            .parse()
            .expect("non-empty request_id parses"),
        sequence: request.sequence,
        session_id: request
            .session_id
            .parse()
            .expect("non-empty session_id parses"),
        signature: signature.parse().expect("86-char base64url signature parses"),
        extensions: extensions
            .into_iter()
            .map(|(key, value)| {
                (
                    key.parse()
                        .expect("extension key matches ^[a-z][a-z0-9_-]*$"),
                    value,
                )
            })
            .collect(),
    })
}

/// Verify a received `ConnectInvokeRequest` against a raw Ed25519 public key
/// (the session peer's hello key), over the **wire form** (see module docs).
/// Fail-closed per contract §7; on success additionally asserts the
/// envelope's `session_id` equals the session bound at establish (step 6 —
/// `envelope_auth_session_unbound` on mismatch; the adapter resolves the
/// peer key from the bound session, so the signer-is-session-peer check is
/// the key itself).
pub(crate) fn verify_invoke_request_auth(
    public_key: &[u8],
    request: &serde_json::Value,
    expected_session_id: &str,
) -> Result<(), EnvelopeAuthError> {
    let signature = require_signature(request)?;
    let signed_object = invoke_request_signed_object(request)?;
    verify_canonical_signature(public_key, &signed_object, &signature)?;
    // Fail closed on a non-string session_id (raw wire form — see
    // verify_session_auth): a non-string can never match the bound session.
    let session_id = request
        .get("session_id")
        .and_then(serde_json::Value::as_str);
    if session_id != Some(expected_session_id) {
        let session_id = session_id.unwrap_or("<non-string>");
        return Err(EnvelopeAuthError::SessionUnbound(format!(
            "session_id {session_id} is not bound to session {expected_session_id}"
        )));
    }
    Ok(())
}

// ── ConnectInvokeResponse ──────────────────────────────────────────────────

/// Sign a `ConnectInvokeResponse` with a raw 32-byte Ed25519 secret key,
/// producing the full wire envelope (`spoke-connect-invoke-response-jcs-v1`).
/// The signed object mirrors the wire branch exactly — `{session_id,
/// sequence, request_id, payload}` for the success branch, `{session_id,
/// sequence, request_id, error}` for the error branch. The branches are
/// never merged.
pub(crate) fn authenticate_invoke_response(
    secret: &[u8],
    response: &InvokeResponseSignInput,
    extensions: EnvelopeExtensions,
) -> Result<ConnectInvokeResponse, CoreError> {
    match response {
        InvokeResponseSignInput::Success {
            session_id,
            sequence,
            request_id,
            payload,
        } => {
            let signature = sign_envelope(
                secret,
                &SignedInvokeResponseSuccess {
                    session_id,
                    sequence: *sequence,
                    request_id,
                    payload,
                },
            )?;
            Ok(ConnectInvokeResponse::Variant0 {
                session_id: session_id.clone(),
                sequence: *sequence,
                request_id: request_id.clone(),
                payload: payload.clone(),
                signature: signature.parse().expect("86-char base64url signature parses"),
                extensions: convert_extensions::<ConnectInvokeResponseVariant0ExtensionsKey>(
                    extensions,
                ),
            })
        }
        InvokeResponseSignInput::Error {
            session_id,
            sequence,
            request_id,
            error,
        } => {
            let signature = sign_envelope(
                secret,
                &SignedInvokeResponseError {
                    session_id,
                    sequence: *sequence,
                    request_id,
                    error,
                },
            )?;
            Ok(ConnectInvokeResponse::Variant1 {
                session_id: session_id.clone(),
                sequence: *sequence,
                request_id: request_id.clone(),
                error: error.clone(),
                signature: signature.parse().expect("86-char base64url signature parses"),
                extensions: convert_extensions::<ConnectInvokeResponseVariant1ExtensionsKey>(
                    extensions,
                ),
            })
        }
    }
}

/// Convert the uniform `EnvelopeExtensions` map to a schema-keyed extension
/// map. Extension keys are wire keys constrained by the schema pattern, so
/// the parse is infallible for schema-valid callers (mirrors the hello
/// `.expect(...)` construction pattern).
fn convert_extensions<K>(
    extensions: EnvelopeExtensions,
) -> HashMap<K, serde_json::Map<String, serde_json::Value>>
where
    K: std::str::FromStr + std::hash::Hash + Eq,
    <K as std::str::FromStr>::Err: std::fmt::Debug,
{
    extensions
        .into_iter()
        .map(|(key, value)| {
            (
                key.parse().expect("extension key matches ^[a-z][a-z0-9_-]*$"),
                value,
            )
        })
        .collect()
}

/// Verify a received `ConnectInvokeResponse` against a raw Ed25519 public
/// key (the session peer's hello key), over the **wire form** (see module
/// docs). Fail-closed per contract §7: the response must carry exactly one
/// of `payload` / `error` (never both — the signed object mirrors the wire
/// branch, never a merged object), the signed object must have exact keys
/// for that branch, and on success the envelope's `session_id` must equal
/// the bound session (step 6).
pub(crate) fn verify_invoke_response_auth(
    public_key: &[u8],
    response: &serde_json::Value,
    expected_session_id: &str,
) -> Result<(), EnvelopeAuthError> {
    let signature = require_signature(response)?;
    let signed_object = invoke_response_signed_object(response)?;
    verify_canonical_signature(public_key, &signed_object, &signature)?;
    // Fail closed on a non-string session_id (raw wire form — see
    // verify_session_auth): a non-string can never match the bound session.
    let session_id = response
        .get("session_id")
        .and_then(serde_json::Value::as_str);
    if session_id != Some(expected_session_id) {
        let session_id = session_id.unwrap_or("<non-string>");
        return Err(EnvelopeAuthError::SessionUnbound(format!(
            "session_id {session_id} is not bound to session {expected_session_id}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // Deterministic test fixtures mirroring the TS unit-test convention
    // (`tests/core/envelope-auth.test.ts`): the envelope emitter is seed
    // `[7u8; 32]`; a distinct key `[8u8; 32]` stands in for a wrong peer /
    // tamper key; the two session peers are `[9u8; 32]` and `[10u8; 32]`.
    // Seeds are produced by a parameterized helper so no key material
    // appears as a literal in a crypto call site (CodeQL fixture rule —
    // same convention as the hello tests' `test_nonce`).
    use super::*;
    use crate::core::peer_id::derive_peer_id_from_ed25519_pubkey;

    const SESSION_ID: &str = "session-0001";

    fn seed(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    fn emitter_seed() -> [u8; 32] {
        seed(7)
    }

    fn other_seed() -> [u8; 32] {
        seed(8)
    }

    fn initiator_seed() -> [u8; 32] {
        seed(9)
    }

    fn responder_seed() -> [u8; 32] {
        seed(10)
    }

    fn pubkey_of(seed: &[u8; 32]) -> [u8; 32] {
        SigningKey::from_bytes(seed).verifying_key().to_bytes()
    }

    fn emitter_pubkey() -> [u8; 32] {
        pubkey_of(&emitter_seed())
    }

    fn other_pubkey() -> [u8; 32] {
        pubkey_of(&other_seed())
    }

    fn peer_id_of(seed: &[u8; 32]) -> String {
        derive_peer_id_from_ed25519_pubkey(&pubkey_of(seed))
    }

    fn initiator_peer_id() -> String {
        peer_id_of(&initiator_seed())
    }

    fn responder_peer_id() -> String {
        peer_id_of(&responder_seed())
    }

    fn opened_at() -> chrono::DateTime<chrono::Utc> {
        "2026-08-05T00:00:00Z".parse().expect("RFC 3339 timestamp")
    }

    /// The locked session signed-field set as a sign input.
    fn session_input() -> SessionSignInput {
        SessionSignInput {
            session_id: SESSION_ID.to_string(),
            initiator_peer_id: initiator_peer_id(),
            responder_peer_id: responder_peer_id(),
            opened_at: opened_at(),
            negotiated_capabilities: vec!["spoke-connect".to_string()],
            initial_sequence: 0,
        }
    }

    /// A request sign input; `extra` lets tests add/override `auth`.
    fn request_input(auth: Option<serde_json::Value>) -> InvokeRequestSignInput {
        InvokeRequestSignInput {
            session_id: SESSION_ID.to_string(),
            sequence: 0,
            request_id: "req-0001".to_string(),
            op: "upsert".to_string(),
            payload: serde_json::json!({
                "collection": "notes",
                "id": "n1",
                "value": { "title": "hello" },
            }),
            auth,
        }
    }

    /// Success-branch response sign input (oneOf branch 1).
    fn response_success_input() -> InvokeResponseSignInput {
        InvokeResponseSignInput::Success {
            session_id: SESSION_ID.to_string(),
            sequence: 0,
            request_id: "req-0001".to_string(),
            payload: serde_json::json!({ "ok": true, "result": { "id": "n1" } }),
        }
    }

    /// Error-branch response sign input (oneOf branch 2).
    fn response_error_input() -> InvokeResponseSignInput {
        InvokeResponseSignInput::Error {
            session_id: SESSION_ID.to_string(),
            sequence: 0,
            request_id: "req-0001".to_string(),
            error: ErrorEnvelope {
                code: "op_unsupported".to_string(),
                details: Default::default(),
                extensions: Default::default(),
                message: "unknown op".to_string(),
            },
        }
    }

    /// Serialize a typed wire envelope to the wire form the verify helpers
    /// consume (the adapter path: build → serialize → send → receive →
    /// parse → verify).
    fn wire_of(value: impl serde::Serialize) -> serde_json::Value {
        serde_json::to_value(value).expect("wire envelope serializes")
    }

    /// Assert a verify call rejects with exactly the locked machine kind.
    fn expect_kind(result: Result<(), EnvelopeAuthError>, kind: EnvelopeAuthErrorKind) {
        let err = result.expect_err("expected an envelope-auth rejection");
        assert_eq!(err.kind(), Some(kind), "unexpected error: {err}");
    }

    /// Swap the final base64url char for a sibling encoding the same 64
    /// bytes with non-zero slack bits (mirrors the TS test helper).
    fn non_canonical_signature(encoded: &str) -> String {
        let last = encoded.chars().last().expect("non-empty signature");
        let sibling = match last {
            'A' => 'B',
            'Q' => 'R',
            'g' => 'h',
            'w' => 'x',
            _ => 'A',
        };
        let mut out = encoded[..encoded.len() - 1].to_string();
        out.push(sibling);
        out
    }

    fn assert_86_char_base64url(signature: &str) {
        assert_eq!(signature.len(), 86, "signature length");
        assert!(
            signature
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')),
            "signature is base64url: {signature}"
        );
    }

    #[test]
    fn exposes_the_three_locked_algorithm_ids_verbatim() {
        assert_eq!(ALGORITHM_SESSION_JCS_V1, "spoke-connect-session-jcs-v1");
        assert_eq!(
            ALGORITHM_INVOKE_REQUEST_JCS_V1,
            "spoke-connect-invoke-request-jcs-v1"
        );
        assert_eq!(
            ALGORITHM_INVOKE_RESPONSE_JCS_V1,
            "spoke-connect-invoke-response-jcs-v1"
        );
    }

    #[test]
    fn session_signed_object_contains_exactly_the_locked_keys() {
        let session =
            authenticate_session(&emitter_seed(), &session_input(), Default::default())
                .expect("sign session");
        let wire = wire_of(&session);
        let signed = session_signed_object(&wire).expect("signed object");
        let bytes = serde_jcs::to_vec(&signed).expect("JCS");
        let jcs = std::str::from_utf8(&bytes).expect("UTF-8 JCS");
        let expected = format!(
            r#"{{"initial_sequence":0,"initiator_peer_id":"{}","negotiated_capabilities":["spoke-connect"],"opened_at":"2026-08-05T00:00:00Z","responder_peer_id":"{}","session_id":"session-0001"}}"#,
            initiator_peer_id(),
            responder_peer_id(),
        );
        assert_eq!(jcs, expected, "signed object is exactly the six locked keys, JCS-ordered");
        // The signature bytes the signer produced verify against these exact
        // canonical bytes — the happy-path verify below proves it end to end.
    }

    #[test]
    fn session_signs_to_86_char_signature_and_verifies_happy_path() {
        let session =
            authenticate_session(&emitter_seed(), &session_input(), Default::default())
                .expect("sign session");
        assert_86_char_base64url(session.signature.as_str());
        assert!(session.extensions.is_empty());
        verify_session_auth(
            &emitter_pubkey(),
            &wire_of(&session),
            &initiator_peer_id(),
            &responder_peer_id(),
        )
        .expect("verify with the emitter hello key + authenticated hellos");
    }

    #[test]
    fn session_rejects_a_tampered_signed_field() {
        let session =
            authenticate_session(&emitter_seed(), &session_input(), Default::default())
                .expect("sign session");
        let mut tampered = wire_of(&session);
        tampered["opened_at"] = serde_json::json!("2026-08-06T00:00:00Z");
        expect_kind(
            verify_session_auth(
                &emitter_pubkey(),
                &tampered,
                &initiator_peer_id(),
                &responder_peer_id(),
            ),
            EnvelopeAuthErrorKind::Invalid,
        );
    }

    #[test]
    fn session_rejects_a_missing_signature() {
        let session =
            authenticate_session(&emitter_seed(), &session_input(), Default::default())
                .expect("sign session");
        let mut missing = wire_of(&session);
        missing
            .as_object_mut()
            .expect("wire object")
            .remove("signature");
        expect_kind(
            verify_session_auth(
                &emitter_pubkey(),
                &missing,
                &initiator_peer_id(),
                &responder_peer_id(),
            ),
            EnvelopeAuthErrorKind::Missing,
        );
    }

    #[test]
    fn session_rejects_an_empty_signature_as_missing() {
        let session =
            authenticate_session(&emitter_seed(), &session_input(), Default::default())
                .expect("sign session");
        let mut empty = wire_of(&session);
        empty
            .as_object_mut()
            .expect("wire object")
            .insert("signature".to_string(), serde_json::Value::String(String::new()));
        expect_kind(
            verify_session_auth(
                &emitter_pubkey(),
                &empty,
                &initiator_peer_id(),
                &responder_peer_id(),
            ),
            EnvelopeAuthErrorKind::Missing,
        );
    }

    #[test]
    fn session_rejects_a_non_canonical_signature_encoding() {
        let session =
            authenticate_session(&emitter_seed(), &session_input(), Default::default())
                .expect("sign session");
        let mut non_canonical = wire_of(&session);
        non_canonical["signature"] = serde_json::json!(
            non_canonical_signature(session.signature.as_str())
        );
        expect_kind(
            verify_session_auth(
                &emitter_pubkey(),
                &non_canonical,
                &initiator_peer_id(),
                &responder_peer_id(),
            ),
            EnvelopeAuthErrorKind::Invalid,
        );
    }

    #[test]
    fn session_rejects_a_signature_verified_with_the_wrong_public_key() {
        let session =
            authenticate_session(&emitter_seed(), &session_input(), Default::default())
                .expect("sign session");
        expect_kind(
            verify_session_auth(
                &other_pubkey(),
                &wire_of(&session),
                &initiator_peer_id(),
                &responder_peer_id(),
            ),
            EnvelopeAuthErrorKind::Invalid,
        );
    }

    #[test]
    fn session_rejects_peer_ids_that_do_not_match_the_authenticated_hellos() {
        let session =
            authenticate_session(&emitter_seed(), &session_input(), Default::default())
                .expect("sign session");
        // Signature verifies (emitter key) but the snapshot's peer ids do
        // not match the authenticated hellos of the current session.
        expect_kind(
            verify_session_auth(
                &emitter_pubkey(),
                &wire_of(&session),
                &peer_id_of(&other_seed()),
                &responder_peer_id(),
            ),
            EnvelopeAuthErrorKind::SessionUnbound,
        );
    }

    #[test]
    fn session_rejects_non_string_peer_ids_as_unbound_without_panicking() {
        // Verify runs on the raw wire form before typed deserialization, so
        // a key-holding signer can craft a signed envelope whose binding
        // fields are not strings. That must fail closed (session_unbound —
        // TS compares the same way) and never panic.
        let session =
            authenticate_session(&emitter_seed(), &session_input(), Default::default())
                .expect("sign session");
        let mut wire = wire_of(&session);
        wire["initiator_peer_id"] = serde_json::json!(123);
        let signed_object = session_signed_object(&wire).expect("signed object");
        let signature = sign_envelope(&emitter_seed(), &signed_object).expect("re-sign");
        wire["signature"] = serde_json::json!(signature);
        expect_kind(
            verify_session_auth(
                &emitter_pubkey(),
                &wire,
                &initiator_peer_id(),
                &responder_peer_id(),
            ),
            EnvelopeAuthErrorKind::SessionUnbound,
        );
    }

    #[test]
    fn session_rejects_an_unknown_wire_key() {
        let session =
            authenticate_session(&emitter_seed(), &session_input(), Default::default())
                .expect("sign session");
        let mut drifted = wire_of(&session);
        drifted
            .as_object_mut()
            .expect("wire object")
            .insert("rogue".to_string(), serde_json::json!("field"));
        expect_kind(
            verify_session_auth(
                &emitter_pubkey(),
                &drifted,
                &initiator_peer_id(),
                &responder_peer_id(),
            ),
            EnvelopeAuthErrorKind::Invalid,
        );
    }

    #[test]
    fn session_carries_the_locked_machine_shape() {
        let session =
            authenticate_session(&emitter_seed(), &session_input(), Default::default())
                .expect("sign session");
        let err = verify_session_auth(
            &other_pubkey(),
            &wire_of(&session),
            &initiator_peer_id(),
            &responder_peer_id(),
        )
        .expect_err("wrong key");
        assert_eq!(EnvelopeAuthError::CODE, "auth_failed");
        assert_eq!(err.kind(), Some(EnvelopeAuthErrorKind::Invalid));
        assert_eq!(
            err.kind().map(EnvelopeAuthErrorKind::as_str),
            Some("envelope_auth_invalid")
        );
    }

    #[test]
    fn request_signs_with_auth_and_verifies_auth_bound_when_present() {
        let auth = serde_json::json!({ "method": "capability-token", "sig": "opaque" });
        let request = authenticate_invoke_request(
            &emitter_seed(),
            &request_input(Some(auth.clone())),
            Default::default(),
        )
        .expect("sign request");
        assert_eq!(request.auth, Some(auth));
        assert_86_char_base64url(request.signature.as_str());
        verify_invoke_request_auth(&emitter_pubkey(), &wire_of(&request), SESSION_ID)
            .expect("verify");
    }

    #[test]
    fn request_signs_without_auth_and_verifies_auth_absent() {
        let request = authenticate_invoke_request(
            &emitter_seed(),
            &request_input(None),
            Default::default(),
        )
        .expect("sign request");
        assert!(request.auth.is_none());
        verify_invoke_request_auth(&emitter_pubkey(), &wire_of(&request), SESSION_ID)
            .expect("verify");
    }

    #[test]
    fn request_signs_with_scalar_auth_and_verifies_opaque_json_bound_verbatim() {
        let auth = serde_json::json!("opaque-token");
        let request = authenticate_invoke_request(
            &emitter_seed(),
            &request_input(Some(auth.clone())),
            Default::default(),
        )
        .expect("sign request");
        assert_eq!(request.auth, Some(auth));
        verify_invoke_request_auth(&emitter_pubkey(), &wire_of(&request), SESSION_ID)
            .expect("verify");
    }

    #[test]
    fn request_signs_with_null_auth_and_verifies_opaque_json_bound_verbatim() {
        let request = authenticate_invoke_request(
            &emitter_seed(),
            &request_input(Some(serde_json::Value::Null)),
            Default::default(),
        )
        .expect("sign request");
        assert_eq!(request.auth, Some(serde_json::Value::Null));
        verify_invoke_request_auth(&emitter_pubkey(), &wire_of(&request), SESSION_ID)
            .expect("verify");
    }

    #[test]
    fn request_signs_with_array_auth_and_verifies_opaque_json_bound_verbatim() {
        let auth = serde_json::json!(["capability-token", { "sig": "opaque" }]);
        let request = authenticate_invoke_request(
            &emitter_seed(),
            &request_input(Some(auth.clone())),
            Default::default(),
        )
        .expect("sign request");
        assert_eq!(request.auth, Some(auth));
        verify_invoke_request_auth(&emitter_pubkey(), &wire_of(&request), SESSION_ID)
            .expect("verify");
    }

    #[test]
    fn request_rejects_a_tampered_scalar_auth() {
        let request = authenticate_invoke_request(
            &emitter_seed(),
            &request_input(Some(serde_json::json!("opaque-token"))),
            Default::default(),
        )
        .expect("sign request");
        let mut tampered = wire_of(&request);
        tampered["auth"] = serde_json::json!("forged-token");
        expect_kind(
            verify_invoke_request_auth(&emitter_pubkey(), &tampered, SESSION_ID),
            EnvelopeAuthErrorKind::Invalid,
        );
    }

    #[test]
    fn request_rejects_a_tampered_payload() {
        let request = authenticate_invoke_request(
            &emitter_seed(),
            &request_input(None),
            Default::default(),
        )
        .expect("sign request");
        let mut tampered = wire_of(&request);
        tampered["payload"] = serde_json::json!({
            "collection": "notes",
            "id": "n1",
            "value": { "title": "tampered" },
        });
        expect_kind(
            verify_invoke_request_auth(&emitter_pubkey(), &tampered, SESSION_ID),
            EnvelopeAuthErrorKind::Invalid,
        );
    }

    #[test]
    fn request_rejects_a_tampered_auth_field_auth_is_trust_affecting_and_bound() {
        let request = authenticate_invoke_request(
            &emitter_seed(),
            &request_input(Some(serde_json::json!({
                "method": "capability-token",
                "sig": "opaque",
            }))),
            Default::default(),
        )
        .expect("sign request");
        let mut tampered = wire_of(&request);
        tampered["auth"] = serde_json::json!({
            "method": "capability-token",
            "sig": "forged",
        });
        expect_kind(
            verify_invoke_request_auth(&emitter_pubkey(), &tampered, SESSION_ID),
            EnvelopeAuthErrorKind::Invalid,
        );
    }

    #[test]
    fn request_rejects_auth_stripped_after_signing() {
        let request = authenticate_invoke_request(
            &emitter_seed(),
            &request_input(Some(serde_json::json!({
                "method": "capability-token",
                "sig": "opaque",
            }))),
            Default::default(),
        )
        .expect("sign request");
        let mut stripped = wire_of(&request);
        stripped
            .as_object_mut()
            .expect("wire object")
            .remove("auth");
        expect_kind(
            verify_invoke_request_auth(&emitter_pubkey(), &stripped, SESSION_ID),
            EnvelopeAuthErrorKind::Invalid,
        );
    }

    #[test]
    fn request_rejects_auth_added_after_signing() {
        let request = authenticate_invoke_request(
            &emitter_seed(),
            &request_input(None),
            Default::default(),
        )
        .expect("sign request");
        let mut forged = wire_of(&request);
        forged["auth"] = serde_json::json!({ "method": "capability-token", "sig": "opaque" });
        expect_kind(
            verify_invoke_request_auth(&emitter_pubkey(), &forged, SESSION_ID),
            EnvelopeAuthErrorKind::Invalid,
        );
    }

    #[test]
    fn request_rejects_a_missing_signature() {
        let request = authenticate_invoke_request(
            &emitter_seed(),
            &request_input(None),
            Default::default(),
        )
        .expect("sign request");
        let mut missing = wire_of(&request);
        missing
            .as_object_mut()
            .expect("wire object")
            .remove("signature");
        expect_kind(
            verify_invoke_request_auth(&emitter_pubkey(), &missing, SESSION_ID),
            EnvelopeAuthErrorKind::Missing,
        );
    }

    #[test]
    fn request_rejects_a_session_id_not_bound_to_the_session() {
        let request = authenticate_invoke_request(
            &emitter_seed(),
            &request_input(None),
            Default::default(),
        )
        .expect("sign request");
        expect_kind(
            verify_invoke_request_auth(&emitter_pubkey(), &wire_of(&request), "session-other"),
            EnvelopeAuthErrorKind::SessionUnbound,
        );
    }

    #[test]
    fn request_rejects_a_non_string_session_id_as_unbound_without_panicking() {
        let request = authenticate_invoke_request(
            &emitter_seed(),
            &request_input(None),
            Default::default(),
        )
        .expect("sign request");
        let mut wire = wire_of(&request);
        wire["session_id"] = serde_json::json!(123);
        let signed_object = invoke_request_signed_object(&wire).expect("signed object");
        let signature = sign_envelope(&emitter_seed(), &signed_object).expect("re-sign");
        wire["signature"] = serde_json::json!(signature);
        expect_kind(
            verify_invoke_request_auth(&emitter_pubkey(), &wire, SESSION_ID),
            EnvelopeAuthErrorKind::SessionUnbound,
        );
    }

    #[test]
    fn request_rejects_an_unknown_wire_key() {
        let request = authenticate_invoke_request(
            &emitter_seed(),
            &request_input(None),
            Default::default(),
        )
        .expect("sign request");
        let mut drifted = wire_of(&request);
        drifted
            .as_object_mut()
            .expect("wire object")
            .insert("rogue".to_string(), serde_json::json!("field"));
        expect_kind(
            verify_invoke_request_auth(&emitter_pubkey(), &drifted, SESSION_ID),
            EnvelopeAuthErrorKind::Invalid,
        );
    }

    #[test]
    fn request_rejects_a_non_32_byte_verify_public_key_with_crypto_local_misuse() {
        let request = authenticate_invoke_request(
            &emitter_seed(),
            &request_input(None),
            Default::default(),
        )
        .expect("sign request");
        let short_key = [7u8; 16];
        let err = verify_invoke_request_auth(&short_key, &wire_of(&request), SESSION_ID)
            .expect_err("short key");
        assert!(matches!(err, EnvelopeAuthError::Crypto(_)));
        assert_eq!(err.kind(), None, "local crypto error carries no wire kind");
    }

    #[test]
    fn response_signs_and_verifies_the_success_branch_payload_signed_object() {
        let response = authenticate_invoke_response(
            &emitter_seed(),
            &response_success_input(),
            Default::default(),
        )
        .expect("sign response");
        assert!(matches!(response, ConnectInvokeResponse::Variant0 { .. }));
        let wire = wire_of(&response);
        assert!(wire.get("payload").is_some());
        assert!(wire.get("error").is_none());
        assert_86_char_base64url(
            match &response {
                ConnectInvokeResponse::Variant0 { signature, .. } => signature.as_str(),
                ConnectInvokeResponse::Variant1 { .. } => unreachable!("success branch"),
            },
        );
        verify_invoke_response_auth(&emitter_pubkey(), &wire, SESSION_ID).expect("verify");
    }

    #[test]
    fn response_signs_and_verifies_the_error_branch_error_signed_object() {
        let response = authenticate_invoke_response(
            &emitter_seed(),
            &response_error_input(),
            Default::default(),
        )
        .expect("sign response");
        assert!(matches!(response, ConnectInvokeResponse::Variant1 { .. }));
        let wire = wire_of(&response);
        assert!(wire.get("error").is_some());
        assert!(wire.get("payload").is_none());
        assert_86_char_base64url(
            match &response {
                ConnectInvokeResponse::Variant1 { signature, .. } => signature.as_str(),
                ConnectInvokeResponse::Variant0 { .. } => unreachable!("error branch"),
            },
        );
        verify_invoke_response_auth(&emitter_pubkey(), &wire, SESSION_ID).expect("verify");
    }

    #[test]
    fn response_rejects_a_tampered_success_payload() {
        let response = authenticate_invoke_response(
            &emitter_seed(),
            &response_success_input(),
            Default::default(),
        )
        .expect("sign response");
        let mut tampered = wire_of(&response);
        tampered["payload"] = serde_json::json!({ "ok": false, "result": { "id": "n2" } });
        expect_kind(
            verify_invoke_response_auth(&emitter_pubkey(), &tampered, SESSION_ID),
            EnvelopeAuthErrorKind::Invalid,
        );
    }

    #[test]
    fn response_rejects_a_tampered_error_envelope() {
        let response = authenticate_invoke_response(
            &emitter_seed(),
            &response_error_input(),
            Default::default(),
        )
        .expect("sign response");
        let mut tampered = wire_of(&response);
        tampered["error"] = serde_json::json!({
            "code": "internal_error",
            "message": "forged",
            "extensions": {},
        });
        expect_kind(
            verify_invoke_response_auth(&emitter_pubkey(), &tampered, SESSION_ID),
            EnvelopeAuthErrorKind::Invalid,
        );
    }

    #[test]
    fn response_rejects_a_response_carrying_both_branches_branches_never_merged() {
        let response = authenticate_invoke_response(
            &emitter_seed(),
            &response_success_input(),
            Default::default(),
        )
        .expect("sign response");
        let mut both = wire_of(&response);
        both["error"] = serde_json::json!({
            "code": "op_unsupported",
            "message": "x",
            "extensions": {},
        });
        expect_kind(
            verify_invoke_response_auth(&emitter_pubkey(), &both, SESSION_ID),
            EnvelopeAuthErrorKind::Invalid,
        );
    }

    #[test]
    fn response_rejects_a_response_carrying_neither_branch() {
        let response = authenticate_invoke_response(
            &emitter_seed(),
            &response_success_input(),
            Default::default(),
        )
        .expect("sign response");
        let mut neither = wire_of(&response);
        neither
            .as_object_mut()
            .expect("wire object")
            .remove("payload");
        expect_kind(
            verify_invoke_response_auth(&emitter_pubkey(), &neither, SESSION_ID),
            EnvelopeAuthErrorKind::Invalid,
        );
    }

    #[test]
    fn response_rejects_a_missing_signature() {
        let response = authenticate_invoke_response(
            &emitter_seed(),
            &response_success_input(),
            Default::default(),
        )
        .expect("sign response");
        let mut missing = wire_of(&response);
        missing
            .as_object_mut()
            .expect("wire object")
            .remove("signature");
        expect_kind(
            verify_invoke_response_auth(&emitter_pubkey(), &missing, SESSION_ID),
            EnvelopeAuthErrorKind::Missing,
        );
    }

    #[test]
    fn response_rejects_a_session_id_not_bound_to_the_session() {
        let response = authenticate_invoke_response(
            &emitter_seed(),
            &response_error_input(),
            Default::default(),
        )
        .expect("sign response");
        expect_kind(
            verify_invoke_response_auth(&emitter_pubkey(), &wire_of(&response), "session-other"),
            EnvelopeAuthErrorKind::SessionUnbound,
        );
    }

    #[test]
    fn typed_deserialization_rejects_wire_shapes_that_cannot_carry_a_kind() {
        // Fail-closed defense-in-depth: the typed structs enforce the schema
        // (required 86-char signature, additionalProperties: false, oneOf
        // branches), so these wire shapes are rejected at deserialization —
        // before verify — with the same outcome (never accepted). The TS
        // runtime whitelist covers the same drift inside its verify call.
        let session =
            authenticate_session(&emitter_seed(), &session_input(), Default::default())
                .expect("sign session");
        let mut drifted = wire_of(&session);
        drifted
            .as_object_mut()
            .expect("wire object")
            .insert("rogue".to_string(), serde_json::json!("field"));
        assert!(
            serde_json::from_value::<ConnectSession>(drifted).is_err(),
            "unknown wire key rejected at deserialization (field-set drift)"
        );

        let mut no_signature = wire_of(&session);
        no_signature
            .as_object_mut()
            .expect("wire object")
            .remove("signature");
        assert!(
            serde_json::from_value::<ConnectSession>(no_signature).is_err(),
            "missing signature rejected at deserialization (schema required)"
        );

        let response = authenticate_invoke_response(
            &emitter_seed(),
            &response_success_input(),
            Default::default(),
        )
        .expect("sign response");
        let mut both = wire_of(&response);
        both["error"] = serde_json::json!({ "code": "x", "message": "x", "extensions": {} });
        assert!(
            serde_json::from_value::<ConnectInvokeResponse>(both).is_err(),
            "both branches rejected at deserialization (oneOf)"
        );

        let mut neither = wire_of(&response);
        neither
            .as_object_mut()
            .expect("wire object")
            .remove("payload");
        assert!(
            serde_json::from_value::<ConnectInvokeResponse>(neither).is_err(),
            "neither branch rejected at deserialization (oneOf)"
        );
    }

    #[test]
    fn cross_verify_request_wire_round_trips_through_typed_serialization() {
        // authenticate produces the typed envelope; serializing it to the
        // wire form and verifying reproduces the signer's canonical bytes.
        let request = authenticate_invoke_request(
            &emitter_seed(),
            &request_input(Some(serde_json::json!({ "method": "capability-token" }))),
            Default::default(),
        )
        .expect("sign request");
        let wire = wire_of(&request);
        verify_invoke_request_auth(&emitter_pubkey(), &wire, SESSION_ID)
            .expect("typed → wire → verify round-trip");
        // And the exact wire bytes match what TS would emit for the same
        // input (JSON form is schema-canonical; JCS parity is pinned by the
        // golden-vector task).
        assert_eq!(
            wire["auth"],
            serde_json::json!({ "method": "capability-token" })
        );
    }
}
