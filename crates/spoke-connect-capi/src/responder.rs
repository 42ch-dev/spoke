//! Responder / serving / tool surface: the `spoke_connect_*` exports that
//! wrap the accepting side of [`spoke_connect::ffi`] — the `ConnectResponder`
//! lifecycle (session info, serving, reverse tool invoke, close) plus the two
//! host callback faces a serving host implements: the D4 `PortsHandler`
//! catalogue and the `ToolHandler`.
//!
//! The rules live in `spoke-connect`: this module converts values, owns
//! handles and projects errors onto the C boundary (see the crate docs for
//! the ownership and status conventions). It creates no runtime, dispatcher,
//! timeout or crypto layer of its own — every call lands on the facade
//! object, which runs on the shared FFI runtime.
//!
//! # Callback tables
//!
//! A host supplies a [`SpokeConnectPortsHandlerTable`] with one C-callable
//! pointer per [`ffi::PortsHandler`] method, or a
//! [`SpokeConnectToolHandlerTable`] with the single [`ffi::ToolHandler`]
//! `handle` pointer; both tables also carry a mandatory `destroy`. The
//! carrier copies the table and takes ownership of `user_data` on success:
//! `destroy(user_data)` runs exactly once, when the last Rust reference (the
//! caller's own handle, plus any responder that registered it) is gone.
//! Callbacks run on the shared runtime's blocking pool and may run
//! concurrently, so context state and `destroy` must be thread-safe and free
//! of thread affinity. They must never call back into this ABI (the facade's
//! re-entrancy rule).
//!
//! A callback answers status `0` with its result as one JSON buffer, or the
//! application reject status `301` with the four-field error record. Any other
//! status — and any result the operation's JSON type cannot decode — is
//! contained by the facade's `INTERNAL_ERROR` row instead of tearing down the
//! serve loop. Every populated callback buffer transfers to Rust on success
//! as well as on error (`A2` §C).

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;
use std::sync::Arc;

use spoke_connect::ffi;

use crate::remote_adapter::{
    transport_handle, write_json, write_optional_text, write_text, SharedForeignTransport,
    SpokeConnectTransport,
};
use crate::{
    borrowed_bytes, borrowed_strings, borrowed_text, contain_release, export, optional_u64,
    release_handle, require_out, take_foreign_error, take_foreign_text, AbiFailure,
    SpokeConnectBuffer, SpokeConnectError, SpokeConnectForeignBuffer, SpokeConnectForeignError,
    SpokeConnectOptionalBuffer, SpokeConnectOptionalU64, SpokeConnectSlice,
    SPOKE_CONNECT_FFI_REJECTED, SPOKE_CONNECT_OK,
};

/// Destroys a callback context. Runs exactly once, after the last Rust
/// reference and in-flight callback are gone; must not unwind.
pub type SpokeConnectCallbackDestroyFn = unsafe extern "C" fn(user_data: *mut c_void);

/// A ports callback that takes one borrowed UTF-8 argument and answers one
/// JSON buffer: `get_knowledge_entry`, `get_relation`,
/// `list_knowledge_entries`, `list_timeline_events`, `put_findings`,
/// `project`, `compute`, `list_fork_timeline_events` and `extract`.
///
/// The argument is borrowed for the duration of the call (empty text crosses
/// as a NULL/zero span); the callback copies anything it retains. The result
/// buffer transfers to Rust and is released exactly once.
pub type SpokeConnectPortsTextFn = unsafe extern "C" fn(
    user_data: *mut c_void,
    input_json: SpokeConnectSlice,
    out_json: *mut SpokeConnectForeignBuffer,
    out_error: *mut SpokeConnectForeignError,
) -> i32;

/// A ports callback that takes one borrowed JSON argument and an optional
/// expected base revision, and answers one JSON buffer:
/// `put_knowledge_entry` and `put_relation`. `expected_base_revision` follows
/// the optional-scalar rule — `present = 0` means no expectation, `present = 1`
/// keeps `value` even when it is zero.
pub type SpokeConnectPortsRevisionFn = unsafe extern "C" fn(
    user_data: *mut c_void,
    input_json: SpokeConnectSlice,
    expected_base_revision: SpokeConnectOptionalU64,
    out_json: *mut SpokeConnectForeignBuffer,
    out_error: *mut SpokeConnectForeignError,
) -> i32;

/// `list_rules`: the rule references cross as an array of borrowed UTF-8
/// slices (pointer + count; a NULL pointer is legal only with count zero).
pub type SpokeConnectPortsListRulesFn = unsafe extern "C" fn(
    user_data: *mut c_void,
    rule_refs: *const SpokeConnectSlice,
    rule_refs_count: usize,
    out_json: *mut SpokeConnectForeignBuffer,
    out_error: *mut SpokeConnectForeignError,
) -> i32;

/// A ports callback that takes no argument and answers one JSON buffer:
/// `list_peer_host_capability_manifests`.
pub type SpokeConnectPortsNoInputFn = unsafe extern "C" fn(
    user_data: *mut c_void,
    out_json: *mut SpokeConnectForeignBuffer,
    out_error: *mut SpokeConnectForeignError,
) -> i32;

/// The tool callback: `handle`. The arguments JSON is borrowed for the
/// duration of the call; the result buffer transfers to Rust and is released
/// exactly once.
pub type SpokeConnectToolHandleFn = unsafe extern "C" fn(
    user_data: *mut c_void,
    arguments_json: SpokeConnectSlice,
    out_json: *mut SpokeConnectForeignBuffer,
    out_error: *mut SpokeConnectForeignError,
) -> i32;

/// Ports callback table: one pointer per [`ffi::PortsHandler`] method plus the
/// mandatory `destroy`. Every pointer is required; a table missing one is
/// rejected as invalid argument, ownership of `user_data` stays with the
/// caller and `destroy` is not called.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SpokeConnectPortsHandlerTable {
    pub get_knowledge_entry: Option<SpokeConnectPortsTextFn>,
    pub put_knowledge_entry: Option<SpokeConnectPortsRevisionFn>,
    pub get_relation: Option<SpokeConnectPortsTextFn>,
    pub put_relation: Option<SpokeConnectPortsRevisionFn>,
    pub list_knowledge_entries: Option<SpokeConnectPortsTextFn>,
    pub list_timeline_events: Option<SpokeConnectPortsTextFn>,
    pub put_findings: Option<SpokeConnectPortsTextFn>,
    pub list_rules: Option<SpokeConnectPortsListRulesFn>,
    pub list_peer_host_capability_manifests: Option<SpokeConnectPortsNoInputFn>,
    pub project: Option<SpokeConnectPortsTextFn>,
    pub compute: Option<SpokeConnectPortsTextFn>,
    pub list_fork_timeline_events: Option<SpokeConnectPortsTextFn>,
    pub extract: Option<SpokeConnectPortsTextFn>,
    pub destroy: Option<SpokeConnectCallbackDestroyFn>,
}

/// Tool callback table: the [`ffi::ToolHandler`] `handle` pointer plus the
/// mandatory `destroy`. Both pointers are required.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SpokeConnectToolHandlerTable {
    pub handle: Option<SpokeConnectToolHandleFn>,
    pub destroy: Option<SpokeConnectCallbackDestroyFn>,
}

/// Opaque responder handle.
#[repr(C)]
pub struct SpokeConnectResponder {
    _private: [u8; 0],
}

/// Opaque foreign ports-handler handle.
#[repr(C)]
pub struct SpokeConnectPortsHandler {
    _private: [u8; 0],
}

/// Opaque foreign tool-handler handle.
#[repr(C)]
pub struct SpokeConnectToolHandler {
    _private: [u8; 0],
}

/// One peer key entry: the peer id it belongs to plus its 32-byte Ed25519
/// public key. Duplicate peer ids reject as invalid input rather than
/// silently choosing one key (`A2` §Ownership table).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SpokeConnectPeerKey {
    pub peer_id: SpokeConnectSlice,
    pub public_key: SpokeConnectSlice,
}

/// Borrows text as a callback argument span. Empty text crosses as a
/// NULL/zero span (`A2` §Calling and representation).
fn text_span(text: &str) -> SpokeConnectSlice {
    SpokeConnectSlice {
        data: if text.is_empty() {
            ptr::null()
        } else {
            text.as_ptr()
        },
        len: text.len(),
    }
}

/// The reverse of [`optional_u64`]: an absent option crosses as
/// `present = 0` with the zero value.
fn to_optional_u64(value: Option<u64>) -> SpokeConnectOptionalU64 {
    match value {
        Some(value) => SpokeConnectOptionalU64 {
            present: 1,
            value,
        },
        None => SpokeConnectOptionalU64 {
            present: 0,
            value: 0,
        },
    }
}

/// A missing callback pointer on an already-constructed handle. Construction
/// requires every pointer, so this is unreachable through this ABI; it is
/// projected as a `Dial` so the facade's containment row answers it rather
/// than panicking inside a callback.
fn missing_callback(what: &str, method: &str) -> ffi::FfiError {
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
unsafe fn callback_json(
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

// ── Foreign-callback `PortsHandler` (D16 serving face) ───────────────────

/// The carrier's [`ffi::PortsHandler`] implementation over a host vtable: one
/// callback call per D4 serve op.
pub(crate) struct ForeignPortsHandler {
    table: SpokeConnectPortsHandlerTable,
    user_data: *mut c_void,
}

// SAFETY: the A2 callback contract requires the host context, its callbacks
// and `destroy` to be thread-safe and free of thread affinity.
unsafe impl Send for ForeignPortsHandler {}
unsafe impl Sync for ForeignPortsHandler {}

impl Drop for ForeignPortsHandler {
    fn drop(&mut self) {
        if let Some(destroy) = self.table.destroy {
            contain_release(|| unsafe { destroy(self.user_data) });
        }
    }
}

impl ForeignPortsHandler {
    fn new(table: SpokeConnectPortsHandlerTable, user_data: *mut c_void) -> Self {
        Self { table, user_data }
    }

    /// Runs one text-in / JSON-out ports callback.
    unsafe fn call_text(
        &self,
        what: &str,
        callback: Option<SpokeConnectPortsTextFn>,
        input_json: SpokeConnectSlice,
    ) -> Result<String, ffi::FfiError> {
        let Some(callback) = callback else {
            return Err(missing_callback("ports handler", what));
        };
        let mut out_json = SpokeConnectForeignBuffer::empty();
        let mut error = SpokeConnectForeignError::empty();
        let status =
            unsafe { callback(self.user_data, input_json, &mut out_json, &mut error) };
        unsafe { callback_json(status, what, &mut out_json, &mut error) }
    }

    /// Runs one revision-carrying ports callback (`put_knowledge_entry` /
    /// `put_relation`); the optional scalar crosses as the optional-u64 struct.
    unsafe fn call_revision(
        &self,
        what: &str,
        callback: Option<SpokeConnectPortsRevisionFn>,
        input_json: SpokeConnectSlice,
        expected_base_revision: Option<u64>,
    ) -> Result<String, ffi::FfiError> {
        let Some(callback) = callback else {
            return Err(missing_callback("ports handler", what));
        };
        let mut out_json = SpokeConnectForeignBuffer::empty();
        let mut error = SpokeConnectForeignError::empty();
        let status = unsafe {
            callback(
                self.user_data,
                input_json,
                to_optional_u64(expected_base_revision),
                &mut out_json,
                &mut error,
            )
        };
        unsafe { callback_json(status, what, &mut out_json, &mut error) }
    }
}

impl ffi::PortsHandler for ForeignPortsHandler {
    fn get_knowledge_entry(&self, entry_id: String) -> Result<String, ffi::FfiError> {
        unsafe {
            self.call_text(
                "get_knowledge_entry",
                self.table.get_knowledge_entry,
                text_span(&entry_id),
            )
        }
    }

    fn put_knowledge_entry(
        &self,
        entry_json: String,
        expected_base_revision: Option<u64>,
    ) -> Result<String, ffi::FfiError> {
        unsafe {
            self.call_revision(
                "put_knowledge_entry",
                self.table.put_knowledge_entry,
                text_span(&entry_json),
                expected_base_revision,
            )
        }
    }

    fn get_relation(&self, relation_id: String) -> Result<String, ffi::FfiError> {
        unsafe {
            self.call_text(
                "get_relation",
                self.table.get_relation,
                text_span(&relation_id),
            )
        }
    }

    fn put_relation(
        &self,
        relation_json: String,
        expected_base_revision: Option<u64>,
    ) -> Result<String, ffi::FfiError> {
        unsafe {
            self.call_revision(
                "put_relation",
                self.table.put_relation,
                text_span(&relation_json),
                expected_base_revision,
            )
        }
    }

    fn list_knowledge_entries(&self, scope_json: String) -> Result<String, ffi::FfiError> {
        unsafe {
            self.call_text(
                "list_knowledge_entries",
                self.table.list_knowledge_entries,
                text_span(&scope_json),
            )
        }
    }

    fn list_timeline_events(&self, scope_json: String) -> Result<String, ffi::FfiError> {
        unsafe {
            self.call_text(
                "list_timeline_events",
                self.table.list_timeline_events,
                text_span(&scope_json),
            )
        }
    }

    fn put_findings(&self, findings_json: String) -> Result<String, ffi::FfiError> {
        unsafe {
            self.call_text(
                "put_findings",
                self.table.put_findings,
                text_span(&findings_json),
            )
        }
    }

    fn list_rules(&self, rule_refs: Vec<String>) -> Result<String, ffi::FfiError> {
        let Some(callback) = self.table.list_rules else {
            return Err(missing_callback("ports handler", "list_rules"));
        };
        let refs: Vec<SpokeConnectSlice> =
            rule_refs.iter().map(|rule_ref| text_span(rule_ref)).collect();
        let mut out_json = SpokeConnectForeignBuffer::empty();
        let mut error = SpokeConnectForeignError::empty();
        let status = unsafe {
            callback(
                self.user_data,
                if refs.is_empty() {
                    ptr::null()
                } else {
                    refs.as_ptr()
                },
                refs.len(),
                &mut out_json,
                &mut error,
            )
        };
        unsafe { callback_json(status, "list_rules", &mut out_json, &mut error) }
    }

    fn list_peer_host_capability_manifests(&self) -> Result<String, ffi::FfiError> {
        let Some(callback) = self.table.list_peer_host_capability_manifests else {
            return Err(missing_callback(
                "ports handler",
                "list_peer_host_capability_manifests",
            ));
        };
        let mut out_json = SpokeConnectForeignBuffer::empty();
        let mut error = SpokeConnectForeignError::empty();
        let status = unsafe { callback(self.user_data, &mut out_json, &mut error) };
        unsafe {
            callback_json(
                status,
                "list_peer_host_capability_manifests",
                &mut out_json,
                &mut error,
            )
        }
    }

    fn project(&self, project_request_json: String) -> Result<String, ffi::FfiError> {
        unsafe {
            self.call_text(
                "project",
                self.table.project,
                text_span(&project_request_json),
            )
        }
    }

    fn compute(&self, compute_request_json: String) -> Result<String, ffi::FfiError> {
        unsafe {
            self.call_text(
                "compute",
                self.table.compute,
                text_span(&compute_request_json),
            )
        }
    }

    fn list_fork_timeline_events(&self, scope_json: String) -> Result<String, ffi::FfiError> {
        unsafe {
            self.call_text(
                "list_fork_timeline_events",
                self.table.list_fork_timeline_events,
                text_span(&scope_json),
            )
        }
    }

    fn extract(&self, extract_request_json: String) -> Result<String, ffi::FfiError> {
        unsafe {
            self.call_text(
                "extract",
                self.table.extract,
                text_span(&extract_request_json),
            )
        }
    }
}

/// Borrowed-handle view of a ports handler: shares the handle's
/// [`ForeignPortsHandler`], so the caller's [`SpokeConnectPortsHandler`] keeps
/// owning the callback context (and its single `destroy`) after a responder
/// composed over it takes its own reference.
pub(crate) struct SharedForeignPortsHandler(Arc<ForeignPortsHandler>);

impl ffi::PortsHandler for SharedForeignPortsHandler {
    fn get_knowledge_entry(&self, entry_id: String) -> Result<String, ffi::FfiError> {
        self.0.get_knowledge_entry(entry_id)
    }

    fn put_knowledge_entry(
        &self,
        entry_json: String,
        expected_base_revision: Option<u64>,
    ) -> Result<String, ffi::FfiError> {
        self.0.put_knowledge_entry(entry_json, expected_base_revision)
    }

    fn get_relation(&self, relation_id: String) -> Result<String, ffi::FfiError> {
        self.0.get_relation(relation_id)
    }

    fn put_relation(
        &self,
        relation_json: String,
        expected_base_revision: Option<u64>,
    ) -> Result<String, ffi::FfiError> {
        self.0.put_relation(relation_json, expected_base_revision)
    }

    fn list_knowledge_entries(&self, scope_json: String) -> Result<String, ffi::FfiError> {
        self.0.list_knowledge_entries(scope_json)
    }

    fn list_timeline_events(&self, scope_json: String) -> Result<String, ffi::FfiError> {
        self.0.list_timeline_events(scope_json)
    }

    fn put_findings(&self, findings_json: String) -> Result<String, ffi::FfiError> {
        self.0.put_findings(findings_json)
    }

    fn list_rules(&self, rule_refs: Vec<String>) -> Result<String, ffi::FfiError> {
        self.0.list_rules(rule_refs)
    }

    fn list_peer_host_capability_manifests(&self) -> Result<String, ffi::FfiError> {
        self.0.list_peer_host_capability_manifests()
    }

    fn project(&self, project_request_json: String) -> Result<String, ffi::FfiError> {
        self.0.project(project_request_json)
    }

    fn compute(&self, compute_request_json: String) -> Result<String, ffi::FfiError> {
        self.0.compute(compute_request_json)
    }

    fn list_fork_timeline_events(&self, scope_json: String) -> Result<String, ffi::FfiError> {
        self.0.list_fork_timeline_events(scope_json)
    }

    fn extract(&self, extract_request_json: String) -> Result<String, ffi::FfiError> {
        self.0.extract(extract_request_json)
    }
}

// ── Foreign-callback `ToolHandler` (D16 serving face) ────────────────────

/// The carrier's [`ffi::ToolHandler`] implementation over a host vtable.
pub(crate) struct ForeignToolHandler {
    table: SpokeConnectToolHandlerTable,
    user_data: *mut c_void,
}

// SAFETY: the A2 callback contract requires the host context, its callbacks
// and `destroy` to be thread-safe and free of thread affinity.
unsafe impl Send for ForeignToolHandler {}
unsafe impl Sync for ForeignToolHandler {}

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

// ── Handle borrows and releases ──────────────────────────────────────────

/// Borrows a responder handle.
unsafe fn responder_handle<'a>(
    handle: *const SpokeConnectResponder,
) -> Result<&'a Arc<ffi::ConnectResponderFFI>, AbiFailure> {
    if handle.is_null() {
        return Err(AbiFailure::invalid("responder is NULL"));
    }
    Ok(unsafe { &*(handle as *const Arc<ffi::ConnectResponderFFI>) })
}

/// Borrows a ports-handler handle.
unsafe fn ports_handler_handle<'a>(
    handle: *const SpokeConnectPortsHandler,
) -> Result<&'a Arc<ForeignPortsHandler>, AbiFailure> {
    if handle.is_null() {
        return Err(AbiFailure::invalid("ports_handler is NULL"));
    }
    Ok(unsafe { &*(handle as *const Arc<ForeignPortsHandler>) })
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

/// Builds the peer-key map from the C array. A duplicate peer id rejects as
/// invalid input rather than silently choosing one key (`A2` §Ownership
/// table).
unsafe fn peer_key_map(
    entries: *const SpokeConnectPeerKey,
    count: usize,
) -> Result<HashMap<String, Vec<u8>>, AbiFailure> {
    let mut keys = HashMap::new();
    if count == 0 {
        if entries.is_null() {
            return Ok(keys);
        }
        return Err(AbiFailure::invalid(
            "peer_keys pointer must be NULL with count zero",
        ));
    }
    if entries.is_null() {
        return Err(AbiFailure::invalid("peer_keys is NULL"));
    }
    let entries = unsafe { std::slice::from_raw_parts(entries, count) };
    for entry in entries {
        let peer_id = unsafe { borrowed_text(entry.peer_id, "peer_keys.peer_id") }?.to_owned();
        let public_key = unsafe { borrowed_bytes(entry.public_key, "peer_keys.public_key") }?.to_vec();
        if keys.insert(peer_id.clone(), public_key).is_some() {
            return Err(AbiFailure::invalid(format!(
                "duplicate peer key for {peer_id}"
            )));
        }
    }
    Ok(keys)
}

/// Creates a ports-handler handle from a callback table. On success ownership
/// of `user_data` transfers to the handle; on failure nothing is transferred
/// and `destroy` is not called.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_ports_handler_new(
    table: *const SpokeConnectPortsHandlerTable,
    user_data: *mut c_void,
    out_handler: *mut *mut SpokeConnectPortsHandler,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_handler = require_out(out_handler, "out_handler")?;
            out_handler.write(ptr::null_mut());
            if table.is_null() {
                return Err(AbiFailure::invalid("table is NULL"));
            }
            let table = *table;
            if table.get_knowledge_entry.is_none()
                || table.put_knowledge_entry.is_none()
                || table.get_relation.is_none()
                || table.put_relation.is_none()
                || table.list_knowledge_entries.is_none()
                || table.list_timeline_events.is_none()
                || table.put_findings.is_none()
                || table.list_rules.is_none()
                || table.list_peer_host_capability_manifests.is_none()
                || table.project.is_none()
                || table.compute.is_none()
                || table.list_fork_timeline_events.is_none()
                || table.extract.is_none()
                || table.destroy.is_none()
            {
                return Err(AbiFailure::invalid(
                    "ports handler table must carry all thirteen callbacks and destroy",
                ));
            }
            let handler = Arc::new(ForeignPortsHandler::new(table, user_data));
            out_handler.write(Box::into_raw(Box::new(handler)) as *mut SpokeConnectPortsHandler);
            Ok(())
        })
    }
}

/// Releases a ports-handler handle. A NULL handle is a no-op. Releasing the
/// last reference runs the host's `destroy` once.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_ports_handler_free(handler: *mut SpokeConnectPortsHandler) {
    unsafe { release_handle(handler as *mut Arc<ForeignPortsHandler>) };
}

/// Creates a tool-handler handle from a callback table. On success ownership
/// of `user_data` transfers to the handle; on failure nothing is transferred
/// and `destroy` is not called.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_tool_handler_new(
    table: *const SpokeConnectToolHandlerTable,
    user_data: *mut c_void,
    out_handler: *mut *mut SpokeConnectToolHandler,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_handler = require_out(out_handler, "out_handler")?;
            out_handler.write(ptr::null_mut());
            if table.is_null() {
                return Err(AbiFailure::invalid("table is NULL"));
            }
            let table = *table;
            if table.handle.is_none() || table.destroy.is_none() {
                return Err(AbiFailure::invalid(
                    "tool handler table must carry handle and destroy",
                ));
            }
            let handler = Arc::new(ForeignToolHandler { table, user_data });
            out_handler.write(Box::into_raw(Box::new(handler)) as *mut SpokeConnectToolHandler);
            Ok(())
        })
    }
}

/// Releases a tool-handler handle. A NULL handle is a no-op. Releasing the
/// last reference runs the host's `destroy` once.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_tool_handler_free(handler: *mut SpokeConnectToolHandler) {
    unsafe { release_handle(handler as *mut Arc<ForeignToolHandler>) };
}

// ── Accepting `ConnectResponder` ─────────────────────────────────────────

/// Accepts a connection over `transport` and returns a responder. The
/// transport handle is borrowed: the caller keeps ownership of it (and of its
/// callback context) and must close/release the responder and then the
/// transport handle.
///
/// Constructor semantics are the facade's: the responder returns in
/// `Handshaking` — the dialer hello is the sync point — and the `Result` slot
/// carries configuration failures only (seed length, manifest JSON, peer-key
/// length). Handshake failures produce no error record; they surface as
/// `state() → "Closed"`. `ports` is optional: a NULL handle preserves the
/// documented absent-ports deny branch.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_responder_new(
    transport: *const SpokeConnectTransport,
    local_seed: SpokeConnectSlice,
    local_manifest_json: SpokeConnectSlice,
    allowlist: *const SpokeConnectSlice,
    allowlist_count: usize,
    peer_keys: *const SpokeConnectPeerKey,
    peer_key_count: usize,
    ports: *const SpokeConnectPortsHandler,
    invoke_timeout_ms: SpokeConnectOptionalU64,
    out_responder: *mut *mut SpokeConnectResponder,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_responder = require_out(out_responder, "out_responder")?;
            out_responder.write(ptr::null_mut());
            let transport = transport_handle(transport)?;
            let local_seed = borrowed_bytes(local_seed, "local_seed")?.to_vec();
            let local_manifest_json =
                borrowed_text(local_manifest_json, "local_manifest_json")?.to_owned();
            let allowlist = borrowed_strings(allowlist, allowlist_count, "allowlist")?;
            let peer_keys = peer_key_map(peer_keys, peer_key_count)?;
            let ports: Option<Box<dyn ffi::PortsHandler>> = if ports.is_null() {
                None
            } else {
                Some(Box::new(SharedForeignPortsHandler(Arc::clone(
                    ports_handler_handle(ports)?,
                ))))
            };
            let invoke_timeout_ms = optional_u64(invoke_timeout_ms, "invoke_timeout_ms")?;
            let responder = ffi::connect_responder_ffi(
                Box::new(SharedForeignTransport::new(Arc::clone(transport))),
                local_seed,
                local_manifest_json,
                allowlist,
                peer_keys,
                ports,
                invoke_timeout_ms,
            )
            .map_err(AbiFailure::from)?;
            out_responder.write(Box::into_raw(Box::new(responder)) as *mut SpokeConnectResponder);
            Ok(())
        })
    }
}

/// Releases a responder handle. A NULL handle is a no-op. Close the session
/// before releasing the handle.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_responder_free(responder: *mut SpokeConnectResponder) {
    unsafe { release_handle(responder as *mut Arc<ffi::ConnectResponderFFI>) };
}

/// Session lifecycle label (`Disconnected` / `Handshaking` / `Established` /
/// `Closed`).
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_responder_state(
    responder: *const SpokeConnectResponder,
    out_state: *mut SpokeConnectBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_state = require_out(out_state, "out_state")?;
            out_state.write(SpokeConnectBuffer::empty());
            write_text(out_state, responder_handle(responder)?.state())
        })
    }
}

/// The established session id, absent before establishment.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_responder_session_id(
    responder: *const SpokeConnectResponder,
    out_session_id: *mut SpokeConnectOptionalBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_session_id = require_out(out_session_id, "out_session_id")?;
            out_session_id.write(SpokeConnectOptionalBuffer::empty());
            write_optional_text(out_session_id, responder_handle(responder)?.session_id())
        })
    }
}

/// The dialer's peer id, absent before establishment.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_responder_remote_peer_id(
    responder: *const SpokeConnectResponder,
    out_peer_id: *mut SpokeConnectOptionalBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_peer_id = require_out(out_peer_id, "out_peer_id")?;
            out_peer_id.write(SpokeConnectOptionalBuffer::empty());
            write_optional_text(out_peer_id, responder_handle(responder)?.remote_peer_id())
        })
    }
}

/// The dialer's hello manifest as JSON, absent before establishment.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_responder_remote_manifest(
    responder: *const SpokeConnectResponder,
    out_manifest_json: *mut SpokeConnectOptionalBuffer,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let out_manifest_json = require_out(out_manifest_json, "out_manifest_json")?;
            out_manifest_json.write(SpokeConnectOptionalBuffer::empty());
            write_optional_text(
                out_manifest_json,
                responder_handle(responder)?.remote_manifest(),
            )
        })
    }
}

/// Registers the tool handler that serves `capability_id` for invokes this
/// responder receives. The handler handle is borrowed: the responder keeps its
/// own reference and does not consume the caller's handle. A non-`tools.` id
/// rejects as invalid input with zero side effect; a valid id is last-wins.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_responder_register_tool_handler(
    responder: *const SpokeConnectResponder,
    capability_id: SpokeConnectSlice,
    handler: *const SpokeConnectToolHandler,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            let capability_id = borrowed_text(capability_id, "capability_id")?.to_owned();
            let handler = tool_handler_handle(handler)?;
            responder_handle(responder)?
                .register_tool_handler(
                    capability_id,
                    Box::new(SharedForeignToolHandler::new(Arc::clone(handler))),
                )
                .map_err(AbiFailure::from)
        })
    }
}

/// Responder→dialer reverse invoke (`tools.<ns>.<tool_id>`): issues the invoke
/// toward the connected peer and returns the tool's result payload as JSON.
/// Same error rows as the dialing adapter's tool invoke.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_responder_invoke_tool(
    responder: *const SpokeConnectResponder,
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
                responder_handle(responder)?.invoke_tool(capability_id, arguments_json),
            )
        })
    }
}

/// Closes the responder's session and its transport. Distinct from
/// [`spoke_connect_responder_free`]: close the session first, then release the
/// handle.
#[no_mangle]
pub unsafe extern "C" fn spoke_connect_responder_close(
    responder: *const SpokeConnectResponder,
    out_error: *mut SpokeConnectError,
) -> i32 {
    unsafe {
        export(out_error, || {
            responder_handle(responder)?.close();
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Condvar, Mutex};
    use std::time::{Duration, Instant};

    use spoke_connect::ffi::PortsHandler as _;

    use crate::remote_adapter::{
        spoke_connect_remote_adapter_close, spoke_connect_remote_adapter_free,
        spoke_connect_remote_adapter_invoke_tool, spoke_connect_remote_adapter_list_rules,
        spoke_connect_remote_adapter_new, spoke_connect_remote_adapter_session_id,
        spoke_connect_remote_adapter_state, SpokeConnectRemoteAdapter,
    };
    use crate::{
        spoke_connect_buffer_free, spoke_connect_error_free, spoke_connect_optional_buffer_free,
        SPOKE_CONNECT_FFI_REJECTED, SPOKE_CONNECT_INVALID_ARGUMENT, SPOKE_CONNECT_OK,
        SPOKE_CONNECT_TRANSPORT_CLOSED,
    };

    // ── Shared fixtures (loaded, never re-derived) ───────────────────────

    const ROUTER_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../spoke-connect/bindings/swift/Smoke/fixtures/multi-peer-router-smoke.json"
    ));

    const LOOPBACK_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../spoke-connect/bindings/swift/Smoke/fixtures/loopback-smoke.json"
    ));

    /// The tool capability this battery registers and invokes.
    const ECHO_TOOL_ID: &str = "tools.test.echo";

    /// The per-invoke deadline the battery dials with, and the short deadline
    /// the timed-out-wait witness uses.
    const INVOKE_TIMEOUT_MS: u64 = 5_000;
    const SHORT_TIMEOUT_MS: u64 = 50;

    /// Static storage the host callbacks hand to Rust — the "a host returning
    /// static storage supplies a no-op release" case (`A2` §Ownership table).
    static DECLINE_MESSAGE: &[u8] = b"this test host does not serve the op";
    static DECLINE_CODE: &[u8] = b"CAPABILITY_PORT_MISSING";
    static UNKNOWN_CODE: &[u8] = b"NOT_A_LOCKED_CODE";
    static FAULT_DETAIL: &[u8] = b"host callback fault";
    static MALFORMED_BODY: &[u8] = b"{ not json";

    /// The reject code [`DECLINE_CODE`] carries, as text.
    const DECLINE_CODE_STR: &str = "CAPABILITY_PORT_MISSING";

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
    struct Identity {
        seed: [u8; 32],
        pubkey: [u8; 32],
        peer_id: String,
        manifest_json: String,
    }

    /// The dialing client (the responder's peer): seed, peer id and manifest
    /// from the router vector, public key from the loopback vector (same
    /// identity).
    fn dialer_identity() -> Identity {
        let router = fixture(ROUTER_FIXTURE);
        let loopback = fixture(LOOPBACK_FIXTURE);
        Identity {
            seed: hex32(&text_field(&router, "seed_client_hex")),
            pubkey: hex32(&text_field(&loopback, "pubkey_client_hex")),
            peer_id: text_field(&router, "peer_id_client"),
            manifest_json: text_field(&router, "client_manifest_json"),
        }
    }

    /// The accepting host under test.
    fn host_identity() -> Identity {
        let router = fixture(ROUTER_FIXTURE);
        Identity {
            seed: hex32(&text_field(&router, "baseline_host_seed_hex")),
            pubkey: hex32(&text_field(&router, "baseline_pubkey_host_hex")),
            peer_id: text_field(&router, "baseline_peer_id_host"),
            manifest_json: text_field(&router, "baseline_manifest_json"),
        }
    }

    /// The fixture manifest plus this battery's tool capability in
    /// `capabilities[]` and a matching `tools[]` entry, so `tools.*` is in the
    /// negotiated set on both ends.
    fn with_tool_capability(manifest_json: &str) -> String {
        let mut manifest: serde_json::Value =
            serde_json::from_str(manifest_json).expect("manifest JSON parses");
        manifest["capabilities"]
            .as_array_mut()
            .expect("capabilities is an array")
            .push(serde_json::json!(ECHO_TOOL_ID));
        manifest["tools"] = serde_json::json!([{
            "schema_version": 1,
            "capability_id": ECHO_TOOL_ID,
            "op": ECHO_TOOL_ID,
            "description": "Echo the arguments",
            "input": { "type": "object" },
            "output": { "type": "object" },
        }]);
        serde_json::to_string(&manifest).expect("the manifest serializes")
    }

    /// A schema-valid L6 rule tagged with the host that serves it, so a served
    /// answer identifies the host.
    fn rule_value(rule_id: &str) -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1,
            "rule_id": rule_id,
            "canonical_name": "No resurrection without foreshadowing",
            "kind": "rule",
            "statement": "Character death reversals require a prior foreshadowing entry.",
            "target_entry_types": ["character", "event"],
            "severity_hint": "error",
            "status": "active",
            "extensions": {},
        })
    }

    // ── Host-side transport harness ──────────────────────────────────────

    fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(|poison| poison.into_inner())
    }

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

    /// One end of a host-owned synchronized message queue pair — the transport
    /// model the `A2` re-entrancy rule requires (callbacks never call back
    /// into the exported loopback helpers).
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

    fn endpoint_pair() -> (Endpoint, Endpoint) {
        let dialer_to_host = Arc::new(Queue::default());
        let host_to_dialer = Arc::new(Queue::default());
        (
            Endpoint {
                outbound: Arc::clone(&dialer_to_host),
                inbound: Arc::clone(&host_to_dialer),
            },
            Endpoint {
                outbound: host_to_dialer,
                inbound: dialer_to_host,
            },
        )
    }

    /// What the host observed about the transport callbacks the carrier ran.
    #[derive(Default)]
    struct TransportLog {
        send: AtomicUsize,
        recv_started: AtomicUsize,
        recv_in_flight: AtomicUsize,
        recv_closed: AtomicUsize,
        release: AtomicUsize,
        destroy: AtomicUsize,
    }

    /// The host transport context behind the A2 transport vtable.
    struct HostTransport {
        endpoint: Endpoint,
        log: Arc<TransportLog>,
    }

    /// Releases a host-owned envelope buffer. The carrier calls it exactly
    /// once per populated buffer.
    unsafe extern "C" fn transport_buffer_release(
        context: *mut c_void,
        data: *const u8,
        len: usize,
    ) {
        let log = unsafe { &*(context as *const TransportLog) };
        log.release.fetch_add(1, Ordering::SeqCst);
        if data.is_null() || len == 0 {
            return;
        }
        unsafe { drop(Box::from_raw(ptr::slice_from_raw_parts_mut(data as *mut u8, len))) };
    }

    unsafe fn write_closed_error(
        log: &Arc<TransportLog>,
        out_error: *mut SpokeConnectForeignError,
    ) {
        if out_error.is_null() {
            return;
        }
        unsafe {
            out_error.write(SpokeConnectForeignError {
                message: SpokeConnectForeignBuffer {
                    data: DECLINE_MESSAGE.as_ptr(),
                    len: DECLINE_MESSAGE.len(),
                    release_context: Arc::as_ptr(log) as *mut c_void,
                    release: Some(transport_static_release),
                },
                code: SpokeConnectForeignBuffer::empty(),
                kind: SpokeConnectForeignBuffer::empty(),
                wire_code: SpokeConnectForeignBuffer::empty(),
            })
        };
    }

    /// Releases the host's static transport-closed message (a counted no-op).
    unsafe extern "C" fn transport_static_release(
        context: *mut c_void,
        _data: *const u8,
        _len: usize,
    ) {
        let log = unsafe { &*(context as *const TransportLog) };
        log.release.fetch_add(1, Ordering::SeqCst);
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
        let mut owned = envelope.into_boxed_slice();
        let data = owned.as_mut_ptr();
        let len = owned.len();
        std::mem::forget(owned);
        unsafe {
            out_envelope.write(SpokeConnectForeignBuffer {
                data,
                len,
                release_context: Arc::as_ptr(&host.log) as *mut c_void,
                release: Some(transport_buffer_release),
            })
        };
        SPOKE_CONNECT_OK
    }

    unsafe extern "C" fn host_close(
        user_data: *mut c_void,
        _out_error: *mut SpokeConnectForeignError,
    ) -> i32 {
        let host = unsafe { &*(user_data as *const HostTransport) };
        host.endpoint.close();
        SPOKE_CONNECT_OK
    }

    unsafe extern "C" fn host_destroy(user_data: *mut c_void) {
        let host = unsafe { Arc::from_raw(user_data as *const HostTransport) };
        host.log.destroy.fetch_add(1, Ordering::SeqCst);
    }

    fn host_transport_table() -> crate::remote_adapter::SpokeConnectTransportTable {
        crate::remote_adapter::SpokeConnectTransportTable {
            send: Some(host_send),
            recv: Some(host_recv),
            close: Some(host_close),
            destroy: Some(host_destroy),
        }
    }

    /// Creates a transport handle over a host queue end.
    fn host_transport(endpoint: Endpoint) -> (*mut SpokeConnectTransport, Arc<TransportLog>) {
        let log = Arc::new(TransportLog::default());
        let host = Arc::new(HostTransport {
            endpoint,
            log: Arc::clone(&log),
        });
        let mut transport: *mut SpokeConnectTransport = ptr::null_mut();
        let mut error = empty_record();
        let status = unsafe {
            crate::remote_adapter::spoke_connect_transport_new(
                &host_transport_table(),
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

    /// A dialer transport for the facade adapter: what the facade's blocking
    /// `ffi::Transport` seam consumes (used where the dialer must serve a
    /// tool, which the adapter C face does not register).
    struct FacadeTransport {
        endpoint: Endpoint,
    }

    impl ffi::Transport for FacadeTransport {
        fn send(&self, envelope: Vec<u8>) -> Result<(), ffi::TransportError> {
            self.endpoint.send(envelope);
            Ok(())
        }

        fn recv(&self) -> Result<Vec<u8>, ffi::TransportError> {
            self.endpoint.recv().ok_or(ffi::TransportError::Closed)
        }

        fn close(&self) -> Result<(), ffi::TransportError> {
            self.endpoint.close();
            Ok(())
        }
    }

    // ── Host ports / tool callback contexts ──────────────────────────────

    /// How the host ports callback answers the next `list_rules` call.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum PortsBehavior {
        /// Serve the catalogue entry.
        Serve,
        /// Answer the application reject status with a code outside the locked
        /// vocabulary, which the facade bridge downgrades to `INTERNAL_ERROR`.
        UnknownRejectCode,
        /// Answer a status ports callbacks do not accept (containment row).
        UnsupportedStatus,
        /// Answer the success status with a body the op's JSON type cannot
        /// decode (containment row).
        MalformedJson,
    }

    impl Default for PortsBehavior {
        fn default() -> Self {
            Self::Serve
        }
    }

    /// A host callback context: the behavior the test steers, the
    /// observations it asserts on, and the counters its release/destroy
    /// callbacks bump. Shared with the test through `Arc`, so the assertions
    /// survive the ownership transfer to the carrier.
    #[derive(Default)]
    struct HostState {
        release: AtomicUsize,
        destroy: AtomicUsize,
        rule_refs: Mutex<Vec<Vec<String>>>,
        revisions: Mutex<Vec<Option<u64>>>,
        tool_arguments: Mutex<Vec<String>>,
        behavior: Mutex<PortsBehavior>,
        /// When set, `list_rules` parks here until the test releases it, so
        /// the battery can observe a callback that is still in flight.
        hold: Mutex<Option<Arc<Hold>>>,
        /// The callback invocations that entered (parked or not).
        entered: AtomicUsize,
    }

    /// A gate a host callback waits on before answering.
    #[derive(Default)]
    struct Hold {
        released: Mutex<bool>,
        signal: Condvar,
    }

    impl Hold {
        fn wait(&self) {
            let mut released = lock(&self.released);
            while !*released {
                released = self
                    .signal
                    .wait(released)
                    .unwrap_or_else(|poison| poison.into_inner());
            }
        }

        fn release(&self) {
            *lock(&self.released) = true;
            self.signal.notify_all();
        }
    }

    /// The host ports callback context behind the A2 ports table.
    struct HostPorts {
        rule_id: String,
        state: Arc<HostState>,
    }

    /// The host tool callback context behind the A2 tool table.
    struct HostTool {
        payload: String,
        state: Arc<HostState>,
    }

    /// Hands one host-owned buffer to Rust: the carrier copies it and calls
    /// `release` exactly once with the same pointer and length.
    unsafe extern "C" fn host_owned_release(context: *mut c_void, data: *const u8, len: usize) {
        let state = unsafe { &*(context as *const HostState) };
        state.release.fetch_add(1, Ordering::SeqCst);
        if data.is_null() || len == 0 {
            return;
        }
        unsafe { drop(Box::from_raw(ptr::slice_from_raw_parts_mut(data as *mut u8, len))) };
    }

    /// Releases the host's static storage (a counted no-op, so the battery can
    /// observe the carrier releasing every populated buffer exactly once).
    unsafe extern "C" fn host_static_release(context: *mut c_void, _data: *const u8, _len: usize) {
        let state = unsafe { &*(context as *const HostState) };
        state.release.fetch_add(1, Ordering::SeqCst);
    }

    fn static_field(state: &Arc<HostState>, text: &'static [u8]) -> SpokeConnectForeignBuffer {
        SpokeConnectForeignBuffer {
            data: text.as_ptr(),
            len: text.len(),
            release_context: Arc::as_ptr(state) as *mut c_void,
            release: Some(host_static_release),
        }
    }

    fn owned_field(state: &Arc<HostState>, bytes: Vec<u8>) -> SpokeConnectForeignBuffer {
        let mut owned = bytes.into_boxed_slice();
        let data = owned.as_mut_ptr();
        let len = owned.len();
        std::mem::forget(owned);
        SpokeConnectForeignBuffer {
            data,
            len,
            release_context: Arc::as_ptr(state) as *mut c_void,
            release: Some(host_owned_release),
        }
    }

    /// Writes an application reject record: code and message over static
    /// storage, both released once through the counted no-op release.
    unsafe fn write_reject(
        state: &Arc<HostState>,
        out_error: *mut SpokeConnectForeignError,
        code: &'static [u8],
        message: &'static [u8],
    ) {
        if out_error.is_null() {
            return;
        }
        unsafe {
            out_error.write(SpokeConnectForeignError {
                message: static_field(state, message),
                code: static_field(state, code),
                kind: SpokeConnectForeignBuffer::empty(),
                wire_code: SpokeConnectForeignBuffer::empty(),
            })
        };
    }

    /// Writes a callback fault: an unsupported status carrying a populated
    /// message/kind pair.
    unsafe fn write_fault(
        state: &Arc<HostState>,
        out_error: *mut SpokeConnectForeignError,
        detail: &'static [u8],
    ) {
        if out_error.is_null() {
            return;
        }
        unsafe {
            out_error.write(SpokeConnectForeignError {
                message: static_field(state, detail),
                code: SpokeConnectForeignBuffer::empty(),
                kind: static_field(state, detail),
                wire_code: SpokeConnectForeignBuffer::empty(),
            })
        };
    }

    /// Declines an op this test host does not serve — the distinction between
    /// an explicit refusing callback and an absent ports face.
    unsafe extern "C" fn host_ports_decline_text(
        user_data: *mut c_void,
        _input_json: SpokeConnectSlice,
        _out_json: *mut SpokeConnectForeignBuffer,
        out_error: *mut SpokeConnectForeignError,
    ) -> i32 {
        let host = unsafe { &*(user_data as *const HostPorts) };
        unsafe { write_reject(&host.state, out_error, DECLINE_CODE, DECLINE_MESSAGE) };
        SPOKE_CONNECT_FFI_REJECTED
    }

    unsafe extern "C" fn host_ports_decline_revision(
        user_data: *mut c_void,
        _input_json: SpokeConnectSlice,
        _expected_base_revision: SpokeConnectOptionalU64,
        _out_json: *mut SpokeConnectForeignBuffer,
        out_error: *mut SpokeConnectForeignError,
    ) -> i32 {
        let host = unsafe { &*(user_data as *const HostPorts) };
        unsafe { write_reject(&host.state, out_error, DECLINE_CODE, DECLINE_MESSAGE) };
        SPOKE_CONNECT_FFI_REJECTED
    }

    unsafe extern "C" fn host_ports_decline_no_input(
        user_data: *mut c_void,
        _out_json: *mut SpokeConnectForeignBuffer,
        out_error: *mut SpokeConnectForeignError,
    ) -> i32 {
        let host = unsafe { &*(user_data as *const HostPorts) };
        unsafe { write_reject(&host.state, out_error, DECLINE_CODE, DECLINE_MESSAGE) };
        SPOKE_CONNECT_FFI_REJECTED
    }

    /// Records the optional revision the boundary handed over, then declines
    /// (this host serves no upsert).
    unsafe extern "C" fn host_ports_put_knowledge_entry(
        user_data: *mut c_void,
        _input_json: SpokeConnectSlice,
        expected_base_revision: SpokeConnectOptionalU64,
        _out_json: *mut SpokeConnectForeignBuffer,
        out_error: *mut SpokeConnectForeignError,
    ) -> i32 {
        let host = unsafe { &*(user_data as *const HostPorts) };
        let revision = match expected_base_revision.present {
            0 => None,
            _ => Some(expected_base_revision.value),
        };
        lock(&host.state.revisions).push(revision);
        unsafe { write_reject(&host.state, out_error, DECLINE_CODE, DECLINE_MESSAGE) };
        SPOKE_CONNECT_FFI_REJECTED
    }

    /// Serves `list_rules`: it records the borrowed reference array the
    /// boundary handed over and answers the rule list that identifies this
    /// host. The behavior switch steers the downgrade and containment rows.
    unsafe extern "C" fn host_ports_list_rules(
        user_data: *mut c_void,
        rule_refs: *const SpokeConnectSlice,
        rule_refs_count: usize,
        out_json: *mut SpokeConnectForeignBuffer,
        out_error: *mut SpokeConnectForeignError,
    ) -> i32 {
        let host = unsafe { &*(user_data as *const HostPorts) };
        host.state.entered.fetch_add(1, Ordering::SeqCst);
        let refs = if rule_refs.is_null() || rule_refs_count == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(rule_refs, rule_refs_count) }
                .iter()
                .map(|entry| {
                    let bytes = if entry.data.is_null() || entry.len == 0 {
                        &[][..]
                    } else {
                        unsafe { std::slice::from_raw_parts(entry.data, entry.len) }
                    };
                    String::from_utf8(bytes.to_vec()).expect("rule references cross as UTF-8")
                })
                .collect()
        };
        lock(&host.state.rule_refs).push(refs);

        let hold = lock(&host.state.hold).clone();
        if let Some(hold) = hold {
            hold.wait();
        }

        match *lock(&host.state.behavior) {
            PortsBehavior::Serve => {
                if !out_json.is_null() {
                    let body = serde_json::to_vec(&[rule_value(&host.rule_id)])
                        .expect("the served rule list serializes");
                    unsafe { out_json.write(owned_field(&host.state, body)) };
                }
                SPOKE_CONNECT_OK
            }
            PortsBehavior::UnknownRejectCode => {
                unsafe { write_reject(&host.state, out_error, UNKNOWN_CODE, DECLINE_MESSAGE) };
                SPOKE_CONNECT_FFI_REJECTED
            }
            PortsBehavior::UnsupportedStatus => {
                unsafe { write_fault(&host.state, out_error, FAULT_DETAIL) };
                // A status ports and tool callbacks do not accept.
                crate::SPOKE_CONNECT_FFI_DIAL
            }
            PortsBehavior::MalformedJson => {
                if !out_json.is_null() {
                    unsafe {
                        out_json.write(owned_field(&host.state, MALFORMED_BODY.to_vec()))
                    };
                }
                SPOKE_CONNECT_OK
            }
        }
    }

    unsafe extern "C" fn host_ports_destroy(user_data: *mut c_void) {
        let host = unsafe { Arc::from_raw(user_data as *const HostPorts) };
        host.state.destroy.fetch_add(1, Ordering::SeqCst);
    }

    fn host_ports_table() -> SpokeConnectPortsHandlerTable {
        SpokeConnectPortsHandlerTable {
            get_knowledge_entry: Some(host_ports_decline_text),
            put_knowledge_entry: Some(host_ports_put_knowledge_entry),
            get_relation: Some(host_ports_decline_text),
            put_relation: Some(host_ports_decline_revision),
            list_knowledge_entries: Some(host_ports_decline_text),
            list_timeline_events: Some(host_ports_decline_text),
            put_findings: Some(host_ports_decline_text),
            list_rules: Some(host_ports_list_rules),
            list_peer_host_capability_manifests: Some(host_ports_decline_no_input),
            project: Some(host_ports_decline_text),
            compute: Some(host_ports_decline_text),
            list_fork_timeline_events: Some(host_ports_decline_text),
            extract: Some(host_ports_decline_text),
            destroy: Some(host_ports_destroy),
        }
    }

    /// Creates a ports-handler handle over a fresh host context and returns
    /// the shared state the assertions read.
    unsafe fn new_host_ports(rule_id: &str) -> (*mut SpokeConnectPortsHandler, Arc<HostState>) {
        let host = Arc::new(HostPorts {
            rule_id: rule_id.to_owned(),
            state: Arc::new(HostState::default()),
        });
        let state = Arc::clone(&host.state);
        let mut handler: *mut SpokeConnectPortsHandler = ptr::null_mut();
        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_ports_handler_new(
                &host_ports_table(),
                Arc::into_raw(host) as *mut c_void,
                &mut handler,
                &mut error,
            )
        };
        assert_eq!(
            status,
            SPOKE_CONNECT_OK,
            "ports handle: {}",
            unsafe { error_fields(&mut error) }.message
        );
        assert!(!handler.is_null());
        (handler, state)
    }

    /// Serves the tool result the host is configured with, recording the
    /// arguments JSON it received.
    unsafe extern "C" fn host_tool_handle(
        user_data: *mut c_void,
        arguments_json: SpokeConnectSlice,
        out_json: *mut SpokeConnectForeignBuffer,
        out_error: *mut SpokeConnectForeignError,
    ) -> i32 {
        let host = unsafe { &*(user_data as *const HostTool) };
        let arguments = if arguments_json.data.is_null() || arguments_json.len == 0 {
            String::new()
        } else {
            String::from_utf8(
                unsafe { std::slice::from_raw_parts(arguments_json.data, arguments_json.len) }
                    .to_vec(),
            )
            .expect("tool arguments cross as UTF-8")
        };
        lock(&host.state.tool_arguments).push(arguments);
        let _ = out_error;
        if out_json.is_null() {
            return SPOKE_CONNECT_OK;
        }
        unsafe {
            out_json.write(owned_field(&host.state, host.payload.clone().into_bytes()))
        };
        SPOKE_CONNECT_OK
    }

    unsafe extern "C" fn host_tool_destroy(user_data: *mut c_void) {
        let host = unsafe { Arc::from_raw(user_data as *const HostTool) };
        host.state.destroy.fetch_add(1, Ordering::SeqCst);
    }

    fn host_tool_table() -> SpokeConnectToolHandlerTable {
        SpokeConnectToolHandlerTable {
            handle: Some(host_tool_handle),
            destroy: Some(host_tool_destroy),
        }
    }

    /// Creates a tool-handler handle answering `payload`, and returns the
    /// shared state the assertions read.
    unsafe fn new_host_tool(payload: &str) -> (*mut SpokeConnectToolHandler, Arc<HostState>) {
        let host = Arc::new(HostTool {
            payload: payload.to_owned(),
            state: Arc::new(HostState::default()),
        });
        let state = Arc::clone(&host.state);
        let mut handler: *mut SpokeConnectToolHandler = ptr::null_mut();
        let mut error = empty_record();
        let status = unsafe {
            spoke_connect_tool_handler_new(
                &host_tool_table(),
                Arc::into_raw(host) as *mut c_void,
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

    fn optional_scalar(value: Option<u64>) -> SpokeConnectOptionalU64 {
        match value {
            Some(value) => SpokeConnectOptionalU64 {
                present: 1,
                value,
            },
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

    /// The responder's lifecycle label.
    unsafe fn responder_state(responder: *const SpokeConnectResponder) -> String {
        let mut state = SpokeConnectBuffer::empty();
        let mut error = empty_record();
        assert_eq!(
            unsafe { spoke_connect_responder_state(responder, &mut state, &mut error) },
            SPOKE_CONNECT_OK,
            "responder state: {}",
            unsafe { error_fields(&mut error) }.message
        );
        unsafe { take_text(&mut state) }
    }

    /// The adapter's lifecycle label.
    unsafe fn adapter_state(adapter: *const SpokeConnectRemoteAdapter) -> String {
        let mut state = SpokeConnectBuffer::empty();
        let mut error = empty_record();
        assert_eq!(
            unsafe { spoke_connect_remote_adapter_state(adapter, &mut state, &mut error) },
            SPOKE_CONNECT_OK,
            "adapter state: {}",
            unsafe { error_fields(&mut error) }.message
        );
        unsafe { take_text(&mut state) }
    }

    /// Waits for the responder to finish its handshake. The transport write
    /// of the responder hello precedes the state transition, so the dialer can
    /// observe `Established` a moment earlier.
    fn await_responder_state(responder: *const SpokeConnectResponder, label: &str) {
        assert!(
            wait_for(
                || unsafe { responder_state(responder) } == label,
                Duration::from_secs(5)
            ),
            "the responder reached {label}"
        );
    }

    /// The responder under test with the handles and logs this battery owns.
    struct Setup {
        responder: *mut SpokeConnectResponder,
        responder_transport: *mut SpokeConnectTransport,
        ports: *mut SpokeConnectPortsHandler,
        adapter: *mut SpokeConnectRemoteAdapter,
        dialer_transport: *mut SpokeConnectTransport,
        peer_log: Arc<TransportLog>,
        dialer_log: Arc<TransportLog>,
    }

    impl Setup {
        /// Closes both sessions and releases every handle this battery owns.
        fn shutdown(self) {
            let mut error = empty_record();
            unsafe { spoke_connect_remote_adapter_close(self.adapter, &mut error) };
            unsafe { spoke_connect_remote_adapter_free(self.adapter) };
            unsafe { crate::remote_adapter::spoke_connect_transport_free(self.dialer_transport) };
            unsafe { spoke_connect_responder_close(self.responder, &mut error) };
            unsafe { spoke_connect_responder_free(self.responder) };
            unsafe { crate::remote_adapter::spoke_connect_transport_free(self.responder_transport) };
            if !self.ports.is_null() {
                unsafe { spoke_connect_ports_handler_free(self.ports) };
            }
        }
    }

    /// Serves the responder side of a fresh host queue pair and dials it back
    /// through the C ABI — the responder is the subject under test.
    fn serve_and_dial(
        responder_manifest: &str,
        dialer_manifest: &str,
        ports: *mut SpokeConnectPortsHandler,
        invoke_timeout_ms: Option<u64>,
    ) -> Setup {
        let dialer = dialer_identity();
        let host = host_identity();
        let (dialer_end, host_end) = endpoint_pair();
        let (responder_transport, peer_log) = host_transport(host_end);
        let mut error = empty_record();

        let responder_allowlist = [slice_of(dialer.peer_id.as_bytes())];
        let responder_keys = [SpokeConnectPeerKey {
            peer_id: slice_of(dialer.peer_id.as_bytes()),
            public_key: slice_of(&dialer.pubkey),
        }];
        let mut responder: *mut SpokeConnectResponder = ptr::null_mut();
        let status = unsafe {
            spoke_connect_responder_new(
                responder_transport,
                slice_of(&host.seed),
                slice_of(responder_manifest.as_bytes()),
                responder_allowlist.as_ptr(),
                responder_allowlist.len(),
                responder_keys.as_ptr(),
                responder_keys.len(),
                ports,
                optional_scalar(invoke_timeout_ms),
                &mut responder,
                &mut error,
            )
        };
        assert_eq!(
            status,
            SPOKE_CONNECT_OK,
            "responder construction: {}",
            unsafe { error_fields(&mut error) }.message
        );
        assert!(!responder.is_null());

        let (dialer_transport, dialer_log) = host_transport(dialer_end);
        let dialer_allowlist = [slice_of(host.peer_id.as_bytes())];
        let mut adapter: *mut SpokeConnectRemoteAdapter = ptr::null_mut();
        let status = unsafe {
            spoke_connect_remote_adapter_new(
                dialer_transport,
                slice_of(&dialer.seed),
                slice_of(dialer_manifest.as_bytes()),
                slice_of(&host.pubkey),
                dialer_allowlist.as_ptr(),
                dialer_allowlist.len(),
                optional_scalar(invoke_timeout_ms),
                &mut adapter,
                &mut error,
            )
        };
        assert_eq!(
            status,
            SPOKE_CONNECT_OK,
            "dial: {}",
            unsafe { error_fields(&mut error) }.message
        );
        assert_eq!(unsafe { adapter_state(adapter) }, "Established");
        await_responder_state(responder, "Established");

        Setup {
            responder,
            responder_transport,
            ports,
            adapter,
            dialer_transport,
            peer_log,
            dialer_log,
        }
    }

    // ── Tests ────────────────────────────────────────────────────────────

    /// The responder C face completes the handshake against a C dialer and
    /// reports the same session the dialer sees; no ports handle is required.
    #[test]
    fn responder_handshake_reaches_established_and_reports_session_identity() {
        let dialer = dialer_identity();
        let host = host_identity();
        let setup = serve_and_dial(
            &host.manifest_json,
            &dialer.manifest_json,
            ptr::null_mut(),
            Some(INVOKE_TIMEOUT_MS),
        );
        let mut error = empty_record();

        assert_eq!(unsafe { responder_state(setup.responder) }, "Established");

        let mut session_id = SpokeConnectOptionalBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_responder_session_id(setup.responder, &mut session_id, &mut error)
            },
            SPOKE_CONNECT_OK
        );
        let session_id = unsafe { take_optional_text(&mut session_id) }.expect("established");
        let mut dialer_session = SpokeConnectOptionalBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_session_id(
                    setup.adapter,
                    &mut dialer_session,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK
        );
        assert_eq!(
            Some(session_id),
            unsafe { take_optional_text(&mut dialer_session) },
            "both ends report the same session id"
        );

        let mut peer_id = SpokeConnectOptionalBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_responder_remote_peer_id(setup.responder, &mut peer_id, &mut error)
            },
            SPOKE_CONNECT_OK
        );
        assert_eq!(
            unsafe { take_optional_text(&mut peer_id) }.as_deref(),
            Some(dialer.peer_id.as_str())
        );

        let mut manifest = SpokeConnectOptionalBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_responder_remote_manifest(setup.responder, &mut manifest, &mut error)
            },
            SPOKE_CONNECT_OK
        );
        let manifest: serde_json::Value = serde_json::from_str(
            &unsafe { take_optional_text(&mut manifest) }.expect("the dialer manifest is cached"),
        )
        .expect("the remote manifest is JSON");
        assert_eq!(manifest["host_id"], serde_json::json!("test-client"));
        assert_eq!(manifest["capabilities"], serde_json::json!(["spoke-baseline"]));

        // The host transport served the handshake and is still borrowed by the
        // responder handle; the dialer's own transport context is likewise
        // still owned by its handle.
        assert!(setup.peer_log.send.load(Ordering::SeqCst) >= 1);
        assert_eq!(setup.peer_log.destroy.load(Ordering::SeqCst), 0);
        assert!(setup.dialer_log.send.load(Ordering::SeqCst) >= 1);
        assert_eq!(setup.dialer_log.destroy.load(Ordering::SeqCst), 0);

        // `close` is distinct from `free`.
        assert_eq!(
            unsafe { spoke_connect_responder_close(setup.responder, &mut error) },
            SPOKE_CONNECT_OK
        );
        assert_eq!(unsafe { responder_state(setup.responder) }, "Closed");
        setup.shutdown();
    }

    /// A table missing a required callback, a NULL table, a NULL handle and an
    /// out-of-range presence flag are boundary failures with no ownership
    /// transfer (`A2` §Calling and representation).
    #[test]
    fn responder_boundary_rejects_incomplete_tables_and_null_handles() {
        let state = Arc::new(HostState::default());
        let context = Arc::into_raw(Arc::clone(&state)) as *mut c_void;
        let mut handler: *mut SpokeConnectPortsHandler = ptr::null_mut();
        let mut error = empty_record();

        assert_eq!(
            unsafe {
                spoke_connect_ports_handler_new(
                    ptr::null(),
                    context,
                    &mut handler,
                    &mut error,
                )
            },
            SPOKE_CONNECT_INVALID_ARGUMENT
        );
        assert!(handler.is_null(), "a rejected creation leaves no handle");
        let fields = unsafe { error_fields(&mut error) };
        assert!(fields.message.contains("table is NULL"), "{}", fields.message);
        assert_eq!(
            state.destroy.load(Ordering::SeqCst),
            0,
            "a rejected creation transfers nothing"
        );

        let incomplete = SpokeConnectPortsHandlerTable {
            extract: None,
            ..host_ports_table()
        };
        let status = unsafe {
            spoke_connect_ports_handler_new(&incomplete, context, &mut handler, &mut error)
        };
        assert_eq!(status, SPOKE_CONNECT_INVALID_ARGUMENT);
        assert!(handler.is_null());
        let fields = unsafe { error_fields(&mut error) };
        assert!(
            fields.message.contains("thirteen callbacks"),
            "{}",
            fields.message
        );

        let tool_state = Arc::new(HostState::default());
        let tool_context = Arc::into_raw(Arc::clone(&tool_state)) as *mut c_void;
        let mut tool: *mut SpokeConnectToolHandler = ptr::null_mut();
        let incomplete = SpokeConnectToolHandlerTable {
            destroy: None,
            ..host_tool_table()
        };
        let status = unsafe {
            spoke_connect_tool_handler_new(&incomplete, tool_context, &mut tool, &mut error)
        };
        assert_eq!(status, SPOKE_CONNECT_INVALID_ARGUMENT);
        assert!(tool.is_null());
        let fields = unsafe { error_fields(&mut error) };
        assert!(fields.message.contains("handle and destroy"), "{}", fields.message);
        assert_eq!(tool_state.destroy.load(Ordering::SeqCst), 0);
        drop(unsafe { Arc::from_raw(tool_context as *const HostState) });

        // A NULL operation handle rejects before anything is touched.
        let mut state_out = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe { spoke_connect_responder_state(ptr::null(), &mut state_out, &mut error) },
            SPOKE_CONNECT_INVALID_ARGUMENT
        );
        let fields = unsafe { error_fields(&mut error) };
        assert!(fields.message.contains("responder is NULL"), "{}", fields.message);

        let dialer = dialer_identity();
        let host = host_identity();
        let peer_keys = [SpokeConnectPeerKey {
            peer_id: slice_of(dialer.peer_id.as_bytes()),
            public_key: slice_of(&dialer.pubkey),
        }];
        let allowlist = [slice_of(dialer.peer_id.as_bytes())];
        let mut responder: *mut SpokeConnectResponder = ptr::null_mut();
        assert_eq!(
            unsafe {
                spoke_connect_responder_new(
                    ptr::null(),
                    slice_of(&host.seed),
                    slice_of(&host.manifest_json.as_bytes()),
                    allowlist.as_ptr(),
                    allowlist.len(),
                    peer_keys.as_ptr(),
                    peer_keys.len(),
                    ptr::null(),
                    optional_scalar(None),
                    &mut responder,
                    &mut error,
                )
            },
            SPOKE_CONNECT_INVALID_ARGUMENT
        );
        assert!(responder.is_null());
        let fields = unsafe { error_fields(&mut error) };
        assert!(fields.message.contains("transport is NULL"), "{}", fields.message);

        // A presence flag outside 0/1 is rejected at the boundary.
        let transport = host_transport(endpoint_pair().1).0;
        assert_eq!(
            unsafe {
                spoke_connect_responder_new(
                    transport,
                    slice_of(&host.seed),
                    slice_of(&host.manifest_json.as_bytes()),
                    allowlist.as_ptr(),
                    allowlist.len(),
                    peer_keys.as_ptr(),
                    peer_keys.len(),
                    ptr::null(),
                    SpokeConnectOptionalU64 {
                        present: 2,
                        value: 0,
                    },
                    &mut responder,
                    &mut error,
                )
            },
            SPOKE_CONNECT_INVALID_ARGUMENT
        );
        let fields = unsafe { error_fields(&mut error) };
        assert!(fields.message.contains("present must be 0 or 1"), "{}", fields.message);

        // A duplicate peer id rejects rather than silently choosing one key.
        let duplicate_keys = [peer_keys[0], peer_keys[0]];
        assert_eq!(
            unsafe {
                spoke_connect_responder_new(
                    transport,
                    slice_of(&host.seed),
                    slice_of(&host.manifest_json.as_bytes()),
                    allowlist.as_ptr(),
                    allowlist.len(),
                    duplicate_keys.as_ptr(),
                    duplicate_keys.len(),
                    ptr::null(),
                    optional_scalar(None),
                    &mut responder,
                    &mut error,
                )
            },
            SPOKE_CONNECT_INVALID_ARGUMENT
        );
        assert!(responder.is_null());
        let fields = unsafe { error_fields(&mut error) };
        assert!(fields.message.contains("duplicate peer key"), "{}", fields.message);

        unsafe { crate::remote_adapter::spoke_connect_transport_free(transport) };
        drop(unsafe { Arc::from_raw(context as *const HostState) });
    }

    /// A ports round trip: the C dialer's call reaches the host callback
    /// through the responder's ports face and the served JSON comes back.
    #[test]
    fn responder_ports_round_trip_serves_the_callback_over_the_c_abi() {
        let dialer = dialer_identity();
        let host = host_identity();
        let (ports, state) = unsafe { new_host_ports("rule-host") };
        let setup = serve_and_dial(
            &host.manifest_json,
            &dialer.manifest_json,
            ports,
            Some(INVOKE_TIMEOUT_MS),
        );
        let mut error = empty_record();

        let releases_before = state.release.load(Ordering::SeqCst);
        let refs = slices_of(&["rule_01HXYZ", "rule_02HLMN"]);
        let mut rules = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_list_rules(
                    setup.adapter,
                    refs.as_ptr(),
                    refs.len(),
                    &mut rules,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK,
            "list_rules: {}",
            unsafe { error_fields(&mut error) }.message
        );
        let rules: serde_json::Value =
            serde_json::from_str(&unsafe { take_text(&mut rules) }).expect("rules JSON");
        assert_eq!(rules, serde_json::json!([rule_value("rule-host")]));
        assert_eq!(
            lock(&state.rule_refs).clone(),
            vec![vec!["rule_01HXYZ".to_owned(), "rule_02HLMN".to_owned()]],
            "the callback received the references the C call carried, as UTF-8 slices"
        );
        assert_eq!(
            state.release.load(Ordering::SeqCst) - releases_before,
            1,
            "the served buffer was released exactly once on the success path"
        );

        // The optional-u64 rule: a matching `put_*` callback receives the
        // revision as a present optional and an absent expectation as
        // `present = 0`. This host declines the op, so the round trip ends in
        // the application reject the callback returned.
        let handler = unsafe { ports_handler_handle(ports) }
            .ok()
            .expect("the ports handle borrows");
        let declined = handler
            .put_knowledge_entry("{}".to_owned(), Some(7))
            .expect_err("this host declines the upsert");
        assert!(matches!(
            declined,
            ffi::FfiError::Rejected { ref code, .. } if code == DECLINE_CODE_STR
        ));
        let declined = handler
            .put_knowledge_entry("{}".to_owned(), None)
            .expect_err("this host declines the upsert");
        assert!(matches!(declined, ffi::FfiError::Rejected { .. }));
        assert_eq!(
            lock(&state.revisions).clone(),
            vec![Some(7), None],
            "the revision crossed as an optional scalar, present and absent"
        );

        setup.shutdown();
    }

    /// Tool registration serves the dialer's invoke, a second registration for
    /// the same id is last-wins, and the replaced context is released once the
    /// responder drops it.
    #[test]
    fn responder_tool_registration_serves_the_invoke_and_replaces_the_handler() {
        let dialer = dialer_identity();
        let host = host_identity();
        let setup = serve_and_dial(
            &with_tool_capability(&host.manifest_json),
            &with_tool_capability(&dialer.manifest_json),
            ptr::null_mut(),
            Some(INVOKE_TIMEOUT_MS),
        );
        let mut error = empty_record();

        let (tool, tool_state) = unsafe { new_host_tool(r#"{"echo":"first"}"#) };
        assert_eq!(
            unsafe {
                spoke_connect_responder_register_tool_handler(
                    setup.responder,
                    slice_of(ECHO_TOOL_ID.as_bytes()),
                    tool,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK,
            "register: {}",
            unsafe { error_fields(&mut error) }.message
        );

        let mut out_json = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_invoke_tool(
                    setup.adapter,
                    slice_of(ECHO_TOOL_ID.as_bytes()),
                    slice_of(br#"{"a":1}"#),
                    &mut out_json,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK,
            "tool invoke: {}",
            unsafe { error_fields(&mut error) }.message
        );
        let result: serde_json::Value =
            serde_json::from_str(&unsafe { take_text(&mut out_json) }).expect("tool result JSON");
        assert_eq!(result, serde_json::json!({ "echo": "first" }));
        assert_eq!(
            lock(&tool_state.tool_arguments).clone(),
            vec![r#"{"a":1}"#.to_owned()],
            "the callback received the arguments object the invoke carried"
        );

        // A non-`tools.` id rejects the D13 grammar with zero side effect.
        assert_eq!(
            unsafe {
                spoke_connect_responder_register_tool_handler(
                    setup.responder,
                    slice_of(b"port.x"),
                    tool,
                    &mut error,
                )
            },
            SPOKE_CONNECT_FFI_REJECTED
        );
        let fields = unsafe { error_fields(&mut error) };
        assert_eq!(fields.code, "INVALID_INPUT");

        // Registration is last-wins: the same id now serves the new handler.
        let (replacement, replacement_state) = unsafe { new_host_tool(r#"{"echo":"second"}"#) };
        assert_eq!(
            unsafe {
                spoke_connect_responder_register_tool_handler(
                    setup.responder,
                    slice_of(ECHO_TOOL_ID.as_bytes()),
                    replacement,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK
        );
        let mut out_json = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_invoke_tool(
                    setup.adapter,
                    slice_of(ECHO_TOOL_ID.as_bytes()),
                    slice_of(br#"{"a":2}"#),
                    &mut out_json,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK
        );
        let result: serde_json::Value =
            serde_json::from_str(&unsafe { take_text(&mut out_json) }).expect("tool result JSON");
        assert_eq!(result, serde_json::json!({ "echo": "second" }));

        // The replaced handler's context is gone once its last reference
        // drops: registration replaced the registry entry, so releasing the
        // caller's handle is the final release.
        unsafe { spoke_connect_tool_handler_free(tool) };
        assert!(
            wait_for(
                || tool_state.destroy.load(Ordering::SeqCst) == 1,
                Duration::from_secs(5)
            ),
            "the replaced handler's context is destroyed once"
        );

        // The registered handle is borrowed: the responder keeps its own
        // reference, so releasing the caller's handle leaves the tool serving.
        unsafe { spoke_connect_tool_handler_free(replacement) };
        assert_eq!(
            replacement_state.destroy.load(Ordering::SeqCst),
            0,
            "the responder still holds the registered reference"
        );
        let mut out_json = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_invoke_tool(
                    setup.adapter,
                    slice_of(ECHO_TOOL_ID.as_bytes()),
                    slice_of(br#"{"a":3}"#),
                    &mut out_json,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK,
            "the borrowed handle is not consumed"
        );
        let result: serde_json::Value =
            serde_json::from_str(&unsafe { take_text(&mut out_json) }).expect("tool result JSON");
        assert_eq!(result, serde_json::json!({ "echo": "second" }));

        setup.shutdown();
        assert!(
            wait_for(
                || replacement_state.destroy.load(Ordering::SeqCst) == 1,
                Duration::from_secs(5)
            ),
            "the register-time context is destroyed once the responder is gone"
        );
        assert_eq!(replacement_state.destroy.load(Ordering::SeqCst), 1);
    }

    /// A callback reject code outside the locked vocabulary downgrades to
    /// `INTERNAL_ERROR` on the wire (`ffi.rs` ports bridge), and the callback's
    /// populated buffers are released exactly once.
    #[test]
    fn responder_unknown_callback_reject_code_downgrades_to_internal_error() {
        let dialer = dialer_identity();
        let host = host_identity();
        let (ports, state) = unsafe { new_host_ports("rule-host") };
        *lock(&state.behavior) = PortsBehavior::UnknownRejectCode;
        let setup = serve_and_dial(
            &host.manifest_json,
            &dialer.manifest_json,
            ports,
            Some(INVOKE_TIMEOUT_MS),
        );
        let mut error = empty_record();

        let refs = slices_of(&["rule_01HXYZ"]);
        let mut out_json = SpokeConnectBuffer::empty();
        let status = unsafe {
            spoke_connect_remote_adapter_list_rules(
                setup.adapter,
                refs.as_ptr(),
                refs.len(),
                &mut out_json,
                &mut error,
            )
        };
        assert_eq!(status, SPOKE_CONNECT_FFI_REJECTED);
        let fields = unsafe { error_fields(&mut error) };
        assert_eq!(
            fields.code, "INTERNAL_ERROR",
            "an unknown reject code downgrades to INTERNAL_ERROR"
        );
        assert!(
            fields.message.contains("does not serve"),
            "the callback message is preserved: {}",
            fields.message
        );
        assert!(
            fields.wire_code.is_empty(),
            "a downgraded reject carries no wire code: {}",
            fields.wire_code
        );
        assert!(
            out_json.data.is_null(),
            "a failed call leaves no result ownership with the caller"
        );
        assert_eq!(
            state.release.load(Ordering::SeqCst),
            2,
            "the reject record's message and code were released exactly once"
        );

        setup.shutdown();
    }

    /// A callback fault (an unsupported status, then an undecodable body) is
    /// contained into `INTERNAL_ERROR` and the serve loop keeps working.
    #[test]
    fn responder_contains_a_callback_fault_and_keeps_serving() {
        let dialer = dialer_identity();
        let host = host_identity();
        let (ports, state) = unsafe { new_host_ports("rule-host") };
        *lock(&state.behavior) = PortsBehavior::UnsupportedStatus;
        let setup = serve_and_dial(
            &host.manifest_json,
            &dialer.manifest_json,
            ports,
            Some(INVOKE_TIMEOUT_MS),
        );
        let mut error = empty_record();
        let refs = slices_of(&["rule_01HXYZ"]);

        let mut out_json = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_list_rules(
                    setup.adapter,
                    refs.as_ptr(),
                    refs.len(),
                    &mut out_json,
                    &mut error,
                )
            },
            SPOKE_CONNECT_FFI_REJECTED
        );
        let fields = unsafe { error_fields(&mut error) };
        assert_eq!(fields.code, "INTERNAL_ERROR");
        assert!(
            fields
                .message
                .contains("unsupported list_rules callback status 300"),
            "the unsupported status was projected as a callback fault: {}",
            fields.message
        );
        assert!(out_json.data.is_null());

        // A result the op's JSON type cannot decode lands on the same
        // containment row.
        *lock(&state.behavior) = PortsBehavior::MalformedJson;
        let mut out_json = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_list_rules(
                    setup.adapter,
                    refs.as_ptr(),
                    refs.len(),
                    &mut out_json,
                    &mut error,
                )
            },
            SPOKE_CONNECT_FFI_REJECTED
        );
        let fields = unsafe { error_fields(&mut error) };
        assert_eq!(fields.code, "INTERNAL_ERROR");
        assert!(
            fields.message.contains("malformed JSON"),
            "the undecodable body is contained: {}",
            fields.message
        );

        // The session survived both faults: a healthy callback still serves.
        *lock(&state.behavior) = PortsBehavior::Serve;
        let mut rules = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_list_rules(
                    setup.adapter,
                    refs.as_ptr(),
                    refs.len(),
                    &mut rules,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK,
            "the serve loop survived the contained faults: {}",
            unsafe { error_fields(&mut error) }.message
        );
        let rules: serde_json::Value =
            serde_json::from_str(&unsafe { take_text(&mut rules) }).expect("rules JSON");
        assert_eq!(rules, serde_json::json!([rule_value("rule-host")]));

        setup.shutdown();
    }

    /// The reverse invoke (responder → dialer) is served by the dialer's own
    /// tool handler. The dialer is driven through the facade because the
    /// adapter C face carries the invoke side of D15/D16, not dialer-side
    /// tool serving.
    #[test]
    fn responder_reverse_invoke_returns_the_dialers_tool_result() {
        struct DialerTool {
            payload: &'static str,
        }

        impl ffi::ToolHandler for DialerTool {
            fn handle(&self, _arguments_json: String) -> Result<String, ffi::FfiError> {
                Ok(self.payload.to_owned())
            }
        }

        let dialer = dialer_identity();
        let host = host_identity();
        let (dialer_end, host_end) = endpoint_pair();
        let (responder_transport, peer_log) = host_transport(host_end);
        let mut error = empty_record();

        let allowlist = [slice_of(dialer.peer_id.as_bytes())];
        let responder_keys = [SpokeConnectPeerKey {
            peer_id: slice_of(dialer.peer_id.as_bytes()),
            public_key: slice_of(&dialer.pubkey),
        }];
        let mut responder: *mut SpokeConnectResponder = ptr::null_mut();
        assert_eq!(
            unsafe {
                spoke_connect_responder_new(
                    responder_transport,
                    slice_of(&host.seed),
                    slice_of(with_tool_capability(&host.manifest_json).as_bytes()),
                    allowlist.as_ptr(),
                    allowlist.len(),
                    responder_keys.as_ptr(),
                    responder_keys.len(),
                    ptr::null(),
                    optional_scalar(Some(INVOKE_TIMEOUT_MS)),
                    &mut responder,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK,
            "responder construction: {}",
            unsafe { error_fields(&mut error) }.message
        );

        let adapter = ffi::connect_remote_adapter_ffi(
            Box::new(FacadeTransport {
                endpoint: dialer_end,
            }),
            dialer.seed.to_vec(),
            with_tool_capability(&dialer.manifest_json),
            host.pubkey.to_vec(),
            vec![host.peer_id.clone()],
            Some(INVOKE_TIMEOUT_MS),
        )
        .expect("the facade dialer establishes");
        assert_eq!(adapter.state(), "Established");
        adapter.register_tool_handler(
            ECHO_TOOL_ID.to_owned(),
            Box::new(DialerTool {
                payload: r#"{"served":"dialer"}"#,
            }),
        )
        .expect("the dialer registers the tool");
        await_responder_state(responder, "Established");

        let mut out_json = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_responder_invoke_tool(
                    responder,
                    slice_of(ECHO_TOOL_ID.as_bytes()),
                    slice_of(br#"{"a":1}"#),
                    &mut out_json,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK,
            "reverse invoke: {}",
            unsafe { error_fields(&mut error) }.message
        );
        let result: serde_json::Value =
            serde_json::from_str(&unsafe { take_text(&mut out_json) }).expect("tool result JSON");
        assert_eq!(result, serde_json::json!({ "served": "dialer" }));

        // A top-level invoke failure crosses with its status and kind intact.
        let mut out_json = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_responder_invoke_tool(
                    responder,
                    slice_of(b"{}"),
                    slice_of(b"{}"),
                    &mut out_json,
                    &mut error,
                )
            },
            SPOKE_CONNECT_FFI_REJECTED
        );
        let fields = unsafe { error_fields(&mut error) };
        assert_eq!(fields.code, "INVALID_INPUT");
        assert!(out_json.data.is_null());

        assert!(peer_log.send.load(Ordering::SeqCst) >= 1);
        assert_eq!(
            unsafe { spoke_connect_responder_close(responder, &mut error) },
            SPOKE_CONNECT_OK
        );
        unsafe { spoke_connect_responder_free(responder) };
        adapter.close();
        unsafe { crate::remote_adapter::spoke_connect_transport_free(responder_transport) };
    }

    /// Every callback context is destroyed exactly once, and only after the
    /// responder that composed over it drops its reference — including a
    /// NULL free (a no-op).
    #[test]
    fn responder_teardown_destroys_the_callback_contexts_exactly_once() {
        let dialer = dialer_identity();
        let host = host_identity();
        let (ports, ports_state) = unsafe { new_host_ports("rule-host") };
        let mut setup = serve_and_dial(
            &with_tool_capability(&host.manifest_json),
            &with_tool_capability(&dialer.manifest_json),
            ports,
            Some(INVOKE_TIMEOUT_MS),
        );
        let mut error = empty_record();
        let (tool, tool_state) = unsafe { new_host_tool(r#"{"echo":"teardown"}"#) };
        assert_eq!(
            unsafe {
                spoke_connect_responder_register_tool_handler(
                    setup.responder,
                    slice_of(ECHO_TOOL_ID.as_bytes()),
                    tool,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK
        );

        // Both callbacks ran, so both contexts are demonstrably live.
        let refs = slices_of(&["rule_01HXYZ"]);
        let mut rules = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_list_rules(
                    setup.adapter,
                    refs.as_ptr(),
                    refs.len(),
                    &mut rules,
                    &mut error,
                )
            },
            SPOKE_CONNECT_OK
        );
        unsafe { spoke_connect_buffer_free(&mut rules) };
        assert_eq!(ports_state.destroy.load(Ordering::SeqCst), 0);

        // A NULL free is a no-op.
        unsafe { spoke_connect_ports_handler_free(ptr::null_mut()) };
        unsafe { spoke_connect_tool_handler_free(ptr::null_mut()) };
        assert_eq!(ports_state.destroy.load(Ordering::SeqCst), 0);
        assert_eq!(tool_state.destroy.load(Ordering::SeqCst), 0);

        // Releasing the caller's handles leaves the responder's references in
        // place: nothing is destroyed yet.
        unsafe { spoke_connect_ports_handler_free(setup.ports) };
        unsafe { spoke_connect_tool_handler_free(tool) };
        setup.ports = ptr::null_mut();
        assert_eq!(
            ports_state.destroy.load(Ordering::SeqCst),
            0,
            "the responder still owns the composed ports reference"
        );
        assert_eq!(
            tool_state.destroy.load(Ordering::SeqCst),
            0,
            "the responder still owns the registered tool reference"
        );

        setup.shutdown();
        assert!(
            wait_for(
                || ports_state.destroy.load(Ordering::SeqCst) == 1
                    && tool_state.destroy.load(Ordering::SeqCst) == 1,
                Duration::from_secs(5)
            ),
            "each callback context is destroyed once the responder is gone"
        );
        assert_eq!(ports_state.destroy.load(Ordering::SeqCst), 1);
        assert_eq!(tool_state.destroy.load(Ordering::SeqCst), 1);
    }

    /// A wait that already timed out does not release the callback context:
    /// the parked host callback keeps the composed reference alive until it
    /// returns.
    #[test]
    fn timed_out_wait_keeps_the_callback_context_alive_until_it_returns() {
        let dialer = dialer_identity();
        let host = host_identity();
        let (ports, state) = unsafe { new_host_ports("rule-host") };
        let hold = Arc::new(Hold::default());
        *lock(&state.hold) = Some(Arc::clone(&hold));
        let mut setup = serve_and_dial(
            &host.manifest_json,
            &dialer.manifest_json,
            ports,
            Some(SHORT_TIMEOUT_MS),
        );
        let mut error = empty_record();

        // The invoke fails on the dialer's per-waiter deadline while the host
        // callback is still parked on the blocking pool.
        let refs = slices_of(&["rule_01HXYZ"]);
        let mut out_json = SpokeConnectBuffer::empty();
        assert_eq!(
            unsafe {
                spoke_connect_remote_adapter_list_rules(
                    setup.adapter,
                    refs.as_ptr(),
                    refs.len(),
                    &mut out_json,
                    &mut error,
                )
            },
            SPOKE_CONNECT_FFI_REJECTED
        );
        let fields = unsafe { error_fields(&mut error) };
        assert_eq!(fields.code, "INTERNAL_ERROR");
        assert_eq!(fields.kind, "timeout", "the per-waiter deadline fired");
        assert!(out_json.data.is_null());
        assert!(
            state.entered.load(Ordering::SeqCst) >= 1,
            "the callback had entered and was still parked"
        );

        // The timed-out wait does not release the context: the in-flight
        // callback and the responder both still hold it.
        unsafe { spoke_connect_ports_handler_free(setup.ports) };
        setup.ports = ptr::null_mut();
        assert_eq!(
            state.destroy.load(Ordering::SeqCst),
            0,
            "an in-flight callback and the responder keep the context alive"
        );

        // Once the callback returns and the responder is gone, the context is
        // destroyed exactly once.
        hold.release();
        setup.shutdown();
        assert!(
            wait_for(
                || state.destroy.load(Ordering::SeqCst) == 1,
                Duration::from_secs(5)
            ),
            "the context is destroyed once after the parked callback returns"
        );
        assert_eq!(state.destroy.load(Ordering::SeqCst), 1);
        assert_eq!(lock(&state.rule_refs).len(), 1);
    }
}
