//! `RemoteAdapter` — drop-in async `BaselinePorts` over a connect session
//! (frozen contract: `.mstar/iterations/v0-iter030/guides/remote-adapter-contract.md`).
//!
//! PUBLIC surface: the async `BaselinePorts` (six families) + the
//! [`connect_remote_adapter`] dial entrypoint + read-only session info
//! ([`RemoteAdapter::state`], `session_id`, `remote_peer_id`,
//! `remote_manifest`) + [`RemoteAdapter::close`].
//!
//! INTERNAL (encapsulated — consumers never touch these): hello sign/verify,
//! allowlist, nonce single-use, sequence allocate/advance, `request_id`
//! correlation, the dispatch gate, optional capability-token attach, the
//! receive-loop demux, and invoke timeout timers. All of it reuses the pure
//! session-core (`crate::core`) — nothing is reimplemented here.
//!
//! Each `BaselinePorts` method maps to a reserved `port.*` product op with an
//! opaque JSON payload (frozen contract §5.2), sent as a `ConnectInvokeRequest`
//! over the [`Transport`], awaited via the adapter-owned receive loop, and
//! deserialized back to `SpokeResult`. The WebSocket implementation is
//! consumer-side; only the loopback [`Transport`] ships in-repo.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};
use spoke_operations::{
    from_error_envelope, spoke_ok, spoke_reject, FindingPort, HostManifestPort, KnowledgeEntryPort,
    RelationPort, RuleQueryPort, ScopeQueryPort, SpokeReject, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::connect::connect_hello::HostCapabilityManifest as ConnectHostCapabilityManifest;
use spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequest;
use spoke_schemas::connect::connect_invoke_response::{
    ConnectInvokeResponse, ErrorEnvelope as WireErrorEnvelope,
};
use spoke_schemas::connect::ConnectHello;
use spoke_schemas::connect::ConnectSession;
use spoke_schemas::{
    Finding, HostCapabilityManifest, KnowledgeEntry, Relation, Rule, Scope, TimelineEvent,
};

use crate::core::{
    check_response_correlation, derive_peer_id_from_ed25519_pubkey, is_allowlisted,
    sign_hello_ed25519, verify_hello_ed25519, CapabilityTokenProof, Correlation, OutboundSequence,
};
use crate::hello::generate_nonce;
use crate::remote::transport::{Transport, TransportError};
use crate::runtime::generate_request_id;

/// Default bounded-wait deadline for the handshake and each invoke, ms
/// (parity with TS `DEFAULT_INVOKE_TIMEOUT_MS`).
const DEFAULT_INVOKE_TIMEOUT_MS: u64 = 5000;

/// Session-lifecycle labels (frozen contract §4) — parity with TS
/// `RemoteAdapterState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteAdapterState {
    Disconnected,
    Handshaking,
    Established,
    Closed,
}

impl RemoteAdapterState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disconnected => "Disconnected",
            Self::Handshaking => "Handshaking",
            Self::Established => "Established",
            Self::Closed => "Closed",
        }
    }
}

impl std::fmt::Display for RemoteAdapterState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Raw Ed25519 keypair material for the local connect peer (mirror of TS
/// `RemoteIdentity`). The 32-byte seed is type-enforced in Rust (TS checks
/// the length at runtime).
#[derive(Debug, Clone)]
pub struct RemoteIdentity {
    pub seed: [u8; 32],
}

/// Dial options for [`connect_remote_adapter`] (mirror of TS
/// `RemoteAdapterOptions`; frozen contract §3.3).
#[derive(Clone)]
pub struct RemoteAdapterOptions {
    /// Message-oriented transport (consumer-provided; loopback ships in-repo).
    pub transport: Arc<dyn Transport>,
    /// This adapter's raw Ed25519 seed.
    pub local_identity: RemoteIdentity,
    /// Host manifest advertised in this adapter's signed hello.
    pub local_manifest: HostCapabilityManifest,
    /// Preconfigured remote Ed25519 public key. How the key is obtained is
    /// transport-adapter-owned; the remote's peer_id is derived from it and
    /// must be on `allowlist` (fail-closed).
    pub remote_pubkey: [u8; 32],
    /// Trusted remote peer ids — must contain the remote's derived peer_id
    /// (fail-closed).
    pub allowlist: Vec<String>,
    /// Bounded-wait deadline for the handshake and each invoke, ms
    /// (default [`DEFAULT_INVOKE_TIMEOUT_MS`]).
    pub invoke_timeout_ms: Option<u64>,
    /// Optional capability-token proof attached as `auth` on outbound invokes
    /// (§3.2/§3.3).
    pub capability_token: Option<CapabilityTokenProof>,
}

/// Dial / handshake failures — thrown before an adapter is returned (frozen
/// contract §8.2 last row; no half-open `BaselinePorts` instance).
#[derive(Debug, thiserror::Error)]
pub enum RemoteAdapterError {
    #[error("{0}")]
    Config(String),
    #[error("handshake failed: {0}")]
    Handshake(String),
    #[error("{0}")]
    Timeout(String),
}

/// Internal invoke-failure classes (frozen contract §8.2). Consumers only
/// ever observe these mapped to `SpokeResult` `INTERNAL_ERROR` rejects with
/// `details.kind` — except [`connect_remote_adapter`], which errors for
/// dial/hello failures (§8.2 last row). Parity with TS `RemoteErrorKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteErrorKind {
    Transport,
    SessionClosed,
    Timeout,
    CorrelationMismatch,
    SequenceExhausted,
}

impl RemoteErrorKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Transport => "transport",
            Self::SessionClosed => "session_closed",
            Self::Timeout => "timeout",
            Self::CorrelationMismatch => "correlation_mismatch",
            Self::SequenceExhausted => "sequence_exhausted",
        }
    }
}

/// Internal invoke failure (mirror of TS `RemoteError`).
#[derive(Debug, Clone)]
struct RemoteError {
    kind: RemoteErrorKind,
    message: String,
}

impl RemoteError {
    fn new(kind: RemoteErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

/// `SpokeResult` reject with `INTERNAL_ERROR` + `details.kind` (contract
/// §8.2 transport/session/timeout/correlation rows).
fn internal_error<T>(kind: &str, message: impl Into<String>) -> SpokeResult<T> {
    let mut details = Map::new();
    details.insert("kind".into(), Value::String(kind.into()));
    spoke_reject(SpokeRejectCode::InternalError, message, Some(details))
}

/// Dispatch-deny wire codes (contract §8.2): the host answered that the op or
/// its required capability is not available → `CAPABILITY_PORT_MISSING`.
fn is_dispatch_deny(code: &str) -> bool {
    matches!(code, "op_unsupported" | "capability_missing")
}

/// Map an error-branch envelope to a `SpokeResult` reject (contract §8.2).
///
/// The error branch carries the codegen-inline wire `ErrorEnvelope`, which is
/// field-identical to `spoke_schemas::ErrorEnvelope`; the shared
/// `from_error_envelope` mapping runs after a lossless value conversion.
fn map_error_envelope(error: &WireErrorEnvelope) -> SpokeReject {
    let mut details = error.details.clone();
    if is_dispatch_deny(&error.code) {
        details.insert("wire_code".into(), Value::String(error.code.clone()));
        return SpokeReject {
            code: SpokeRejectCode::CapabilityPortMissing,
            message: error.message.clone(),
            details: Some(details),
        };
    }
    let Ok(data_error) = serde_json::from_value::<spoke_schemas::ErrorEnvelope>(
        serde_json::to_value(error).expect("wire error envelope serializes"),
    ) else {
        return SpokeReject {
            code: SpokeRejectCode::InvalidInput,
            message: "unmappable error envelope".into(),
            details: Some(details),
        };
    };
    from_error_envelope(&data_error)
}

/// Negotiated capabilities = intersection of both hosts' manifests
/// (normative rule, `spoke-connect.md` §Negotiation), deterministic in local
/// manifest order — same rule as `crate::node`.
fn negotiated_capabilities(
    local: &HostCapabilityManifest,
    remote: &HostCapabilityManifest,
) -> Vec<String> {
    local
        .capabilities
        .iter()
        .filter(|cap| {
            remote
                .capabilities
                .iter()
                .any(|remote_cap| remote_cap == *cap)
        })
        .cloned()
        .collect()
}

/// Convert the ops/data `HostCapabilityManifest` to the field-identical
/// hello wire type (codegen-inline shapes; lossless).
fn connect_manifest(manifest: &HostCapabilityManifest) -> ConnectHostCapabilityManifest {
    serde_json::from_value(serde_json::to_value(manifest).expect("manifest serializes"))
        .expect("field-identical manifest converts")
}

/// Established-session state, per peer (frozen contract §4: `peer_id`,
/// `session_id`, outbound sequence, negotiated capabilities — adapter-local,
/// never a process-global singleton).
struct EstablishedSession {
    session_id: String,
    /// The verified remote peer id (the peer the session is bound to).
    responder_peer_id: String,
    /// The negotiated capability intersection — part of the frozen per-peer
    /// state for the multi-peer registry (contract §4); the outbound dispatch
    /// gate itself is host-side, so nothing reads this in the single-peer
    /// client yet.
    #[allow(dead_code)]
    negotiated_capabilities: Vec<String>,
    sequence: Mutex<OutboundSequence>,
}

/// A parked invoke: correlation material + the waiter channel + the timeout
/// task (mirror of TS `PendingInvoke`; the timeout task plays `setTimeout`).
struct PendingInvoke {
    correlation: Correlation,
    tx: tokio::sync::mpsc::Sender<Result<ConnectInvokeResponse, RemoteError>>,
    timeout_task: tokio::task::JoinHandle<()>,
}

/// Single-peer async `BaselinePorts` proxy over an established connect
/// session. Construct via [`connect_remote_adapter`] — the adapter is only
/// reachable in `Established` (or `Closed` after `close`); port calls fail
/// closed on any other state.
pub struct RemoteAdapter {
    transport: Arc<dyn Transport>,
    invoke_timeout: Duration,
    capability_token: Option<CapabilityTokenProof>,

    state: Mutex<RemoteAdapterState>,
    session: Mutex<Option<EstablishedSession>>,
    remote_manifest: Mutex<Option<HostCapabilityManifest>>,
    pending: Arc<Mutex<HashMap<String, PendingInvoke>>>,
    receive_loop_running: AtomicBool,
}

impl RemoteAdapter {
    fn new(
        transport: Arc<dyn Transport>,
        invoke_timeout: Duration,
        capability_token: Option<CapabilityTokenProof>,
    ) -> Self {
        Self {
            transport,
            invoke_timeout,
            capability_token,
            state: Mutex::new(RemoteAdapterState::Disconnected),
            session: Mutex::new(None),
            remote_manifest: Mutex::new(None),
            pending: Arc::new(Mutex::new(HashMap::new())),
            receive_loop_running: AtomicBool::new(false),
        }
    }

    /// Read-only session state (contract §4 labels).
    #[must_use]
    pub fn state(&self) -> RemoteAdapterState {
        *self.state.lock().expect("state lock")
    }

    /// The remote-assigned session id (`None` before establishment). TS
    /// returns `""`; Rust uses `Option` as the idiomatic equivalent.
    #[must_use]
    pub fn session_id(&self) -> Option<String> {
        self.session
            .lock()
            .expect("session lock")
            .as_ref()
            .map(|session| session.session_id.clone())
    }

    /// The verified remote peer id (`None` before establishment).
    #[must_use]
    pub fn remote_peer_id(&self) -> Option<String> {
        self.session
            .lock()
            .expect("session lock")
            .as_ref()
            .map(|session| session.responder_peer_id.clone())
    }

    /// The remote peer's `HostCapabilityManifest`, from the authenticated
    /// hello `host` (contract §7). `None` before establishment.
    #[must_use]
    pub fn remote_manifest(&self) -> Option<HostCapabilityManifest> {
        self.remote_manifest.lock().expect("manifest lock").clone()
    }

    /// Release the session and transport. Idempotent; pending invokes fail
    /// with `INTERNAL_ERROR` `details.kind = "session_closed"` (contract §6
    /// "Transport close mid-flight" / §8.2).
    pub fn close(&self) {
        self.close_session("local shutdown");
    }

    // ── Session lifecycle (internal; called by connect_remote_adapter) ────

    /// Dial-only: the adapter is in `Handshaking` while dialing.
    fn begin_handshake(&self) {
        *self.state.lock().expect("state lock") = RemoteAdapterState::Handshaking;
    }

    /// Dial-only: bind the authenticated session and start the receive loop.
    fn establish(
        self: &Arc<Self>,
        session: EstablishedSession,
        remote_manifest: HostCapabilityManifest,
    ) {
        *self.session.lock().expect("session lock") = Some(session);
        *self.remote_manifest.lock().expect("manifest lock") = Some(remote_manifest);
        *self.state.lock().expect("state lock") = RemoteAdapterState::Established;
        self.spawn_receive_loop();
    }

    /// All failure paths that make the session unusable: mark `Closed`, fail
    /// every pending waiter with `session_closed`, and release the transport
    /// (fire-and-forget, mirroring TS `closeSession`).
    fn close_session(&self, reason: &str) {
        {
            let mut state = self.state.lock().expect("state lock");
            if *state == RemoteAdapterState::Closed {
                return;
            }
            *state = RemoteAdapterState::Closed;
        }

        let entries: Vec<PendingInvoke> = self
            .pending
            .lock()
            .expect("pending lock")
            .drain()
            .map(|(_request_id, entry)| entry)
            .collect();
        for entry in entries {
            entry.timeout_task.abort();
            let _ = entry.tx.try_send(Err(RemoteError::new(
                RemoteErrorKind::SessionClosed,
                format!("connect session closed: {reason}"),
            )));
        }

        let transport = Arc::clone(&self.transport);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = transport.close().await;
            });
        }
    }

    // ── Receive loop (adapter-owned; port methods never call recv) ────────

    fn spawn_receive_loop(self: &Arc<Self>) {
        if self.receive_loop_running.swap(true, Ordering::SeqCst) {
            return;
        }
        let adapter = Arc::clone(self);
        tokio::spawn(async move {
            adapter.receive_loop().await;
            adapter.receive_loop_running.store(false, Ordering::SeqCst);
        });
    }

    async fn receive_loop(&self) {
        while *self.state.lock().expect("state lock") == RemoteAdapterState::Established {
            let bytes = match self.transport.recv().await {
                Ok(bytes) => bytes,
                Err(error) => {
                    // Transport loss: fail every pending waiter and transition
                    // to `Closed` (contract §6 / §8.2). Decode failure counts
                    // as transport loss, same as TS `decodeJsonMessage` in the
                    // recv try-block.
                    self.close_session(&format!("transport loss: {error}"));
                    return;
                }
            };
            let doc: Value = match serde_json::from_slice(&bytes) {
                Ok(doc) => doc,
                Err(error) => {
                    self.close_session(&format!("transport loss: {error}"));
                    return;
                }
            };
            let Ok(response) = serde_json::from_value::<ConnectInvokeResponse>(doc) else {
                // Post-handshake stray envelope (hello / session / unknown
                // shape): ignored. Unexpected invoke requests are host-role —
                // out of the single-peer client scope (contract §4).
                continue;
            };

            // Demux by request_id; unknown/duplicate responses are dropped
            // (protocol v1 defines no retry).
            let correlation = Correlation::from(&response);
            let request_id = correlation.request_id.clone();
            let entry = self
                .pending
                .lock()
                .expect("pending lock")
                .remove(&request_id);
            let Some(entry) = entry else {
                continue;
            };
            entry.timeout_task.abort();
            let outcome = check_response_correlation(&entry.correlation, &correlation)
                .map(|()| response)
                .map_err(|_| {
                    RemoteError::new(
                        RemoteErrorKind::CorrelationMismatch,
                        "response echo fields do not match the request",
                    )
                });
            let _ = entry.tx.try_send(outcome);
        }
    }

    // ── Invoke path ───────────────────────────────────────────────────────

    /// Build the outbound `ConnectInvokeRequest`: atomic outbound sequence
    /// allocation, fresh `request_id`, optional capability-token `auth`.
    /// Session unusable → `Err(RemoteError)`; sequence exhaustion closes the
    /// session (contract §6 "Sequence overflow").
    fn build_request(&self, op: &str, payload: Value) -> Result<ConnectInvokeRequest, RemoteError> {
        let state = *self.state.lock().expect("state lock");
        if state != RemoteAdapterState::Established {
            return Err(RemoteError::new(
                RemoteErrorKind::SessionClosed,
                format!("connect session is not established (state {state})"),
            ));
        }
        let session_guard = self.session.lock().expect("session lock");
        let Some(session) = session_guard.as_ref() else {
            return Err(RemoteError::new(
                RemoteErrorKind::SessionClosed,
                format!("connect session is not established (state {state})"),
            ));
        };
        let allocate_result = {
            let mut sequence_guard = session.sequence.lock().expect("sequence lock");
            sequence_guard.allocate()
        };
        let sequence = match allocate_result {
            Ok(sequence) => sequence,
            Err(_) => {
                drop(session_guard);
                self.close_session("outbound sequence exhausted");
                return Err(RemoteError::new(
                    RemoteErrorKind::SequenceExhausted,
                    "outbound sequence space exhausted — reopen session",
                ));
            }
        };
        let request_id = generate_request_id()
            .map_err(|error| RemoteError::new(RemoteErrorKind::Transport, error.to_string()))?;
        let auth = self
            .capability_token
            .as_ref()
            .map(|token| serde_json::to_value(token).expect("capability token proof serializes"));
        Ok(ConnectInvokeRequest {
            auth,
            extensions: HashMap::new(),
            op: op.parse().map_err(
                |error: spoke_schemas::connect::connect_invoke_request::error::ConversionError| {
                    RemoteError::new(
                        RemoteErrorKind::Transport,
                        format!("invalid op {op:?}: {error}"),
                    )
                },
            )?,
            payload,
            request_id: request_id.parse().map_err(|error| {
                RemoteError::new(
                    RemoteErrorKind::Transport,
                    format!("invalid request_id: {error}"),
                )
            })?,
            sequence: sequence as i64,
            session_id: session.session_id.parse().map_err(|error| {
                RemoteError::new(
                    RemoteErrorKind::Transport,
                    format!("invalid session_id: {error}"),
                )
            })?,
        })
    }

    /// Send one op invoke and resolve with its correlated response envelope.
    /// Errors with [`RemoteError`] on timeout / transport failure / session
    /// close / correlation mismatch / sequence exhaustion.
    async fn invoke_op(
        &self,
        op: &str,
        payload: Value,
    ) -> Result<ConnectInvokeResponse, RemoteError> {
        let request = self.build_request(op, payload)?;
        let request_id = request.request_id.to_string();

        // Register the pending waiter + its timeout task (TS `setTimeout`):
        // on elapse, remove the entry and fail only this waiter — the session
        // stays open (contract §6 "Invoke timeout").
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let timeout_task = {
            let pending = Arc::clone(&self.pending);
            let tx_timeout = tx.clone();
            let timeout = self.invoke_timeout;
            let request_id = request_id.clone();
            let op_owned = op.to_owned();
            tokio::spawn(async move {
                tokio::time::sleep(timeout).await;
                let fired = pending
                    .lock()
                    .expect("pending lock")
                    .remove(&request_id)
                    .is_some();
                if fired {
                    let _ = tx_timeout.try_send(Err(RemoteError::new(
                        RemoteErrorKind::Timeout,
                        format!(
                            "invoke {op_owned} ({request_id}) timed out after {}ms",
                            timeout.as_millis()
                        ),
                    )));
                }
            })
        };
        self.pending.lock().expect("pending lock").insert(
            request_id.clone(),
            PendingInvoke {
                correlation: Correlation::from(&request),
                tx: tx.clone(),
                timeout_task,
            },
        );

        // Encode + send. A synchronous encode failure or an async send
        // failure settles this invoke now — no dead entry waits out the
        // timeout (TS mirrors both paths).
        let envelope = match serde_json::to_vec(&request) {
            Ok(envelope) => envelope,
            Err(error) => {
                self.fail_pending_send(&request_id, &format!("invoke encode failed: {error}"));
                return Err(RemoteError::new(
                    RemoteErrorKind::Transport,
                    format!("invoke encode failed: {error}"),
                ));
            }
        };
        if let Err(error) = self.transport.send(&envelope).await {
            self.fail_pending_send(&request_id, &format!("invoke send failed: {error}"));
            return Err(RemoteError::new(
                RemoteErrorKind::Transport,
                format!("invoke send failed: {error}"),
            ));
        }

        match rx.recv().await {
            Some(result) => result,
            None => Err(RemoteError::new(
                RemoteErrorKind::Transport,
                "invoke channel closed unexpectedly",
            )),
        }
    }

    /// Cleanup for a send/encode failure: remove the pending entry, abort its
    /// timeout task, and settle the waiter with a `transport` error.
    fn fail_pending_send(&self, request_id: &str, message: &str) {
        if let Some(entry) = self
            .pending
            .lock()
            .expect("pending lock")
            .remove(request_id)
        {
            entry.timeout_task.abort();
            let _ = entry.tx.try_send(Err(RemoteError::new(
                RemoteErrorKind::Transport,
                message.to_owned(),
            )));
        }
    }

    /// Invoke a port op and map the response to `SpokeResult` (contract
    /// §5.3/§8). Port calls always settle to `SpokeResult` (§8.3).
    async fn invoke_mapped<T: DeserializeOwned>(&self, op: &str, payload: Value) -> SpokeResult<T> {
        match self.invoke_op(op, payload).await {
            Ok(ConnectInvokeResponse::Variant0 { payload, .. }) => {
                match serde_json::from_value(payload) {
                    Ok(value) => spoke_ok(value),
                    Err(error) => internal_error(
                        "transport",
                        format!("response payload decode failed: {error}"),
                    ),
                }
            }
            Ok(ConnectInvokeResponse::Variant1 { error, .. }) => {
                SpokeResult::Reject(map_error_envelope(&error))
            }
            Err(error) => internal_error(error.kind.as_str(), error.message),
        }
    }
}

// ── BaselinePorts (async) — port-method → `port.*` catalogue (§5.2) ───────

#[async_trait]
impl KnowledgeEntryPort for RemoteAdapter {
    async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
        self.invoke_mapped("port.knowledge.get", json!({ "entry_id": entry_id }))
            .await
    }

    async fn put_knowledge_entry(
        &self,
        entry: KnowledgeEntry,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry> {
        self.invoke_mapped(
            "port.knowledge.put",
            json!({
                "entry": entry,
                "expected_base_revision": expected_base_revision,
            }),
        )
        .await
    }
}

#[async_trait]
impl RelationPort for RemoteAdapter {
    async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
        self.invoke_mapped("port.relation.get", json!({ "relation_id": relation_id }))
            .await
    }

    async fn put_relation(
        &self,
        relation: Relation,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<Relation> {
        self.invoke_mapped(
            "port.relation.put",
            json!({
                "relation": relation,
                "expected_base_revision": expected_base_revision,
            }),
        )
        .await
    }
}

#[async_trait]
impl ScopeQueryPort for RemoteAdapter {
    async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
        self.invoke_mapped(
            "port.scope.list_knowledge_entries",
            json!({ "scope": scope }),
        )
        .await
    }

    async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
        self.invoke_mapped("port.scope.list_timeline_events", json!({ "scope": scope }))
            .await
    }
}

#[async_trait]
impl FindingPort for RemoteAdapter {
    async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
        self.invoke_mapped("port.finding.put", json!({ "findings": findings }))
            .await
    }
}

#[async_trait]
impl RuleQueryPort for RemoteAdapter {
    async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
        self.invoke_mapped("port.rule.list", json!({ "rule_refs": rule_refs }))
            .await
    }
}

#[async_trait]
impl HostManifestPort for RemoteAdapter {
    /// HostManifestPort: "self" on a RemoteAdapter is the **remote** peer
    /// (contract §7). Returns the authenticated hello `host` from the session
    /// cache — cache-only, no network round-trip.
    async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest> {
        match self.remote_manifest.lock().expect("manifest lock").clone() {
            Some(manifest) => spoke_ok(manifest),
            None => internal_error("session_closed", "connect session is not established"),
        }
    }

    /// Proxy to the remote's product-seeded peer list (contract §7).
    async fn list_peer_host_capability_manifests(
        &self,
    ) -> SpokeResult<Vec<HostCapabilityManifest>> {
        self.invoke_mapped("port.host.list_peer_manifests", json!({}))
            .await
    }
}

// The blanket impl in spoke-operations lifts the six traits above into
// `BaselinePorts`, so `RemoteAdapter` is a drop-in for
// `orchestrate_upsert(remote, req)` etc.

/// Dial a remote peer over `transport`: perform the signed hello exchange +
/// session snapshot, then return an `Established` adapter. Errors on any
/// handshake / allowlist / verification failure (contract §3.3/§8.2 — no
/// half-open `BaselinePorts` instance).
pub async fn connect_remote_adapter(
    options: RemoteAdapterOptions,
) -> Result<Arc<RemoteAdapter>, RemoteAdapterError> {
    let RemoteAdapterOptions {
        transport,
        local_identity,
        local_manifest,
        remote_pubkey,
        allowlist,
        invoke_timeout_ms,
        capability_token,
    } = options;
    let invoke_timeout =
        Duration::from_millis(invoke_timeout_ms.unwrap_or(DEFAULT_INVOKE_TIMEOUT_MS));

    // Fail-closed before any transport I/O: the remote peer must be on the
    // allowlist (TS dial does the same checks first).
    let remote_peer_id = derive_peer_id_from_ed25519_pubkey(&remote_pubkey);
    if !is_allowlisted(&allowlist, &remote_peer_id) {
        return Err(RemoteAdapterError::Config(format!(
            "remote peer {remote_peer_id} is not on the allowlist (fail-closed)"
        )));
    }
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&local_identity.seed);
    let local_pubkey = signing_key.verifying_key().to_bytes();
    let local_peer_id = derive_peer_id_from_ed25519_pubkey(&local_pubkey);

    let adapter = Arc::new(RemoteAdapter::new(
        transport,
        invoke_timeout,
        capability_token,
    ));
    adapter.begin_handshake();

    let outcome = async {
        // 1. Send our signed hello (nonce generated internally — single-use).
        let nonce =
            generate_nonce().map_err(|error| RemoteAdapterError::Config(error.to_string()))?;
        let hello = sign_hello_ed25519(
            &local_identity.seed,
            &nonce,
            &connect_manifest(&local_manifest),
        )
        .map_err(|error| {
            RemoteAdapterError::Handshake(format!("local hello sign failed: {error}"))
        })?;
        let bytes = serde_json::to_vec(&hello).map_err(|error| {
            RemoteAdapterError::Handshake(format!("hello encode failed: {error}"))
        })?;
        bounded(adapter.transport.send(&bytes), invoke_timeout, "hello send")
            .await?
            .map_err(|error| {
                RemoteAdapterError::Handshake(format!("hello send failed: {error}"))
            })?;

        // 2. Await + verify the server's signed hello (signature + identity).
        let recv: Result<Vec<u8>, TransportError> =
            bounded(adapter.transport.recv(), invoke_timeout, "server hello").await?;
        let bytes = recv.map_err(|error| {
            RemoteAdapterError::Handshake(format!("server hello recv failed: {error}"))
        })?;
        let hello: ConnectHello = serde_json::from_slice(&bytes).map_err(|_| {
            RemoteAdapterError::Handshake("expected ConnectHello from server".into())
        })?;
        verify_hello_ed25519(&remote_pubkey, &remote_peer_id, &hello).map_err(|error| {
            RemoteAdapterError::Handshake(format!("remote hello verification failed: {error}"))
        })?;
        let remote_manifest: HostCapabilityManifest = {
            let wire: ConnectHostCapabilityManifest = hello.host;
            serde_json::from_value(serde_json::to_value(&wire).expect("hello host serializes"))
                .map_err(|error| {
                    RemoteAdapterError::Handshake(format!(
                        "remote hello host decode failed: {error}"
                    ))
                })?
        };

        // 3. Await + validate the responder-assigned session snapshot.
        let recv: Result<Vec<u8>, TransportError> =
            bounded(adapter.transport.recv(), invoke_timeout, "session snapshot").await?;
        let bytes = recv.map_err(|error| {
            RemoteAdapterError::Handshake(format!("session snapshot recv failed: {error}"))
        })?;
        let session_doc: ConnectSession = serde_json::from_slice(&bytes).map_err(|_| {
            RemoteAdapterError::Handshake(
                "expected ConnectSession snapshot after server hello".into(),
            )
        })?;
        if session_doc.initiator_peer_id.as_str() != local_peer_id
            || session_doc.responder_peer_id.as_str() != remote_peer_id
        {
            return Err(RemoteAdapterError::Handshake(
                "session snapshot peer ids do not match the authenticated hellos".into(),
            ));
        }
        if session_doc.session_id.as_str().is_empty() {
            return Err(RemoteAdapterError::Handshake(
                "session snapshot session_id must not be empty".into(),
            ));
        }
        if session_doc.initial_sequence != 0 {
            return Err(RemoteAdapterError::Handshake(
                "session snapshot initial_sequence must be 0 for protocol_version 1".into(),
            ));
        }

        // 4. Bind the authenticated session and start the receive loop.
        let negotiated = negotiated_capabilities(&local_manifest, &remote_manifest);
        adapter.establish(
            EstablishedSession {
                session_id: session_doc.session_id.to_string(),
                responder_peer_id: remote_peer_id,
                negotiated_capabilities: negotiated,
                sequence: Mutex::new(OutboundSequence::new()),
            },
            remote_manifest,
        );
        Ok(())
    }
    .await;

    match outcome {
        Ok(()) => Ok(adapter),
        Err(error) => {
            // Handshake rejection: release the transport so the peer sees a
            // clean disconnect (mirrors the TS dial rejection path).
            adapter.close();
            Err(error)
        }
    }
}

/// Bound `future` against the handshake/invoke deadline; elapse maps to a
/// [`RemoteAdapterError::Timeout`] with the TS dial message shape
/// (`connect: {what} timed out after {ms}ms`).
async fn bounded<F>(
    future: F,
    timeout: Duration,
    what: &str,
) -> Result<F::Output, RemoteAdapterError>
where
    F: std::future::Future,
{
    tokio::time::timeout(timeout, future).await.map_err(|_| {
        RemoteAdapterError::Timeout(format!(
            "connect: {what} timed out after {}ms",
            timeout.as_millis()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::MAX_SEQUENCE;
    use crate::remote::transport::loopback_transport_pair;

    /// Outbound sequence exhaustion: the next `allocate` fails, the session
    /// is closed (no wrap-around), and the port call settles to
    /// `INTERNAL_ERROR` `details.kind = "sequence_exhausted"` (contract
    /// §8.2 / §6 "Sequence overflow").
    ///
    /// This sits in the adapter module (not `tests/remote_loopback.rs`)
    /// because the private counter is only reachable through the test-only
    /// `OutboundSequence::set_next` (`#[cfg(test)] pub(crate)`), which
    /// integration tests cannot see; the TS twin (`setNext`) covers the same
    /// path in the loopback suite.
    #[tokio::test]
    async fn sequence_exhaustion_maps_to_internal_error_and_closes_session() {
        let pair = loopback_transport_pair();
        let adapter = Arc::new(RemoteAdapter::new(
            Arc::new(pair.client),
            Duration::from_millis(DEFAULT_INVOKE_TIMEOUT_MS),
            None,
        ));
        adapter.begin_handshake();
        let mut sequence = OutboundSequence::new();
        sequence.set_next(MAX_SEQUENCE + 1);
        let manifest: HostCapabilityManifest = serde_json::from_value(json!({
            "schema_version": 1,
            "host_id": "test-host",
            "roles": ["data-store"],
            "capabilities": ["spoke-baseline"],
            "namespaces": ["toy_world"],
            "extensions": {},
        }))
        .expect("valid manifest");
        adapter.establish(
            EstablishedSession {
                session_id: "test-session-exhausted".into(),
                responder_peer_id: "test-remote-peer".into(),
                sequence: Mutex::new(sequence),
            },
            manifest,
        );

        let result = adapter.get_knowledge_entry("kb_tw_mira").await;
        match result {
            SpokeResult::Ok(_) => panic!("exhausted sequence must reject"),
            SpokeResult::Reject(reject) => {
                assert_eq!(reject.code, SpokeRejectCode::InternalError);
                assert_eq!(
                    reject
                        .details
                        .as_ref()
                        .and_then(|details| details.get("kind"))
                        .and_then(|kind| kind.as_str()),
                    Some("sequence_exhausted")
                );
            }
        }
        // Exhaustion makes the session unusable (no wrap-around) — Closed.
        assert_eq!(adapter.state(), RemoteAdapterState::Closed);
    }
}
