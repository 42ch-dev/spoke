//! Neutral carrier seam: the foreign-callback bridges both export faces share.
//!
//! [`remote_adapter`](crate::remote_adapter) and [`responder`](crate::responder)
//! each own their exported `spoke_connect_*` entries, but both cross the same
//! two foreign-callback boundaries and write their results through the same
//! projections, so neither can own them without the other reaching into its
//! internals:
//!
//! - the `Transport` callback context a host supplies — a dialer dials over it,
//!   a responder is created over it ([`ForeignTransport`] / [`SharedForeignTransport`]),
//! - the `ToolHandler` callback context a host supplies — either face registers
//!   it ([`ForeignToolHandler`] / [`SharedForeignToolHandler`]),
//! - the callback-call plumbing both bridges use ([`callback_json`] and friends), and
//! - the owned JSON/text projections every JSON-returning entry writes.
//!
//! The two vtable *records* stay with the face whose C surface declares them
//! (`SpokeConnectTransportTable` in `remote_adapter`, `SpokeConnectToolHandlerTable`
//! in `responder`); this module consumes them and owns everything that crosses
//! the boundary above them.
//!
//! Nothing here is exported through the C ABI: the `*_new` / `*_free` entries
//! that hand these bridges out stay with the face they belong to.

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use spoke_connect::ffi;

use crate::remote_adapter::SpokeConnectTransportTable;
use crate::responder::SpokeConnectToolHandlerTable;
use crate::{
    contain_release, owned_buffer, take_foreign_bytes, take_foreign_error, take_foreign_text,
    AbiFailure, SpokeConnectBuffer, SpokeConnectForeignBuffer, SpokeConnectForeignError,
    SpokeConnectOptionalBuffer, SpokeConnectSlice, SPOKE_CONNECT_FFI_REJECTED,
    SPOKE_CONNECT_OK, SPOKE_CONNECT_TRANSPORT_CLOSED, SPOKE_CONNECT_TRANSPORT_IO,
};

// ── Foreign-callback transport bridge (A2) ──────────────────────────────

/// Opaque foreign-callback `Transport` handle.
#[repr(C)]
pub struct SpokeConnectTransport {
    _private: [u8; 0],
}

/// The carrier's [`ffi::Transport`] implementation over a host vtable.
pub(crate) struct ForeignTransport {
    table: SpokeConnectTransportTable,
    user_data: *mut c_void,
    /// Set by the first `close`, so a `recv` entered afterwards fails fast
    /// without touching the host (a `recv` already waiting is unblocked by
    /// the host's own close callback).
    closed: AtomicBool,
    /// The first `close` outcome, reused by every later or concurrent
    /// caller: the host close runs once and concurrent closes never
    /// serialize behind a pending `recv`.
    close_result: Mutex<Option<Result<(), ffi::TransportError>>>,
}

// SAFETY: the A2 callback contract requires the host context, its callbacks
// and `destroy` to be thread-safe and free of thread affinity.
unsafe impl Send for ForeignTransport {}
unsafe impl Sync for ForeignTransport {}

impl Drop for ForeignTransport {
    fn drop(&mut self) {
        if let Some(destroy) = self.table.destroy {
            contain_release(|| unsafe { destroy(self.user_data) });
        }
    }
}

/// Maps a non-zero callback status onto the transport error vocabulary. The
/// record is read and released either way, so a host that populates buffers
/// on a failure status does not leak them. An unknown status is contained as
/// a transport I/O failure rather than propagated.
unsafe fn transport_error(status: i32, error: &mut SpokeConnectForeignError) -> ffi::TransportError {
    let (message, _code, kind, _wire_code) = unsafe { take_foreign_error(error) };
    let detail = if message.is_empty() { kind } else { message };
    match status {
        SPOKE_CONNECT_TRANSPORT_CLOSED => ffi::TransportError::Closed,
        SPOKE_CONNECT_TRANSPORT_IO => ffi::TransportError::Io(if detail.is_empty() {
            "transport I/O failure".to_owned()
        } else {
            detail
        }),
        other => ffi::TransportError::Io(format!(
            "unsupported transport callback status {other}: {detail}"
        )),
    }
}

/// Turns one callback return into the transport vocabulary and drains its
/// error record on **every** path: a host may populate any of the four
/// buffers on any status, and `A2` §C requires each populated buffer to
/// transfer to Rust and be released exactly once on success as well as on
/// error. The status mapping is unchanged — only the release happens here.
unsafe fn callback_result(
    status: i32,
    error: &mut SpokeConnectForeignError,
) -> Result<(), ffi::TransportError> {
    if status != SPOKE_CONNECT_OK {
        return Err(unsafe { transport_error(status, error) });
    }
    // `take_foreign_error` is the single drain for the record's four fields;
    // on a success return the values are unused and only their release
    // matters.
    let _ = unsafe { take_foreign_error(error) };
    Ok(())
}

impl ForeignTransport {
    pub(crate) fn new(table: SpokeConnectTransportTable, user_data: *mut c_void) -> Self {
        Self {
            table,
            user_data,
            closed: AtomicBool::new(false),
            close_result: Mutex::new(None),
        }
    }

    /// Borrows an envelope as the callback input span; an empty envelope
    /// crosses as a NULL/zero span.
    fn span(envelope: &[u8]) -> SpokeConnectSlice {
        SpokeConnectSlice {
            data: if envelope.is_empty() {
                ptr::null()
            } else {
                envelope.as_ptr()
            },
            len: envelope.len(),
        }
    }
}

impl ffi::Transport for ForeignTransport {
    fn send(&self, envelope: Vec<u8>) -> Result<(), ffi::TransportError> {
        let Some(send) = self.table.send else {
            return Err(ffi::TransportError::Io(
                "foreign transport table has no send callback".to_owned(),
            ));
        };
        let mut error = SpokeConnectForeignError::empty();
        let status = unsafe { send(self.user_data, Self::span(&envelope), &mut error) };
        unsafe { callback_result(status, &mut error) }
    }

    fn recv(&self) -> Result<Vec<u8>, ffi::TransportError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(ffi::TransportError::Closed);
        }
        let Some(recv) = self.table.recv else {
            return Err(ffi::TransportError::Io(
                "foreign transport table has no recv callback".to_owned(),
            ));
        };
        let mut buffer = SpokeConnectForeignBuffer::empty();
        let mut error = SpokeConnectForeignError::empty();
        let status = unsafe { recv(self.user_data, &mut buffer, &mut error) };
        let bytes = unsafe { take_foreign_bytes(&mut buffer) };
        // A populated buffer on an error status is released by
        // `take_foreign_bytes`; the bytes themselves are discarded.
        unsafe { callback_result(status, &mut error) }?;
        Ok(bytes)
    }

    fn close(&self) -> Result<(), ffi::TransportError> {
        self.closed.store(true, Ordering::SeqCst);
        let mut slot = self
            .close_result
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(result) = slot.as_ref() {
            return result.clone();
        }
        let result = match self.table.close {
            Some(close) => {
                let mut error = SpokeConnectForeignError::empty();
                let status = unsafe { close(self.user_data, &mut error) };
                unsafe { callback_result(status, &mut error) }
            }
            None => Err(ffi::TransportError::Io(
                "foreign transport table has no close callback".to_owned(),
            )),
        };
        *slot = Some(result.clone());
        result
    }
}

/// Borrowed-handle view of a transport: shares the handle's
/// [`ForeignTransport`], so the caller's [`SpokeConnectTransport`] keeps
/// owning the callback context (and its single `destroy`) after an adapter
/// dialed over it takes its own reference.
pub(crate) struct SharedForeignTransport(Arc<ForeignTransport>);

impl SharedForeignTransport {
    /// A borrowed view over a transport handle's callback context.
    pub(crate) fn new(transport: Arc<ForeignTransport>) -> Self {
        Self(transport)
    }
}

impl ffi::Transport for SharedForeignTransport {
    fn send(&self, envelope: Vec<u8>) -> Result<(), ffi::TransportError> {
        self.0.send(envelope)
    }

    fn recv(&self) -> Result<Vec<u8>, ffi::TransportError> {
        self.0.recv()
    }

    fn close(&self) -> Result<(), ffi::TransportError> {
        self.0.close()
    }
}

/// Borrows a transport handle.
pub(crate) unsafe fn transport_handle<'a>(
    handle: *const SpokeConnectTransport,
) -> Result<&'a Arc<ForeignTransport>, AbiFailure> {
    if handle.is_null() {
        return Err(AbiFailure::invalid("transport is NULL"));
    }
    Ok(unsafe { &*(handle as *const Arc<ForeignTransport>) })
}

// ── Foreign-callback `ToolHandler` bridge (D16 serving face) ────────────

/// Opaque foreign tool-handler handle.
#[repr(C)]
pub struct SpokeConnectToolHandler {
    _private: [u8; 0],
}

/// Borrows text as a callback argument span. Empty text crosses as a
/// NULL/zero span (`A2` §Calling and representation).
pub(crate) fn text_span(text: &str) -> SpokeConnectSlice {
    SpokeConnectSlice {
        data: if text.is_empty() {
            ptr::null()
        } else {
            text.as_ptr()
        },
        len: text.len(),
    }
}

/// A missing callback pointer on an already-constructed handle. Construction
/// requires every pointer, so this is unreachable through this ABI; it is
/// projected as a `Dial` so the facade's containment row answers it rather
/// than panicking inside a callback.
pub(crate) fn missing_callback(what: &str, method: &str) -> ffi::FfiError {
    ffi::FfiError::Dial {
        kind: "callback".to_owned(),
        message: format!("foreign {what} table has no {method} callback"),
    }
}

/// Reads one ports/tool callback return and drains both of its buffers on
/// **every** path.
///
/// Status `0` answers the callback's JSON text, the application reject status
/// answers the four-field error record, and any other status is projected as a
/// `Dial` so the facade contains it into `INTERNAL_ERROR` (`A2` §Status and
/// error projection — ports and tool callbacks accept `0` / `301`).
///
/// A callback may populate the result buffer on a failure status, or the error
/// record on success; each populated buffer transfers to Rust and is released
/// exactly once, so both are drained here whatever the status.
pub(crate) unsafe fn callback_json(
    status: i32,
    what: &str,
    out_json: &mut SpokeConnectForeignBuffer,
    error: &mut SpokeConnectForeignError,
) -> Result<String, ffi::FfiError> {
    // Presence must be captured before the record is drained: a NULL field
    // means absent, while a present empty string has non-NULL data with zero
    // length.
    let kind_present = !error.kind.data.is_null();
    let wire_code_present = !error.wire_code.data.is_null();
    let json = unsafe { take_foreign_text(out_json) };
    let (message, code, kind, wire_code) = unsafe { take_foreign_error(error) };
    match status {
        SPOKE_CONNECT_OK => Ok(json),
        SPOKE_CONNECT_FFI_REJECTED => Err(ffi::FfiError::Rejected {
            code,
            message,
            kind: kind_present.then_some(kind),
            wire_code: wire_code_present.then_some(wire_code),
        }),
        other => {
            let detail = if message.is_empty() { kind } else { message };
            Err(ffi::FfiError::Dial {
                kind: "callback".to_owned(),
                message: format!("unsupported {what} callback status {other}: {detail}"),
            })
        }
    }
}

/// The carrier's [`ffi::ToolHandler`] implementation over a host vtable.
pub(crate) struct ForeignToolHandler {
    table: SpokeConnectToolHandlerTable,
    user_data: *mut c_void,
}

// SAFETY: the A2 callback contract requires the host context, its callbacks
// and `destroy` to be thread-safe and free of thread affinity.
unsafe impl Send for ForeignToolHandler {}
unsafe impl Sync for ForeignToolHandler {}

impl ForeignToolHandler {
    /// A handler handle over a host vtable. Fields stay private: the
    /// constructors that hand one out are the exporting faces.
    pub(crate) fn new(table: SpokeConnectToolHandlerTable, user_data: *mut c_void) -> Self {
        Self { table, user_data }
    }
}

impl Drop for ForeignToolHandler {
    fn drop(&mut self) {
        if let Some(destroy) = self.table.destroy {
            contain_release(|| unsafe { destroy(self.user_data) });
        }
    }
}

impl ffi::ToolHandler for ForeignToolHandler {
    fn handle(&self, arguments_json: String) -> Result<String, ffi::FfiError> {
        let Some(callback) = self.table.handle else {
            return Err(missing_callback("tool handler", "handle"));
        };
        let mut out_json = SpokeConnectForeignBuffer::empty();
        let mut error = SpokeConnectForeignError::empty();
        let status = unsafe {
            callback(
                self.user_data,
                text_span(&arguments_json),
                &mut out_json,
                &mut error,
            )
        };
        unsafe { callback_json(status, "tool handler", &mut out_json, &mut error) }
    }
}

/// Borrowed-handle view of a tool handler: shares the handle's
/// [`ForeignToolHandler`], so the caller's [`SpokeConnectToolHandler`] keeps
/// owning the callback context (and its single `destroy`) after a responder
/// registered it.
pub(crate) struct SharedForeignToolHandler(Arc<ForeignToolHandler>);

impl SharedForeignToolHandler {
    /// A borrowed view over a tool-handler handle's callback context.
    pub(crate) fn new(handler: Arc<ForeignToolHandler>) -> Self {
        Self(handler)
    }
}

impl ffi::ToolHandler for SharedForeignToolHandler {
    fn handle(&self, arguments_json: String) -> Result<String, ffi::FfiError> {
        self.0.handle(arguments_json)
    }
}

/// Borrows a tool-handler handle.
pub(crate) unsafe fn tool_handler_handle<'a>(
    handle: *const SpokeConnectToolHandler,
) -> Result<&'a Arc<ForeignToolHandler>, AbiFailure> {
    if handle.is_null() {
        return Err(AbiFailure::invalid("tool_handler is NULL"));
    }
    Ok(unsafe { &*(handle as *const Arc<ForeignToolHandler>) })
}

// ── Output projections ───────────────────────────────────────────────────

/// Writes the owned JSON a `Result<String, FfiError>` call produced, or
/// projects its failure. The caller has already required and zeroed
/// `out_json`.
pub(crate) unsafe fn write_json(
    out_json: *mut SpokeConnectBuffer,
    result: Result<String, ffi::FfiError>,
) -> Result<(), AbiFailure> {
    let json = result.map_err(AbiFailure::from)?;
    unsafe { out_json.write(owned_buffer(json.as_bytes())) };
    Ok(())
}

/// Writes the owned text a `String` result produced.
pub(crate) unsafe fn write_text(
    out_text: *mut SpokeConnectBuffer,
    text: String,
) -> Result<(), AbiFailure> {
    unsafe { out_text.write(owned_buffer(text.as_bytes())) };
    Ok(())
}

/// Writes an optional text: absent means `present = 0`; a present empty
/// string keeps non-NULL data with zero length.
pub(crate) unsafe fn write_optional_text(
    out_text: *mut SpokeConnectOptionalBuffer,
    text: Option<String>,
) -> Result<(), AbiFailure> {
    let optional = match text {
        Some(text) => SpokeConnectOptionalBuffer {
            present: 1,
            value: owned_buffer(text.as_bytes()),
        },
        None => SpokeConnectOptionalBuffer::empty(),
    };
    unsafe { out_text.write(optional) };
    Ok(())
}
