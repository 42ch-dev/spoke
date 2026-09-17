//! Adapter / router / transport surface: the `spoke_connect_*` exports that
//! wrap the client side of [`spoke_connect::ffi`] — the dialing
//! `RemoteAdapter` (session info, the baseline port families, the optional
//! port families, the `extract` op, the tool-invoke face and the tool-serving
//! registration), the `MultiPeerRouter` (registry, the same port families and
//! the two manifest aggregation views) and the transports a host supplies or
//! consumes (the foreign-callback vtable plus the in-memory loopback pair).
//!
//! The rules live in `spoke-connect`: this module converts values, owns
//! handles and projects errors onto the C boundary (see the crate docs for
//! the ownership and status conventions). It creates no runtime, dispatcher,
//! timeout or crypto layer of its own — every call lands on the facade
//! objects, which run on the shared FFI runtime.
//!
//! # Callback transports
//!
//! A host supplies a [`SpokeConnectTransportTable`] with one C-callable
//! pointer per [`ffi::Transport`] method plus a mandatory `destroy`. The
//! carrier copies the table and takes ownership of `user_data` on success:
//! `destroy(user_data)` runs exactly once, when the last Rust reference (the
//! caller's own [`SpokeConnectTransport`] handle and any adapter dialed over
//! it) is gone. Callbacks run on the shared runtime's blocking pool and may
//! run concurrently, so `recv` may block while `close` runs on another
//! thread — the host's `close` must unblock a waiting `recv` with
//! transport-closed, and both must be thread-safe. They must never call back
//! into this ABI (the facade's re-entrancy rule).

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use spoke_connect::ffi;

use crate::responder::{tool_handler_handle, SharedForeignToolHandler, SpokeConnectToolHandler};
use crate::{
    borrowed_bytes, borrowed_strings, borrowed_text, contain_release, export, optional_u64,
    owned_buffer, release_handle, require_out, take_foreign_bytes, take_foreign_error, AbiFailure,
    SpokeConnectBuffer, SpokeConnectError, SpokeConnectForeignBuffer, SpokeConnectForeignError,
    SpokeConnectOptionalBuffer, SpokeConnectOptionalU64, SpokeConnectSlice, SPOKE_CONNECT_OK,
    SPOKE_CONNECT_TRANSPORT_CLOSED, SPOKE_CONNECT_TRANSPORT_IO,
};

// ── Foreign-callback transport vtable (A2) ───────────────────────────────

/// Send one envelope. `envelope` is borrowed for the duration of the call;
/// the host copies anything it retains. Returns status 0, or 400 / 401 with
/// an error record.
pub type SpokeConnectTransportSendFn = unsafe extern "C" fn(
    user_data: *mut c_void,
    envelope: SpokeConnectSlice,
    out_error: *mut SpokeConnectForeignError,
) -> i32;

/// Receive the next inbound envelope into `out_envelope`, blocking until one
/// arrives or the transport closes. The returned buffer transfers to Rust,
/// which copies it and then calls its release exactly once; a zero buffer
/// needs no release. Returns status 0, or 400 / 401 with an error record.
pub type SpokeConnectTransportRecvFn = unsafe extern "C" fn(
    user_data: *mut c_void,
    out_envelope: *mut SpokeConnectForeignBuffer,
    out_error: *mut SpokeConnectForeignError,
) -> i32;

/// Release the transport's resources. Idempotent: it must also unblock a
/// pending or later `recv` with transport-closed, and must be safe to run
/// concurrently with `recv` and with a concurrent `close`.
pub type SpokeConnectTransportCloseFn = unsafe extern "C" fn(
    user_data: *mut c_void,
    out_error: *mut SpokeConnectForeignError,
) -> i32;

/// Destroy `user_data`; runs exactly once, after the last Rust reference and
/// in-flight callback are gone. Must not unwind.
pub type SpokeConnectTransportDestroyFn = unsafe extern "C" fn(user_data: *mut c_void);

/// Transport callback table. Every pointer is required; a table missing one
/// is rejected as invalid argument, ownership of `user_data` stays with the
/// caller and `destroy` is not called.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SpokeConnectTransportTable {
    pub send: Option<SpokeConnectTransportSendFn>,
    pub recv: Option<SpokeConnectTransportRecvFn>,
    pub close: Option<SpokeConnectTransportCloseFn>,
    pub destroy: Option<SpokeConnectTransportDestroyFn>,
}

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
    fn new(table: SpokeConnectTransportTable, user_data: *mut c_void) -> Self {
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

/// Creates a transport handle from a callback table. On success ownership of
/// `user_data` transfers to the handle; on failure nothing is transferred
/// and `destroy` is not called.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_transport_new(
    table: *const SpokeConnectTransportTable,
    user_data: *mut c_void,
    out_transport: *mut *mut SpokeConnectTransport,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_transport = require_out(out_transport, "out_transport")?;
            out_transport.write(ptr::null_mut());
            if table.is_null() {
                return Err(AbiFailure::invalid("table is NULL"));
            }
            let table = *table;
            if table.send.is_none()
                || table.recv.is_none()
                || table.close.is_none()
                || table.destroy.is_none()
            {
                return Err(AbiFailure::invalid(
                    "transport table must carry send, recv, close and destroy",
                ));
            }
            let transport = Arc::new(ForeignTransport::new(table, user_data));
            out_transport.write(Box::into_raw(Box::new(transport)) as *mut SpokeConnectTransport);
            Ok(())
        })
    }
}

/// Releases a transport handle. A NULL handle is a no-op. Releasing the last
/// reference runs the host's `destroy` once.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_transport_free(transport: *mut SpokeConnectTransport) {
    unsafe { release_handle(transport as *mut Arc<ForeignTransport>) };
}

// ── Loopback transports (AR-7 production parity) ─────────────────────────

/// Opaque loopback transport pair handle.
#[repr(C)]
pub struct SpokeConnectLoopbackTransportPair {
    _private: [u8; 0],
}

/// Opaque loopback transport end handle.
#[repr(C)]
pub struct SpokeConnectLoopbackTransport {
    _private: [u8; 0],
}

/// Borrows a loopback pair handle.
unsafe fn loopback_pair<'a>(
    handle: *const SpokeConnectLoopbackTransportPair,
) -> Result<&'a Arc<ffi::LoopbackTransportPair>, AbiFailure> {
    if handle.is_null() {
        return Err(AbiFailure::invalid("pair is NULL"));
    }
    Ok(unsafe { &*(handle as *const Arc<ffi::LoopbackTransportPair>) })
}

/// Borrows a loopback end handle.
unsafe fn loopback_end<'a>(
    handle: *const SpokeConnectLoopbackTransport,
) -> Result<&'a Arc<ffi::LoopbackTransport>, AbiFailure> {
    if handle.is_null() {
        return Err(AbiFailure::invalid("loopback_transport is NULL"));
    }
    Ok(unsafe { &*(handle as *const Arc<ffi::LoopbackTransport>) })
}

/// Creates a back-to-back in-memory loopback pair (client + server ends of
/// one connection) so a host can drive the transport surface without a
/// network carrier.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_loopback_transport_pair_new(
    out_pair: *mut *mut SpokeConnectLoopbackTransportPair,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_pair = require_out(out_pair, "out_pair")?;
            out_pair.write(ptr::null_mut());
            let pair = Arc::new(ffi::loopback_transport_pair());
            out_pair.write(
                Box::into_raw(Box::new(pair)) as *mut SpokeConnectLoopbackTransportPair
            );
            Ok(())
        })
    }
}

/// The client end of the pair. The returned handle is owned by the caller;
/// the pair must outlive nothing (the end holds its own reference).
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_loopback_transport_pair_client(
    pair: *const SpokeConnectLoopbackTransportPair,
    out_transport: *mut *mut SpokeConnectLoopbackTransport,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_transport = require_out(out_transport, "out_transport")?;
            out_transport.write(ptr::null_mut());
            let client = loopback_pair(pair)?.client();
            out_transport.write(
                Box::into_raw(Box::new(client)) as *mut SpokeConnectLoopbackTransport
            );
            Ok(())
        })
    }
}

/// The server end of the pair.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_loopback_transport_pair_server(
    pair: *const SpokeConnectLoopbackTransportPair,
    out_transport: *mut *mut SpokeConnectLoopbackTransport,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_transport = require_out(out_transport, "out_transport")?;
            out_transport.write(ptr::null_mut());
            let server = loopback_pair(pair)?.server();
            out_transport.write(
                Box::into_raw(Box::new(server)) as *mut SpokeConnectLoopbackTransport
            );
            Ok(())
        })
    }
}

/// Releases a loopback pair handle. A NULL handle is a no-op.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_loopback_transport_pair_free(
    pair: *mut SpokeConnectLoopbackTransportPair,
) {
    unsafe { release_handle(pair as *mut Arc<ffi::LoopbackTransportPair>) };
}

/// Sends one envelope: it is delivered to the peer end's `recv`.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_loopback_transport_send(
    transport: *const SpokeConnectLoopbackTransport,
    envelope: SpokeConnectSlice,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let transport = loopback_end(transport)?;
            let envelope = borrowed_bytes(envelope, "envelope")?;
            transport
                .send(envelope.to_vec())
                .map_err(AbiFailure::from)?;
            Ok(())
        })
    }
}

/// Receives the next inbound envelope. Blocks the calling thread until one
/// arrives or the connection closes.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_loopback_transport_recv(
    transport: *const SpokeConnectLoopbackTransport,
    out_envelope: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_envelope = require_out(out_envelope, "out_envelope")?;
            out_envelope.write(SpokeConnectBuffer::empty());
            let transport = loopback_end(transport)?;
            let envelope = transport.recv().map_err(AbiFailure::from)?;
            out_envelope.write(owned_buffer(&envelope));
            Ok(())
        })
    }
}

/// Closes the whole loopback connection (both directions). Idempotent.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_loopback_transport_close(
    transport: *const SpokeConnectLoopbackTransport,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            loopback_end(transport)?.close().map_err(AbiFailure::from)?;
            Ok(())
        })
    }
}

/// Releases a loopback end handle. A NULL handle is a no-op.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_loopback_transport_free(
    transport: *mut SpokeConnectLoopbackTransport,
) {
    unsafe { release_handle(transport as *mut Arc<ffi::LoopbackTransport>) };
}

// ── Dialing `RemoteAdapter` ──────────────────────────────────────────────

/// Opaque `RemoteAdapter` handle.
#[repr(C)]
pub struct SpokeConnectRemoteAdapter {
    _private: [u8; 0],
}

/// Borrows an adapter handle.
unsafe fn remote_adapter<'a>(
    handle: *const SpokeConnectRemoteAdapter,
) -> Result<&'a Arc<ffi::RemoteAdapterFFI>, AbiFailure> {
    if handle.is_null() {
        return Err(AbiFailure::invalid("adapter is NULL"));
    }
    Ok(unsafe { &*(handle as *const Arc<ffi::RemoteAdapterFFI>) })
}

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

/// Dials a remote peer over `transport` and returns an established adapter.
/// The transport handle is borrowed: the caller keeps ownership of it (and
/// of its callback context) and must close/release the adapter and then the
/// transport handle.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_new(
    transport: *const SpokeConnectTransport,
    local_seed: SpokeConnectSlice,
    local_manifest_json: SpokeConnectSlice,
    remote_pubkey: SpokeConnectSlice,
    allowlist: *const SpokeConnectSlice,
    allowlist_count: usize,
    invoke_timeout_ms: SpokeConnectOptionalU64,
    out_adapter: *mut *mut SpokeConnectRemoteAdapter,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_adapter = require_out(out_adapter, "out_adapter")?;
            out_adapter.write(ptr::null_mut());
            let transport = transport_handle(transport)?;
            let local_seed = borrowed_bytes(local_seed, "local_seed")?;
            let local_manifest_json = borrowed_text(local_manifest_json, "local_manifest_json")?;
            let remote_pubkey = borrowed_bytes(remote_pubkey, "remote_pubkey")?;
            let allowlist = borrowed_strings(allowlist, allowlist_count, "allowlist")?;
            let invoke_timeout_ms = optional_u64(invoke_timeout_ms, "invoke_timeout_ms")?;
            let adapter = ffi::connect_remote_adapter_ffi(
                Box::new(SharedForeignTransport(Arc::clone(transport))),
                local_seed.to_vec(),
                local_manifest_json.to_owned(),
                remote_pubkey.to_vec(),
                allowlist,
                invoke_timeout_ms,
            )
            .map_err(AbiFailure::from)?;
            out_adapter.write(Box::into_raw(Box::new(adapter)) as *mut SpokeConnectRemoteAdapter);
            Ok(())
        })
    }
}

/// Releases an adapter handle. A NULL handle is a no-op. Close the adapter
/// before releasing its handle.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_free(
    adapter: *mut SpokeConnectRemoteAdapter,
) {
    unsafe { release_handle(adapter as *mut Arc<ffi::RemoteAdapterFFI>) };
}

/// Session lifecycle label (`Disconnected` / `Handshaking` / `Established` /
/// `Closed`).
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_state(
    adapter: *const SpokeConnectRemoteAdapter,
    out_state: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_state = require_out(out_state, "out_state")?;
            out_state.write(SpokeConnectBuffer::empty());
            write_text(out_state, remote_adapter(adapter)?.state())
        })
    }
}

/// The established session id, absent before establishment.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_session_id(
    adapter: *const SpokeConnectRemoteAdapter,
    out_session_id: *mut SpokeConnectOptionalBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_session_id = require_out(out_session_id, "out_session_id")?;
            out_session_id.write(SpokeConnectOptionalBuffer::empty());
            write_optional_text(out_session_id, remote_adapter(adapter)?.session_id())
        })
    }
}

/// The remote peer's id, absent before establishment.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_remote_peer_id(
    adapter: *const SpokeConnectRemoteAdapter,
    out_peer_id: *mut SpokeConnectOptionalBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_peer_id = require_out(out_peer_id, "out_peer_id")?;
            out_peer_id.write(SpokeConnectOptionalBuffer::empty());
            write_optional_text(out_peer_id, remote_adapter(adapter)?.remote_peer_id())
        })
    }
}

/// The remote peer's hello manifest as JSON, absent before establishment.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_remote_manifest(
    adapter: *const SpokeConnectRemoteAdapter,
    out_manifest_json: *mut SpokeConnectOptionalBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_manifest_json = require_out(out_manifest_json, "out_manifest_json")?;
            out_manifest_json.write(SpokeConnectOptionalBuffer::empty());
            write_optional_text(out_manifest_json, remote_adapter(adapter)?.remote_manifest())
        })
    }
}

/// Baseline `HostManifestPort` call: the remote host's own manifest.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_get_host_capability_manifest(
    adapter: *const SpokeConnectRemoteAdapter,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            write_json(
                out_json,
                remote_adapter(adapter)?.get_host_capability_manifest(),
            )
        })
    }
}

/// Baseline `KnowledgeEntryPort` read.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_get_knowledge_entry(
    adapter: *const SpokeConnectRemoteAdapter,
    entry_id: SpokeConnectSlice,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let entry_id = borrowed_text(entry_id, "entry_id")?.to_owned();
            write_json(
                out_json,
                remote_adapter(adapter)?.get_knowledge_entry(entry_id),
            )
        })
    }
}

/// Baseline `KnowledgeEntryPort` upsert. `expected_base_revision` is an
/// optional scalar: absent means no expectation.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_put_knowledge_entry(
    adapter: *const SpokeConnectRemoteAdapter,
    entry_json: SpokeConnectSlice,
    expected_base_revision: SpokeConnectOptionalU64,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let entry_json = borrowed_text(entry_json, "entry_json")?.to_owned();
            let expected_base_revision =
                optional_u64(expected_base_revision, "expected_base_revision")?;
            write_json(
                out_json,
                remote_adapter(adapter)?.put_knowledge_entry(entry_json, expected_base_revision),
            )
        })
    }
}

/// Baseline `RelationPort` read.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_get_relation(
    adapter: *const SpokeConnectRemoteAdapter,
    relation_id: SpokeConnectSlice,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let relation_id = borrowed_text(relation_id, "relation_id")?.to_owned();
            write_json(
                out_json,
                remote_adapter(adapter)?.get_relation(relation_id),
            )
        })
    }
}

/// Baseline `RelationPort` upsert. `expected_base_revision` is an optional
/// scalar: absent means no expectation.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_put_relation(
    adapter: *const SpokeConnectRemoteAdapter,
    relation_json: SpokeConnectSlice,
    expected_base_revision: SpokeConnectOptionalU64,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let relation_json = borrowed_text(relation_json, "relation_json")?.to_owned();
            let expected_base_revision =
                optional_u64(expected_base_revision, "expected_base_revision")?;
            write_json(
                out_json,
                remote_adapter(adapter)?.put_relation(relation_json, expected_base_revision),
            )
        })
    }
}

/// Baseline `ScopeQueryPort` read over a JSON `Scope`.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_list_knowledge_entries(
    adapter: *const SpokeConnectRemoteAdapter,
    scope_json: SpokeConnectSlice,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let scope_json = borrowed_text(scope_json, "scope_json")?.to_owned();
            write_json(
                out_json,
                remote_adapter(adapter)?.list_knowledge_entries(scope_json),
            )
        })
    }
}

/// Baseline `ScopeQueryPort` timeline read over a JSON `Scope`.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_list_timeline_events(
    adapter: *const SpokeConnectRemoteAdapter,
    scope_json: SpokeConnectSlice,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let scope_json = borrowed_text(scope_json, "scope_json")?.to_owned();
            write_json(
                out_json,
                remote_adapter(adapter)?.list_timeline_events(scope_json),
            )
        })
    }
}

/// Baseline `FindingPort` write over a JSON array of findings.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_put_findings(
    adapter: *const SpokeConnectRemoteAdapter,
    findings_json: SpokeConnectSlice,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let findings_json = borrowed_text(findings_json, "findings_json")?.to_owned();
            write_json(
                out_json,
                remote_adapter(adapter)?.put_findings(findings_json),
            )
        })
    }
}

/// Baseline `RuleQueryPort` read over an array of rule references.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_list_rules(
    adapter: *const SpokeConnectRemoteAdapter,
    rule_refs: *const SpokeConnectSlice,
    rule_refs_count: usize,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let rule_refs = borrowed_strings(rule_refs, rule_refs_count, "rule_refs")?;
            write_json(out_json, remote_adapter(adapter)?.list_rules(rule_refs))
        })
    }
}

/// Baseline `HostManifestPort` view: the peer manifests this session knows.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_list_peer_host_capability_manifests(
    adapter: *const SpokeConnectRemoteAdapter,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            write_json(
                out_json,
                remote_adapter(adapter)?.list_peer_host_capability_manifests(),
            )
        })
    }
}

/// Optional `l2-computable` face: project the session's computable view on
/// the remote peer.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_project(
    adapter: *const SpokeConnectRemoteAdapter,
    project_request_json: SpokeConnectSlice,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let request = borrowed_text(project_request_json, "project_request_json")?.to_owned();
            write_json(out_json, remote_adapter(adapter)?.project(request))
        })
    }
}

/// Optional `l2-computable` face: apply computable updates on the remote
/// peer.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_compute(
    adapter: *const SpokeConnectRemoteAdapter,
    compute_request_json: SpokeConnectSlice,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let request = borrowed_text(compute_request_json, "compute_request_json")?.to_owned();
            write_json(out_json, remote_adapter(adapter)?.compute(request))
        })
    }
}

/// Optional `l5-fork` face: query the remote peer's fork-branch timeline.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_list_fork_timeline_events(
    adapter: *const SpokeConnectRemoteAdapter,
    scope_json: SpokeConnectSlice,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let scope_json = borrowed_text(scope_json, "scope_json")?.to_owned();
            write_json(
                out_json,
                remote_adapter(adapter)?.list_fork_timeline_events(scope_json),
            )
        })
    }
}

/// Core `extract` op: delegate the whole extraction to a peer that
/// negotiated `ke-extraction`.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_extract(
    adapter: *const SpokeConnectRemoteAdapter,
    extract_request_json: SpokeConnectSlice,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let request = borrowed_text(extract_request_json, "extract_request_json")?.to_owned();
            write_json(out_json, remote_adapter(adapter)?.extract(request))
        })
    }
}

/// Tool-invoke face (`tools.<ns>.<tool_id>`): issue the invoke toward the
/// remote peer and return the tool's result payload as JSON.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_invoke_tool(
    adapter: *const SpokeConnectRemoteAdapter,
    capability_id: SpokeConnectSlice,
    arguments_json: SpokeConnectSlice,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let capability_id = borrowed_text(capability_id, "capability_id")?.to_owned();
            let arguments_json = borrowed_text(arguments_json, "arguments_json")?.to_owned();
            write_json(
                out_json,
                remote_adapter(adapter)?.invoke_tool(capability_id, arguments_json),
            )
        })
    }
}

/// Dialer-side tool-serving registration (D16): serves `tools.*` reverse
/// invokes from the remote peer with a foreign-callback handler. The handler
/// handle is borrowed: the adapter keeps its own reference and does not
/// consume the caller's handle. A non-`tools.` id rejects as invalid input
/// with zero side effect; a valid id is last-wins.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_register_tool_handler(
    adapter: *const SpokeConnectRemoteAdapter,
    capability_id: SpokeConnectSlice,
    handler: *const SpokeConnectToolHandler,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let capability_id = borrowed_text(capability_id, "capability_id")?.to_owned();
            let handler = tool_handler_handle(handler)?;
            remote_adapter(adapter)?
                .register_tool_handler(
                    capability_id,
                    Box::new(SharedForeignToolHandler::new(Arc::clone(handler))),
                )
                .map_err(AbiFailure::from)
        })
    }
}

/// Closes the adapter's session and its transport. Distinct from
/// [`spoke_connect_remote_adapter_free`]: close the session first, then
/// release the handle.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_remote_adapter_close(
    adapter: *const SpokeConnectRemoteAdapter,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            remote_adapter(adapter)?.close();
            Ok(())
        })
    }
}

// ── Multi-peer capability router ─────────────────────────────────────────

/// Opaque `MultiPeerRouter` handle.
#[repr(C)]
pub struct SpokeConnectMultiPeerRouter {
    _private: [u8; 0],
}

/// Borrows a router handle.
unsafe fn multi_peer_router<'a>(
    handle: *const SpokeConnectMultiPeerRouter,
) -> Result<&'a Arc<ffi::MultiPeerRouterFFI>, AbiFailure> {
    if handle.is_null() {
        return Err(AbiFailure::invalid("router is NULL"));
    }
    Ok(unsafe { &*(handle as *const Arc<ffi::MultiPeerRouterFFI>) })
}

/// Creates an empty router. Consumers dial each peer's adapter and register
/// the established adapter.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_multi_peer_router_new(
    out_router: *mut *mut SpokeConnectMultiPeerRouter,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_router = require_out(out_router, "out_router")?;
            out_router.write(ptr::null_mut());
            let router = ffi::new_multi_peer_router_ffi();
            out_router.write(Box::into_raw(Box::new(router)) as *mut SpokeConnectMultiPeerRouter);
            Ok(())
        })
    }
}

/// Releases a router handle. A NULL handle is a no-op. The router releases
/// its references to registered adapters; it never closes them.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_multi_peer_router_free(
    router: *mut SpokeConnectMultiPeerRouter,
) {
    unsafe { release_handle(router as *mut Arc<ffi::MultiPeerRouterFFI>) };
}

/// Registers an established adapter and returns its peer id. The adapter
/// handle is borrowed: the router keeps its own reference and does not
/// consume the caller's handle.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_multi_peer_router_register_peer(
    router: *const SpokeConnectMultiPeerRouter,
    adapter: *const SpokeConnectRemoteAdapter,
    out_peer_id: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_peer_id = require_out(out_peer_id, "out_peer_id")?;
            out_peer_id.write(SpokeConnectBuffer::empty());
            let adapter = Arc::clone(remote_adapter(adapter)?);
            let peer_id = multi_peer_router(router)?
                .register_peer(adapter)
                .map_err(AbiFailure::from)?;
            out_peer_id.write(owned_buffer(peer_id.as_bytes()));
            Ok(())
        })
    }
}

/// Removes a peer from selection. An unknown peer id leaves the registry
/// unchanged.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_multi_peer_router_unregister_peer(
    router: *const SpokeConnectMultiPeerRouter,
    peer_id: SpokeConnectSlice,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let peer_id = borrowed_text(peer_id, "peer_id")?.to_owned();
            multi_peer_router(router)?.unregister_peer(peer_id);
            Ok(())
        })
    }
}

/// The registered peer ids in registration order, as a JSON array of
/// strings.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_multi_peer_router_list_peers(
    router: *const SpokeConnectMultiPeerRouter,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let peers = multi_peer_router(router)?.list_peers();
            let json = serde_json::to_string(&peers).map_err(|error| {
                AbiFailure::from(ffi::FfiError::Rejected {
                    code: "INTERNAL_ERROR".to_owned(),
                    message: format!("peer list serialize failed: {error}"),
                    kind: Some("transport".to_owned()),
                    wire_code: None,
                })
            })?;
            out_json.write(owned_buffer(json.as_bytes()));
            Ok(())
        })
    }
}

/// Composed `HostManifestPort` view: the union of the connected peers'
/// capabilities / roles / namespaces / tools under the router's own host id.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_multi_peer_router_get_host_capability_manifest(
    router: *const SpokeConnectMultiPeerRouter,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            write_json(
                out_json,
                multi_peer_router(router)?.get_host_capability_manifest(),
            )
        })
    }
}

/// Baseline `KnowledgeEntryPort` read, routed to the selected peer.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_multi_peer_router_get_knowledge_entry(
    router: *const SpokeConnectMultiPeerRouter,
    entry_id: SpokeConnectSlice,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let entry_id = borrowed_text(entry_id, "entry_id")?.to_owned();
            write_json(
                out_json,
                multi_peer_router(router)?.get_knowledge_entry(entry_id),
            )
        })
    }
}

/// Baseline `KnowledgeEntryPort` upsert, routed to the selected peer.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_multi_peer_router_put_knowledge_entry(
    router: *const SpokeConnectMultiPeerRouter,
    entry_json: SpokeConnectSlice,
    expected_base_revision: SpokeConnectOptionalU64,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let entry_json = borrowed_text(entry_json, "entry_json")?.to_owned();
            let expected_base_revision =
                optional_u64(expected_base_revision, "expected_base_revision")?;
            write_json(
                out_json,
                multi_peer_router(router)?.put_knowledge_entry(entry_json, expected_base_revision),
            )
        })
    }
}

/// Baseline `RelationPort` read, routed to the selected peer.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_multi_peer_router_get_relation(
    router: *const SpokeConnectMultiPeerRouter,
    relation_id: SpokeConnectSlice,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let relation_id = borrowed_text(relation_id, "relation_id")?.to_owned();
            write_json(
                out_json,
                multi_peer_router(router)?.get_relation(relation_id),
            )
        })
    }
}

/// Baseline `RelationPort` upsert, routed to the selected peer.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_multi_peer_router_put_relation(
    router: *const SpokeConnectMultiPeerRouter,
    relation_json: SpokeConnectSlice,
    expected_base_revision: SpokeConnectOptionalU64,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let relation_json = borrowed_text(relation_json, "relation_json")?.to_owned();
            let expected_base_revision =
                optional_u64(expected_base_revision, "expected_base_revision")?;
            write_json(
                out_json,
                multi_peer_router(router)?.put_relation(relation_json, expected_base_revision),
            )
        })
    }
}

/// Baseline `ScopeQueryPort` read, routed to the selected peer.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_multi_peer_router_list_knowledge_entries(
    router: *const SpokeConnectMultiPeerRouter,
    scope_json: SpokeConnectSlice,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let scope_json = borrowed_text(scope_json, "scope_json")?.to_owned();
            write_json(
                out_json,
                multi_peer_router(router)?.list_knowledge_entries(scope_json),
            )
        })
    }
}

/// Baseline `ScopeQueryPort` timeline read, routed to the selected peer.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_multi_peer_router_list_timeline_events(
    router: *const SpokeConnectMultiPeerRouter,
    scope_json: SpokeConnectSlice,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let scope_json = borrowed_text(scope_json, "scope_json")?.to_owned();
            write_json(
                out_json,
                multi_peer_router(router)?.list_timeline_events(scope_json),
            )
        })
    }
}

/// Baseline `FindingPort` write, routed to the selected peer.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_multi_peer_router_put_findings(
    router: *const SpokeConnectMultiPeerRouter,
    findings_json: SpokeConnectSlice,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let findings_json = borrowed_text(findings_json, "findings_json")?.to_owned();
            write_json(
                out_json,
                multi_peer_router(router)?.put_findings(findings_json),
            )
        })
    }
}

/// Baseline `RuleQueryPort` read, routed to the selected peer.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_multi_peer_router_list_rules(
    router: *const SpokeConnectMultiPeerRouter,
    rule_refs: *const SpokeConnectSlice,
    rule_refs_count: usize,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let rule_refs = borrowed_strings(rule_refs, rule_refs_count, "rule_refs")?;
            write_json(out_json, multi_peer_router(router)?.list_rules(rule_refs))
        })
    }
}

/// Per-peer `HostManifestPort` view: one entry per connected peer, each the
/// peer's own cached hello manifest.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_multi_peer_router_list_peer_host_capability_manifests(
    router: *const SpokeConnectMultiPeerRouter,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            write_json(
                out_json,
                multi_peer_router(router)?.list_peer_host_capability_manifests(),
            )
        })
    }
}

/// Tool-invoke face: the router selects the peer advertising the exact tool
/// capability and delegates the invoke to its adapter. The router exposes no
/// `extract` method (`spoke-remote-adapter.md` D11).
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_multi_peer_router_invoke_tool(
    router: *const SpokeConnectMultiPeerRouter,
    capability_id: SpokeConnectSlice,
    arguments_json: SpokeConnectSlice,
    out_json: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_json = require_out(out_json, "out_json")?;
            out_json.write(SpokeConnectBuffer::empty());
            let capability_id = borrowed_text(capability_id, "capability_id")?.to_owned();
            let arguments_json = borrowed_text(arguments_json, "arguments_json")?.to_owned();
            write_json(
                out_json,
                multi_peer_router(router)?.invoke_tool(capability_id, arguments_json),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::{HashMap, VecDeque};
    use std::sync::atomic::AtomicUsize;
    use std::sync::Condvar;
    use std::time::{Duration, Instant};

    use crate::{
        spoke_connect_buffer_free, spoke_connect_error_free, spoke_connect_optional_buffer_free,
        SPOKE_CONNECT_FFI_DIAL, SPOKE_CONNECT_FFI_REJECTED, SPOKE_CONNECT_INVALID_ARGUMENT,
    };
    use crate::responder::{
        spoke_connect_tool_handler_free, spoke_connect_tool_handler_new,
        SpokeConnectToolHandlerTable,
    };

    // ── Shared fixtures (loaded, never re-derived) ───────────────────────

    /// The cross-language identity vector: seeds, public keys, wire peer ids
    /// and manifests for the client and the four host identities this
    /// battery dials.
    const ROUTER_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../spoke-connect/bindings/swift/Smoke/fixtures/multi-peer-router-smoke.json"
    ));

    /// The loopback vector supplies the client's public key (the router
    /// vector pins the same seed and peer id, but not the key).
    const LOOPBACK_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../spoke-connect/bindings/swift/Smoke/fixtures/loopback-smoke.json"
    ));

    /// The tool capability this battery serves and selects on.
    const PICKED_TOOL_ID: &str = "tools.test.pick";

    /// Bounded waits: a generous per-invoke deadline and a short dial
    /// deadline for the failure paths.
    const INVOKE_TIMEOUT_MS: u64 = 5_000;
    const DIAL_TIMEOUT_MS: u64 = 40;

    /// The static message a host attaches to a transport-closed callback
    /// error (the "static storage supplies a no-op release" case).
    static CLOSED_MESSAGE: &[u8] = b"host queue closed";

    fn fixture(source: &str) -> serde_json::Value {
        serde_json::from_str(source).expect("the fixture parses")
    }

    fn text_field(fixture: &serde_json::Value, key: &str) -> String {
        fixture
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("fixture field {key} is a string"))
            .to_owned()
    }

    fn hex32(value: &str) -> [u8; 32] {
        assert_eq!(value.len(), 64, "32-byte hex field");
        let mut out = [0u8; 32];
        for (index, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).expect("hex byte");
        }
        out
    }

    /// One fixture identity: the raw seed, its Ed25519 public key, the derived
    /// wire peer id and the manifest it advertises.
    #[derive(Clone)]
    struct Identity {
        seed: [u8; 32],
        pubkey: [u8; 32],
        peer_id: String,
        manifest_json: String,
    }

    fn identity(
        fixture: &serde_json::Value,
        seed_key: &str,
        pubkey_key: &str,
        peer_id_key: &str,
        manifest_key: &str,
    ) -> Identity {
        Identity {
            seed: hex32(&text_field(fixture, seed_key)),
            pubkey: hex32(&text_field(fixture, pubkey_key)),
            peer_id: text_field(fixture, peer_id_key),
            manifest_json: text_field(fixture, manifest_key),
        }
    }

    /// The dialing client: seed, peer id and manifest from the router vector,
    /// public key from the loopback vector (same identity).
    fn client_identity() -> Identity {
        let router = fixture(ROUTER_FIXTURE);
        let loopback = fixture(LOOPBACK_FIXTURE);
        Identity {
            seed: hex32(&text_field(&router, "seed_client_hex")),
            pubkey: hex32(&text_field(&loopback, "pubkey_client_hex")),
            peer_id: text_field(&router, "peer_id_client"),
            manifest_json: text_field(&router, "client_manifest_json"),
        }
    }

    fn host_baseline() -> Identity {
        identity(
            &fixture(ROUTER_FIXTURE),
            "baseline_host_seed_hex",
            "baseline_pubkey_host_hex",
            "baseline_peer_id_host",
            "baseline_manifest_json",
        )
    }

    fn host_computable() -> Identity {
        identity(
            &fixture(ROUTER_FIXTURE),
            "computable_host_seed_hex",
            "computable_pubkey_host_hex",
            "computable_peer_id_host",
            "computable_manifest_json",
        )
    }

    fn host_alpha() -> Identity {
        identity(
            &fixture(ROUTER_FIXTURE),
            "alpha_host_seed_hex",
            "alpha_pubkey_host_hex",
            "alpha_peer_id_host",
            "alpha_manifest_json",
        )
    }

    fn host_beta() -> Identity {
        identity(
            &fixture(ROUTER_FIXTURE),
            "beta_host_seed_hex",
            "beta_pubkey_host_hex",
            "beta_peer_id_host",
            "beta_manifest_json",
        )
    }

    /// The fixture manifest plus the tool capability this battery serves.
    fn with_tool_capability(manifest_json: &str) -> String {
        let mut manifest: serde_json::Value =
            serde_json::from_str(manifest_json).expect("manifest JSON parses");
        manifest["capabilities"]
            .as_array_mut()
            .expect("capabilities is an array")
            .push(serde_json::json!(PICKED_TOOL_ID));
        serde_json::to_string(&manifest).expect("manifest serializes")
    }

    /// A schema-valid L6 rule tagged with the peer that serves it, so a
    /// routed answer identifies the selected peer.
    fn rule_value(rule_id: &str) -> serde_json::Value {
        serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "rule_id": rule_id,
            "canonical_name": "No resurrection without foreshadowing",
            "kind": "rule",
            "statement": "Character death reversals require a prior foreshadowing entry.",
            "target_entry_types": ["character", "event"],
            "severity_hint": "error",
            "status": "active",
            "extensions": {},
        }))
        .expect("rule JSON matches the L6 schema")
    }

    // ── Host-side transport harness ──────────────────────────────────────

    fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    /// One direction of the host's synchronized message queue.
    #[derive(Default)]
    struct QueueState {
        envelopes: VecDeque<Vec<u8>>,
        closed: bool,
    }

    #[derive(Default)]
    struct Queue {
        state: Mutex<QueueState>,
        signal: Condvar,
    }

    impl Queue {
        fn push(&self, envelope: Vec<u8>) {
            let mut state = lock(&self.state);
            if state.closed {
                return;
            }
            state.envelopes.push_back(envelope);
            self.signal.notify_all();
        }

        /// Blocks the calling thread until an envelope arrives or the queue
        /// closes.
        fn pop_blocking(&self) -> Option<Vec<u8>> {
            let mut state = lock(&self.state);
            loop {
                if let Some(envelope) = state.envelopes.pop_front() {
                    return Some(envelope);
                }
                if state.closed {
                    return None;
                }
                state = self
                    .signal
                    .wait(state)
                    .unwrap_or_else(|poison| poison.into_inner());
            }
        }

        fn close(&self) {
            let mut state = lock(&self.state);
            state.closed = true;
            self.signal.notify_all();
        }
    }

    /// One end of the queue pair.
    #[derive(Clone)]
    struct Endpoint {
        outbound: Arc<Queue>,
        inbound: Arc<Queue>,
    }

    impl Endpoint {
        fn send(&self, envelope: Vec<u8>) {
            self.outbound.push(envelope);
        }

        fn recv(&self) -> Option<Vec<u8>> {
            self.inbound.pop_blocking()
        }

        fn close(&self) {
            self.outbound.close();
            self.inbound.close();
        }
    }

    /// A connected pair of host queue ends (the host-owned message transport
    /// the re-entrancy rule requires: callbacks never call back into the
    /// exported loopback helpers).
    fn endpoint_pair() -> (Endpoint, Endpoint) {
        let client_to_peer = Arc::new(Queue::default());
        let peer_to_client = Arc::new(Queue::default());
        (
            Endpoint {
                outbound: Arc::clone(&client_to_peer),
                inbound: Arc::clone(&peer_to_client),
            },
            Endpoint {
                outbound: peer_to_client,
                inbound: client_to_peer,
            },
        )
    }

    /// What the host observed about the callbacks the carrier ran.
    #[derive(Default)]
    struct CallbackLog {
        send: AtomicUsize,
        recv_started: AtomicUsize,
        recv_in_flight: AtomicUsize,
        recv_delivered: AtomicUsize,
        recv_closed: AtomicUsize,
        close: AtomicUsize,
        /// Buffer releases the host's release callbacks observed.
        release: AtomicUsize,
        destroy: AtomicUsize,
    }

    /// The host transport context behind the A2 vtable.
    struct HostTransport {
        endpoint: Endpoint,
        log: Arc<CallbackLog>,
    }

    /// Releases the host-owned buffer handed to Rust by `host_recv`. The
    /// carrier calls it exactly once per populated buffer.
    unsafe extern "C" fn host_buffer_release(context: *mut c_void, data: *const u8, len: usize) {
        let log = unsafe { &*(context as *const CallbackLog) };
        log.release.fetch_add(1, Ordering::SeqCst);
        if data.is_null() || len == 0 {
            return;
        }
        unsafe { drop(Box::from_raw(ptr::slice_from_raw_parts_mut(data as *mut u8, len))) };
    }

    /// Releases the host's static transport-closed message (a no-op release
    /// that is still counted, so the battery can observe the carrier
    /// releasing a callback error exactly once).
    unsafe extern "C" fn host_static_release(context: *mut c_void, _data: *const u8, _len: usize) {
        let log = unsafe { &*(context as *const CallbackLog) };
        log.release.fetch_add(1, Ordering::SeqCst);
    }

    unsafe fn write_closed_error(log: &Arc<CallbackLog>, out_error: *mut SpokeConnectForeignError) {
        if out_error.is_null() {
            return;
        }
        unsafe {
            out_error.write(SpokeConnectForeignError {
                message: SpokeConnectForeignBuffer {
                    data: CLOSED_MESSAGE.as_ptr(),
                    len: CLOSED_MESSAGE.len(),
                    release_context: Arc::as_ptr(log) as *mut c_void,
                    release: Some(host_static_release),
                },
                code: SpokeConnectForeignBuffer::empty(),
                kind: SpokeConnectForeignBuffer::empty(),
                wire_code: SpokeConnectForeignBuffer::empty(),
            })
        };
    }

    unsafe extern "C" fn host_send(
        user_data: *mut c_void,
        envelope: SpokeConnectSlice,
        _out_error: *mut SpokeConnectForeignError,
    ) -> i32 {
        let host = unsafe { &*(user_data as *const HostTransport) };
        host.log.send.fetch_add(1, Ordering::SeqCst);
        let bytes = if envelope.data.is_null() || envelope.len == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(envelope.data, envelope.len) }.to_vec()
        };
        host.endpoint.send(bytes);
        SPOKE_CONNECT_OK
    }

    unsafe extern "C" fn host_recv(
        user_data: *mut c_void,
        out_envelope: *mut SpokeConnectForeignBuffer,
        out_error: *mut SpokeConnectForeignError,
    ) -> i32 {
        let host = unsafe { &*(user_data as *const HostTransport) };
        host.log.recv_started.fetch_add(1, Ordering::SeqCst);
        host.log.recv_in_flight.fetch_add(1, Ordering::SeqCst);
        let envelope = host.endpoint.recv();
        host.log.recv_in_flight.fetch_sub(1, Ordering::SeqCst);
        let Some(envelope) = envelope else {
            host.log.recv_closed.fetch_add(1, Ordering::SeqCst);
            unsafe { write_closed_error(&host.log, out_error) };
            return SPOKE_CONNECT_TRANSPORT_CLOSED;
        };
        if out_envelope.is_null() {
            return SPOKE_CONNECT_OK;
        }
        if envelope.is_empty() {
            unsafe { out_envelope.write(SpokeConnectForeignBuffer::empty()) };
            return SPOKE_CONNECT_OK;
        }
        host.log.recv_delivered.fetch_add(1, Ordering::SeqCst);
        let mut owned = envelope.into_boxed_slice();
        let data = owned.as_mut_ptr();
        let len = owned.len();
        std::mem::forget(owned);
        unsafe {
            out_envelope.write(SpokeConnectForeignBuffer {
                data,
                len,
                release_context: Arc::as_ptr(&host.log) as *mut c_void,
                release: Some(host_buffer_release),
            })
        };
        SPOKE_CONNECT_OK
    }

    unsafe extern "C" fn host_close(
        user_data: *mut c_void,
        _out_error: *mut SpokeConnectForeignError,
    ) -> i32 {
        let host = unsafe { &*(user_data as *const HostTransport) };
        host.log.close.fetch_add(1, Ordering::SeqCst);
        host.endpoint.close();
        SPOKE_CONNECT_OK
    }

    unsafe extern "C" fn host_destroy(user_data: *mut c_void) {
        let host = unsafe { Arc::from_raw(user_data as *const HostTransport) };
        host.log.destroy.fetch_add(1, Ordering::SeqCst);
    }

    fn host_table() -> SpokeConnectTransportTable {
        SpokeConnectTransportTable {
            send: Some(host_send),
            recv: Some(host_recv),
            close: Some(host_close),
            destroy: Some(host_destroy),
        }
    }

    /// A host that reports success while still populating the callback error
    /// record. `A2` §C transfers every populated callback buffer to Rust on
    /// success as well as on error, so each of these must be released.
    static OK_DIAGNOSTIC: &[u8] = b"diagnostic attached to an OK callback";

    struct DiagnosticsTransport {
        log: Arc<CallbackLog>,
    }

    /// Writes all four diagnostic fields as populated buffers over static
    /// storage with a counted no-op release — the "static storage supplies a
    /// no-op release" case, made observable.
    unsafe fn write_ok_diagnostics(
        log: &Arc<CallbackLog>,
        out_error: *mut SpokeConnectForeignError,
    ) {
        if out_error.is_null() {
            return;
        }
        let field = || SpokeConnectForeignBuffer {
            data: OK_DIAGNOSTIC.as_ptr(),
            len: OK_DIAGNOSTIC.len(),
            release_context: Arc::as_ptr(log) as *mut c_void,
            release: Some(host_static_release),
        };
        unsafe {
            out_error.write(SpokeConnectForeignError {
                message: field(),
                code: field(),
                kind: field(),
                wire_code: field(),
            })
        };
    }

    unsafe extern "C" fn diagnostics_send(
        user_data: *mut c_void,
        _envelope: SpokeConnectSlice,
        out_error: *mut SpokeConnectForeignError,
    ) -> i32 {
        let host = unsafe { &*(user_data as *const DiagnosticsTransport) };
        host.log.send.fetch_add(1, Ordering::SeqCst);
        unsafe { write_ok_diagnostics(&host.log, out_error) };
        SPOKE_CONNECT_OK
    }

    unsafe extern "C" fn diagnostics_recv(
        user_data: *mut c_void,
        out_envelope: *mut SpokeConnectForeignBuffer,
        out_error: *mut SpokeConnectForeignError,
    ) -> i32 {
        let host = unsafe { &*(user_data as *const DiagnosticsTransport) };
        host.log.recv_started.fetch_add(1, Ordering::SeqCst);
        if !out_envelope.is_null() {
            unsafe { out_envelope.write(SpokeConnectForeignBuffer::empty()) };
        }
        unsafe { write_ok_diagnostics(&host.log, out_error) };
        SPOKE_CONNECT_OK
    }

    unsafe extern "C" fn diagnostics_close(
        user_data: *mut c_void,
        out_error: *mut SpokeConnectForeignError,
    ) -> i32 {
        let host = unsafe { &*(user_data as *const DiagnosticsTransport) };
        host.log.close.fetch_add(1, Ordering::SeqCst);
        unsafe { write_ok_diagnostics(&host.log, out_error) };
        SPOKE_CONNECT_OK
    }

    unsafe extern "C" fn diagnostics_destroy(user_data: *mut c_void) {
        let host = unsafe { Arc::from_raw(user_data as *const DiagnosticsTransport) };
        host.log.destroy.fetch_add(1, Ordering::SeqCst);
    }

    fn diagnostics_table() -> SpokeConnectTransportTable {
        SpokeConnectTransportTable {
            send: Some(diagnostics_send),
            recv: Some(diagnostics_recv),
            close: Some(diagnostics_close),
            destroy: Some(diagnostics_destroy),
        }
    }

    /// The peer end of the same queues, driven through the facade's blocking
    /// [`ffi::Transport`] seam (what a serving host runs).
    struct PeerTransport {
        endpoint: Endpoint,
        log: Arc<CallbackLog>,
    }

    impl ffi::Transport for PeerTransport {
        fn send(&self, envelope: Vec<u8>) -> Result<(), ffi::TransportError> {
            self.log.send.fetch_add(1, Ordering::SeqCst);
            self.endpoint.send(envelope);
            Ok(())
        }

        fn recv(&self) -> Result<Vec<u8>, ffi::TransportError> {
            self.log.recv_started.fetch_add(1, Ordering::SeqCst);
            self.endpoint
                .recv()
                .ok_or(ffi::TransportError::Closed)
        }

        fn close(&self) -> Result<(), ffi::TransportError> {
            self.log.close.fetch_add(1, Ordering::SeqCst);
            self.endpoint.close();
            Ok(())
        }
    }

    // ── Serving peer harness ─────────────────────────────────────────────

    /// A serving ports face: it answers `port.rule.list` with the rule that
    /// identifies this peer and records the references it was asked for.
    struct TestPorts {
        rule_id: String,
        calls: Arc<Mutex<Vec<Vec<String>>>>,
    }

    fn unsupported(what: &str) -> ffi::FfiError {
        ffi::FfiError::Rejected {
            code: "CAPABILITY_PORT_MISSING".to_owned(),
            message: format!("this test peer does not serve {what}"),
            kind: None,
            wire_code: None,
        }
    }

    impl ffi::PortsHandler for TestPorts {
        fn get_knowledge_entry(&self, _entry_id: String) -> Result<String, ffi::FfiError> {
            Err(unsupported("port.knowledge.get"))
        }

        fn put_knowledge_entry(
            &self,
            _entry_json: String,
            _expected_base_revision: Option<u64>,
        ) -> Result<String, ffi::FfiError> {
            Err(unsupported("port.knowledge.put"))
        }

        fn get_relation(&self, _relation_id: String) -> Result<String, ffi::FfiError> {
            Err(unsupported("port.relation.get"))
        }

        fn put_relation(
            &self,
            _relation_json: String,
            _expected_base_revision: Option<u64>,
        ) -> Result<String, ffi::FfiError> {
            Err(unsupported("port.relation.put"))
        }

        fn list_knowledge_entries(&self, _scope_json: String) -> Result<String, ffi::FfiError> {
            Err(unsupported("port.scope.list_knowledge_entries"))
        }

        fn list_timeline_events(&self, _scope_json: String) -> Result<String, ffi::FfiError> {
            Err(unsupported("port.scope.list_timeline_events"))
        }

        fn put_findings(&self, _findings_json: String) -> Result<String, ffi::FfiError> {
            Err(unsupported("port.finding.put"))
        }

        fn list_rules(&self, rule_refs: Vec<String>) -> Result<String, ffi::FfiError> {
            lock(&self.calls).push(rule_refs);
            Ok(serde_json::to_string(&[rule_value(&self.rule_id)])
                .expect("the rule list serializes"))
        }

        fn list_peer_host_capability_manifests(&self) -> Result<String, ffi::FfiError> {
            Ok("[]".to_owned())
        }

        fn project(&self, _project_request_json: String) -> Result<String, ffi::FfiError> {
            Err(unsupported("port.computable.project"))
        }

        fn compute(&self, _compute_request_json: String) -> Result<String, ffi::FfiError> {
            Err(unsupported("port.computable.compute"))
        }

        fn list_fork_timeline_events(&self, _scope_json: String) -> Result<String, ffi::FfiError> {
            Err(unsupported("port.fork.list_timeline_events"))
        }

        fn extract(&self, _extract_request_json: String) -> Result<String, ffi::FfiError> {
            Err(unsupported("extract"))
        }
    }

    /// A tool handler a peer serves under [`PICKED_TOOL_ID`].
    struct PickTool {
        payload: &'static str,
    }

    impl ffi::ToolHandler for PickTool {
        fn handle(&self, _arguments_json: String) -> Result<String, ffi::FfiError> {
            Ok(self.payload.to_owned())
        }
    }

    /// Starts a facade responder on the peer end of a fresh queue pair and
    /// returns the client end for the C-ABI dial.
    fn start_peer(
        dialer: &Identity,
        peer: &Identity,
        manifest_json: &str,
        rule_id: &str,
        pick_payload: Option<&'static str>,
    ) -> (
        Arc<ffi::ConnectResponderFFI>,
        Endpoint,
        Arc<CallbackLog>,
        Arc<Mutex<Vec<Vec<String>>>>,
    ) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let (client_end, peer_end) = endpoint_pair();
        let log = Arc::new(CallbackLog::default());
        let responder = ffi::connect_responder_ffi(
            Box::new(PeerTransport {
                endpoint: peer_end,
                log: Arc::clone(&log),
            }),
            peer.seed.to_vec(),
            manifest_json.to_owned(),
            vec![dialer.peer_id.clone()],
            HashMap::from([(dialer.peer_id.clone(), dialer.pubkey.to_vec())]),
            Some(Box::new(TestPorts {
                rule_id: rule_id.to_owned(),
                calls: Arc::clone(&calls),
            })),
            Some(INVOKE_TIMEOUT_MS),
        )
        .expect("the responder constructs");
        if let Some(payload) = pick_payload {
            responder
                .register_tool_handler(PICKED_TOOL_ID.to_owned(), Box::new(PickTool { payload }))
                .expect("the peer registers the pick tool");
        }
        (responder, client_end, log, calls)
    }

    // ── C ABI helpers ────────────────────────────────────────────────────

    fn slice_of(bytes: &[u8]) -> SpokeConnectSlice {
        SpokeConnectSlice {
            data: if bytes.is_empty() {
                ptr::null()
            } else {
                bytes.as_ptr()
            },
            len: bytes.len(),
        }
    }

    fn slices_of(values: &[&str]) -> Vec<SpokeConnectSlice> {
        values.iter().map(|value| slice_of(value.as_bytes())).collect()
    }

    fn optional_u64(value: Option<u64>) -> SpokeConnectOptionalU64 {
        match value {
            Some(value) => SpokeConnectOptionalU64 {
                present: 1,
                value,
            },
            // Absent: `present = 0` with the zero value.
            None => SpokeConnectOptionalU64 {
                present: 0,
                value: 0,
            },
        }
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

    fn buffer_text(buffer: &SpokeConnectBuffer) -> String {
        if buffer.data.is_null() {
            return String::new();
        }
        String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(buffer.data, buffer.len) })
            .into_owned()
    }

    /// Copies an owned buffer out and releases it.
    unsafe fn take_text(buffer: &mut SpokeConnectBuffer) -> String {
        let text = buffer_text(buffer);
        unsafe { spoke_connect_buffer_free(buffer) };
        text
    }

    /// Copies the optional's text out (absent → `None`) and releases it.
    unsafe fn take_optional_text(optional: &mut SpokeConnectOptionalBuffer) -> Option<String> {
        let text = (optional.present == 1).then(|| buffer_text(&optional.value));
        unsafe { spoke_connect_optional_buffer_free(optional) };
        text
    }

    /// The four textual fields of an error record, released afterwards.
    struct ErrorFields {
        message: String,
        code: String,
        kind: String,
        wire_code: String,
    }

    unsafe fn error_fields(error: &mut SpokeConnectError) -> ErrorFields {
        let fields = ErrorFields {
            message: buffer_text(&error.message),
            code: buffer_text(&error.code),
            kind: buffer_text(&error.kind),
            wire_code: buffer_text(&error.wire_code),
        };
        unsafe { spoke_connect_error_free(error) };
        fields
    }

    /// Creates a transport handle over a host queue end.
    fn host_transport(endpoint: Endpoint) -> (*mut SpokeConnectTransport, Arc<CallbackLog>) {
        let log = Arc::new(CallbackLog::default());
        let host = Arc::new(HostTransport {
            endpoint,
            log: Arc::clone(&log),
        });
        let mut transport: *mut SpokeConnectTransport = ptr::null_mut();
        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_transport_new(
                &host_table(),
                Arc::into_raw(host) as *mut c_void,
                &mut transport,
                &mut error,
            )
        };
        assert_eq!(
            status,
            SPOKE_CONNECT_OK,
            "transport handle: {}",
            unsafe { error_fields(&mut error) }.message
        );
        assert!(!transport.is_null());
        (transport, log)
    }

    // ── C tool-handler harness (D16) ─────────────────────────────────────

    /// The state a pure-C tool-handler context shares with the assertions.
    #[derive(Default)]
    struct AdapterToolState {
        arguments: Mutex<Vec<String>>,
        destroy: AtomicUsize,
    }

    /// The host tool callback context behind the A2 tool table.
    struct AdapterTool {
        payload: String,
        state: Arc<AdapterToolState>,
    }

    /// Releases the owned JSON a tool callback handed to Rust.
    unsafe extern "C" fn adapter_tool_release(_context: *mut c_void, data: *const u8, len: usize) {
        if data.is_null() || len == 0 {
            return;
        }
        unsafe { drop(Box::from_raw(ptr::slice_from_raw_parts_mut(data as *mut u8, len))) };
    }

    /// Serves the configured result payload, recording the arguments JSON it
    /// received.
    unsafe extern "C" fn adapter_tool_handle(
        user_data: *mut c_void,
        arguments_json: SpokeConnectSlice,
        out_json: *mut SpokeConnectForeignBuffer,
        _out_error: *mut SpokeConnectForeignError,
    ) -> i32 {
        let tool = unsafe { &*(user_data as *const AdapterTool) };
        let arguments = if arguments_json.data.is_null() || arguments_json.len == 0 {
            String::new()
        } else {
            String::from_utf8(
                unsafe { std::slice::from_raw_parts(arguments_json.data, arguments_json.len) }
                    .to_vec(),
            )
            .expect("tool arguments cross as UTF-8")
        };
        lock(&tool.state.arguments).push(arguments);
        if out_json.is_null() {
            return SPOKE_CONNECT_OK;
        }
        let mut owned = tool.payload.clone().into_bytes().into_boxed_slice();
        let data = owned.as_mut_ptr();
        let len = owned.len();
        std::mem::forget(owned);
        unsafe {
            out_json.write(SpokeConnectForeignBuffer {
                data,
                len,
                release_context: ptr::null_mut(),
                release: Some(adapter_tool_release),
            })
        };
        SPOKE_CONNECT_OK
    }

    unsafe extern "C" fn adapter_tool_destroy(user_data: *mut c_void) {
        let tool = unsafe { Arc::from_raw(user_data as *const AdapterTool) };
        tool.state.destroy.fetch_add(1, Ordering::SeqCst);
    }

    fn adapter_tool_table() -> SpokeConnectToolHandlerTable {
        SpokeConnectToolHandlerTable {
            handle: Some(adapter_tool_handle),
            destroy: Some(adapter_tool_destroy),
        }
    }

    /// Creates a tool-handler handle answering `payload` through the C ABI,
    /// and returns the shared state the assertions read.
    unsafe fn new_adapter_tool(
        payload: &str,
    ) -> (*mut SpokeConnectToolHandler, Arc<AdapterToolState>) {
        let tool = Arc::new(AdapterTool {
            payload: payload.to_owned(),
            state: Arc::new(AdapterToolState::default()),
        });
        let state = Arc::clone(&tool.state);
        let mut handler: *mut SpokeConnectToolHandler = ptr::null_mut();
        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_tool_handler_new(
                &adapter_tool_table(),
                Arc::into_raw(tool) as *mut c_void,
                &mut handler,
                &mut error,
            )
        };
        assert_eq!(
            status,
            SPOKE_CONNECT_OK,
            "tool handle: {}",
            unsafe { error_fields(&mut error) }.message
        );
        assert!(!handler.is_null());
        (handler, state)
    }

    /// Dials through the C ABI over `transport`. The transport handle is
    /// borrowed, as a host would borrow its own.
    fn dial(
        transport: *mut SpokeConnectTransport,
        dialer: &Identity,
        manifest_json: &str,
        peer: &Identity,
        timeout_ms: Option<u64>,
    ) -> (i32, *mut SpokeConnectRemoteAdapter, SpokeConnectError) {
        let remote_pubkey = peer.pubkey;
        let allowlist = [slice_of(peer.peer_id.as_bytes())];
        let mut adapter: *mut SpokeConnectRemoteAdapter = ptr::null_mut();
        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_remote_adapter_new(
                transport,
                slice_of(&dialer.seed),
                slice_of(manifest_json.as_bytes()),
                slice_of(&remote_pubkey),
                allowlist.as_ptr(),
                allowlist.len(),
                optional_u64(timeout_ms),
                &mut adapter,
                &mut error,
            )
        };
        (status, adapter, error)
    }

    /// A client dialed against one serving peer, with the handles this test
    /// owns.
    struct Setup {
        adapter: *mut SpokeConnectRemoteAdapter,
        transport: *mut SpokeConnectTransport,
        responder: Arc<ffi::ConnectResponderFFI>,
        client_log: Arc<CallbackLog>,
        peer_log: Arc<CallbackLog>,
        peer_id: String,
        rule_calls: Arc<Mutex<Vec<Vec<String>>>>,
    }

    fn setup(
        dialer: &Identity,
        client_manifest_json: &str,
        peer: &Identity,
        peer_manifest_json: &str,
        rule_id: &str,
        pick_payload: Option<&'static str>,
    ) -> Setup {
        let (responder, client_end, peer_log, rule_calls) =
            start_peer(dialer, peer, peer_manifest_json, rule_id, pick_payload);
        let (transport, client_log) = host_transport(client_end);
        let (status, adapter, mut error) = dial(
            transport,
            dialer,
            client_manifest_json,
            peer,
            Some(INVOKE_TIMEOUT_MS),
        );
        assert_eq!(
            status,
            SPOKE_CONNECT_OK,
            "dial failed: {} (kind {})",
            unsafe { error_fields(&mut error) }.message,
            unsafe { error_fields(&mut error) }.kind
        );
        Setup {
            adapter,
            transport,
            responder,
            client_log,
            peer_log,
            peer_id: peer.peer_id.clone(),
            rule_calls,
        }
    }

    impl Setup {
        /// Closes the session, releases both handles and stops the peer.
        fn shutdown(self) {
            let mut error = empty_record();
            unsafe { spoke_connect_remote_adapter_close(self.adapter, &mut error) };
            unsafe { spoke_connect_remote_adapter_free(self.adapter) };
            unsafe { spoke_connect_transport_free(self.transport) };
            self.responder.close();
        }
    }

    /// Polls `predicate` until it holds or `deadline` elapses.
    fn wait_for(predicate: impl Fn() -> bool, deadline: Duration) -> bool {
        let started = Instant::now();
        while started.elapsed() < deadline {
            if predicate() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        predicate()
    }

    /// The peer ids a router lists, in registration order.
    unsafe fn router_peers(router: *const SpokeConnectMultiPeerRouter) -> Vec<String> {
        let mut json = SpokeConnectBuffer::empty();
        let mut error = empty_record();
        let status = unsafe { spoke_connect_multi_peer_router_list_peers(router, &mut json, &mut error) };
        assert_eq!(
            status,
            SPOKE_CONNECT_OK,
            "list_peers: {}",
            unsafe { error_fields(&mut error) }.message
        );
        let text = unsafe { take_text(&mut json) };
        serde_json::from_str(&text).expect("list_peers returns a JSON array of peer ids")
    }

    /// The `rule_id` of the first rule in a `list_rules` payload.
    fn served_rule_id(json: &str) -> String {
        let rules: serde_json::Value =
            serde_json::from_str(json).expect("list_rules returns a JSON array");
        rules[0]["rule_id"]
            .as_str()
            .expect("the served rule carries a rule_id")
            .to_owned()
    }

    /// Invokes `port.rule.list` on a router and returns the served rule id.
    unsafe fn router_rule_id(
        router: *const SpokeConnectMultiPeerRouter,
        rule_refs: &[&str],
    ) -> String {
        let refs = slices_of(rule_refs);
        let mut json = SpokeConnectBuffer::empty();
        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_multi_peer_router_list_rules(
                router,
                refs.as_ptr(),
                refs.len(),
                &mut json,
                &mut error,
            )
        };
        assert_eq!(
            status,
            SPOKE_CONNECT_OK,
            "router list_rules: {}",
            unsafe { error_fields(&mut error) }.message
        );
        served_rule_id(&unsafe { take_text(&mut json) })
    }

    /// Creates an empty router handle.
    unsafe fn new_router() -> *mut SpokeConnectMultiPeerRouter {
        let mut router: *mut SpokeConnectMultiPeerRouter = ptr::null_mut();
        let mut error = empty_record();
        assert_eq!(
            unsafe { spoke_connect_multi_peer_router_new(&mut router, &mut error) },
            SPOKE_CONNECT_OK,
            "router construction: {}",
            unsafe { error_fields(&mut error) }.message
        );
        router
    }

    /// Registers an adapter handle and returns the peer id the router
    /// reports.
    unsafe fn router_register(
        router: *const SpokeConnectMultiPeerRouter,
        adapter: *const SpokeConnectRemoteAdapter,
    ) -> String {
        let mut peer_id = SpokeConnectBuffer::empty();
        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_multi_peer_router_register_peer(router, adapter, &mut peer_id, &mut error)
        };
        assert_eq!(
            status,
            SPOKE_CONNECT_OK,
            "register_peer: {}",
            unsafe { error_fields(&mut error) }.message
        );
        unsafe { take_text(&mut peer_id) }
    }

    /// Invokes a tool on a router.
    unsafe fn router_invoke_tool(
        router: *const SpokeConnectMultiPeerRouter,
        capability_id: &str,
    ) -> (i32, SpokeConnectBuffer, SpokeConnectError) {
        let mut json = SpokeConnectBuffer::empty();
        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_multi_peer_router_invoke_tool(
                router,
                slice_of(capability_id.as_bytes()),
                slice_of(b"{}"),
                &mut json,
                &mut error,
            )
        };
        (status, json, error)
    }

    // ── Tests ────────────────────────────────────────────────────────────

    /// The fixtures are the shared identity vector: the derived peer id of
    /// each pinned public key must equal the pinned peer id (through the
    /// core export), so a wrong key pairing cannot silently weaken a session.
    #[test]
    fn fixture_identities_derive_their_pinned_peer_ids() {
        for identity in [
            client_identity(),
            host_baseline(),
            host_computable(),
            host_alpha(),
            host_beta(),
        ] {
            let mut peer_id = SpokeConnectBuffer::empty();
            let mut error = empty_record();
            let status = unsafe {
                crate::core::spoke_connect_derive_peer_id_from_ed25519_pubkey(
                    slice_of(&identity.pubkey),
                    &mut peer_id,
                    &mut error,
                )
            };
            assert_eq!(status, SPOKE_CONNECT_OK, "peer id derivation");
            assert_eq!(
                unsafe { take_text(&mut peer_id) },
                identity.peer_id,
                "the fixture's public key derives its pinned peer id"
            );
        }
    }

    #[test]
    fn transport_creation_transfers_context_only_on_success() {
        let log = Arc::new(CallbackLog::default());

        // A rejected creation transfers nothing: the caller still owns the
        // context it passed and no destroy call happens.
        let context = Arc::into_raw(Arc::clone(&log)) as *mut c_void;
        let mut transport: *mut SpokeConnectTransport = ptr::null_mut();
        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_transport_new(ptr::null(), context, &mut transport, &mut error)
        };
        assert_eq!(status, SPOKE_CONNECT_INVALID_ARGUMENT);
        assert!(transport.is_null(), "a rejected creation leaves no handle");
        let fields = unsafe { error_fields(&mut error) };
        assert!(fields.message.contains("table is NULL"), "{}", fields.message);
        assert_eq!(log.destroy.load(Ordering::SeqCst), 0, "no destroy on failure");
        drop(unsafe { Arc::from_raw(context as *const CallbackLog) });

        // A table missing a required callback is rejected the same way.
        let incomplete = SpokeConnectTransportTable {
            close: None,
            ..host_table()
        };
        let mut transport: *mut SpokeConnectTransport = ptr::null_mut();
        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_transport_new(&incomplete, ptr::null_mut(), &mut transport, &mut error)
        };
        assert_eq!(status, SPOKE_CONNECT_INVALID_ARGUMENT);
        assert!(transport.is_null());
        let fields = unsafe { error_fields(&mut error) };
        assert!(fields.message.contains("send, recv, close and destroy"), "{}", fields.message);
    }

    #[test]
    fn failed_dial_keeps_the_callers_transport_context_and_releases_it_once() {
        let dialer = client_identity();
        let peer = host_baseline();
        // The peer end stays open and never answers: the dial times out with
        // a host recv still in flight.
        let (client_end, silent_peer) = endpoint_pair();
        let (transport, log) = host_transport(client_end);

        let (status, adapter, mut error) = dial(
            transport,
            &dialer,
            &dialer.manifest_json,
            &peer,
            Some(DIAL_TIMEOUT_MS),
        );
        assert_eq!(status, SPOKE_CONNECT_FFI_DIAL, "a dial failure crosses as status 300");
        assert!(adapter.is_null(), "a failed dial yields no adapter handle");
        let fields = unsafe { error_fields(&mut error) };
        assert_eq!(fields.kind, "timeout", "the dial kind is preserved");
        assert!(fields.code.is_empty(), "a dial failure carries no reject code");
        assert!(log.send.load(Ordering::SeqCst) >= 1, "the host transport was used");

        // The constructor borrowed the handle, and the abandoned blocking
        // recv still owns a reference: the caller's free cannot destroy the
        // context yet.
        assert!(log.recv_started.load(Ordering::SeqCst) >= 1, "a recv was in flight");
        unsafe { spoke_connect_transport_free(transport) };
        assert_eq!(
            log.destroy.load(Ordering::SeqCst),
            0,
            "destroy waits for the last in-flight callback, not just the handle"
        );
        unsafe { spoke_connect_transport_free(ptr::null_mut()) };
        assert_eq!(log.destroy.load(Ordering::SeqCst), 0, "a NULL free is a no-op");

        // Once the host queue closes, the in-flight recv returns and the
        // context is destroyed exactly once.
        silent_peer.close();
        assert!(
            wait_for(
                || log.destroy.load(Ordering::SeqCst) == 1,
                Duration::from_secs(5)
            ),
            "destroy runs exactly once, after the last reference is gone"
        );
        assert_eq!(log.destroy.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn transport_context_outlives_the_adapter_until_every_reference_drops() {
        let dialer = client_identity();
        let peer = host_baseline();
        let setup = setup(
            &dialer,
            &dialer.manifest_json,
            &peer,
            &peer.manifest_json,
            "rule-baseline",
            None,
        );

        assert_eq!(setup.client_log.destroy.load(Ordering::SeqCst), 0);
        let mut error = empty_record();
        assert_eq!(
            unsafe { spoke_connect_remote_adapter_close(setup.adapter, &mut error) },
            SPOKE_CONNECT_OK
        );
        unsafe { spoke_connect_remote_adapter_free(setup.adapter) };
        assert_eq!(
            setup.client_log.destroy.load(Ordering::SeqCst),
            0,
            "the caller's transport handle still owns the context"
        );
        unsafe { spoke_connect_transport_free(setup.transport) };
        assert!(
            wait_for(
                || setup.client_log.destroy.load(Ordering::SeqCst) == 1,
                Duration::from_secs(5)
            ),
            "destroy runs once the closing task and the receive loop are gone"
        );
        assert_eq!(setup.client_log.destroy.load(Ordering::SeqCst), 1);
        setup.responder.close();
    }

    #[test]
    fn adapter_close_unblocks_a_pending_recv_and_is_idempotent() {
        let dialer = client_identity();
        let peer = host_baseline();
        let setup = setup(
            &dialer,
            &dialer.manifest_json,
            &peer,
            &peer.manifest_json,
            "rule-baseline",
            None,
        );

        // The established adapter's receive loop parks in a blocking host
        // recv.
        assert!(
            wait_for(
                || setup.client_log.recv_in_flight.load(Ordering::SeqCst) >= 1,
                Duration::from_secs(5)
            ),
            "the receive loop parks in a blocking recv"
        );

        // Closing must not wait for that recv: the host close callback
        // unblocks it from another thread.
        let started = Instant::now();
        let mut error = empty_record();
        assert_eq!(
            unsafe { spoke_connect_remote_adapter_close(setup.adapter, &mut error) },
            SPOKE_CONNECT_OK
        );
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "close returned while a recv was still blocked"
        );
        assert!(
            wait_for(
                || setup.client_log.recv_in_flight.load(Ordering::SeqCst) == 0,
                Duration::from_secs(3)
            ),
            "close unblocked the pending recv"
        );
        assert!(
            setup.client_log.recv_closed.load(Ordering::SeqCst) >= 1,
            "the unblocked recv reported transport-closed"
        );

        // Every populated callback buffer was released exactly once (the
        // delivered envelopes plus the closed-queue static message).
        let closed = setup.client_log.recv_closed.load(Ordering::SeqCst);
        let delivered = setup.client_log.recv_delivered.load(Ordering::SeqCst);
        assert!(delivered >= 1, "the handshake envelopes were delivered");
        assert_eq!(
            setup.client_log.release.load(Ordering::SeqCst),
            closed + delivered,
            "each populated callback buffer was released exactly once"
        );

        // A second close is idempotent and does not re-run the foreign
        // close.
        assert_eq!(
            unsafe { spoke_connect_remote_adapter_close(setup.adapter, &mut error) },
            SPOKE_CONNECT_OK
        );
        assert_eq!(
            setup.client_log.close.load(Ordering::SeqCst),
            1,
            "the foreign close ran exactly once"
        );

        setup.shutdown();
    }

    #[test]
    fn transport_close_is_idempotent_under_concurrent_callers() {
        use spoke_connect::ffi::Transport as _;

        let (client_end, peer_end) = endpoint_pair();
        let (transport, log) = host_transport(client_end);
        let shared = unsafe { Arc::clone(&*(transport as *const Arc<ForeignTransport>)) };

        // A recv is parked while the closes run: close must not serialize
        // behind it.
        let reader = {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || shared.recv())
        };
        assert!(
            wait_for(
                || log.recv_in_flight.load(Ordering::SeqCst) >= 1,
                Duration::from_secs(5)
            ),
            "the reader parks in a blocking recv"
        );

        let closers: Vec<_> = (0..4)
            .map(|_| {
                let shared = Arc::clone(&shared);
                std::thread::spawn(move || shared.close())
            })
            .collect();
        for closer in closers {
            assert!(
                closer.join().expect("close thread").is_ok(),
                "every concurrent close reports success"
            );
        }
        assert_eq!(
            log.close.load(Ordering::SeqCst),
            1,
            "the foreign close ran exactly once across concurrent callers"
        );
        assert!(
            reader.join().expect("recv thread").is_err(),
            "close unblocked the parked recv with transport-closed"
        );
        assert_eq!(log.recv_closed.load(Ordering::SeqCst), 1);

        // Drop the test's own reference and the handle: with no callback in
        // flight the context is destroyed exactly once.
        drop(shared);
        unsafe { spoke_connect_transport_free(transport) };
        peer_end.close();
        assert!(
            wait_for(
                || log.destroy.load(Ordering::SeqCst) == 1,
                Duration::from_secs(5)
            ),
            "destroy runs after the parked recv returns"
        );
    }

    #[test]
    fn ok_callback_error_buffers_are_released_on_every_return_path() {
        use spoke_connect::ffi::Transport as _;

        let log = Arc::new(CallbackLog::default());
        let host = Arc::new(DiagnosticsTransport {
            log: Arc::clone(&log),
        });
        let mut transport: *mut SpokeConnectTransport = ptr::null_mut();
        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_transport_new(
                &diagnostics_table(),
                Arc::into_raw(host) as *mut c_void,
                &mut transport,
                &mut error,
            )
        };
        assert_eq!(
            status,
            SPOKE_CONNECT_OK,
            "transport handle: {}",
            unsafe { error_fields(&mut error) }.message
        );
        let shared = unsafe { Arc::clone(&*(transport as *const Arc<ForeignTransport>)) };

        // A compliant host may attach diagnostics to a success return: the
        // populated buffers still transfer to Rust and are released exactly
        // once, on `send`, on a successful `recv` and on a successful
        // `close` alike.
        assert!(
            shared.send(vec![1, 2, 3]).is_ok(),
            "an OK send stays a success"
        );
        assert_eq!(
            log.release.load(Ordering::SeqCst),
            4,
            "send released the four diagnostic buffers its OK return carried"
        );

        assert_eq!(
            shared.recv().expect("an OK recv succeeds"),
            Vec::<u8>::new()
        );
        assert_eq!(
            log.release.load(Ordering::SeqCst),
            8,
            "recv released the four diagnostic buffers its OK return carried"
        );

        assert!(shared.close().is_ok(), "an OK close stays a success");
        assert_eq!(
            log.release.load(Ordering::SeqCst),
            12,
            "close released the four diagnostic buffers its OK return carried"
        );

        // The cached close result reuses the drained record: no second
        // foreign close and no second release.
        assert!(shared.close().is_ok());
        assert_eq!(log.close.load(Ordering::SeqCst), 1);
        assert_eq!(log.release.load(Ordering::SeqCst), 12);

        drop(shared);
        unsafe { spoke_connect_transport_free(transport) };
        assert_eq!(log.destroy.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn adapter_metadata_and_baseline_round_trip_cross_the_c_abi() {
        let dialer = client_identity();
        let peer = host_baseline();
        let setup = setup(
            &dialer,
            &dialer.manifest_json,
            &peer,
            &peer.manifest_json,
            "rule-baseline",
            None,
        );
        let mut error = empty_record();

        let mut state = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe { spoke_connect_remote_adapter_state(setup.adapter, &mut state, &mut error) },
            SPOKE_CONNECT_OK
        );
        assert_eq!(unsafe { take_text(&mut state) }, "Established");

        let mut session_id = SpokeConnectOptionalBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_session_id(setup.adapter, &mut session_id, &mut error)
            },
            SPOKE_CONNECT_OK
        );
        let session_id = unsafe { take_optional_text(&mut session_id) }.expect("established");
        assert_eq!(
            Some(session_id),
            setup.responder.session_id(),
            "both ends report the same session id"
        );

        let mut remote_peer_id = SpokeConnectOptionalBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_remote_peer_id(
                    setup.adapter,
                    &mut remote_peer_id,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK
        );
        assert_eq!(
            unsafe { take_optional_text(&mut remote_peer_id) }.as_deref(),
            Some(setup.peer_id.as_str())
        );

        let mut remote_manifest = SpokeConnectOptionalBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_remote_manifest(
                    setup.adapter,
                    &mut remote_manifest,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK
        );
        let manifest: serde_json::Value = serde_json::from_str(
            &unsafe { take_optional_text(&mut remote_manifest) }.expect("remote manifest"),
        )
        .expect("the remote manifest is JSON");
        assert_eq!(manifest["host_id"], serde_json::json!("host-baseline"));
        assert_eq!(
            manifest["capabilities"],
            serde_json::json!(["spoke-baseline"])
        );

        // One baseline port round trip: the request crosses the C boundary,
        // reaches the peer's ports face and the served JSON comes back.
        let rule_refs = slices_of(&["rule_01HXYZ"]);
        let mut rules = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_list_rules(
                    setup.adapter,
                    rule_refs.as_ptr(),
                    rule_refs.len(),
                    &mut rules,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK
        );
        let rules: serde_json::Value =
            serde_json::from_str(&unsafe { take_text(&mut rules) }).expect("rules JSON");
        assert_eq!(rules, serde_json::json!([rule_value("rule-baseline")]));
        assert_eq!(
            lock(&setup.rule_calls).clone(),
            vec![vec!["rule_01HXYZ".to_owned()]],
            "the peer served exactly the references the C call carried"
        );

        // `extract` parses at the boundary: a malformed request rejects
        // locally with zero wire traffic.
        let sends_before = setup.peer_log.send.load(Ordering::SeqCst);
        let mut out_json = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_extract(
                    setup.adapter,
                    slice_of(b"{}"),
                    &mut out_json,
                    &mut error,
                )
            },
            SPOKE_CONNECT_FFI_REJECTED
        );
        let fields = unsafe { error_fields(&mut error) };
        assert_eq!(fields.code, "INVALID_INPUT");
        assert!(fields.wire_code.is_empty(), "a local reject carries no wire code");
        assert!(
            out_json.data.is_null(),
            "a failed call leaves no result ownership with the caller"
        );
        assert_eq!(
            setup.peer_log.send.load(Ordering::SeqCst),
            sends_before,
            "a boundary reject sends nothing on the wire"
        );

        // A NULL transport handle is rejected before the dial is attempted.
        let (status, adapter, mut error) = dial(
            ptr::null_mut(),
            &client_identity(),
            "{}",
            &host_baseline(),
            None,
        );
        assert_eq!(status, SPOKE_CONNECT_INVALID_ARGUMENT);
        assert!(adapter.is_null());
        let fields = unsafe { error_fields(&mut error) };
        assert!(fields.message.contains("transport is NULL"), "{}", fields.message);

        // A presence flag outside 0/1 is rejected at the boundary, with zero
        // wire traffic.
        let mut out_json = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_put_knowledge_entry(
                    setup.adapter,
                    slice_of(b"{}"),
                    SpokeConnectOptionalU64 {
                        present: 2,
                        value: 0,
                    },
                    &mut out_json,
                    &mut error,
                )
            },
            SPOKE_CONNECT_INVALID_ARGUMENT
        );
        let fields = unsafe { error_fields(&mut error) };
        assert!(fields.message.contains("present must be 0 or 1"), "{}", fields.message);
        assert_eq!(
            setup.peer_log.send.load(Ordering::SeqCst),
            sends_before,
            "a boundary reject sends nothing on the wire"
        );

        // `close` is distinct from `free`.
        assert_eq!(
            unsafe { spoke_connect_remote_adapter_close(setup.adapter, &mut error) },
            SPOKE_CONNECT_OK
        );
        let mut state = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe { spoke_connect_remote_adapter_state(setup.adapter, &mut state, &mut error) },
            SPOKE_CONNECT_OK
        );
        assert_eq!(unsafe { take_text(&mut state) }, "Closed");
        setup.shutdown();
    }

    #[test]
    fn adapter_capability_deny_crosses_as_a_rejected_error() {
        let dialer = client_identity();
        // The peer advertises `l2-computable` only, so the negotiated set is
        // empty and a baseline port invoke is denied on the wire.
        let peer = host_computable();
        let setup = setup(
            &dialer,
            &dialer.manifest_json,
            &peer,
            &peer.manifest_json,
            "rule-computable",
            None,
        );
        let mut error = empty_record();

        let rule_refs = slices_of(&["rule_01HXYZ"]);
        let mut out_json = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_list_rules(
                    setup.adapter,
                    rule_refs.as_ptr(),
                    rule_refs.len(),
                    &mut out_json,
                    &mut error,
                )
            },
            SPOKE_CONNECT_FFI_REJECTED
        );
        let fields = unsafe { error_fields(&mut error) };
        assert_eq!(fields.code, "CAPABILITY_PORT_MISSING");
        assert_eq!(
            fields.wire_code, "op_unsupported",
            "the deny came from the peer's dispatch gate, not the boundary"
        );
        assert!(fields.kind.is_empty(), "a dispatch deny carries no kind");
        assert!(
            fields.message.contains("port.rule.list") || fields.message.contains("op_unsupported"),
            "the deny identifies the op: {}",
            fields.message
        );
        assert!(
            out_json.data.is_null(),
            "a denied call leaves no result ownership with the caller"
        );
        assert!(
            setup.peer_log.send.load(Ordering::SeqCst) >= 1,
            "the denied invoke reached the wire"
        );

        setup.shutdown();
    }

    /// A pure-C dialer serves a reverse invoke: the adapter handle, the tool
    /// handler handle and the registration all cross the C ABI, and the peer
    /// on the far end issues the invoke the dialer's foreign callback
    /// answers.
    #[test]
    fn adapter_tool_registration_serves_a_reverse_invoke_from_the_c_dialer() {
        let dialer = client_identity();
        let peer = host_baseline();
        let client_manifest = with_tool_capability(&dialer.manifest_json);
        let peer_manifest = with_tool_capability(&peer.manifest_json);
        let setup = setup(
            &dialer,
            &client_manifest,
            &peer,
            &peer_manifest,
            "rule-host",
            None,
        );
        let mut error = empty_record();

        // The serving handle is created through the C ABI and registered
        // through the adapter's own C registration face.
        let (tool, tool_state) = unsafe { new_adapter_tool(r#"{"served":"dialer"}"#) };
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_register_tool_handler(
                    setup.adapter,
                    slice_of(PICKED_TOOL_ID.as_bytes()),
                    tool,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK,
            "register: {}",
            unsafe { error_fields(&mut error) }.message
        );

        // The peer's reverse invoke is answered by the C callback, which
        // observed the arguments object the invoke carried.
        let result = setup
            .responder
            .invoke_tool(PICKED_TOOL_ID.to_owned(), r#"{"a":1}"#.to_owned())
            .expect("the peer's reverse invoke reaches the dialer's tool");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result).expect("tool result JSON"),
            serde_json::json!({ "served": "dialer" })
        );
        assert_eq!(
            lock(&tool_state.arguments).clone(),
            vec![r#"{"a":1}"#.to_owned()],
            "the C callback received the arguments the peer sent"
        );

        // A non-`tools.` id rejects through the facade's own projection, with
        // zero side effect: the registered handler keeps serving.
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_register_tool_handler(
                    setup.adapter,
                    slice_of(b"port.x"),
                    tool,
                    &mut error,
                )
            },
            SPOKE_CONNECT_FFI_REJECTED
        );
        let fields = unsafe { error_fields(&mut error) };
        assert_eq!(fields.code, "INVALID_INPUT");
        assert!(fields.kind.is_empty(), "a grammar reject carries no kind");
        assert!(
            fields.wire_code.is_empty(),
            "a grammar reject carries no wire code"
        );
        assert!(
            fields.message.contains("port.x"),
            "the reject names the offending id: {}",
            fields.message
        );
        let result = setup
            .responder
            .invoke_tool(PICKED_TOOL_ID.to_owned(), r#"{"a":2}"#.to_owned())
            .expect("the rejected registration left the registry unchanged");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result).expect("tool result JSON"),
            serde_json::json!({ "served": "dialer" })
        );

        // The handle is borrowed: releasing the caller's handle leaves the
        // adapter's own reference serving.
        unsafe { spoke_connect_tool_handler_free(tool) };
        assert_eq!(
            tool_state.destroy.load(Ordering::SeqCst),
            0,
            "the adapter still owns the registered reference"
        );
        let result = setup
            .responder
            .invoke_tool(PICKED_TOOL_ID.to_owned(), r#"{"a":3}"#.to_owned())
            .expect("the borrowed handle is not consumed");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result).expect("tool result JSON"),
            serde_json::json!({ "served": "dialer" })
        );

        // Registration is last-wins: the same id serves the replacement, and
        // the replaced context is released once its last reference drops.
        let (replacement, replacement_state) =
            unsafe { new_adapter_tool(r#"{"served":"replacement"}"#) };
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_register_tool_handler(
                    setup.adapter,
                    slice_of(PICKED_TOOL_ID.as_bytes()),
                    replacement,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK
        );
        let result = setup
            .responder
            .invoke_tool(PICKED_TOOL_ID.to_owned(), r#"{"a":4}"#.to_owned())
            .expect("the replacement serves the same id");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result).expect("tool result JSON"),
            serde_json::json!({ "served": "replacement" })
        );
        assert!(
            wait_for(
                || tool_state.destroy.load(Ordering::SeqCst) == 1,
                Duration::from_secs(5)
            ),
            "the replaced handler's context is destroyed once"
        );
        unsafe { spoke_connect_tool_handler_free(replacement) };
        assert_eq!(
            replacement_state.destroy.load(Ordering::SeqCst),
            0,
            "the adapter still owns the replacement reference"
        );

        setup.shutdown();
        assert!(
            wait_for(
                || replacement_state.destroy.load(Ordering::SeqCst) == 1,
                Duration::from_secs(5)
            ),
            "the registered context is destroyed once the adapter is gone"
        );
        assert_eq!(replacement_state.destroy.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn router_registration_lists_peers_and_retains_the_registered_adapter() {
        let dialer = client_identity();
        let client_manifest = with_tool_capability(&dialer.manifest_json);
        let alpha = host_alpha();
        let beta = host_beta();
        let alpha_setup = setup(
            &dialer,
            &client_manifest,
            &alpha,
            &alpha.manifest_json,
            "rule-alpha",
            None,
        );
        let beta_setup = setup(
            &dialer,
            &client_manifest,
            &beta,
            &with_tool_capability(&beta.manifest_json),
            "rule-beta",
            Some(r#"{"picked":"host-beta"}"#),
        );

        let router = unsafe { new_router() };
        assert_eq!(
            unsafe { router_register(router, alpha_setup.adapter) },
            alpha_setup.peer_id
        );
        assert_eq!(
            unsafe { router_register(router, beta_setup.adapter) },
            beta_setup.peer_id
        );
        assert_eq!(
            unsafe { router_peers(router) },
            vec![alpha_setup.peer_id.clone(), beta_setup.peer_id.clone()],
            "peers list in registration order"
        );

        // The router holds its own reference: releasing the caller's alpha
        // handle leaves the peer registered and serving.
        unsafe { spoke_connect_remote_adapter_free(alpha_setup.adapter) };
        assert_eq!(
            unsafe { router_peers(router) },
            vec![alpha_setup.peer_id.clone(), beta_setup.peer_id.clone()]
        );
        assert_eq!(
            unsafe { router_rule_id(router, &["rule_01HXYZ"]) },
            "rule-alpha",
            "the retained alpha adapter served the routed call"
        );
        assert_eq!(lock(&alpha_setup.rule_calls).len(), 1);
        assert!(lock(&beta_setup.rule_calls).is_empty());

        // An unknown peer id leaves the registry unchanged.
        let mut error = empty_record();
        assert_eq!(
            unsafe {
                spoke_connect_multi_peer_router_unregister_peer(
                    router,
                    slice_of(b"12D3KooWNotRegistered"),
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK
        );
        assert_eq!(
            unsafe { router_peers(router) },
            vec![alpha_setup.peer_id.clone(), beta_setup.peer_id.clone()]
        );

        // Unregistering alpha drops it from selection; beta still answers.
        assert_eq!(
            unsafe {
                spoke_connect_multi_peer_router_unregister_peer(
                    router,
                    slice_of(alpha_setup.peer_id.as_bytes()),
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK
        );
        assert_eq!(
            unsafe { router_peers(router) },
            vec![beta_setup.peer_id.clone()]
        );
        assert_eq!(
            unsafe { router_rule_id(router, &["rule_01HXYZ"]) },
            "rule-beta"
        );

        unsafe { spoke_connect_multi_peer_router_free(router) };
        // Releasing the router releases its references and never closes the
        // caller-owned adapters.
        let mut error = empty_record();
        assert_eq!(
            unsafe { spoke_connect_remote_adapter_close(beta_setup.adapter, &mut error) },
            SPOKE_CONNECT_OK
        );
        unsafe { spoke_connect_remote_adapter_free(beta_setup.adapter) };
        unsafe { spoke_connect_transport_free(alpha_setup.transport) };
        unsafe { spoke_connect_transport_free(beta_setup.transport) };
        alpha_setup.responder.close();
        beta_setup.responder.close();
    }

    #[test]
    fn router_manifest_views_compose_the_peer_set() {
        let dialer = client_identity();
        let client_manifest = with_tool_capability(&dialer.manifest_json);
        let alpha = host_alpha();
        let beta = host_beta();
        let alpha_setup = setup(
            &dialer,
            &client_manifest,
            &alpha,
            &alpha.manifest_json,
            "rule-alpha",
            None,
        );
        let beta_setup = setup(
            &dialer,
            &client_manifest,
            &beta,
            &with_tool_capability(&beta.manifest_json),
            "rule-beta",
            None,
        );
        let router = unsafe { new_router() };
        unsafe { router_register(router, alpha_setup.adapter) };
        unsafe { router_register(router, beta_setup.adapter) };
        let mut error = empty_record();

        // Composed view: the union under the router's own host id.
        let mut composed = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_multi_peer_router_get_host_capability_manifest(
                    router,
                    &mut composed,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK
        );
        let composed: serde_json::Value =
            serde_json::from_str(&unsafe { take_text(&mut composed) }).expect("composed manifest JSON");
        assert_eq!(composed["host_id"], serde_json::json!("multi-peer-router"));
        let mut capabilities: Vec<String> = composed["capabilities"]
            .as_array()
            .expect("capabilities array")
            .iter()
            .map(|capability| capability.as_str().expect("string capability").to_owned())
            .collect();
        capabilities.sort();
        let mut expected = vec![
            "l2-computable".to_owned(),
            "spoke-baseline".to_owned(),
            PICKED_TOOL_ID.to_owned(),
        ];
        expected.sort();
        assert_eq!(capabilities, expected);
        assert_eq!(
            composed["extensions"]["router"]["peers"],
            serde_json::json!([alpha_setup.peer_id.clone(), beta_setup.peer_id.clone()]),
            "contributing peers, ordered by peer id"
        );

        // Per-peer view: each connected peer's own cached hello manifest.
        let mut per_peer = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_multi_peer_router_list_peer_host_capability_manifests(
                    router,
                    &mut per_peer,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK
        );
        let per_peer: serde_json::Value =
            serde_json::from_str(&unsafe { take_text(&mut per_peer) }).expect("per-peer manifests");
        let per_peer = per_peer.as_array().expect("array of manifests");
        assert_eq!(per_peer.len(), 2);
        assert_eq!(per_peer[0]["host_id"], serde_json::json!("host-alpha"));
        assert_eq!(per_peer[0]["capabilities"], serde_json::json!(["spoke-baseline"]));
        assert_eq!(per_peer[1]["host_id"], serde_json::json!("host-beta"));
        assert_eq!(
            per_peer[1]["capabilities"],
            serde_json::json!(["spoke-baseline", "l2-computable", PICKED_TOOL_ID])
        );

        unsafe { spoke_connect_multi_peer_router_free(router) };
        alpha_setup.shutdown();
        beta_setup.shutdown();
    }

    #[test]
    fn router_selects_the_capable_peer_and_reports_no_capable_peer() {
        let dialer = client_identity();
        let client_manifest = with_tool_capability(&dialer.manifest_json);
        let alpha = host_alpha();
        let beta = host_beta();
        let alpha_setup = setup(
            &dialer,
            &client_manifest,
            &alpha,
            &alpha.manifest_json,
            "rule-alpha",
            None,
        );
        let beta_setup = setup(
            &dialer,
            &client_manifest,
            &beta,
            &with_tool_capability(&beta.manifest_json),
            "rule-beta",
            Some(r#"{"picked":"host-beta"}"#),
        );
        let router = unsafe { new_router() };
        unsafe { router_register(router, alpha_setup.adapter) };
        unsafe { router_register(router, beta_setup.adapter) };

        // Both peers advertise the baseline capability: the deterministic
        // lowest-peer-id tie-break selects host-alpha.
        assert_eq!(
            unsafe { router_rule_id(router, &["rule_01HXYZ"]) },
            "rule-alpha"
        );

        // Only host-beta advertises the tool capability, so the router
        // delegates the invoke to it.
        let (status, mut picked, mut error) =
            unsafe { router_invoke_tool(router, PICKED_TOOL_ID) };
        assert_eq!(
            status,
            SPOKE_CONNECT_OK,
            "router invoke_tool: {}",
            unsafe { error_fields(&mut error) }.message
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&unsafe { take_text(&mut picked) })
                .expect("tool result JSON"),
            serde_json::json!({"picked": "host-beta"})
        );

        // The adapter's own tool face answers the same way.
        let mut direct = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_invoke_tool(
                    beta_setup.adapter,
                    slice_of(PICKED_TOOL_ID.as_bytes()),
                    slice_of(b"{}"),
                    &mut direct,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&unsafe { take_text(&mut direct) })
                .expect("tool result JSON"),
            serde_json::json!({"picked": "host-beta"})
        );

        // No capable peer is a terminal reject with the locked detail pair.
        let (status, mut out_json, mut error) =
            unsafe { router_invoke_tool(router, "tools.absent.tool") };
        assert_eq!(status, SPOKE_CONNECT_FFI_REJECTED);
        let fields = unsafe { error_fields(&mut error) };
        assert_eq!(fields.code, "CAPABILITY_PORT_MISSING");
        assert_eq!(fields.kind, "no_capable_peer");
        assert_eq!(fields.wire_code, "no_capable_peer");
        assert!(
            fields.message.contains("tools.absent.tool"),
            "the reject names the capability: {}",
            fields.message
        );
        assert!(out_json.data.is_null());
        unsafe { spoke_connect_buffer_free(&mut out_json) };

        unsafe { spoke_connect_multi_peer_router_free(router) };
        alpha_setup.shutdown();
        beta_setup.shutdown();
    }

    #[test]
    fn loopback_pair_ends_round_trip_through_the_c_abi() {
        let mut error = empty_record();
        let mut pair: *mut SpokeConnectLoopbackTransportPair = ptr::null_mut();
        assert_eq!(
            unsafe { spoke_connect_loopback_transport_pair_new(&mut pair, &mut error) },
            SPOKE_CONNECT_OK
        );
        assert!(!pair.is_null());
        let mut client: *mut SpokeConnectLoopbackTransport = ptr::null_mut();
        let mut server: *mut SpokeConnectLoopbackTransport = ptr::null_mut();
        assert_eq!(
            unsafe { spoke_connect_loopback_transport_pair_client(pair, &mut client, &mut error) },
            SPOKE_CONNECT_OK
        );
        assert_eq!(
            unsafe { spoke_connect_loopback_transport_pair_server(pair, &mut server, &mut error) },
            SPOKE_CONNECT_OK
        );

        assert_eq!(
            unsafe {
                spoke_connect_loopback_transport_send(client, slice_of(b"client-to-server"), &mut error)
            },
            SPOKE_CONNECT_OK
        );
        let mut received = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe { spoke_connect_loopback_transport_recv(server, &mut received, &mut error) },
            SPOKE_CONNECT_OK
        );
        assert_eq!(unsafe { take_text(&mut received) }, "client-to-server");

        assert_eq!(
            unsafe {
                spoke_connect_loopback_transport_send(server, slice_of(b"server-to-client"), &mut error)
            },
            SPOKE_CONNECT_OK
        );
        assert_eq!(
            unsafe { spoke_connect_loopback_transport_recv(client, &mut received, &mut error) },
            SPOKE_CONNECT_OK
        );
        assert_eq!(unsafe { take_text(&mut received) }, "server-to-client");

        // Closing one end closes the connection: the peer's next recv fails
        // fast with transport-closed.
        assert_eq!(
            unsafe { spoke_connect_loopback_transport_close(client, &mut error) },
            SPOKE_CONNECT_OK
        );
        assert_eq!(
            unsafe { spoke_connect_loopback_transport_recv(server, &mut received, &mut error) },
            SPOKE_CONNECT_TRANSPORT_CLOSED
        );
        let fields = unsafe { error_fields(&mut error) };
        assert!(fields.message.contains("closed"), "{}", fields.message);
        assert!(received.data.is_null());

        // Boundary validation: a NULL out pointer and a NULL/non-zero span
        // are invalid arguments, reported with or without an error record.
        assert_eq!(
            unsafe { spoke_connect_loopback_transport_pair_new(ptr::null_mut(), &mut error) },
            SPOKE_CONNECT_INVALID_ARGUMENT
        );
        let fields = unsafe { error_fields(&mut error) };
        assert!(fields.message.contains("NULL"), "{}", fields.message);
        assert_eq!(
            unsafe {
                spoke_connect_loopback_transport_send(
                    client,
                    SpokeConnectSlice {
                        data: ptr::null(),
                        len: 4,
                    },
                    ptr::null_mut(),
                )
            },
            SPOKE_CONNECT_INVALID_ARGUMENT,
            "a NULL error record still returns the status"
        );

        unsafe { spoke_connect_loopback_transport_free(client) };
        unsafe { spoke_connect_loopback_transport_free(server) };
        unsafe { spoke_connect_loopback_transport_pair_free(pair) };
        unsafe { spoke_connect_loopback_transport_free(ptr::null_mut()) };
        unsafe { spoke_connect_loopback_transport_pair_free(ptr::null_mut()) };
    }
}
