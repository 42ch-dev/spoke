// Minimal test responder — the dialed (responder) side of a loopback pair
// for the RemoteAdapter tool-serving tests (mirror of TS
// `tests/remote/minimal-responder.ts`).
//
// Performs the signed-hello handshake over `transport` in the background
// (the client's dial is the synchronization point), then serves forward
// invokes (through registered tool handlers or the dispatch-deny branch)
// and correlates responses to its own reverse invokes (`issue_invoke`).
//
// The serving pipeline mirrors the frozen §4 order (classify → stray →
// peek → verify → advance → gate → handler → signed response) and the
// loopback host's envelope-auth re-derivation (the core helpers are
// `pub(crate)` and unreachable from integration tests).
//
// This double is deliberately independent of the production responder
// (`crates/spoke-connect/src/remote/responder.rs`): the adapter tests drive
// the adapter's serving path against it, and the responder tests drive the
// production responder against the real `connect_remote_adapter`.

use crate::loopback_oracle::{
    sign_envelope, verify_invoke_request_envelope, HostEnvelopeAuthError,
};
use spoke_connect::core::{
    derive_peer_id_from_ed25519_pubkey, dispatch_allowed, is_allowlisted, sign_hello_ed25519,
    verify_hello_ed25519, InboundSequence, NonceStore, OutboundSequence,
};
use spoke_connect::remote::{ToolHandler, Transport};
use spoke_operations::{
    from_error_envelope, spoke_ok, spoke_reject, to_error_envelope, SpokeReject, SpokeRejectCode,
    SpokeResult,
};
use spoke_schemas::connect::connect_hello::HostCapabilityManifest as ConnectHostCapabilityManifest;
use spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequest;
use spoke_schemas::connect::ConnectHello;
use spoke_schemas::HostCapabilityManifest;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

/// Default bounded-wait deadline for each reverse-invoke waiter, ms (parity
/// with TS `DEFAULT_INVOKE_TIMEOUT_MS`).
const DEFAULT_INVOKE_TIMEOUT_MS: u64 = 2000;

/// Dispatch-deny wire codes (D7): deny → `CAPABILITY_PORT_MISSING` +
/// `wire_code` (mirrors `remote_adapter.rs::map_error_envelope`).
fn map_error_envelope(error: &spoke_schemas::connect::connect_invoke_response::ErrorEnvelope) -> SpokeReject {
    let mut details = error.details.clone();
    if matches!(error.code.as_str(), "op_unsupported" | "capability_missing") {
        details.insert("wire_code".into(), Value::String(error.code.clone()));
        return SpokeReject {
            code: SpokeRejectCode::CapabilityPortMissing,
            message: error.message.clone(),
            details: Some(details),
        };
    }
    if error.code == "auth_failed" {
        return SpokeReject {
            code: SpokeRejectCode::InternalError,
            message: error.message.clone(),
            details: if error.details.is_empty() {
                None
            } else {
                Some(error.details.clone())
            },
        };
    }
    let Ok(data_error) = serde_json::from_value::<spoke_schemas::ErrorEnvelope>(
        serde_json::to_value(error).expect("wire error envelope serializes"),
    ) else {
        return SpokeReject {
            code: SpokeRejectCode::InvalidInput,
            message: "unmappable error envelope".to_owned(),
            details: None,
        };
    };
    from_error_envelope(&data_error)
}

#[derive(Debug, Default)]
pub struct MinimalResponderStats {
    pub hellos_verified: AtomicUsize,
    pub handlers_run: AtomicUsize,
    pub reverse_invokes_issued: AtomicUsize,
    pub responses_verified: AtomicUsize,
    pub sequence_rejections: AtomicUsize,
    pub auth_rejections: AtomicUsize,
    pub dispatch_denials: AtomicUsize,
}

struct MinimalSession {
    session_id: String,
    negotiated_capabilities: Vec<String>,
    inbound: Mutex<InboundSequence>,
    outbound: Mutex<OutboundSequence>,
}

struct PendingReverse {
    tx: tokio::sync::mpsc::Sender<SpokeResult<Value>>,
    timeout_task: tokio::task::JoinHandle<()>,
}

/// `INTERNAL_ERROR` reject with `details.kind` (mirror of the adapter's
/// `internal_error`).
fn internal_error<T>(kind: &str, message: impl Into<String>) -> SpokeResult<T> {
    spoke_reject(
        SpokeRejectCode::InternalError,
        message,
        Some(serde_json::Map::from_iter([(
            "kind".into(),
            Value::String(kind.into()),
        )])),
    )
}

/// Test-only response override hook (malformed-response fixtures).
type ResponseOverride = Option<Box<dyn Fn(&ConnectInvokeRequest) -> Option<Value> + Send + Sync>>;

struct MinimalResponderInner {
    transport: Arc<dyn Transport>,
    seed: [u8; 32],
    client_pubkey: [u8; 32],
    allowlist: Vec<String>,
    manifest: HostCapabilityManifest,
    invoke_timeout: Duration,
    /// Test-only: when a request maps to `Some(envelope)`, that envelope is
    /// sent verbatim instead of the responder's real response
    /// (malformed-response fixtures, e.g. a success payload without a
    /// `result` key).
    response_override: ResponseOverride,
    nonce_store: Mutex<NonceStore>,
    session: Mutex<Option<MinimalSession>>,
    tool_handlers: Mutex<HashMap<String, ToolHandler>>,
    pending: Arc<Mutex<HashMap<String, PendingReverse>>>,
    closed: AtomicBool,
}

pub struct MinimalResponderOptions {
    pub transport: Arc<dyn Transport>,
    pub seed: [u8; 32],
    pub client_pubkey: [u8; 32],
    pub allowlist: Vec<String>,
    pub manifest: HostCapabilityManifest,
    pub invoke_timeout_ms: Option<u64>,
    pub response_override: ResponseOverride,
}

pub struct MinimalResponder {
    inner: Arc<MinimalResponderInner>,
    pub stats: Arc<MinimalResponderStats>,
}

impl MinimalResponder {
    /// Register a tool handler served for forward invokes.
    pub fn register_tool_handler(&self, capability_id: &str, handler: ToolHandler) {
        self.inner
            .tool_handlers
            .lock()
            .expect("tool handlers lock")
            .insert(capability_id.to_owned(), handler);
    }

    /// Issue a reverse invoke toward the dialer. `sequence` is `None` for
    /// the outbound counter, or a verbatim wire sequence for tamper /
    /// sequence-gap fixtures (the counter is NOT advanced in that case).
    pub async fn issue_invoke(
        &self,
        op: &str,
        args: Value,
        sequence: Option<i64>,
    ) -> SpokeResult<Value> {
        let (session_id, seq) = {
            let session_guard = self.inner.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                return internal_error(
                    "session_closed",
                    "connect session is not established",
                );
            };
            let seq = match sequence {
                Some(seq) => seq,
                None => {
                    let mut outbound = session.outbound.lock().expect("outbound lock");
                    match outbound.allocate() {
                        Ok(seq) => seq as i64,
                        Err(_) => {
                            return internal_error(
                                "sequence_exhausted",
                                "outbound sequence space exhausted",
                            );
                        }
                    }
                }
            };
            (session.session_id.clone(), seq)
        };
        self.stats
            .reverse_invokes_issued
            .fetch_add(1, Ordering::SeqCst);
        let request_id = format!("minimal-responder-{}", uuid_request_id());
        // The exact signed object for `spoke-connect-invoke-request-jcs-v1`
        // (envelope-auth contract §2/§3): `{session_id, sequence,
        // request_id, op, payload}`.
        let signed_object = json!({
            "session_id": session_id,
            "sequence": seq,
            "request_id": request_id,
            "op": op,
            "payload": { "arguments": args },
        });
        let signature = sign_envelope(&self.inner.seed, &signed_object);
        let mut wire = signed_object.as_object().expect("object").clone();
        wire.insert("extensions".into(), json!({}));
        wire.insert("signature".into(), json!(signature));
        let envelope = serde_json::to_vec(&Value::Object(wire)).expect("envelope serializes");

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let timeout_task = {
            let pending = Arc::clone(&self.inner.pending);
            let tx_timeout = tx.clone();
            let timeout = self.inner.invoke_timeout;
            let request_id = request_id.clone();
            let op = op.to_owned();
            tokio::spawn(async move {
                tokio::time::sleep(timeout).await;
                let fired = pending
                    .lock()
                    .expect("pending lock")
                    .remove(&request_id)
                    .is_some();
                if fired {
                    let _ = tx_timeout.try_send(internal_error(
                        "timeout",
                        format!(
                            "reverse invoke {op} ({request_id}) timed out after {}ms",
                            timeout.as_millis()
                        ),
                    ));
                }
            })
        };
        self.inner.pending.lock().expect("pending lock").insert(
            request_id.clone(),
            PendingReverse {
                tx: tx.clone(),
                timeout_task,
            },
        );
        if let Err(error) = self.inner.transport.send(&envelope).await {
            if let Some(entry) = self.inner.pending.lock().expect("pending lock").remove(&request_id) {
                entry.timeout_task.abort();
                let _ = entry.tx.try_send(internal_error(
                    "transport",
                    format!("reverse invoke send failed: {error}"),
                ));
            }
        }
        match rx.recv().await {
            Some(result) => result,
            None => internal_error("transport", "reverse invoke channel closed unexpectedly"),
        }
    }

    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        let transport = Arc::clone(&self.inner.transport);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = transport.close().await;
            });
        }
    }
}

/// UUID v4 request id (mirror of the adapter's `generate_request_id`).
fn uuid_request_id() -> String {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).expect("CSPRNG works in tests");
    uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .to_string()
}

/// Start a minimal responder: performs the signed-hello handshake over
/// `transport` in the background (the client's dial is the synchronization
/// point), then serves forward invokes and correlates responses to its own
/// reverse invokes.
pub async fn start_minimal_responder(
    options: MinimalResponderOptions,
) -> Arc<MinimalResponder> {
    let client_peer_id = derive_peer_id_from_ed25519_pubkey(&options.client_pubkey);
    let responder_peer_id = derive_peer_id_from_ed25519_pubkey(
        &ed25519_dalek::SigningKey::from_bytes(&options.seed)
            .verifying_key()
            .to_bytes(),
    );
    let session_id = "test-session-reverse-0001".to_owned();
    let inner = Arc::new(MinimalResponderInner {
        transport: options.transport,
        seed: options.seed,
        client_pubkey: options.client_pubkey,
        allowlist: options.allowlist,
        manifest: options.manifest.clone(),
        invoke_timeout: Duration::from_millis(
            options.invoke_timeout_ms.unwrap_or(DEFAULT_INVOKE_TIMEOUT_MS),
        ),
        response_override: options.response_override,
        nonce_store: Mutex::new(NonceStore::new()),
        session: Mutex::new(None),
        tool_handlers: Mutex::new(HashMap::new()),
        pending: Arc::new(Mutex::new(HashMap::new())),
        closed: AtomicBool::new(false),
    });
    let stats = Arc::new(MinimalResponderStats::default());
    let responder = Arc::new(MinimalResponder {
        inner: Arc::clone(&inner),
        stats: Arc::clone(&stats),
    });

    let runner_inner = Arc::clone(&inner);
    let runner_stats = Arc::clone(&stats);
    tokio::spawn(async move {
        match handshake(&runner_inner, &runner_stats, &session_id, &responder_peer_id, &client_peer_id).await {
            Ok(()) => {}
            Err(_) => {
                let _ = runner_inner.transport.close().await;
            }
        }
        serve(&runner_inner, &runner_stats).await;
    });

    responder
}

/// Serve loop (request-first classification; response demux for reverse
/// invokes; stray envelopes ignored).
async fn serve(inner: &Arc<MinimalResponderInner>, stats: &Arc<MinimalResponderStats>) {
    while !inner.closed.load(Ordering::SeqCst) {
        let bytes = match inner.transport.recv().await {
            Ok(bytes) => bytes,
            Err(_) => return, // transport closed — stop serving
        };
        let doc: Value = match serde_json::from_slice(&bytes) {
            Ok(doc) => doc,
            Err(_) => continue, // unparseable frame — test double ignores
        };
        if doc.get("op").is_some() {
            serve_invoke(inner, stats, &doc).await;
            continue;
        }
        if doc.get("payload").is_some() || doc.get("error").is_some() {
            demux_response(inner, stats, &doc).await;
            continue;
        }
        // Stray envelope (hello / session / unknown shape) — ignored.
    }
}

/// Signed-hello handshake (allowlist → hello verify → nonce → dial-bound
/// responder hello → signed session snapshot).
async fn handshake(
    inner: &MinimalResponderInner,
    stats: &MinimalResponderStats,
    session_id: &str,
    responder_peer_id: &str,
    client_peer_id: &str,
) -> Result<(), String> {
    let bytes = inner.transport.recv().await.map_err(|error| error.to_string())?;
    let hello: ConnectHello = serde_json::from_slice(&bytes)
        .map_err(|_| "expected ConnectHello from client".to_string())?;
    if !is_allowlisted(&inner.allowlist, hello.peer_id.as_str()) {
        return Err(format!("peer {} not on allowlist", hello.peer_id.as_str()));
    }
    verify_hello_ed25519(&inner.client_pubkey, client_peer_id, &hello, None)
        .map_err(|error| format!("client hello verification failed: {error}"))?;
    if !inner
        .nonce_store
        .lock()
        .expect("nonce lock")
        .check_and_record(hello.peer_id.as_str(), hello.nonce.as_str())
    {
        return Err("nonce replay".to_string());
    }
    stats.hellos_verified.fetch_add(1, Ordering::SeqCst);

    let remote_manifest_wire: ConnectHostCapabilityManifest = hello.host;
    let remote_manifest: HostCapabilityManifest =
        serde_json::from_value(serde_json::to_value(&remote_manifest_wire).expect("serializes"))
            .map_err(|error| error.to_string())?;
    let negotiated: Vec<String> = inner
        .manifest
        .capabilities
        .iter()
        .filter(|cap| remote_manifest.capabilities.iter().any(|remote| remote == *cap))
        .cloned()
        .collect();
    *inner.session.lock().expect("session lock") = Some(MinimalSession {
        session_id: session_id.to_owned(),
        negotiated_capabilities: negotiated.clone(),
        inbound: Mutex::new(InboundSequence::new()),
        outbound: Mutex::new(OutboundSequence::new()),
    });

    // Dial-bound responder hello, then the signed session snapshot
    // (empty-intersection fallback preserved: wire snapshot requires ≥1
    // capability).
    let nonce = uuid_nonce();
    let hello = sign_hello_ed25519(
        &inner.seed,
        &nonce,
        &serde_json::from_value(
            serde_json::to_value(&inner.manifest).expect("manifest serializes"),
        )
        .expect("field-identical manifest converts"),
        Some(hello.nonce.as_str()),
    )
    .map_err(|error| format!("responder hello sign failed: {error}"))?;
    let bytes = serde_json::to_vec(&hello).map_err(|error| error.to_string())?;
    inner.transport.send(&bytes).await.map_err(|error| error.to_string())?;

    let snapshot = json!({
        "session_id": session_id,
        "initiator_peer_id": client_peer_id,
        "responder_peer_id": responder_peer_id,
        "opened_at": "2026-01-01T00:00:00Z",
        "negotiated_capabilities": if negotiated.is_empty() {
            json!(["spoke-baseline"])
        } else {
            json!(negotiated)
        },
        "initial_sequence": 0,
    });
    let signature = sign_envelope(&inner.seed, &snapshot);
    let mut wire = snapshot.as_object().expect("object").clone();
    wire.insert("extensions".into(), json!({}));
    wire.insert("signature".into(), json!(signature));
    let bytes = serde_json::to_vec(&Value::Object(wire)).map_err(|error| error.to_string())?;
    inner.transport.send(&bytes).await.map_err(|error| error.to_string())?;
    Ok(())
}

/// 22-char nonce (above the 16-char wire floor; mirror of the loopback
/// host's `host_nonce`).
fn uuid_nonce() -> String {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).expect("CSPRNG works in tests");
    let uuid = uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .to_string();
    uuid.replace('-', "")
}

/// Gate phase — sequence peek (non-mutating) → envelope-auth verify →
/// advance (auth-before-advance). `None` for stray requests, a rejection
/// spec for gate failures, or `Some(())` when dispatch may run.
async fn run_gate(
    inner: &MinimalResponderInner,
    stats: &MinimalResponderStats,
    doc: &Value,
) -> Option<Result<(), (String, String, Option<Value>)>> {
    let session_id = {
        let session_guard = inner.session.lock().expect("session lock");
        let Some(session) = session_guard.as_ref() else {
            return None; // stray — no established session
        };
        session.session_id.clone()
    };
    let sequence = doc.get("sequence").and_then(Value::as_i64)?;
    let peek_ok = {
        let session_guard = inner.session.lock().expect("session lock");
        let session = session_guard.as_ref()?;
        let inbound = session.inbound.lock().expect("inbound lock");
        inbound.peek(sequence).is_ok()
    };
    if !peek_ok {
        stats.sequence_rejections.fetch_add(1, Ordering::SeqCst);
        return Some(Err((
            "invalid_sequence".to_owned(),
            format!("inbound sequence {sequence} is not the next expected"),
            None,
        )));
    }
    if let Err(error) = verify_invoke_request_envelope(doc, &session_id, &inner.client_pubkey) {
        stats.auth_rejections.fetch_add(1, Ordering::SeqCst);
        return Some(Err((
            "auth_failed".to_owned(),
            error.message,
            Some(json!({ "kind": error.kind })),
        )));
    }
    {
        let session_guard = inner.session.lock().expect("session lock");
        let session = session_guard.as_ref()?;
        let advance_result = session.inbound.lock().expect("inbound lock").advance(sequence);
        if advance_result.is_err() {
            return Some(Err((
                "invalid_sequence".to_owned(),
                format!("inbound sequence {sequence} is not the next expected"),
                None,
            )));
        }
    }
    Some(Ok(()))
}

/// Dispatch phase — runs after the serialized gate; may interleave.
async fn handle_invoke(
    inner: &MinimalResponderInner,
    stats: &MinimalResponderStats,
    doc: &Value,
) {
    let Some(op) = doc.get("op").and_then(Value::as_str) else {
        return;
    };
    let (session_id, negotiated) = {
        let session_guard = inner.session.lock().expect("session lock");
        let Some(session) = session_guard.as_ref() else {
            return;
        };
        (session.session_id.clone(), session.negotiated_capabilities.clone())
    };
    let sequence = doc.get("sequence").and_then(Value::as_i64).unwrap_or_default();
    let request_id = doc.get("request_id").and_then(Value::as_str).unwrap_or_default();

    // Dispatch gate — `dispatch_allowed`-level logic (frozen §3): `tools.*`
    // ops require the op string itself, evaluated against
    // `negotiated_capabilities`.
    if !dispatch_allowed(op, &negotiated) {
        stats.dispatch_denials.fetch_add(1, Ordering::SeqCst);
        send_error_envelope(inner, doc, "op_unsupported", &format!("op {op} is not authorized by this session"), None).await;
        return;
    }
    // Test-only response override (malformed-response fixtures).
    if let Some(override_fn) = inner.response_override.as_ref() {
        let request: ConnectInvokeRequest = match serde_json::from_value(doc.clone()) {
            Ok(request) => request,
            Err(_) => return,
        };
        if let Some(envelope) = override_fn(&request) {
            // Re-sign the override envelope with the responder's seed.
            let mut signed = envelope
                .as_object()
                .expect("override envelope is an object")
                .clone();
            signed.remove("extensions");
            signed.remove("signature");
            let signature = sign_envelope(&inner.seed, &Value::Object(signed));
            let mut wire = envelope
                .as_object()
                .expect("override envelope is an object")
                .clone();
            wire.insert("signature".into(), json!(signature));
            let bytes = serde_json::to_vec(&Value::Object(wire)).expect("serializes");
            let _ = inner.transport.send(&bytes).await;
            return;
        }
    }
    // Handler or deny (fail-closed serving, frozen §4).
    let handler = inner.tool_handlers.lock().expect("tool handlers lock").get(op).cloned();
    let Some(handler) = handler else {
        stats.dispatch_denials.fetch_add(1, Ordering::SeqCst);
        send_error_envelope(inner, doc, "op_unsupported", &format!("no handler registered for {op}"), None).await;
        return;
    };
    let arguments = doc
        .get("payload")
        .and_then(|payload| payload.get("arguments"))
        .filter(|arguments| arguments.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let result = match futures::future::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(
        handler(arguments),
    ))
    .await
    {
        Ok(result) => result,
        Err(_) => spoke_reject(SpokeRejectCode::InternalError, "tool handler panicked", None),
    };
    stats.handlers_run.fetch_add(1, Ordering::SeqCst);
    if let SpokeResult::Ok(value) = result {
        send_ok_response(inner, &session_id, sequence, request_id, json!({ "result": value })).await;
    } else if let SpokeResult::Reject(reject) = result {
        send_reject_response(inner, &session_id, sequence, request_id, &reject).await;
    }
}

/// Serve one inbound invoke: gate (serialized) then dispatch (concurrent).
async fn serve_invoke(
    inner: &Arc<MinimalResponderInner>,
    stats: &Arc<MinimalResponderStats>,
    doc: &Value,
) {
    match run_gate(inner, stats, doc).await {
        None => return, // stray — ignored
        Some(Err((code, message, details))) => {
            send_error_envelope(inner, doc, &code, &message, details.as_ref()).await;
            return;
        }
        Some(Ok(())) => {}
    }
    let doc = doc.clone();
    let inner = Arc::clone(inner);
    let stats = Arc::clone(stats);
    tokio::spawn(async move {
        handle_invoke(&inner, &stats, &doc).await;
    });
}

async fn send_error_envelope(
    inner: &MinimalResponderInner,
    doc: &Value,
    code: &str,
    message: &str,
    details: Option<&Value>,
) {
    let Some(session_id) = doc.get("session_id").and_then(Value::as_str) else {
        return;
    };
    let Some(sequence) = doc.get("sequence").and_then(Value::as_i64) else {
        return;
    };
    let Some(request_id) = doc.get("request_id").and_then(Value::as_str) else {
        return;
    };
    let mut error = serde_json::Map::new();
    error.insert("code".into(), Value::String(code.to_owned()));
    error.insert("message".into(), Value::String(message.to_owned()));
    if let Some(details) = details {
        error.insert("details".into(), details.clone());
    }
    error.insert("extensions".into(), json!({}));
    let signed_object = json!({
        "session_id": session_id,
        "sequence": sequence,
        "request_id": request_id,
        "error": Value::Object(error),
    });
    let signature = sign_envelope(&inner.seed, &signed_object);
    let mut wire = signed_object.as_object().expect("object").clone();
    wire.insert("extensions".into(), json!({}));
    wire.insert("signature".into(), json!(signature));
    let bytes = serde_json::to_vec(&Value::Object(wire)).expect("serializes");
    let _ = inner.transport.send(&bytes).await;
}

async fn send_ok_response(
    inner: &MinimalResponderInner,
    session_id: &str,
    sequence: i64,
    request_id: &str,
    payload: Value,
) {
    let signed_object = json!({
        "session_id": session_id,
        "sequence": sequence,
        "request_id": request_id,
        "payload": payload,
    });
    let signature = sign_envelope(&inner.seed, &signed_object);
    let mut wire = signed_object.as_object().expect("object").clone();
    wire.insert("extensions".into(), json!({}));
    wire.insert("signature".into(), json!(signature));
    let bytes = serde_json::to_vec(&Value::Object(wire)).expect("serializes");
    let _ = inner.transport.send(&bytes).await;
}

async fn send_reject_response(
    inner: &MinimalResponderInner,
    session_id: &str,
    sequence: i64,
    request_id: &str,
    reject: &SpokeReject,
) {
    let error = to_error_envelope(reject);
    send_error_envelope_value(inner, session_id, sequence, request_id, &error).await;
}

async fn send_error_envelope_value(
    inner: &MinimalResponderInner,
    session_id: &str,
    sequence: i64,
    request_id: &str,
    error: &spoke_schemas::ErrorEnvelope,
) {
    let signed_object = json!({
        "session_id": session_id,
        "sequence": sequence,
        "request_id": request_id,
        "error": error,
    });
    let signature = sign_envelope(&inner.seed, &signed_object);
    let mut wire = signed_object.as_object().expect("object").clone();
    wire.insert("extensions".into(), json!({}));
    wire.insert("signature".into(), json!(signature));
    let bytes = serde_json::to_vec(&Value::Object(wire)).expect("serializes");
    let _ = inner.transport.send(&bytes).await;
}

/// Demux a response envelope to its pending reverse-invoke waiter:
/// correlation echo check first, then envelope-auth verify against the
/// dialer's hello public key.
async fn demux_response(
    inner: &MinimalResponderInner,
    stats: &MinimalResponderStats,
    doc: &Value,
) {
    let Some(request_id) = doc.get("request_id").and_then(Value::as_str) else {
        return;
    };
    let entry = inner.pending.lock().expect("pending lock").remove(request_id);
    let Some(entry) = entry else {
        return; // unknown/duplicate response — dropped
    };
    entry.timeout_task.abort();
    let session_id = inner
        .session
        .lock()
        .expect("session lock")
        .as_ref()
        .map(|session| session.session_id.clone());
    let Some(session_id) = session_id else {
        return;
    };
    let mapped: SpokeResult<Value> = match verify_invoke_response_envelope(
        doc,
        &session_id,
        &inner.client_pubkey,
    ) {
        Ok(()) => {
            stats.responses_verified.fetch_add(1, Ordering::SeqCst);
            if let Some(error) = doc.get("error") {
                let error: spoke_schemas::connect::connect_invoke_response::ErrorEnvelope =
                    match serde_json::from_value(error.clone()) {
                        Ok(error) => error,
                        Err(error) => {
                            let _ = entry.tx.try_send(internal_error(
                                "transport",
                                format!("response error decode failed: {error}"),
                            ));
                            return;
                        }
                    };
                SpokeResult::Reject(map_error_envelope(&error))
            } else if doc
                .get("payload")
                .and_then(|payload| payload.get("result"))
                .is_some()
            {
                spoke_ok(
                    doc.get("payload")
                        .and_then(|payload| payload.get("result"))
                        .expect("checked")
                        .clone(),
                )
            } else {
                internal_error(
                    "transport",
                    "response payload does not carry a result key",
                )
            }
        }
        Err(error) => internal_error(
            error.kind.unwrap_or("envelope_auth_invalid"),
            error.message,
        ),
    };
    let _ = entry.tx.try_send(mapped);
}

/// Verify an inbound invoke RESPONSE envelope (correlation echo + signature)
/// against the dialer's hello public key. Test-double re-derivation (the
/// core helper is `pub(crate)`).
fn verify_invoke_response_envelope(
    wire: &Value,
    session_id: &str,
    client_pubkey: &[u8; 32],
) -> Result<(), HostEnvelopeAuthError> {
    let signature = wire.get("signature").and_then(Value::as_str).ok_or_else(|| {
        HostEnvelopeAuthError::invalid("response signature missing")
    })?;
    let wire_session = wire.get("session_id").and_then(Value::as_str).ok_or_else(|| {
        HostEnvelopeAuthError::invalid("response session_id missing")
    })?;
    if wire_session != session_id {
        return Err(HostEnvelopeAuthError::session_unbound(
            "response session_id does not match the bound session",
        ));
    }
    // The signed object mirrors the wire branch exactly: common echo fields
    // + the branch key.
    let mut signed = serde_json::Map::new();
    for key in ["session_id", "sequence", "request_id"] {
        signed.insert(key.to_owned(), wire.get(key).cloned().unwrap_or(Value::Null));
    }
    if let Some(payload) = wire.get("payload") {
        signed.insert("payload".into(), payload.clone());
    } else if let Some(error) = wire.get("error") {
        signed.insert("error".into(), error.clone());
    } else {
        return Err(HostEnvelopeAuthError::invalid(
            "response carries neither payload nor error branch",
        ));
    }
    // Decode the base64url signature.
    use base64::Engine;
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| HostEnvelopeAuthError::invalid("signature is not valid base64url"))?;
    if raw.len() != 64 {
        return Err(HostEnvelopeAuthError::invalid(
            "signature is not exactly 64 bytes",
        ));
    }
    let public_key = ed25519_dalek::VerifyingKey::from_bytes(client_pubkey)
        .map_err(|_| HostEnvelopeAuthError::invalid("invalid Ed25519 public key"))?;
    let signature = ed25519_dalek::Signature::from_slice(&raw)
        .map_err(|_| HostEnvelopeAuthError::invalid("invalid Ed25519 signature"))?;
    let canonical = serde_jcs::to_vec(&Value::Object(signed))
        .map_err(|_| HostEnvelopeAuthError::invalid("JCS canonicalization failed"))?;
    use ed25519_dalek::Verifier;
    public_key
        .verify(&canonical, &signature)
        .map_err(|_| HostEnvelopeAuthError::invalid("signature does not verify"))
}


