//! SPOKE Connect C ABI carrier — hand-written C boundary over the
//! [`spoke_connect::ffi`] Rust facade.
//!
//! This crate performs pointer validation, C/Rust value conversion, handle
//! ownership and error projection only. The connect rules themselves (session
//! core, dispatch, cryptography, sequencing, adapter behavior) stay in
//! `spoke-connect`; nothing is reimplemented here.
//!
//! # Boundary conventions
//!
//! - Fallible operations return an `int32_t` status and write their result to
//!   caller-supplied out parameters. On failure they write a
//!   [`SpokeConnectError`] record unless `out_error` is NULL.
//! - Borrowed byte/text inputs cross as [`SpokeConnectSlice`] (pointer +
//!   length). `NULL` is legal only with length zero; text is validated as
//!   UTF-8; the length is authoritative (no `strlen`).
//! - Owned outputs cross as [`SpokeConnectBuffer`]: allocated by this library
//!   with a trailing NUL byte (excluded from `len`) and released by
//!   [`spoke_connect_buffer_free`] or the owning `*_free` function.
//! - Object handles are opaque, separately named structs; each pointer owns a
//!   boxed Rust handle and is released exactly once with its `*_free`
//!   function. A NULL free is a no-op.
//! - Out pointers are mandatory, writable, non-aliasing and zeroed before work
//!   starts. A NULL out pointer or a NULL/non-zero input span is invalid
//!   argument.
//! - Every exported entry contains unwinding before it crosses C: a caught
//!   panic becomes status [`SPOKE_CONNECT_PANIC`]. Release functions contain a
//!   destructor panic and return without unwinding.
//!
//! Rust cannot validate forged, dangling or concurrently freed non-NULL
//! pointers: those are caller contract violations, not recoverable errors.

#![deny(unsafe_op_in_unsafe_fn)]

use std::any::Any;
use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

mod bridge;
mod core;
mod remote_adapter;
mod responder;

/// Record-layout report consumed by the C ABI gate
/// (`tooling/connect/cpp-symbol-check.mjs`); test-only, never exported.
#[cfg(test)]
mod abi_layout;

/// The C boundary revision exposed by [`spoke_connect_abi_version`]; distinct
/// from the connect hello protocol version.
pub const SPOKE_CONNECT_ABI_VERSION: u64 = 1;

/// Call succeeded.
pub const SPOKE_CONNECT_OK: i32 = 0;
/// C-boundary validation failure (NULL out pointer, NULL/non-zero input span,
/// non-UTF-8 text). Local to the boundary.
pub const SPOKE_CONNECT_INVALID_ARGUMENT: i32 = 1;
/// A caught panic inside the wrapper. Local to the boundary.
pub const SPOKE_CONNECT_PANIC: i32 = 2;

/// Core hello-gate / identity failures (`spoke_connect::ffi::CoreError`).
pub const SPOKE_CONNECT_INVALID_HELLO_SIGNATURE: i32 = 100;
/// See [`SPOKE_CONNECT_INVALID_HELLO_SIGNATURE`].
pub const SPOKE_CONNECT_NONCE_REPLAY: i32 = 101;
/// See [`SPOKE_CONNECT_INVALID_HELLO_SIGNATURE`].
pub const SPOKE_CONNECT_HANDSHAKE_FAILED: i32 = 102;
/// See [`SPOKE_CONNECT_INVALID_HELLO_SIGNATURE`].
pub const SPOKE_CONNECT_INVALID_NONCE: i32 = 103;
/// See [`SPOKE_CONNECT_INVALID_HELLO_SIGNATURE`].
pub const SPOKE_CONNECT_CRYPTO: i32 = 104;
/// See [`SPOKE_CONNECT_INVALID_HELLO_SIGNATURE`].
pub const SPOKE_CONNECT_JCS: i32 = 105;
/// See [`SPOKE_CONNECT_INVALID_HELLO_SIGNATURE`].
pub const SPOKE_CONNECT_TOKEN_INVALID: i32 = 106;
/// See [`SPOKE_CONNECT_INVALID_HELLO_SIGNATURE`].
pub const SPOKE_CONNECT_PROTOCOL_VERSION_MISMATCH: i32 = 107;

/// Core invoke-path failures (`spoke_connect::ffi::CoreInvokeError`).
pub const SPOKE_CONNECT_SEQUENCE_EXHAUSTED: i32 = 200;
/// See [`SPOKE_CONNECT_SEQUENCE_EXHAUSTED`].
pub const SPOKE_CONNECT_INBOUND_SEQUENCE_MISMATCH: i32 = 201;
/// See [`SPOKE_CONNECT_SEQUENCE_EXHAUSTED`].
pub const SPOKE_CONNECT_CORRELATION_MISMATCH: i32 = 202;

/// Adapter/responder dial failure before any session exists
/// (`spoke_connect::ffi::FfiError::Dial`); `kind` carries the dial kind.
pub const SPOKE_CONNECT_FFI_DIAL: i32 = 300;
/// Invoke-path application rejection
/// (`spoke_connect::ffi::FfiError::Rejected`); `code` carries the reject code
/// and `kind` / `wire_code` are populated where the facade exposes them.
pub const SPOKE_CONNECT_FFI_REJECTED: i32 = 301;
/// The foreign callback `Transport` is closed (its own vocabulary, mapped
/// from [`spoke_connect::ffi::TransportError::Closed`]).
pub const SPOKE_CONNECT_TRANSPORT_CLOSED: i32 = 400;
/// Transport-level I/O failure, including a contained foreign-callback fault
/// (mapped from [`spoke_connect::ffi::TransportError::Io`]).
pub const SPOKE_CONNECT_TRANSPORT_IO: i32 = 401;

/// Borrowed byte/text input: `data` is valid for the duration of the call.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SpokeConnectSlice {
    pub data: *const u8,
    pub len: usize,
}

/// Owned output buffer: `len` payload bytes plus a trailing NUL byte written
/// by this library.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SpokeConnectBuffer {
    pub data: *mut u8,
    pub len: usize,
}

impl SpokeConnectBuffer {
    /// The empty buffer: no allocation, nothing to release.
    pub(crate) const fn empty() -> Self {
        Self {
            data: ptr::null_mut(),
            len: 0,
        }
    }
}

/// Optional owned buffer: `present = 0` means absent (zero value); `present =
/// 1` means the contained buffer is owned, including a present empty string.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SpokeConnectOptionalBuffer {
    pub present: u8,
    pub value: SpokeConnectBuffer,
}

impl SpokeConnectOptionalBuffer {
    /// The absent optional: `present = 0`, zero value.
    pub(crate) const fn empty() -> Self {
        Self {
            present: 0,
            value: SpokeConnectBuffer::empty(),
        }
    }
}

/// Optional unsigned scalar: `present = 0` means absent (zero value);
/// `present = 1` means `value` is meaningful. Any other `present` value is
/// invalid input.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SpokeConnectOptionalU64 {
    pub present: u8,
    pub value: u64,
}

/// Converts an optional scalar crossing from C. Only `0` / `1` are accepted
/// presence values; a present value keeps its payload even when zero.
pub(crate) fn optional_u64(
    value: SpokeConnectOptionalU64,
    what: &str,
) -> Result<Option<u64>, AbiFailure> {
    match value.present {
        0 => Ok(None),
        1 => Ok(Some(value.value)),
        other => Err(AbiFailure::invalid(format!(
            "{what}: present must be 0 or 1, got {other}"
        ))),
    }
}

/// Buffer returned by a foreign callback: `data` is host-owned and valid
/// until the release function runs. Rust copies the bytes and then calls
/// `release` exactly once with `release_context` and the same pointer/length.
/// A NULL `data` (or zero `len`) is empty and needs no release; a host that
/// returns static storage supplies a no-op release.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SpokeConnectForeignBuffer {
    pub data: *const u8,
    pub len: usize,
    pub release_context: *mut c_void,
    pub release: Option<unsafe extern "C" fn(*mut c_void, *const u8, usize)>,
}

impl SpokeConnectForeignBuffer {
    /// The empty callback buffer.
    pub(crate) const fn empty() -> Self {
        Self {
            data: ptr::null(),
            len: 0,
            release_context: ptr::null_mut(),
            release: None,
        }
    }
}

/// Failure record a foreign callback writes: the same four textual fields as
/// [`SpokeConnectError`], carried as foreign buffers.
#[repr(C)]
pub struct SpokeConnectForeignError {
    pub message: SpokeConnectForeignBuffer,
    pub code: SpokeConnectForeignBuffer,
    pub kind: SpokeConnectForeignBuffer,
    pub wire_code: SpokeConnectForeignBuffer,
}

impl SpokeConnectForeignError {
    /// The empty callback error record.
    pub(crate) const fn empty() -> Self {
        Self {
            message: SpokeConnectForeignBuffer::empty(),
            code: SpokeConnectForeignBuffer::empty(),
            kind: SpokeConnectForeignBuffer::empty(),
            wire_code: SpokeConnectForeignBuffer::empty(),
        }
    }
}

/// Copies a foreign buffer's bytes and releases it exactly once. Only a
/// populated buffer (non-NULL data and non-zero length) is released — a zero
/// buffer is empty and needs no release. The buffer is zeroed afterwards, so
/// a second read is empty and releases nothing.
pub(crate) unsafe fn take_foreign_bytes(buffer: &mut SpokeConnectForeignBuffer) -> Vec<u8> {
    let bytes = if buffer.data.is_null() || buffer.len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(buffer.data, buffer.len) }.to_vec()
    };
    if !buffer.data.is_null() && buffer.len > 0 {
        if let Some(release) = buffer.release {
            unsafe { release(buffer.release_context, buffer.data, buffer.len) };
        }
    }
    *buffer = SpokeConnectForeignBuffer::empty();
    bytes
}

/// Copies a foreign callback's text buffer. Invalid UTF-8 is rendered
/// lossily — callback text is diagnostic, never a wire value.
pub(crate) unsafe fn take_foreign_text(buffer: &mut SpokeConnectForeignBuffer) -> String {
    String::from_utf8_lossy(&unsafe { take_foreign_bytes(buffer) }).into_owned()
}

/// Reads and releases every field of a foreign callback error record.
pub(crate) unsafe fn take_foreign_error(
    error: &mut SpokeConnectForeignError,
) -> (String, String, String, String) {
    unsafe {
        (
            take_foreign_text(&mut error.message),
            take_foreign_text(&mut error.code),
            take_foreign_text(&mut error.kind),
            take_foreign_text(&mut error.wire_code),
        )
    }
}

/// Converts a facade `FfiError` into the boundary projection: `Dial` keeps its
/// `kind`, `Rejected` keeps its code plus the optional `kind` / `wire_code`
/// exactly as the facade exposes them (`A2` §Status and error projection).
impl From<spoke_connect::ffi::FfiError> for AbiFailure {
    fn from(error: spoke_connect::ffi::FfiError) -> Self {
        match error {
            spoke_connect::ffi::FfiError::Dial { kind, message } => AbiFailure {
                status: SPOKE_CONNECT_FFI_DIAL,
                message,
                code: None,
                kind: Some(kind),
                wire_code: None,
                expected: 0,
                actual: 0,
            },
            spoke_connect::ffi::FfiError::Rejected {
                code,
                message,
                kind,
                wire_code,
            } => AbiFailure {
                status: SPOKE_CONNECT_FFI_REJECTED,
                message,
                code: Some(code),
                kind,
                wire_code,
                expected: 0,
                actual: 0,
            },
        }
    }
}

/// Converts the callback transport's own error vocabulary into the boundary
/// projection (statuses 400 / 401).
impl From<spoke_connect::ffi::TransportError> for AbiFailure {
    fn from(error: spoke_connect::ffi::TransportError) -> Self {
        match error {
            spoke_connect::ffi::TransportError::Closed => {
                AbiFailure::at(SPOKE_CONNECT_TRANSPORT_CLOSED, "transport is closed")
            }
            spoke_connect::ffi::TransportError::Io(message) => {
                AbiFailure::at(SPOKE_CONNECT_TRANSPORT_IO, message)
            }
        }
    }
}

/// Failure out-record. `message` is owned; `code` / `kind` / `wire_code` are
/// owned where the underlying error carries them and NULL otherwise;
/// `expected` / `actual` carry an inbound sequence mismatch.
#[repr(C)]
pub struct SpokeConnectError {
    pub message: SpokeConnectBuffer,
    pub code: SpokeConnectBuffer,
    pub kind: SpokeConnectBuffer,
    pub wire_code: SpokeConnectBuffer,
    pub expected: u64,
    pub actual: i64,
}

/// A projected failure: the status an exported function returns plus the
/// fields of the record it writes.
pub(crate) struct AbiFailure {
    pub(crate) status: i32,
    pub(crate) message: String,
    /// Application reject code ([`SPOKE_CONNECT_FFI_REJECTED`] only).
    pub(crate) code: Option<String>,
    /// Dial kind or internal-error kind, where the underlying error carries
    /// one.
    pub(crate) kind: Option<String>,
    /// Wire code of a dispatch deny or an unknown wire code, where present.
    pub(crate) wire_code: Option<String>,
    pub(crate) expected: u64,
    pub(crate) actual: i64,
}

impl AbiFailure {
    /// A C-boundary validation failure (never an error the facade produced).
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::at(SPOKE_CONNECT_INVALID_ARGUMENT, message)
    }

    /// A facade failure that carries only a status and a message.
    pub(crate) fn at(status: i32, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            code: None,
            kind: None,
            wire_code: None,
            expected: 0,
            actual: 0,
        }
    }

    /// A failure that also carries the inbound sequence pair.
    pub(crate) fn sequence(status: i32, message: impl Into<String>, expected: u64, actual: i64) -> Self {
        Self {
            expected,
            actual,
            ..Self::at(status, message)
        }
    }
}

/// Runs `body` and projects its outcome onto the C boundary, containing
/// unwinding. Shared by every exported entry.
pub(crate) unsafe fn export(
    out_error: *mut SpokeConnectError,
    body: impl FnOnce() -> Result<(), AbiFailure>,
) -> i32 {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(())) => SPOKE_CONNECT_OK,
        Ok(Err(failure)) => {
            unsafe { write_error(out_error, &failure) };
            failure.status
        }
        Err(payload) => {
            let failure = AbiFailure::at(SPOKE_CONNECT_PANIC, panic_message(&*payload));
            unsafe { write_error(out_error, &failure) };
            SPOKE_CONNECT_PANIC
        }
    }
}

/// Runs a release body, containing a destructor panic so it never unwinds
/// across the C boundary.
pub(crate) fn contain_release(body: impl FnOnce()) {
    let _ = catch_unwind(AssertUnwindSafe(body));
}

/// Writes `failure` into the caller's record. A NULL record is allowed: the
/// status is still returned.
pub(crate) unsafe fn write_error(out_error: *mut SpokeConnectError, failure: &AbiFailure) {
    if out_error.is_null() {
        return;
    }
    let record = SpokeConnectError {
        message: owned_buffer(failure.message.as_bytes()),
        code: optional_buffer(failure.code.as_deref()),
        kind: optional_buffer(failure.kind.as_deref()),
        wire_code: optional_buffer(failure.wire_code.as_deref()),
        expected: failure.expected,
        actual: failure.actual,
    };
    unsafe { out_error.write(record) };
}

/// An owned buffer for a carried field, or the empty buffer when absent. A
/// present empty string stays distinguishable from absence (non-NULL data,
/// zero length).
fn optional_buffer(text: Option<&str>) -> SpokeConnectBuffer {
    match text {
        Some(text) => owned_buffer(text.as_bytes()),
        None => SpokeConnectBuffer::empty(),
    }
}

/// Requires a non-NULL out pointer.
pub(crate) fn require_out<T>(out: *mut T, what: &str) -> Result<*mut T, AbiFailure> {
    if out.is_null() {
        return Err(AbiFailure::invalid(format!("{what} is NULL")));
    }
    Ok(out)
}

/// Copies `bytes` into a library-owned buffer with a trailing NUL byte.
pub(crate) fn owned_buffer(bytes: &[u8]) -> SpokeConnectBuffer {
    let mut owned = Vec::with_capacity(bytes.len() + 1);
    owned.extend_from_slice(bytes);
    owned.push(0);
    let mut boxed = owned.into_boxed_slice();
    let data = boxed.as_mut_ptr();
    std::mem::forget(boxed);
    SpokeConnectBuffer {
        data,
        len: bytes.len(),
    }
}

/// Releases a library-owned buffer and zeroes the struct. A zero buffer (NULL
/// data) is a no-op.
pub(crate) unsafe fn release_buffer(buffer: &mut SpokeConnectBuffer) {
    if !buffer.data.is_null() {
        let raw = ptr::slice_from_raw_parts_mut(buffer.data, buffer.len + 1);
        unsafe { drop(Box::from_raw(raw)) };
    }
    *buffer = SpokeConnectBuffer::empty();
}

/// Releases a boxed object handle. A NULL handle is a no-op.
pub(crate) unsafe fn release_handle<T>(handle: *mut T) {
    if handle.is_null() {
        return;
    }
    contain_release(|| unsafe { drop(Box::from_raw(handle)) });
}

/// Borrows a [`SpokeConnectSlice`] as bytes.
pub(crate) unsafe fn borrowed_bytes<'a>(
    value: SpokeConnectSlice,
    what: &str,
) -> Result<&'a [u8], AbiFailure> {
    if value.len > isize::MAX as usize {
        return Err(AbiFailure::invalid(format!(
            "{what}: length {} exceeds isize::MAX",
            value.len
        )));
    }
    if value.data.is_null() {
        if value.len == 0 {
            return Ok(&[]);
        }
        return Err(AbiFailure::invalid(format!(
            "{what}: NULL data with non-zero length {}",
            value.len
        )));
    }
    Ok(unsafe { std::slice::from_raw_parts(value.data, value.len) })
}

/// Borrows a [`SpokeConnectSlice`] as UTF-8 text.
pub(crate) unsafe fn borrowed_text<'a>(
    value: SpokeConnectSlice,
    what: &str,
) -> Result<&'a str, AbiFailure> {
    let bytes = unsafe { borrowed_bytes(value, what) }?;
    std::str::from_utf8(bytes).map_err(|_| AbiFailure::invalid(format!("{what}: not valid UTF-8")))
}

/// Borrows a pointer + count array of slices. A NULL pointer is legal only
/// with count zero.
pub(crate) unsafe fn borrowed_slices<'a>(
    values: *const SpokeConnectSlice,
    count: usize,
    what: &str,
) -> Result<&'a [SpokeConnectSlice], AbiFailure> {
    if count == 0 {
        return Ok(&[]);
    }
    if values.is_null() {
        return Err(AbiFailure::invalid(format!(
            "{what}: NULL pointer with non-zero count {count}"
        )));
    }
    let bytes = count
        .checked_mul(std::mem::size_of::<SpokeConnectSlice>())
        .ok_or_else(|| AbiFailure::invalid(format!("{what}: count {count} overflows")))?;
    if bytes > isize::MAX as usize {
        return Err(AbiFailure::invalid(format!(
            "{what}: count {count} exceeds isize::MAX"
        )));
    }
    Ok(unsafe { std::slice::from_raw_parts(values, count) })
}

/// Borrows a pointer + count array of slices and validates each entry as
/// UTF-8 text.
pub(crate) unsafe fn borrowed_strings(
    values: *const SpokeConnectSlice,
    count: usize,
    what: &str,
) -> Result<Vec<String>, AbiFailure> {
    let entries = unsafe { borrowed_slices(values, count, what) }?;
    entries
        .iter()
        .map(|entry| unsafe { borrowed_text(*entry, what) }.map(str::to_owned))
        .collect()
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&'static str>() {
        return (*text).to_owned();
    }
    if let Some(text) = payload.downcast_ref::<String>() {
        return text.clone();
    }
    "panic without a string payload".to_owned()
}

/// Reports the C boundary revision. Not the hello protocol version.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_abi_version(
    out_version: *mut u64,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_version = require_out(out_version, "out_version")?;
            out_version.write(SPOKE_CONNECT_ABI_VERSION);
            Ok(())
        })
    }
}

/// Releases an owned buffer produced by this library and zeroes the struct.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_buffer_free(buffer: *mut SpokeConnectBuffer) {
    if buffer.is_null() {
        return;
    }
    contain_release(|| unsafe { release_buffer(&mut *buffer) });
}

/// Releases the buffer contained in an optional buffer and marks it absent.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_optional_buffer_free(
    optional: *mut SpokeConnectOptionalBuffer,
) {
    if optional.is_null() {
        return;
    }
    contain_release(|| unsafe {
        let optional = &mut *optional;
        release_buffer(&mut optional.value);
        optional.present = 0;
    });
}

/// Releases every field of an error record and zeroes the struct.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_error_free(error: *mut SpokeConnectError) {
    if error.is_null() {
        return;
    }
    contain_release(|| unsafe {
        let error = &mut *error;
        release_buffer(&mut error.message);
        release_buffer(&mut error.code);
        release_buffer(&mut error.kind);
        release_buffer(&mut error.wire_code);
        error.expected = 0;
        error.actual = 0;
    });
}
