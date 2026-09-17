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
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

mod core;

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
    pub(crate) expected: u64,
    pub(crate) actual: i64,
}

impl AbiFailure {
    /// A C-boundary validation failure (never an error the facade produced).
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self {
            status: SPOKE_CONNECT_INVALID_ARGUMENT,
            message: message.into(),
            expected: 0,
            actual: 0,
        }
    }

    /// A facade failure that carries only a status and a message.
    pub(crate) fn at(status: i32, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            expected: 0,
            actual: 0,
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
        code: SpokeConnectBuffer::empty(),
        kind: SpokeConnectBuffer::empty(),
        wire_code: SpokeConnectBuffer::empty(),
        expected: failure.expected,
        actual: failure.actual,
    };
    unsafe { out_error.write(record) };
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
