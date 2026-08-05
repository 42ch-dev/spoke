//! uniffi-exported sync-core facade, behind the non-default `ffi` feature.
//!
//! This module is the FFI contract for foreign-language bindings (Swift
//! first — see the crate README "Binding facade" section). It re-exposes the
//! pure session core ([`crate::core`]) with a uniffi-compatible surface:
//! every exported function and object maps 1:1 onto a core rule, and the
//! core itself stays untouched, dependency-pure, and uniffi-free.
//!
//! Boundary conventions:
//! - Keys cross as raw `Vec<u8>` — uniffi has no fixed-size array type — and
//!   are validated to exactly 32 bytes inside the wrapper.
//! - Peer ids cross as `String`; the host manifest and the hello envelope
//!   cross as JSON strings, deserialized with `serde_json` inside Rust, so
//!   no generated schema types appear on the FFI surface.
//! - Errors map to thin FFI-facing enums that mirror
//!   [`crate::core::CoreError`] / [`crate::core::CoreInvokeError`]
//!   variant-for-variant; the core enums are unchanged.
//!
//! ## Envelope-auth deferral (no new FFI APIs)
//!
//! The v2 envelope-authentication helpers ([`crate::core::envelope_auth`]:
//! `authenticate_*` / `verify_*` for `ConnectSession`, `ConnectInvokeRequest`,
//! `ConnectInvokeResponse`) are deliberately **not** exposed through this
//! facade. The frozen envelope-auth contract §9 locks "no new FFI APIs":
//! bindings keep calling the encapsulated RemoteAdapter / connect-client
//! surfaces, which attach and verify authenticators internally — verify
//! helpers are never host-callable (encapsulation hard rule), so widening the
//! FFI surface would add binding-parity surface for no consumer benefit. The
//! parity gate for the new auth surface is the TS↔Rust session core
//! (`crates/spoke-connect/src/core/envelope_auth.rs` ↔
//! `packages/spoke-connect-ts/src/core/envelope-auth.ts`): canonical bytes,
//! algorithm ids, and verify outcomes. Binding golden-parity smokes covering
//! hello stay green and are not extended to envelope auth.

use std::sync::{Arc, Mutex};
#[cfg(feature = "ffi")]
use std::sync::OnceLock;

#[cfg(feature = "ffi")]
static FFI_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Process-wide tokio runtime for the FFI surface (AR-1: multi-thread).
#[cfg(feature = "ffi")]
pub(crate) fn ffi_runtime() -> &'static tokio::runtime::Runtime {
    FFI_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("cdylib tokio runtime initializes once")
    })
}

use spoke_schemas::connect::connect_hello::HostCapabilityManifest;
use spoke_schemas::connect::ConnectHello;

use crate::core::{CoreError as CoreErrorImpl, CoreInvokeError as CoreInvokeErrorImpl};

/// FFI-facing mirror of [`crate::core::CoreError`] (hello-gate / identity
/// failures). Mapped 1:1 in [`From<CoreErrorImpl>`].
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CoreError {
    /// The hello signature did not verify against the peer's public key (or
    /// the signature is not valid base64url / not 64 bytes).
    #[error("hello signature invalid")]
    InvalidHelloSignature,
    /// The `(peer_id, nonce)` pair was already accepted.
    #[error("hello nonce replayed")]
    NonceReplay,
    /// Handshake-level failure (protocol version, peer id binding, …).
    #[error("handshake failed: {reason}")]
    HandshakeFailed { reason: String },
    /// The hello nonce does not satisfy the wire constraints (minLength 16).
    #[error("invalid hello nonce: {message}")]
    InvalidNonce { message: String },
    /// Cryptography-level failure (invalid key bytes, base64 decoding, …).
    #[error("crypto: {message}")]
    Crypto { message: String },
    /// RFC 8785 JCS canonicalization / serialization of the signed object
    /// failed.
    #[error("JCS canonicalization failed: {message}")]
    Jcs { message: String },
    /// A capability-token proof failed validation (malformed shape, bad
    /// signature, untrusted issuer, subject/audience/expiry mismatch, or
    /// claim-rule violation).
    #[error("capability token invalid: {message}")]
    TokenInvalid { message: String },
}

impl From<CoreErrorImpl> for CoreError {
    fn from(error: CoreErrorImpl) -> Self {
        match error {
            CoreErrorImpl::InvalidHelloSignature => Self::InvalidHelloSignature,
            CoreErrorImpl::NonceReplay => Self::NonceReplay,
            CoreErrorImpl::HandshakeFailed { reason } => Self::HandshakeFailed { reason },
            CoreErrorImpl::InvalidNonce(message) => Self::InvalidNonce { message },
            CoreErrorImpl::Crypto(message) => Self::Crypto { message },
            CoreErrorImpl::Jcs(message) => Self::Jcs { message },
            CoreErrorImpl::TokenInvalid(message) => Self::TokenInvalid { message },
        }
    }
}

/// FFI-facing mirror of [`crate::core::CoreInvokeError`] (invoke-path
/// sequence / correlation failures). Mapped 1:1 in
/// [`From<CoreInvokeErrorImpl>`].
#[derive(Debug, thiserror::Error, uniffi::Error)]
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

impl From<CoreInvokeErrorImpl> for CoreInvokeError {
    fn from(error: CoreInvokeErrorImpl) -> Self {
        match error {
            CoreInvokeErrorImpl::SequenceExhausted => Self::SequenceExhausted,
            CoreInvokeErrorImpl::InboundSequenceMismatch { expected, actual } => {
                Self::InboundSequenceMismatch { expected, actual }
            }
            CoreInvokeErrorImpl::CorrelationMismatch => Self::CorrelationMismatch,
        }
    }
}

/// Require `bytes` to be exactly 32 bytes (an Ed25519 secret or public key).
fn ed25519_key(bytes: Vec<u8>, what: &str) -> Result<[u8; 32], CoreError> {
    let len = bytes.len();
    bytes.try_into().map_err(|_| CoreError::Crypto {
        message: format!("expected a 32-byte {what}, got {len}"),
    })
}

/// Derive the wire `peer_id` string for a 32-byte Ed25519 public key.
///
/// The result matches rust-libp2p `PeerId::to_string()` for the same key
/// (locked by the golden-vector tests).
#[uniffi::export]
pub fn derive_peer_id_from_ed25519_pubkey(pubkey: Vec<u8>) -> Result<String, CoreError> {
    let pubkey = ed25519_key(pubkey, "Ed25519 public key")?;
    Ok(crate::core::derive_peer_id_from_ed25519_pubkey(&pubkey))
}

/// Sign a hello with a raw Ed25519 secret key (32 bytes), returning the
/// signed `ConnectHello` envelope as a JSON string.
///
/// `nonce` must meet the wire floor (minLength 16). `host_json` is the
/// canonical JSON of the `HostCapabilityManifest` embedded in
/// `ConnectHello.host`.
#[uniffi::export]
pub fn sign_hello_ed25519(
    secret: Vec<u8>,
    nonce: String,
    host_json: String,
) -> Result<String, CoreError> {
    let secret = ed25519_key(secret, "Ed25519 secret key")?;
    // The FFI boundary cannot carry the typed `HostCapabilityManifest`, so
    // it crosses as JSON; a parse failure is a malformed-input handshake
    // failure (the core takes the typed manifest and never sees raw JSON).
    let manifest: HostCapabilityManifest =
        serde_json::from_str(&host_json).map_err(|e| CoreError::HandshakeFailed {
            reason: format!("invalid host manifest JSON: {e}"),
        })?;
    let hello =
        crate::core::sign_hello_ed25519(&secret, &nonce, &manifest, None).map_err(CoreError::from)?;
    // Every envelope field is serializable, so a failure here is a
    // serialization defect — mapped to the canonicalization-family variant.
    serde_json::to_string(&hello).map_err(|e| CoreError::Jcs {
        message: format!("serialize hello: {e}"),
    })
}

/// Verify a received hello against a 32-byte Ed25519 public key.
///
/// `expected_peer_id` is the authenticated remote peer; `hello_json` is the
/// JSON string of the received `ConnectHello` envelope. Fails on protocol
/// version mismatch, public-key / peer-id binding mismatch, or an invalid
/// signature.
#[uniffi::export]
pub fn verify_hello_ed25519(
    public_key: Vec<u8>,
    expected_peer_id: String,
    hello_json: String,
) -> Result<(), CoreError> {
    let public_key = ed25519_key(public_key, "Ed25519 public key")?;
    let hello: ConnectHello =
        serde_json::from_str(&hello_json).map_err(|e| CoreError::HandshakeFailed {
            reason: format!("invalid hello JSON: {e}"),
        })?;
    crate::core::verify_hello_ed25519(&public_key, &expected_peer_id, &hello, None)
        .map_err(CoreError::from)
}

/// Single-use `(peer_id, nonce)` replay store — thread-safe FFI wrapper over
/// the core store.
#[derive(uniffi::Object)]
pub struct NonceStore {
    inner: Mutex<crate::core::NonceStore>,
}

#[uniffi::export]
impl NonceStore {
    /// Creates an empty store.
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(crate::core::NonceStore::new()),
        })
    }

    /// Records `(peer_id, nonce)` unless it was already accepted; returns
    /// `false` on replay. Call only after the hello passed every earlier
    /// gate (allowlist, signature) so a rejected hello is not burned.
    #[must_use]
    pub fn check_and_record(&self, peer_id: String, nonce: String) -> bool {
        // Poisoning is unreachable today (no panicking payloads), but a
        // poisoned lock must not permanently brick the FFI object.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .check_and_record(&peer_id, &nonce)
    }
}

/// Whether `peer_id` is on the allowlist. Fails closed: an empty allowlist
/// rejects every peer.
#[uniffi::export]
#[must_use]
pub fn is_allowlisted(allowlist: Vec<String>, peer_id: String) -> bool {
    crate::core::is_allowlisted(&allowlist, &peer_id)
}

/// Outbound sequence counter — thread-safe FFI wrapper over the core
/// counter, starting at 0.
#[derive(uniffi::Object)]
pub struct OutboundSequence {
    inner: Mutex<crate::core::OutboundSequence>,
}

#[uniffi::export]
impl OutboundSequence {
    /// Creates a counter starting at 0 (the first allocate returns 0).
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(crate::core::OutboundSequence::new()),
        })
    }

    /// Assigns the next outbound sequence; on exhaustion (past the JSON-safe
    /// wire maximum) `SequenceExhausted` is returned and the counter stays
    /// exhausted — sequences never wrap. The caller must close the session.
    pub fn allocate(&self) -> Result<u64, CoreInvokeError> {
        // Poisoning is unreachable today (no panicking payloads), but a
        // poisoned lock must not permanently brick the FFI object.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .allocate()
            .map_err(CoreInvokeError::from)
    }
}

/// Inbound sequence expectation — thread-safe FFI wrapper over the core
/// expectation, starting at 0.
#[derive(uniffi::Object)]
pub struct InboundSequence {
    inner: Mutex<crate::core::InboundSequence>,
}

#[uniffi::export]
impl InboundSequence {
    /// Creates an expectation starting at 0 (the first accepted sequence).
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(crate::core::InboundSequence::new()),
        })
    }

    /// Accepts `sequence` iff it equals the next expected inbound sequence;
    /// on acceptance the expectation advances by 1 and the new expectation
    /// is returned. A replayed or out-of-order sequence yields
    /// `InboundSequenceMismatch` and the expectation is left unchanged — the
    /// caller must reject the invoke without dispatching it.
    pub fn advance(&self, sequence: i64) -> Result<u64, CoreInvokeError> {
        // Poisoning is unreachable today (no panicking payloads), but a
        // poisoned lock must not permanently brick the FFI object.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .advance(sequence)
            .map_err(CoreInvokeError::from)
    }
}

/// Checks that a response echoes the request's `session_id` / `sequence` /
/// `request_id` — the three echo fields, flattened to primitives.
#[uniffi::export]
pub fn check_response_correlation(
    expected_session_id: String,
    expected_sequence: u64,
    expected_request_id: String,
    actual_session_id: String,
    actual_sequence: u64,
    actual_request_id: String,
) -> Result<(), CoreInvokeError> {
    // Wire sequences are i64; a value above i64::MAX can never match a wire
    // echo, so it fails correlation.
    let expected_sequence =
        i64::try_from(expected_sequence).map_err(|_| CoreInvokeError::CorrelationMismatch)?;
    let actual_sequence =
        i64::try_from(actual_sequence).map_err(|_| CoreInvokeError::CorrelationMismatch)?;
    crate::core::check_response_correlation(
        &crate::core::Correlation {
            session_id: expected_session_id,
            sequence: expected_sequence,
            request_id: expected_request_id,
        },
        &crate::core::Correlation {
            session_id: actual_session_id,
            sequence: actual_sequence,
            request_id: actual_request_id,
        },
    )
    .map_err(CoreInvokeError::from)
}

/// Whether `op` may be dispatched in a session with
/// `negotiated_capabilities`. Fails closed: an unknown `op` has no core-table
/// requirement and is not authorized by this gate (hosts answer
/// `op_unsupported`).
#[uniffi::export]
#[must_use]
pub fn dispatch_allowed(op: String, negotiated_capabilities: Vec<String>) -> bool {
    crate::core::dispatch_allowed(&op, &negotiated_capabilities)
}

/// The capability required to dispatch `op`, per the protocol v1 core-op
/// table; `None` for product-defined ops.
#[uniffi::export]
#[must_use]
pub fn required_capability(op: String) -> Option<String> {
    crate::core::required_capability(&op).map(str::to_owned)
}

/// The connect protocol version exchanged in `ConnectHello` (protocol
/// version 1 is current).
#[uniffi::export]
#[must_use]
pub fn protocol_version() -> u64 {
    crate::core::PROTOCOL_VERSION
}

#[cfg(feature = "ffi")]
#[cfg(test)]
mod runtime_tests {
    use super::ffi_runtime;
    use std::thread;

    #[test]
    fn ffi_runtime_is_lazy_initialized_once_and_reused_across_threads() {
        let main_runtime = ffi_runtime();
        main_runtime.handle().block_on(async {});

        let main_addr = main_runtime as *const tokio::runtime::Runtime as usize;
        let handles: Vec<_> = (0..2)
            .map(|_| {
                thread::spawn(|| {
                    let thread_runtime = ffi_runtime();
                    thread_runtime.handle().block_on(async {});
                    thread_runtime as *const tokio::runtime::Runtime as usize
                })
            })
            .collect();

        for handle in handles {
            let thread_addr = handle.join().expect("thread joined");
            assert_eq!(main_addr, thread_addr);
            assert_eq!(main_addr, ffi_runtime() as *const tokio::runtime::Runtime as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::golden::{golden, golden_pubkey, golden_seed};

    /// Golden key pair (seed bytes 1..=32) — the same fixtures as the core
    /// golden-vector tests, so these wrapper tests assert byte parity with
    /// the core surface. Loaded from the shared cross-language fixture
    /// `tests/fixtures/golden-hello.json` (SSOT); the golden signature,
    /// peer id, and manifest JSON are the fixture's pinned output bytes.

    #[test]
    fn derive_peer_id_via_ffi_matches_golden() {
        assert_eq!(
            derive_peer_id_from_ed25519_pubkey(golden_pubkey().to_vec()).expect("derives"),
            golden().peer_id
        );
    }

    #[test]
    fn derive_peer_id_rejects_wrong_key_length() {
        let err = derive_peer_id_from_ed25519_pubkey(vec![0u8; 31]).expect_err("short key");
        assert!(matches!(err, CoreError::Crypto { .. }));
        let err = derive_peer_id_from_ed25519_pubkey(vec![0u8; 33]).expect_err("long key");
        assert!(matches!(err, CoreError::Crypto { .. }));
    }

    #[test]
    fn sign_hello_via_ffi_matches_golden_signature() {
        let hello_json = sign_hello_ed25519(
            golden_seed().to_vec(),
            golden().nonce.clone(),
            golden().manifest_json.clone(),
        )
        .expect("signs");
        let hello: ConnectHello = serde_json::from_str(&hello_json).expect("hello JSON parses");
        assert_eq!(hello.peer_id.as_str(), golden().peer_id.as_str());
        assert_eq!(hello.signature.as_str(), golden().signature_b64u.as_str());
    }

    #[test]
    fn sign_verify_round_trip_via_ffi_json() {
        let hello_json = sign_hello_ed25519(
            golden_seed().to_vec(),
            golden().nonce.clone(),
            golden().manifest_json.clone(),
        )
        .expect("signs");
        verify_hello_ed25519(
            golden_pubkey().to_vec(),
            golden().peer_id.clone(),
            hello_json,
        )
        .expect("verifies");
    }

    #[test]
    fn tampered_hello_json_fails_verification_with_mapped_error() {
        let hello_json = sign_hello_ed25519(
            golden_seed().to_vec(),
            golden().nonce.clone(),
            golden().manifest_json.clone(),
        )
        .expect("signs");
        let tampered = hello_json.replace("data-store", "checker");
        let err = verify_hello_ed25519(
            golden_pubkey().to_vec(),
            golden().peer_id.clone(),
            tampered,
        )
        .expect_err("tampered host");
        assert!(matches!(err, CoreError::InvalidHelloSignature));
    }

    #[test]
    fn malformed_hello_json_fails_verification_with_mapped_error() {
        // Non-JSON input and valid JSON with the wrong shape both fail
        // serde parsing, which the wrapper maps to a handshake failure —
        // the same mapping as malformed host manifest JSON on the sign path
        // (sign_hello_ed25519 also returns HandshakeFailed for unparseable
        // JSON), so sign and verify are consistent.
        for malformed in ["not json".to_owned(), r#"{"not":"a-hello"}"#.to_owned()] {
            let err = verify_hello_ed25519(
                golden_pubkey().to_vec(),
                golden().peer_id.clone(),
                malformed,
            )
            .expect_err("malformed hello");
            assert!(matches!(err, CoreError::HandshakeFailed { .. }));
        }
    }

    #[test]
    fn bad_nonce_and_bad_manifest_json_map_to_errors() {
        let err = sign_hello_ed25519(
            golden_seed().to_vec(),
            ["short"].join("-"),
            golden().manifest_json.clone(),
        )
        .expect_err("short nonce");
        assert!(matches!(err, CoreError::InvalidNonce { .. }));

        let err = sign_hello_ed25519(
            golden_seed().to_vec(),
            golden().nonce.clone(),
            "not json".to_owned(),
        )
        .expect_err("malformed manifest");
        assert!(matches!(err, CoreError::HandshakeFailed { .. }));
    }

    #[test]
    fn nonce_store_object_rejects_replay() {
        let store = NonceStore::new();
        assert!(store.check_and_record("peer-a".to_owned(), "nonce-1".to_owned()));
        assert!(!store.check_and_record("peer-a".to_owned(), "nonce-1".to_owned()));
        // Nonce scoping is per sender peer_id.
        assert!(store.check_and_record("peer-b".to_owned(), "nonce-1".to_owned()));
    }

    #[test]
    fn allowlist_check_via_ffi_fails_closed() {
        let allowlist = vec!["peer-a".to_owned(), "peer-b".to_owned()];
        assert!(is_allowlisted(allowlist.clone(), "peer-a".to_owned()));
        assert!(!is_allowlisted(allowlist, "peer-c".to_owned()));
        assert!(!is_allowlisted(Vec::new(), "peer-a".to_owned()));
    }

    #[test]
    fn sequence_objects_allocate_and_advance() {
        let outbound = OutboundSequence::new();
        assert_eq!(outbound.allocate().expect("first"), 0);
        assert_eq!(outbound.allocate().expect("second"), 1);

        let inbound = InboundSequence::new();
        assert_eq!(inbound.advance(0).expect("first"), 1);
        let err = inbound.advance(0).expect_err("replay");
        assert!(matches!(
            err,
            CoreInvokeError::InboundSequenceMismatch {
                expected: 1,
                actual: 0
            }
        ));
    }

    #[test]
    fn outbound_sequence_exhaustion_maps_through_wrapper() {
        // Position the core counter at the wire maximum via the test-only
        // setter (no 2^53 allocations), then drive exhaustion through the
        // FFI wrapper to exercise the error mapping end to end.
        let mut core_seq = crate::core::OutboundSequence::new();
        core_seq.set_next(crate::core::MAX_SEQUENCE);
        let outbound = OutboundSequence {
            inner: Mutex::new(core_seq),
        };
        assert_eq!(
            outbound.allocate().expect("last valid"),
            crate::core::MAX_SEQUENCE
        );
        let err = outbound.allocate().expect_err("exhausted");
        assert!(matches!(err, CoreInvokeError::SequenceExhausted));
        // Still exhausted — no wrap-around.
        let err = outbound.allocate().expect_err("still exhausted");
        assert!(matches!(err, CoreInvokeError::SequenceExhausted));
    }

    #[test]
    fn correlation_check_via_flattened_primitives() {
        check_response_correlation(
            "sess-1".to_owned(),
            0,
            "req-1".to_owned(),
            "sess-1".to_owned(),
            0,
            "req-1".to_owned(),
        )
        .expect("exact echo passes");

        let err = check_response_correlation(
            "sess-1".to_owned(),
            0,
            "req-1".to_owned(),
            "sess-1".to_owned(),
            1,
            "req-1".to_owned(),
        )
        .expect_err("sequence mismatch");
        assert!(matches!(err, CoreInvokeError::CorrelationMismatch));
    }

    #[test]
    fn correlation_sequences_above_i64_max_fail_guarded() {
        // Wire sequences are i64; a u64 above i64::MAX can never match a
        // wire echo, so the guard fails correlation on either side.
        let over_i64 = i64::MAX as u64 + 1;
        let err = check_response_correlation(
            "sess-1".to_owned(),
            over_i64,
            "req-1".to_owned(),
            "sess-1".to_owned(),
            0,
            "req-1".to_owned(),
        )
        .expect_err("expected sequence above i64::MAX");
        assert!(matches!(err, CoreInvokeError::CorrelationMismatch));

        let err = check_response_correlation(
            "sess-1".to_owned(),
            0,
            "req-1".to_owned(),
            "sess-1".to_owned(),
            over_i64,
            "req-1".to_owned(),
        )
        .expect_err("actual sequence above i64::MAX");
        assert!(matches!(err, CoreInvokeError::CorrelationMismatch));
    }

    #[test]
    fn dispatch_gate_required_capability_and_protocol_version() {
        assert!(dispatch_allowed(
            "check".to_owned(),
            vec!["spoke-baseline".to_owned()]
        ));
        assert!(!dispatch_allowed(
            "check".to_owned(),
            vec!["l2-computable".to_owned()]
        ));
        assert!(!dispatch_allowed(
            "custom-op".to_owned(),
            vec!["spoke-baseline".to_owned()]
        ));
        assert_eq!(
            required_capability("check".to_owned()).as_deref(),
            Some("spoke-baseline")
        );
        assert_eq!(
            required_capability("project".to_owned()).as_deref(),
            Some("l2-computable")
        );
        assert_eq!(required_capability("custom-op".to_owned()), None);
        assert_eq!(protocol_version(), 1);
    }
}
