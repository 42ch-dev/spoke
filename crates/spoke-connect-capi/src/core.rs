//! Core session-rule surface: the `spoke_connect_*` exports that wrap the
//! core functions and objects of [`spoke_connect::ffi`] (identity derivation,
//! hello sign/verify, nonce store, allowlist, sequence counters, response
//! correlation and the dispatch gate).
//!
//! The rules live in `spoke-connect`; this module only converts values, owns
//! handles and projects errors onto the C boundary (see the crate docs for the
//! ownership and status conventions).

use std::ptr;
use std::sync::Arc;

use spoke_connect::ffi;

use crate::{
    borrowed_bytes, borrowed_strings, borrowed_text, contain_release, export, owned_buffer,
    release_handle, require_out, AbiFailure, SpokeConnectBuffer, SpokeConnectError,
    SpokeConnectOptionalBuffer, SpokeConnectSlice, SPOKE_CONNECT_CORRELATION_MISMATCH,
    SPOKE_CONNECT_CRYPTO, SPOKE_CONNECT_HANDSHAKE_FAILED, SPOKE_CONNECT_INBOUND_SEQUENCE_MISMATCH,
    SPOKE_CONNECT_INVALID_HELLO_SIGNATURE, SPOKE_CONNECT_INVALID_NONCE, SPOKE_CONNECT_JCS,
    SPOKE_CONNECT_NONCE_REPLAY, SPOKE_CONNECT_PROTOCOL_VERSION_MISMATCH,
    SPOKE_CONNECT_SEQUENCE_EXHAUSTED, SPOKE_CONNECT_TOKEN_INVALID,
};

/// Opaque `(peer_id, nonce)` replay store handle.
#[repr(C)]
pub struct SpokeConnectNonceStore {
    _private: [u8; 0],
}

/// Opaque outbound sequence counter handle.
#[repr(C)]
pub struct SpokeConnectOutboundSequence {
    _private: [u8; 0],
}

/// Opaque inbound sequence expectation handle.
#[repr(C)]
pub struct SpokeConnectInboundSequence {
    _private: [u8; 0],
}

/// Projects a facade hello-gate / identity failure. The status identifies the
/// variant; the variant's own message or reason is carried through verbatim
/// (the payload-less variants carry their `Display` text).
impl From<ffi::CoreError> for AbiFailure {
    fn from(error: ffi::CoreError) -> Self {
        let (status, message) = match &error {
            ffi::CoreError::InvalidHelloSignature => {
                (SPOKE_CONNECT_INVALID_HELLO_SIGNATURE, error.to_string())
            }
            ffi::CoreError::NonceReplay => (SPOKE_CONNECT_NONCE_REPLAY, error.to_string()),
            ffi::CoreError::HandshakeFailed { reason } => {
                (SPOKE_CONNECT_HANDSHAKE_FAILED, reason.clone())
            }
            ffi::CoreError::InvalidNonce { message } => {
                (SPOKE_CONNECT_INVALID_NONCE, message.clone())
            }
            ffi::CoreError::Crypto { message } => (SPOKE_CONNECT_CRYPTO, message.clone()),
            ffi::CoreError::Jcs { message } => (SPOKE_CONNECT_JCS, message.clone()),
            ffi::CoreError::TokenInvalid { message } => {
                (SPOKE_CONNECT_TOKEN_INVALID, message.clone())
            }
            ffi::CoreError::ProtocolVersionMismatch { reason } => {
                (SPOKE_CONNECT_PROTOCOL_VERSION_MISMATCH, reason.clone())
            }
        };
        AbiFailure::at(status, message)
    }
}

/// Projects a facade invoke-path failure. An inbound sequence mismatch keeps
/// both numeric fields alongside the status and message.
impl From<ffi::CoreInvokeError> for AbiFailure {
    fn from(error: ffi::CoreInvokeError) -> Self {
        let (status, message) = match &error {
            ffi::CoreInvokeError::SequenceExhausted => {
                (SPOKE_CONNECT_SEQUENCE_EXHAUSTED, error.to_string())
            }
            ffi::CoreInvokeError::InboundSequenceMismatch { .. } => {
                (SPOKE_CONNECT_INBOUND_SEQUENCE_MISMATCH, error.to_string())
            }
            ffi::CoreInvokeError::CorrelationMismatch => {
                (SPOKE_CONNECT_CORRELATION_MISMATCH, error.to_string())
            }
        };
        match error {
            ffi::CoreInvokeError::InboundSequenceMismatch { expected, actual } => AbiFailure {
                status,
                message,
                expected,
                actual,
            },
            _ => AbiFailure::at(status, message),
        }
    }
}

/// Borrows a nonce-store handle.
unsafe fn nonce_store<'a>(
    handle: *const SpokeConnectNonceStore,
) -> Result<&'a Arc<ffi::NonceStore>, AbiFailure> {
    if handle.is_null() {
        return Err(AbiFailure::invalid("nonce_store is NULL"));
    }
    Ok(unsafe { &*(handle as *const Arc<ffi::NonceStore>) })
}

/// Borrows an outbound-sequence handle.
unsafe fn outbound_sequence<'a>(
    handle: *const SpokeConnectOutboundSequence,
) -> Result<&'a Arc<ffi::OutboundSequence>, AbiFailure> {
    if handle.is_null() {
        return Err(AbiFailure::invalid("outbound_sequence is NULL"));
    }
    Ok(unsafe { &*(handle as *const Arc<ffi::OutboundSequence>) })
}

/// Borrows an inbound-sequence handle.
unsafe fn inbound_sequence<'a>(
    handle: *const SpokeConnectInboundSequence,
) -> Result<&'a Arc<ffi::InboundSequence>, AbiFailure> {
    if handle.is_null() {
        return Err(AbiFailure::invalid("inbound_sequence is NULL"));
    }
    Ok(unsafe { &*(handle as *const Arc<ffi::InboundSequence>) })
}

/// Derives the wire `peer_id` string for a 32-byte Ed25519 public key.
///
/// `pubkey` is a borrowed 32-byte key; `out_peer_id` receives the owned
/// peer-id string.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_derive_peer_id_from_ed25519_pubkey(
    pubkey: SpokeConnectSlice,
    out_peer_id: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_peer_id = require_out(out_peer_id, "out_peer_id")?;
            out_peer_id.write(SpokeConnectBuffer::empty());
            let pubkey = borrowed_bytes(pubkey, "pubkey")?;
            let peer_id = ffi::derive_peer_id_from_ed25519_pubkey(pubkey.to_vec())?;
            out_peer_id.write(owned_buffer(peer_id.as_bytes()));
            Ok(())
        })
    }
}

/// Signs a hello with a raw 32-byte Ed25519 secret key.
///
/// `nonce` is borrowed UTF-8 (wire floor: 16 characters); `host_json` is the
/// canonical JSON of the host capability manifest. `out_hello_json` receives
/// the owned signed `ConnectHello` envelope as JSON.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_sign_hello_ed25519(
    secret: SpokeConnectSlice,
    nonce: SpokeConnectSlice,
    host_json: SpokeConnectSlice,
    out_hello_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_hello_json = require_out(out_hello_json, "out_hello_json")?;
            out_hello_json.write(SpokeConnectBuffer::empty());
            let secret = borrowed_bytes(secret, "secret")?;
            let nonce = borrowed_text(nonce, "nonce")?;
            let host_json = borrowed_text(host_json, "host_json")?;
            let hello =
                ffi::sign_hello_ed25519(secret.to_vec(), nonce.to_owned(), host_json.to_owned())?;
            out_hello_json.write(owned_buffer(hello.as_bytes()));
            Ok(())
        })
    }
}

/// Verifies a received hello against a 32-byte Ed25519 public key.
///
/// `expected_peer_id` is the authenticated remote peer; `hello_json` is the
/// received `ConnectHello` envelope as JSON.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_verify_hello_ed25519(
    public_key: SpokeConnectSlice,
    expected_peer_id: SpokeConnectSlice,
    hello_json: SpokeConnectSlice,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let public_key = borrowed_bytes(public_key, "public_key")?;
            let expected_peer_id = borrowed_text(expected_peer_id, "expected_peer_id")?;
            let hello_json = borrowed_text(hello_json, "hello_json")?;
            ffi::verify_hello_ed25519(
                public_key.to_vec(),
                expected_peer_id.to_owned(),
                hello_json.to_owned(),
            )?;
            Ok(())
        })
    }
}

/// Whether `peer_id` is on the allowlist.
///
/// `allowlist` is a borrowed array of UTF-8 slices; an empty allowlist rejects
/// every peer. `out_allowed` receives 0 or 1.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_is_allowlisted(
    allowlist: *const SpokeConnectSlice,
    allowlist_count: usize,
    peer_id: SpokeConnectSlice,
    out_allowed: *mut u8,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_allowed = require_out(out_allowed, "out_allowed")?;
            out_allowed.write(0);
            let allowlist = borrowed_strings(allowlist, allowlist_count, "allowlist")?;
            let peer_id = borrowed_text(peer_id, "peer_id")?;
            let allowed = ffi::is_allowlisted(allowlist, peer_id.to_owned());
            out_allowed.write(u8::from(allowed));
            Ok(())
        })
    }
}

/// Checks that a response echoes the request's `session_id`, `sequence` and
/// `request_id`.
///
/// The facade narrows the unsigned sequences onto the wire `i64` range; a
/// value above it can never match an echo and fails correlation.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_check_response_correlation(
    expected_session_id: SpokeConnectSlice,
    expected_sequence: u64,
    expected_request_id: SpokeConnectSlice,
    actual_session_id: SpokeConnectSlice,
    actual_sequence: u64,
    actual_request_id: SpokeConnectSlice,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let expected_session_id = borrowed_text(expected_session_id, "expected_session_id")?;
            let expected_request_id = borrowed_text(expected_request_id, "expected_request_id")?;
            let actual_session_id = borrowed_text(actual_session_id, "actual_session_id")?;
            let actual_request_id = borrowed_text(actual_request_id, "actual_request_id")?;
            ffi::check_response_correlation(
                expected_session_id.to_owned(),
                expected_sequence,
                expected_request_id.to_owned(),
                actual_session_id.to_owned(),
                actual_sequence,
                actual_request_id.to_owned(),
            )?;
            Ok(())
        })
    }
}

/// Whether `op` may be dispatched with `negotiated_capabilities`.
///
/// `negotiated_capabilities` is a borrowed array of UTF-8 slices; an unknown
/// `op` fails closed. `out_allowed` receives 0 or 1.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_dispatch_allowed(
    op: SpokeConnectSlice,
    negotiated_capabilities: *const SpokeConnectSlice,
    negotiated_count: usize,
    out_allowed: *mut u8,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_allowed = require_out(out_allowed, "out_allowed")?;
            out_allowed.write(0);
            let op = borrowed_text(op, "op")?;
            let negotiated = borrowed_strings(
                negotiated_capabilities,
                negotiated_count,
                "negotiated_capabilities",
            )?;
            let allowed = ffi::dispatch_allowed(op.to_owned(), negotiated);
            out_allowed.write(u8::from(allowed));
            Ok(())
        })
    }
}

/// The capability required to dispatch `op`, per the protocol v1 core-op
/// table.
///
/// `out_capability` receives `present = 1` with the owned capability name, or
/// `present = 0` for a product-defined op with no core-table requirement.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_required_capability(
    op: SpokeConnectSlice,
    out_capability: *mut SpokeConnectOptionalBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_capability = require_out(out_capability, "out_capability")?;
            out_capability.write(SpokeConnectOptionalBuffer::empty());
            let op = borrowed_text(op, "op")?;
            let capability = match ffi::required_capability(op.to_owned()) {
                Some(capability) => SpokeConnectOptionalBuffer {
                    present: 1,
                    value: owned_buffer(capability.as_bytes()),
                },
                None => SpokeConnectOptionalBuffer::empty(),
            };
            out_capability.write(capability);
            Ok(())
        })
    }
}

/// The connect protocol version exchanged in `ConnectHello`.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_protocol_version(
    out_version: *mut u64,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_version = require_out(out_version, "out_version")?;
            out_version.write(ffi::protocol_version());
            Ok(())
        })
    }
}

/// Creates an empty `(peer_id, nonce)` replay store.
///
/// On success `out_handle` receives an owned handle; on failure it stays NULL
/// and no ownership transfers.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_nonce_store_new(
    out_handle: *mut *mut SpokeConnectNonceStore,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_handle = require_out(out_handle, "out_handle")?;
            out_handle.write(ptr::null_mut());
            let handle =
                Box::into_raw(Box::new(ffi::NonceStore::new())) as *mut SpokeConnectNonceStore;
            out_handle.write(handle);
            Ok(())
        })
    }
}

/// Records `(peer_id, nonce)` unless the pair was already accepted.
///
/// `out_recorded` receives 1 when the pair was fresh, 0 on replay.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_nonce_store_check_and_record(
    handle: *const SpokeConnectNonceStore,
    peer_id: SpokeConnectSlice,
    nonce: SpokeConnectSlice,
    out_recorded: *mut u8,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_recorded = require_out(out_recorded, "out_recorded")?;
            out_recorded.write(0);
            let store = nonce_store(handle)?;
            let peer_id = borrowed_text(peer_id, "peer_id")?;
            let nonce = borrowed_text(nonce, "nonce")?;
            let recorded = store.check_and_record(peer_id.to_owned(), nonce.to_owned());
            out_recorded.write(u8::from(recorded));
            Ok(())
        })
    }
}

/// Releases a nonce-store handle. A NULL handle is a no-op.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_nonce_store_free(handle: *mut SpokeConnectNonceStore) {
    contain_release(|| unsafe { release_handle(handle as *mut Arc<ffi::NonceStore>) });
}

/// Creates an outbound sequence counter starting at 0.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_outbound_sequence_new(
    out_handle: *mut *mut SpokeConnectOutboundSequence,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_handle = require_out(out_handle, "out_handle")?;
            out_handle.write(ptr::null_mut());
            let handle = Box::into_raw(Box::new(ffi::OutboundSequence::new()))
                as *mut SpokeConnectOutboundSequence;
            out_handle.write(handle);
            Ok(())
        })
    }
}

/// Assigns the next outbound sequence.
///
/// `out_sequence` receives the assigned value; sequence exhaustion is reported
/// as a failure status and the counter stays exhausted (sequences never wrap).
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_outbound_sequence_allocate(
    handle: *const SpokeConnectOutboundSequence,
    out_sequence: *mut u64,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_sequence = require_out(out_sequence, "out_sequence")?;
            out_sequence.write(0);
            let sequence = outbound_sequence(handle)?;
            let assigned = sequence.allocate()?;
            out_sequence.write(assigned);
            Ok(())
        })
    }
}

/// Releases an outbound-sequence handle. A NULL handle is a no-op.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_outbound_sequence_free(
    handle: *mut SpokeConnectOutboundSequence,
) {
    contain_release(|| unsafe { release_handle(handle as *mut Arc<ffi::OutboundSequence>) });
}

/// Creates an inbound sequence expectation starting at 0.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_inbound_sequence_new(
    out_handle: *mut *mut SpokeConnectInboundSequence,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_handle = require_out(out_handle, "out_handle")?;
            out_handle.write(ptr::null_mut());
            let handle = Box::into_raw(Box::new(ffi::InboundSequence::new()))
                as *mut SpokeConnectInboundSequence;
            out_handle.write(handle);
            Ok(())
        })
    }
}

/// Accepts `sequence` iff it is the next expected inbound sequence.
///
/// `out_next_expected` receives the advanced expectation. A replayed or
/// out-of-order sequence fails without advancing the expectation, and the
/// caller must reject the invoke without dispatching it.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_inbound_sequence_advance(
    handle: *const SpokeConnectInboundSequence,
    sequence: i64,
    out_next_expected: *mut u64,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_next_expected = require_out(out_next_expected, "out_next_expected")?;
            out_next_expected.write(0);
            let inbound = inbound_sequence(handle)?;
            let next_expected = inbound.advance(sequence)?;
            out_next_expected.write(next_expected);
            Ok(())
        })
    }
}

/// Releases an inbound-sequence handle. A NULL handle is a no-op.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_inbound_sequence_free(
    handle: *mut SpokeConnectInboundSequence,
) {
    contain_release(|| unsafe { release_handle(handle as *mut Arc<ffi::InboundSequence>) });
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{
        spoke_connect_abi_version, spoke_connect_buffer_free, spoke_connect_error_free,
        spoke_connect_optional_buffer_free, write_error, SPOKE_CONNECT_INVALID_ARGUMENT,
        SPOKE_CONNECT_OK,
    };

    /// The shared cross-language golden hello vector (crate SSOT): inputs and
    /// pinned output bytes. Loaded, never re-derived.
    const GOLDEN_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../spoke-connect/tests/fixtures/golden-hello.json"
    ));

    fn golden() -> serde_json::Value {
        serde_json::from_str(GOLDEN_FIXTURE).expect("golden-hello.json parses")
    }

    fn hex32(value: &str) -> [u8; 32] {
        assert_eq!(value.len(), 64, "32-byte hex field");
        let mut out = [0u8; 32];
        for (index, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).expect("hex byte");
        }
        out
    }

    fn bytes(value: &[u8]) -> SpokeConnectSlice {
        SpokeConnectSlice {
            data: value.as_ptr(),
            len: value.len(),
        }
    }

    fn text(value: &str) -> SpokeConnectSlice {
        bytes(value.as_bytes())
    }

    fn slices_of(values: &[&str]) -> Vec<SpokeConnectSlice> {
        values.iter().map(|value| text(value)).collect()
    }

    fn empty_record() -> SpokeConnectError {
        SpokeConnectError {
            message: SpokeConnectBuffer::empty(),
            code: SpokeConnectBuffer::empty(),
            kind: SpokeConnectBuffer::empty(),
            wire_code: SpokeConnectBuffer::empty(),
            expected: 0,
            actual: 0,
        }
    }

    /// Copies an owned buffer out and releases it.
    unsafe fn take_bytes(mut buffer: SpokeConnectBuffer) -> Vec<u8> {
        let bytes = if buffer.data.is_null() {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(buffer.data, buffer.len) }.to_vec()
        };
        unsafe { spoke_connect_buffer_free(&mut buffer) };
        bytes
    }

    unsafe fn take_text(buffer: SpokeConnectBuffer) -> String {
        String::from_utf8(unsafe { take_bytes(buffer) }).expect("output is UTF-8")
    }

    /// The record's `message`, after releasing the record.
    unsafe fn message_of(error: &mut SpokeConnectError) -> String {
        let message = if error.message.data.is_null() {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(error.message.data, error.message.len) }.to_vec()
        };
        unsafe { spoke_connect_error_free(error) };
        String::from_utf8(message).expect("error message is UTF-8")
    }

    /// A record snapshot; `None` means the field was absent (NULL, zero
    /// length).
    struct Snapshot {
        message: String,
        code: Option<String>,
        kind: Option<String>,
        wire_code: Option<String>,
        expected: u64,
        actual: i64,
    }

    /// Copies the record's fields and releases the record.
    unsafe fn snapshot(error: &mut SpokeConnectError) -> Snapshot {
        let field = |buffer: &SpokeConnectBuffer| {
            if buffer.data.is_null() {
                None
            } else {
                Some(
                    String::from_utf8(
                        unsafe { std::slice::from_raw_parts(buffer.data, buffer.len) }.to_vec(),
                    )
                    .expect("record field is UTF-8"),
                )
            }
        };
        let snapshot = Snapshot {
            message: field(&error.message).unwrap_or_default(),
            code: field(&error.code),
            kind: field(&error.kind),
            wire_code: field(&error.wire_code),
            expected: error.expected,
            actual: error.actual,
        };
        unsafe { spoke_connect_error_free(error) };
        snapshot
    }

    /// A golden-signed initiator hello, as JSON.
    unsafe fn golden_hello_json(golden: &serde_json::Value) -> String {
        let secret = hex32(golden["seed_hex"].as_str().expect("seed_hex"));
        let mut hello = SpokeConnectBuffer::empty();
        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_sign_hello_ed25519(
                bytes(&secret),
                text(golden["nonce"].as_str().expect("nonce")),
                text(golden["manifest_json"].as_str().expect("manifest_json")),
                &mut hello,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_OK, "{}", unsafe {
            message_of(&mut error)
        });
        unsafe { take_text(hello) }
    }

    #[test]
    fn abi_version_is_reported_and_its_out_pointer_is_mandatory() {
        let mut version = 0u64;
        let mut error = empty_record();
        let status = unsafe { spoke_connect_abi_version(&mut version, &mut error) };
        assert_eq!(status, SPOKE_CONNECT_OK);
        assert_eq!(version, crate::SPOKE_CONNECT_ABI_VERSION);
        unsafe { spoke_connect_error_free(&mut error) };

        // A NULL out pointer is invalid argument...
        let status = unsafe { spoke_connect_abi_version(ptr::null_mut(), &mut error) };
        assert_eq!(status, SPOKE_CONNECT_INVALID_ARGUMENT);
        assert!(!unsafe { message_of(&mut error) }.is_empty());

        // ...and a NULL record still yields the status, without one.
        let status = unsafe { spoke_connect_abi_version(ptr::null_mut(), ptr::null_mut()) };
        assert_eq!(status, SPOKE_CONNECT_INVALID_ARGUMENT);
    }

    #[test]
    fn golden_peer_id_derivation_matches_the_shared_vector() {
        let golden = golden();
        let pubkey = hex32(golden["pubkey_hex"].as_str().expect("pubkey_hex"));
        let mut peer_id = SpokeConnectBuffer::empty();
        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_derive_peer_id_from_ed25519_pubkey(
                bytes(&pubkey),
                &mut peer_id,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_OK, "{}", unsafe {
            message_of(&mut error)
        });
        // The owned buffer carries a trailing NUL byte outside `len`.
        assert_eq!(unsafe { *peer_id.data.add(peer_id.len) }, 0);
        assert_eq!(
            unsafe { take_text(peer_id) },
            golden["peer_id"].as_str().expect("peer_id")
        );
    }

    #[test]
    fn golden_hello_signature_and_verification_match_the_shared_vector() {
        let golden = golden();
        let pubkey = hex32(golden["pubkey_hex"].as_str().expect("pubkey_hex"));
        let peer_id = golden["peer_id"].as_str().expect("peer_id");
        let hello_json = unsafe { golden_hello_json(&golden) };

        let hello: serde_json::Value =
            serde_json::from_str(&hello_json).expect("signed hello parses");
        assert_eq!(hello["peer_id"].as_str(), Some(peer_id));
        assert_eq!(
            hello["signature"].as_str(),
            Some(golden["signature_b64u"].as_str().expect("signature_b64u"))
        );
        assert_eq!(hello["protocol_version"].as_u64(), Some(1));

        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_verify_hello_ed25519(
                bytes(&pubkey),
                text(peer_id),
                text(&hello_json),
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_OK, "{}", unsafe {
            message_of(&mut error)
        });
    }

    #[test]
    fn verify_rejects_a_tampered_signature() {
        let golden = golden();
        let pubkey = hex32(golden["pubkey_hex"].as_str().expect("pubkey_hex"));
        let mut hello: serde_json::Value =
            serde_json::from_str(&unsafe { golden_hello_json(&golden) }).expect("hello parses");
        hello["signature"] = serde_json::Value::from("not-a-valid-ed25519-signature");
        let tampered = serde_json::to_string(&hello).expect("serialize");

        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_verify_hello_ed25519(
                bytes(&pubkey),
                text(golden["peer_id"].as_str().expect("peer_id")),
                text(&tampered),
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_INVALID_HELLO_SIGNATURE);
        let record = unsafe { snapshot(&mut error) };
        assert!(!record.message.is_empty());
        assert!(record.code.is_none() && record.kind.is_none() && record.wire_code.is_none());
    }

    #[test]
    fn verify_rejects_a_peer_id_the_key_does_not_derive() {
        let golden = golden();
        let pubkey = hex32(golden["pubkey_hex"].as_str().expect("pubkey_hex"));
        let hello_json = unsafe { golden_hello_json(&golden) };
        let peer_id = golden["peer_id"].as_str().expect("peer_id");
        // A different (well-formed) peer id than the key derives.
        let unbound = format!("{}x", &peer_id[..peer_id.len() - 1]);

        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_verify_hello_ed25519(
                bytes(&pubkey),
                text(&unbound),
                text(&hello_json),
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_HANDSHAKE_FAILED);
        assert!(!unsafe { message_of(&mut error) }.is_empty());
    }

    #[test]
    fn verify_rejects_a_protocol_version_mismatch() {
        let golden = golden();
        let pubkey = hex32(golden["pubkey_hex"].as_str().expect("pubkey_hex"));
        let mut hello: serde_json::Value =
            serde_json::from_str(&unsafe { golden_hello_json(&golden) }).expect("hello parses");
        hello["protocol_version"] = serde_json::Value::from(2u64);
        let mismatched = serde_json::to_string(&hello).expect("serialize");

        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_verify_hello_ed25519(
                bytes(&pubkey),
                text(golden["peer_id"].as_str().expect("peer_id")),
                text(&mismatched),
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_PROTOCOL_VERSION_MISMATCH);
        assert!(
            unsafe { message_of(&mut error) }.contains("unsupported protocol_version 2"),
            "the dedicated kind carries its reason"
        );
    }

    #[test]
    fn sign_rejects_a_below_floor_nonce() {
        let golden = golden();
        let secret = hex32(golden["seed_hex"].as_str().expect("seed_hex"));
        // The golden nonce truncated below the wire floor (minLength 16).
        let short = &golden["nonce"].as_str().expect("nonce")[..8];
        let mut hello = SpokeConnectBuffer::empty();
        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_sign_hello_ed25519(
                bytes(&secret),
                text(short),
                text(golden["manifest_json"].as_str().expect("manifest_json")),
                &mut hello,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_INVALID_NONCE);
        assert!(hello.data.is_null(), "no result ownership on failure");
        assert!(!unsafe { message_of(&mut error) }.is_empty());
    }

    #[test]
    fn derive_peer_id_rejects_a_non_32_byte_key() {
        let golden = golden();
        let pubkey = hex32(golden["pubkey_hex"].as_str().expect("pubkey_hex"));
        let mut peer_id = SpokeConnectBuffer::empty();
        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_derive_peer_id_from_ed25519_pubkey(
                bytes(&pubkey[..31]),
                &mut peer_id,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_CRYPTO);
        assert!(peer_id.data.is_null(), "no result ownership on failure");
        assert!(unsafe { message_of(&mut error) }.contains("32-byte"));
    }

    #[test]
    fn nonce_store_is_single_use_per_peer() {
        let golden = golden();
        let peer_id = golden["peer_id"].as_str().expect("peer_id");
        let nonce = golden["nonce"].as_str().expect("nonce");
        let mut store = ptr::null_mut();
        let mut error = empty_record();
        let status = unsafe { spoke_connect_nonce_store_new(&mut store, &mut error) };
        assert_eq!(status, SPOKE_CONNECT_OK, "{}", unsafe {
            message_of(&mut error)
        });
        assert!(!store.is_null());

        let mut recorded = 0u8;
        let status = unsafe {
            spoke_connect_nonce_store_check_and_record(
                store,
                text(peer_id),
                text(nonce),
                &mut recorded,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_OK);
        assert_eq!(recorded, 1, "first use is accepted");

        let status = unsafe {
            spoke_connect_nonce_store_check_and_record(
                store,
                text(peer_id),
                text(nonce),
                &mut recorded,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_OK);
        assert_eq!(recorded, 0, "replay is rejected");

        // Nonce scoping is per sender peer id.
        let other_peer = format!("{}x", &peer_id[..peer_id.len() - 1]);
        let status = unsafe {
            spoke_connect_nonce_store_check_and_record(
                store,
                text(&other_peer),
                text(nonce),
                &mut recorded,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_OK);
        assert_eq!(recorded, 1, "another peer may reuse the nonce value");

        unsafe { spoke_connect_nonce_store_free(store) };
    }

    #[test]
    fn allowlist_is_fail_closed() {
        let golden = golden();
        let peer_id = golden["peer_id"].as_str().expect("peer_id");
        let allowlist = slices_of(&[peer_id, "12D3KooWNotListedNotListedNotListedNotListed"]);
        let mut allowed = 0u8;
        let mut error = empty_record();

        let status = unsafe {
            spoke_connect_is_allowlisted(
                allowlist.as_ptr(),
                allowlist.len(),
                text(peer_id),
                &mut allowed,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_OK);
        assert_eq!(allowed, 1, "listed peer is accepted");

        let status = unsafe {
            spoke_connect_is_allowlisted(
                allowlist.as_ptr(),
                allowlist.len(),
                text("12D3KooWOtherPeerOtherPeerOtherPeerOtherPeer"),
                &mut allowed,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_OK);
        assert_eq!(allowed, 0, "unlisted peer is rejected");

        // An empty allowlist rejects every peer (NULL is legal with count 0).
        let status = unsafe {
            spoke_connect_is_allowlisted(ptr::null(), 0, text(peer_id), &mut allowed, &mut error)
        };
        assert_eq!(status, SPOKE_CONNECT_OK);
        assert_eq!(allowed, 0, "fail-closed");
        unsafe { spoke_connect_error_free(&mut error) };
    }

    #[test]
    fn outbound_sequence_allocates_from_zero() {
        let mut sequence = ptr::null_mut();
        let mut error = empty_record();
        let status = unsafe { spoke_connect_outbound_sequence_new(&mut sequence, &mut error) };
        assert_eq!(status, SPOKE_CONNECT_OK, "{}", unsafe {
            message_of(&mut error)
        });

        for expected in 0..3u64 {
            let mut assigned = 7u64;
            let status = unsafe {
                spoke_connect_outbound_sequence_allocate(sequence, &mut assigned, &mut error)
            };
            assert_eq!(status, SPOKE_CONNECT_OK);
            assert_eq!(assigned, expected);
        }
        unsafe { spoke_connect_outbound_sequence_free(sequence) };
    }

    #[test]
    fn inbound_sequence_advances_and_rejects_replay() {
        let mut inbound = ptr::null_mut();
        let mut error = empty_record();
        let status = unsafe { spoke_connect_inbound_sequence_new(&mut inbound, &mut error) };
        assert_eq!(status, SPOKE_CONNECT_OK, "{}", unsafe {
            message_of(&mut error)
        });

        let mut next_expected = 0u64;
        let status = unsafe {
            spoke_connect_inbound_sequence_advance(inbound, 0, &mut next_expected, &mut error)
        };
        assert_eq!(status, SPOKE_CONNECT_OK);
        assert_eq!(next_expected, 1);

        // Replay: rejected, expectation unchanged, both numbers reported.
        let status = unsafe {
            spoke_connect_inbound_sequence_advance(inbound, 0, &mut next_expected, &mut error)
        };
        assert_eq!(status, SPOKE_CONNECT_INBOUND_SEQUENCE_MISMATCH);
        assert_eq!(next_expected, 0, "no result on failure");
        let record = unsafe { snapshot(&mut error) };
        assert_eq!(record.expected, 1);
        assert_eq!(record.actual, 0);
        assert!(!record.message.is_empty());

        // Out-of-order: same classification.
        let status = unsafe {
            spoke_connect_inbound_sequence_advance(inbound, 2, &mut next_expected, &mut error)
        };
        assert_eq!(status, SPOKE_CONNECT_INBOUND_SEQUENCE_MISMATCH);
        let record = unsafe { snapshot(&mut error) };
        assert_eq!(record.expected, 1);
        assert_eq!(record.actual, 2);

        // A negative wire sequence is a mismatch, not a wrap-around.
        let status = unsafe {
            spoke_connect_inbound_sequence_advance(inbound, -1, &mut next_expected, &mut error)
        };
        assert_eq!(status, SPOKE_CONNECT_INBOUND_SEQUENCE_MISMATCH);
        let record = unsafe { snapshot(&mut error) };
        assert_eq!(record.actual, -1);

        // The counter is still usable and advances on the expected sequence.
        let status = unsafe {
            spoke_connect_inbound_sequence_advance(inbound, 1, &mut next_expected, &mut error)
        };
        assert_eq!(status, SPOKE_CONNECT_OK);
        assert_eq!(next_expected, 2);
        unsafe { spoke_connect_inbound_sequence_free(inbound) };
    }

    #[test]
    fn response_correlation_checks_every_echo_field() {
        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_check_response_correlation(
                text("session-1"),
                3,
                text("request-1"),
                text("session-1"),
                3,
                text("request-1"),
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_OK, "{}", unsafe {
            message_of(&mut error)
        });

        for (session, sequence, request) in [
            ("other-session", 3u64, "request-1"),
            ("session-1", 4, "request-1"),
            ("session-1", 3, "other-request"),
        ] {
            let status = unsafe {
                spoke_connect_check_response_correlation(
                    text("session-1"),
                    3,
                    text("request-1"),
                    text(session),
                    sequence,
                    text(request),
                    &mut error,
                )
            };
            assert_eq!(
                status, SPOKE_CONNECT_CORRELATION_MISMATCH,
                "{session}/{sequence}/{request}"
            );
            assert!(!unsafe { message_of(&mut error) }.is_empty());
        }

        // Above the wire i64 range an echo can never match.
        let status = unsafe {
            spoke_connect_check_response_correlation(
                text("session-1"),
                u64::MAX,
                text("request-1"),
                text("session-1"),
                u64::MAX,
                text("request-1"),
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_CORRELATION_MISMATCH);
        unsafe { spoke_connect_error_free(&mut error) };
    }

    #[test]
    fn dispatch_gate_requires_the_negotiated_capability() {
        let mut allowed = 0u8;
        let mut error = empty_record();

        let baseline = slices_of(&["spoke-baseline"]);
        let status = unsafe {
            spoke_connect_dispatch_allowed(
                text("upsert"),
                baseline.as_ptr(),
                baseline.len(),
                &mut allowed,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_OK);
        assert_eq!(allowed, 1, "negotiated baseline authorizes baseline ops");

        let status = unsafe {
            spoke_connect_dispatch_allowed(text("upsert"), ptr::null(), 0, &mut allowed, &mut error)
        };
        assert_eq!(status, SPOKE_CONNECT_OK);
        assert_eq!(allowed, 0, "empty negotiated set denies");

        let status = unsafe {
            spoke_connect_dispatch_allowed(
                text("extract"),
                baseline.as_ptr(),
                baseline.len(),
                &mut allowed,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_OK);
        assert_eq!(allowed, 0, "baseline does not authorize extract");

        // A self-describing tool op requires the exact capability string.
        let tool = slices_of(&["tools.math.add"]);
        let status = unsafe {
            spoke_connect_dispatch_allowed(
                text("tools.math.add"),
                tool.as_ptr(),
                tool.len(),
                &mut allowed,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_OK);
        assert_eq!(allowed, 1);

        let status = unsafe {
            spoke_connect_dispatch_allowed(
                text("tools.math.add"),
                baseline.as_ptr(),
                baseline.len(),
                &mut allowed,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_OK);
        assert_eq!(allowed, 0, "no umbrella capability authorizes a tool op");

        let status = unsafe {
            spoke_connect_dispatch_allowed(
                text("product-defined-op"),
                baseline.as_ptr(),
                baseline.len(),
                &mut allowed,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_OK);
        assert_eq!(allowed, 0, "unknown ops fail closed");
        unsafe { spoke_connect_error_free(&mut error) };
    }

    #[test]
    fn required_capability_reports_the_core_table_and_absence() {
        let mut capability = SpokeConnectOptionalBuffer::empty();
        let mut error = empty_record();

        let status = unsafe {
            spoke_connect_required_capability(text("extract"), &mut capability, &mut error)
        };
        assert_eq!(status, SPOKE_CONNECT_OK);
        assert_eq!(capability.present, 1);
        assert_eq!(unsafe { take_text(capability.value) }, "ke-extraction");

        let status = unsafe {
            spoke_connect_required_capability(
                text("product-defined-op"),
                &mut capability,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_OK);
        assert_eq!(capability.present, 0, "no core-table requirement");
        assert!(capability.value.data.is_null());
        unsafe { spoke_connect_optional_buffer_free(&mut capability) };
        unsafe { spoke_connect_error_free(&mut error) };
    }

    #[test]
    fn protocol_version_is_reported() {
        let mut version = 0u64;
        let mut error = empty_record();
        let status = unsafe { spoke_connect_protocol_version(&mut version, &mut error) };
        assert_eq!(status, SPOKE_CONNECT_OK);
        assert_eq!(version, 1);
        unsafe { spoke_connect_error_free(&mut error) };
    }

    #[test]
    fn every_core_error_variant_maps_to_its_status() {
        let cases = [
            (ffi::CoreError::InvalidHelloSignature, 100),
            (ffi::CoreError::NonceReplay, 101),
            (
                ffi::CoreError::HandshakeFailed {
                    reason: "binding".to_owned(),
                },
                102,
            ),
            (
                ffi::CoreError::InvalidNonce {
                    message: "too short".to_owned(),
                },
                103,
            ),
            (
                ffi::CoreError::Crypto {
                    message: "bad key".to_owned(),
                },
                104,
            ),
            (
                ffi::CoreError::Jcs {
                    message: "canonicalization".to_owned(),
                },
                105,
            ),
            (
                ffi::CoreError::TokenInvalid {
                    message: "claims".to_owned(),
                },
                106,
            ),
            (
                ffi::CoreError::ProtocolVersionMismatch {
                    reason: "unsupported protocol_version 2 (expected 1)".to_owned(),
                },
                107,
            ),
        ];
        assert_eq!(cases.len(), 8, "every CoreError variant is covered");

        for (error, status) in cases {
            let payload = match &error {
                ffi::CoreError::HandshakeFailed { reason }
                | ffi::CoreError::ProtocolVersionMismatch { reason } => reason.clone(),
                ffi::CoreError::InvalidNonce { message }
                | ffi::CoreError::Crypto { message }
                | ffi::CoreError::Jcs { message }
                | ffi::CoreError::TokenInvalid { message } => message.clone(),
                _ => error.to_string(),
            };
            let failure = AbiFailure::from(error);
            assert_eq!(failure.status, status);
            assert_eq!(failure.message, payload, "the variant payload is preserved");
            assert_eq!(failure.expected, 0);
            assert_eq!(failure.actual, 0);

            let mut record = empty_record();
            unsafe { write_error(&mut record, &failure) };
            let record = unsafe { snapshot(&mut record) };
            assert_eq!(record.message, payload);
            assert!(record.code.is_none() && record.kind.is_none() && record.wire_code.is_none());
        }
    }

    #[test]
    fn every_core_invoke_error_variant_maps_to_its_status() {
        let exhausted = AbiFailure::from(ffi::CoreInvokeError::SequenceExhausted);
        assert_eq!(exhausted.status, SPOKE_CONNECT_SEQUENCE_EXHAUSTED);
        assert!(!exhausted.message.is_empty());

        let mismatched = AbiFailure::from(ffi::CoreInvokeError::InboundSequenceMismatch {
            expected: 3,
            actual: 7,
        });
        assert_eq!(mismatched.status, SPOKE_CONNECT_INBOUND_SEQUENCE_MISMATCH);
        assert_eq!(mismatched.expected, 3);
        assert_eq!(mismatched.actual, 7);

        let mut record = empty_record();
        unsafe { write_error(&mut record, &mismatched) };
        let record = unsafe { snapshot(&mut record) };
        assert_eq!(record.expected, 3);
        assert_eq!(record.actual, 7);
        assert!(!record.message.is_empty());

        let correlation = AbiFailure::from(ffi::CoreInvokeError::CorrelationMismatch);
        assert_eq!(correlation.status, SPOKE_CONNECT_CORRELATION_MISMATCH);
        assert!(!correlation.message.is_empty());
    }

    #[test]
    fn malformed_inputs_are_rejected_as_invalid_argument() {
        let golden = golden();
        let peer_id = golden["peer_id"].as_str().expect("peer_id");
        let secret = hex32(golden["seed_hex"].as_str().expect("seed_hex"));
        let mut error = empty_record();
        let mut hello = SpokeConnectBuffer::empty();

        // NULL data with a non-zero length.
        let status = unsafe {
            spoke_connect_sign_hello_ed25519(
                SpokeConnectSlice {
                    data: ptr::null(),
                    len: 32,
                },
                text(golden["nonce"].as_str().expect("nonce")),
                text(golden["manifest_json"].as_str().expect("manifest_json")),
                &mut hello,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_INVALID_ARGUMENT);
        assert!(unsafe { message_of(&mut error) }.contains("non-zero length"));

        // Non-UTF-8 text.
        let invalid = [0xffu8, 0xfe];
        let status = unsafe {
            spoke_connect_sign_hello_ed25519(
                bytes(&secret),
                bytes(&invalid),
                text(golden["manifest_json"].as_str().expect("manifest_json")),
                &mut hello,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_INVALID_ARGUMENT);
        assert!(unsafe { message_of(&mut error) }.contains("UTF-8"));

        // NULL array with a non-zero count.
        let mut allowed = 0u8;
        let status = unsafe {
            spoke_connect_is_allowlisted(ptr::null(), 1, text(peer_id), &mut allowed, &mut error)
        };
        assert_eq!(status, SPOKE_CONNECT_INVALID_ARGUMENT);
        assert!(unsafe { message_of(&mut error) }.contains("non-zero count"));

        // NULL out pointer.
        let status = unsafe {
            spoke_connect_derive_peer_id_from_ed25519_pubkey(
                bytes(&hex32(golden["pubkey_hex"].as_str().expect("pubkey_hex"))),
                ptr::null_mut(),
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_INVALID_ARGUMENT);
        assert!(unsafe { message_of(&mut error) }.contains("out_peer_id"));

        // NULL object handles.
        let store: *const SpokeConnectNonceStore = ptr::null();
        let status = unsafe {
            spoke_connect_nonce_store_check_and_record(
                store,
                text(peer_id),
                text(golden["nonce"].as_str().expect("nonce")),
                &mut allowed,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_INVALID_ARGUMENT);
        assert!(unsafe { message_of(&mut error) }.contains("NULL"));

        let mut next_expected = 0u64;
        let inbound: *const SpokeConnectInboundSequence = ptr::null();
        let status = unsafe {
            spoke_connect_inbound_sequence_advance(inbound, 0, &mut next_expected, &mut error)
        };
        assert_eq!(status, SPOKE_CONNECT_INVALID_ARGUMENT);

        let outbound: *const SpokeConnectOutboundSequence = ptr::null();
        let mut assigned = 0u64;
        let status = unsafe {
            spoke_connect_outbound_sequence_allocate(outbound, &mut assigned, &mut error)
        };
        assert_eq!(status, SPOKE_CONNECT_INVALID_ARGUMENT);
        unsafe { spoke_connect_error_free(&mut error) };
    }

    #[test]
    fn releases_are_total_and_zero_their_targets() {
        let golden = golden();
        let pubkey = hex32(golden["pubkey_hex"].as_str().expect("pubkey_hex"));
        let mut peer_id = SpokeConnectBuffer::empty();
        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_derive_peer_id_from_ed25519_pubkey(
                bytes(&pubkey),
                &mut peer_id,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_OK);
        assert!(!peer_id.data.is_null());
        unsafe { spoke_connect_buffer_free(&mut peer_id) };
        assert!(peer_id.data.is_null() && peer_id.len == 0, "zeroed");

        // Releasing an empty buffer and a NULL target is a no-op.
        unsafe { spoke_connect_buffer_free(&mut peer_id) };
        unsafe { spoke_connect_buffer_free(ptr::null_mut()) };
        unsafe { spoke_connect_error_free(ptr::null_mut()) };
        unsafe { spoke_connect_optional_buffer_free(ptr::null_mut()) };
        unsafe { spoke_connect_nonce_store_free(ptr::null_mut()) };
        unsafe { spoke_connect_outbound_sequence_free(ptr::null_mut()) };
        unsafe { spoke_connect_inbound_sequence_free(ptr::null_mut()) };

        // A zeroed record and an absent optional release cleanly.
        let mut empty = empty_record();
        unsafe { spoke_connect_error_free(&mut empty) };
        assert!(empty.message.data.is_null() && empty.expected == 0 && empty.actual == 0);
        let mut optional = SpokeConnectOptionalBuffer::empty();
        unsafe { spoke_connect_optional_buffer_free(&mut optional) };
        assert_eq!(optional.present, 0);
    }
}
