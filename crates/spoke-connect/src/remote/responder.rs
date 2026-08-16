//! `connect_responder` — productized connect responder (frozen contract
//! `tool-contracts.md` §6, ported from the demo-server recipe
//! `examples/connect-demo/server/src/host/connect-host.ts`).
//!
//! PUBLIC surface: the [`connect_responder`] factory + the responder
//! object's [`ConnectResponder::register_tool_handler`],
//! [`ConnectResponder::invoke_tool`] (the reverse face), read-only session
//! info ([`ConnectResponder::state`], `session_id`, `remote_peer_id`,
//! `remote_manifest`), and [`ConnectResponder::close`]. Consumers never
//! touch envelope-auth / sequence internals.
//!
//! The responder owns the server end of a message-oriented [`Transport`]:
//!
//! - **handshake**: allowlist fail-closed check FIRST → hello verify
//!   (`peer_id` binding via the preconfigured `peer_keys` pubkey) → nonce
//!   replay record → dial-bound responder hello (5-field signed object
//!   carrying the initiator's nonce as `peer_nonce`) → signed
//!   `ConnectSession` snapshot. Empty-intersection fallback preserved
//!   verbatim from the demo: the wire snapshot requires ≥1 negotiated
//!   capability, so a degenerate empty-intersection dial emits
//!   `["spoke-baseline"]` — the dialer derives its own set from the hellos,
//!   so the fallback has no authorization impact (documented carried-over
//!   behavior).
//! - **serve**: per inbound invoke, a serialized gate (peek → verify →
//!   advance; the async verify cannot interleave with a concurrent invoke)
//!   followed by a concurrent dispatch phase: `tools.*` ops run the
//!   registered tool handler (or deny `op_unsupported`), `port.*` ops run
//!   the D4 catalogue against the injected async [`BaselinePorts`] (absent
//!   `ports` still answers the dispatch-deny branch), and unknown ops are
//!   denied. A failed envelope-auth verify produces no handler side effect
//!   and no session-state mutation (auth-before-advance, spec §Verify
//!   rules). An unparseable inbound frame closes the connection (carried
//!   over from the demo).
//! - **invoke_tool**: the reverse face — outbound counter, request signing,
//!   send-tail wire-order serialization, response correlation +
//!   envelope-auth verify, per-waiter timeout, and the deferred-send
//!   poison-close mirror (a waiter that settles while its send is still
//!   queued means the allocated outbound sequence never hit the wire —
//!   close the session, same semantics as the adapter's `invoke_op`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::FutureExt;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Map, Value};
use spoke_operations::{
    parse_tool_capability_id, spoke_ok, spoke_reject, to_error_envelope, BaselinePorts,
    SpokeReject, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::connect::connect_hello::HostCapabilityManifest as ConnectHostCapabilityManifest;
use spoke_schemas::connect::connect_invoke_response::ConnectInvokeResponse;
use spoke_schemas::connect::ConnectHello;
use spoke_schemas::{Finding, HostCapabilityManifest, KnowledgeEntry, Relation, Scope};

use crate::core::{
    authenticate_invoke_request, authenticate_invoke_response, authenticate_session,
    check_response_correlation, derive_peer_id_from_ed25519_pubkey, dispatch_allowed,
    is_allowlisted, sign_hello_ed25519, verify_hello_ed25519, verify_invoke_request_auth,
    verify_invoke_response_auth, Correlation, EnvelopeAuthError, InboundSequence,
    InvokeRequestSignInput, InvokeResponseSignInput, NonceStore, OutboundSequence, SessionSignInput,
};
use crate::hello::generate_nonce;
use crate::remote::remote_adapter::{
    connect_manifest, internal_error, is_connect_invoke_request, map_error_envelope,
    wire_response_correlation, RemoteError as ResponderError, RemoteErrorKind as ResponderErrorKind,
    RemoteIdentity, ToolHandler,
};
use crate::remote::transport::Transport;
use crate::runtime::generate_request_id;

/// Default bounded-wait deadline for each reverse-invoke waiter, ms (parity
/// with TS `DEFAULT_INVOKE_TIMEOUT_MS`).
const DEFAULT_INVOKE_TIMEOUT_MS: u64 = 5000;

/// Session id prefix — deterministic per dialing peer (opaque, not
/// schema-enforced; parity with TS `SESSION_ID_PREFIX`).
const SESSION_ID_PREFIX: &str = "connect-responder-session-";

/// Session-lifecycle labels (frozen contract §4 labels; identical to
/// [`crate::remote::RemoteAdapterState`] — one state vocabulary).
pub type ConnectResponderState = crate::remote::RemoteAdapterState;

/// Product `op_capability_requirements` map (D4): every baseline `port.*`
/// op requires `spoke-baseline`. The core `required_capability` table
/// returns `None` for `port.*`, so WITHOUT this map every port invoke would
/// be denied `op_unsupported`.
fn port_op_requirement(op: &str) -> Option<&'static str> {
    match op {
        "port.knowledge.get" => Some("spoke-baseline"),
        "port.knowledge.put" => Some("spoke-baseline"),
        "port.relation.get" => Some("spoke-baseline"),
        "port.relation.put" => Some("spoke-baseline"),
        "port.scope.list_knowledge_entries" => Some("spoke-baseline"),
        "port.scope.list_timeline_events" => Some("spoke-baseline"),
        "port.finding.put" => Some("spoke-baseline"),
        "port.rule.list" => Some("spoke-baseline"),
        "port.host.list_peer_manifests" => Some("spoke-baseline"),
        _ => None,
    }
}

/// Deserialize one opaque payload field (defensive — the dispatch gate
/// already ran; a malformed payload answers `INVALID_INPUT`).
fn payload_field<T: DeserializeOwned>(payload: &Value, field: &str, op: &str) -> SpokeResult<T> {
    let Some(value) = payload.get(field) else {
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            format!("invalid {op} payload: missing {field}"),
            None,
        );
    };
    match serde_json::from_value(value.clone()) {
        Ok(value) => spoke_ok(value),
        Err(error) => spoke_reject(
            SpokeRejectCode::InvalidInput,
            format!("invalid {op} payload: {error}"),
            None,
        ),
    }
}

/// Map a `port.*` op + payload to the injected adapter method per the D4
/// catalogue. The dispatch gate (capability check) has already run when this
/// is called; unknown ops reject `CAPABILITY_PORT_MISSING` as a safety net
/// for host misconfiguration (the gate denies them first).
async fn dispatch_port_op(
    op: &str,
    payload: &Value,
    ports: &(dyn BaselinePorts + Send + Sync),
) -> SpokeResult<Value> {
    let field = |name: &str| payload_field::<Value>(payload, name, op);
    match op {
        "port.knowledge.get" => {
            let entry_id = match field("entry_id") {
                SpokeResult::Ok(Value::String(id)) => id,
                _ => {
                    return spoke_reject(
                        SpokeRejectCode::InvalidInput,
                        "invalid port.knowledge.get payload",
                        None,
                    );
                }
            };
            map_result(ports.get_knowledge_entry(&entry_id).await)
        }
        "port.knowledge.put" => {
            let entry = match payload_field::<KnowledgeEntry>(payload, "entry", op) {
                SpokeResult::Ok(value) => value,
                SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
            };
            let expected = match payload_field::<Option<u64>>(
                payload,
                "expected_base_revision",
                op,
            ) {
                SpokeResult::Ok(value) => value,
                SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
            };
            map_result(ports.put_knowledge_entry(entry, expected).await)
        }
        "port.relation.get" => {
            let relation_id = match field("relation_id") {
                SpokeResult::Ok(Value::String(id)) => id,
                _ => {
                    return spoke_reject(
                        SpokeRejectCode::InvalidInput,
                        "invalid port.relation.get payload",
                        None,
                    );
                }
            };
            map_result(ports.get_relation(&relation_id).await)
        }
        "port.relation.put" => {
            let relation = match payload_field::<Relation>(payload, "relation", op) {
                SpokeResult::Ok(value) => value,
                SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
            };
            let expected = match payload_field::<Option<u64>>(
                payload,
                "expected_base_revision",
                op,
            ) {
                SpokeResult::Ok(value) => value,
                SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
            };
            map_result(ports.put_relation(relation, expected).await)
        }
        "port.scope.list_knowledge_entries" => {
            let scope = match payload_field::<Scope>(payload, "scope", op) {
                SpokeResult::Ok(value) => value,
                SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
            };
            map_result(ports.list_knowledge_entries(&scope).await)
        }
        "port.scope.list_timeline_events" => {
            let scope = match payload_field::<Scope>(payload, "scope", op) {
                SpokeResult::Ok(value) => value,
                SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
            };
            map_result(ports.list_timeline_events(&scope).await)
        }
        "port.finding.put" => {
            let findings = match payload_field::<Vec<Finding>>(payload, "findings", op) {
                SpokeResult::Ok(value) => value,
                SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
            };
            map_result(ports.put_findings(findings).await)
        }
        "port.rule.list" => {
            let rule_refs = match payload_field::<Vec<String>>(payload, "rule_refs", op) {
                SpokeResult::Ok(value) => value,
                SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
            };
            map_result(ports.list_rules(&rule_refs).await)
        }
        "port.host.list_peer_manifests" => {
            map_result(ports.list_peer_host_capability_manifests().await)
        }
        _ => SpokeResult::Reject(SpokeReject {
            code: SpokeRejectCode::CapabilityPortMissing,
            message: format!("unimplemented port op {op}"),
            details: None,
        }),
    }
}

/// Map an adapter `SpokeResult<T>` to the opaque success `Value` (or
/// preserve the reject).
fn map_result<T: Serialize>(result: SpokeResult<T>) -> SpokeResult<Value> {
    match result {
        SpokeResult::Ok(value) => match serde_json::to_value(&value) {
            Ok(payload) => spoke_ok(payload),
            Err(error) => spoke_reject(
                SpokeRejectCode::InternalError,
                format!("port response serialize failed: {error}"),
                None,
            ),
        },
        SpokeResult::Reject(reject) => SpokeResult::Reject(reject),
    }
}

/// A parked reverse invoke: correlation material + the waiter channel + the
/// timeout task (mirror of the adapter's `PendingInvoke`).
struct PendingReverseInvoke {
    correlation: Correlation,
    tx: tokio::sync::mpsc::Sender<Result<ConnectInvokeResponse, ResponderError>>,
    timeout_task: tokio::task::JoinHandle<()>,
}

/// Gate-phase outcome for an inbound invoke (see
/// [`ConnectResponder::run_gate`]).
enum ServeGateResult {
    /// Stray request (no established session) — ignored, no response.
    Stray,
    /// Gate rejection: answer the error branch with these wire fields.
    Denied {
        code: String,
        message: String,
        details: Option<Map<String, Value>>,
    },
    /// Gate passed — dispatch may run.
    Ok,
}

/// Established-session state, per peer.
struct ResponderSession {
    session_id: String,
    /// The verified dialer's hello Ed25519 public key (set at handshake) —
    /// every inbound post-hello envelope is verified against it.
    client_pubkey: [u8; 32],
    /// The session's negotiated capabilities (intersection of the local and
    /// dialer hello manifests) — the dispatch gate for inbound invokes.
    negotiated_capabilities: Vec<String>,
    inbound: Mutex<InboundSequence>,
    outbound: Mutex<OutboundSequence>,
}

/// Connect responder over one established session (frozen contract §6).
/// Construct via [`connect_responder`] — the factory alone yields an
/// un-established responder whose handshake runs in the background; the
/// dialer's hello is the synchronization point.
pub struct ConnectResponder {
    transport: Arc<dyn Transport>,
    /// This responder's raw Ed25519 hello-identity seed.
    local_secret: [u8; 32],
    /// Host manifest advertised in the responder's signed hello.
    manifest: HostCapabilityManifest,
    /// Trusted dialer peer ids (fail-closed allowlist).
    allowlist: Vec<String>,
    /// Preconfigured dialer Ed25519 public keys by peer_id (responder-owned
    /// trust config). A dialing peer on `allowlist` without a preconfigured
    /// key fails the handshake (fail-closed).
    peer_keys: HashMap<String, [u8; 32]>,
    /// Local async `BaselinePorts` served on the remote side via the D4
    /// catalogue. Absent `ports` still answers `port.*` invokes with the
    /// dispatch-deny branch (documented behavior).
    ports: Option<Arc<dyn BaselinePorts + Send + Sync>>,
    /// Bounded-wait deadline for each reverse-invoke waiter, ms.
    invoke_timeout: Duration,
    /// Dialer hello nonce single-use record (handshake replay protection).
    nonce_store: Mutex<NonceStore>,
    state: Mutex<ConnectResponderState>,
    session: Mutex<Option<ResponderSession>>,
    remote_manifest: Mutex<Option<HostCapabilityManifest>>,
    /// Tool-handler registry for `tools.*` invokes (frozen contract §6):
    /// `register_tool_handler` fills it; the serving path looks it up by
    /// exact capability id. The local manifest's `tools[]` (carried through
    /// hello) is the discovery source — this registry MUST NOT mutate the
    /// manifest; a registry/manifest mismatch is surfaced at invoke time:
    /// a manifest-declared tool with no registered handler is denied
    /// fail-closed (op_unsupported → CAPABILITY_PORT_MISSING);
    /// `validate_manifest_tools` checks manifest-internal consistency
    /// only.
    tool_handlers: Mutex<HashMap<String, ToolHandler>>,
    pending: Arc<Mutex<HashMap<String, PendingReverseInvoke>>>,
    serve_loop_running: AtomicBool,
    /// Outbound send serialization tail. Sequences are allocated
    /// synchronously in call order, but the send of invoke N must not start
    /// until the send of invoke N−1 finished (the dialer's strict inbound
    /// gate rejects out-of-order requests). Mirror of the TS `#sendTail`
    /// promise chain; like the adapter's lock-first `invoke_op`, the tail
    /// lock is acquired BEFORE the outbound allocation so the wire order
    /// matches the allocation order. The waiter is registered before the
    /// tail (its timeout clock starts at call time), so a timeout that fires
    /// while a send is queued behind the tail is observable as a settled
    /// waiter (the deferred-send poison-close mirror, frozen §6).
    send_tail: tokio::sync::Mutex<()>,
}

impl ConnectResponder {
    fn new(options: ConnectResponderOptions) -> Self {
        Self {
            transport: options.transport,
            local_secret: options.identity.seed,
            manifest: options.manifest,
            allowlist: options.allowlist,
            peer_keys: options.peer_keys,
            ports: options.ports,
            invoke_timeout: Duration::from_millis(
                options.invoke_timeout_ms.unwrap_or(DEFAULT_INVOKE_TIMEOUT_MS),
            ),
            nonce_store: Mutex::new(NonceStore::new()),
            state: Mutex::new(ConnectResponderState::Disconnected),
            session: Mutex::new(None),
            remote_manifest: Mutex::new(None),
            tool_handlers: Mutex::new(HashMap::new()),
            pending: Arc::new(Mutex::new(HashMap::new())),
            serve_loop_running: AtomicBool::new(false),
            send_tail: tokio::sync::Mutex::new(()),
        }
    }

    /// Read-only session state (frozen contract §6 labels).
    #[must_use]
    pub fn state(&self) -> ConnectResponderState {
        *self.state.lock().expect("state lock")
    }

    /// The assigned session id (`None` before establishment; the Rust
    /// adapter convention — TS returns `""`).
    #[must_use]
    pub fn session_id(&self) -> Option<String> {
        self.session
            .lock()
            .expect("session lock")
            .as_ref()
            .map(|session| session.session_id.clone())
    }

    /// The verified dialing peer id (`None` before establishment).
    #[must_use]
    pub fn remote_peer_id(&self) -> Option<String> {
        // The session's `initiator_peer_id` — stored as the session binding
        // (the responder is the responder role; the peer id is the dialer's).
        self.session
            .lock()
            .expect("session lock")
            .as_ref()
            .map(|session| session.client_pubkey)
            .map(|pubkey| derive_peer_id_from_ed25519_pubkey(&pubkey))
    }

    /// The dialing peer's `HostCapabilityManifest`, from the authenticated
    /// hello `host` (discovery after auth). `None` before establishment
    /// (the Rust adapter convention; TS throws).
    #[must_use]
    pub fn remote_manifest(&self) -> Option<HostCapabilityManifest> {
        self.remote_manifest.lock().expect("manifest lock").clone()
    }

    /// Release the session and transport. Idempotent; pending reverse
    /// invokes fail with `INTERNAL_ERROR` `details.kind = "session_closed"`.
    pub fn close(&self) {
        self.close_session("local shutdown");
    }

    // ── Tool serving + reverse invoke (frozen contract §6) ────────────────

    /// Register a handler for a `tools.<ns>.<tool_id>` capability served on
    /// this responder. Grammar-asserted: a non-`tools.` id panics
    /// (programmer misuse, like the generated-type ergonomics of
    /// `tool_capability_id`). Duplicate registration for the same id
    /// OVERWRITES the previous handler (last-wins, documented). The registry
    /// does NOT mutate the local manifest — descriptor truth for discovery
    /// stays in the manifest's `tools[]` (sent through hello).
    pub fn register_tool_handler(&self, capability_id: &str, handler: ToolHandler) {
        match parse_tool_capability_id(capability_id) {
            SpokeResult::Ok(_) => {}
            SpokeResult::Reject(reject) => {
                panic!("{}", reject.message);
            }
        }
        self.tool_handlers
            .lock()
            .expect("tool handlers lock")
            .insert(capability_id.to_owned(), handler);
    }

    /// Reverse tool-invoke face (frozen contract §6): issue a
    /// `ConnectInvokeRequest` with `op = capability_id` toward the dialer
    /// and resolve with the tool's `result` (extracted from the success
    /// `payload = { result: <opaque JSON> }`). Deny answers map via the
    /// existing D7 row (`op_unsupported` / `capability_missing` →
    /// `CAPABILITY_PORT_MISSING` with `details.wire_code` preserved).
    pub async fn invoke_tool(&self, capability_id: &str, arguments: Value) -> SpokeResult<Value> {
        // Fail fast on a non-tool capability id (the op string IS the
        // capability string; a non-`tools.` id is a programming error).
        match parse_tool_capability_id(capability_id) {
            SpokeResult::Ok(_) => {}
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        }
        // Tool invoke payload shape: `{ "arguments": <opaque JSON> }` (§4).
        let response = match self
            .invoke_op(capability_id, json!({ "arguments": arguments }))
            .await
        {
            Ok(response) => response,
            Err(error) => return internal_error(error.kind.as_str(), error.message),
        };
        match response {
            ConnectInvokeResponse::Variant1 { error, .. } => {
                SpokeResult::Reject(map_error_envelope(&error))
            }
            ConnectInvokeResponse::Variant0 { payload, .. } => {
                // Tool success-payload gate (frozen §4): success is
                // `payload = { "result": <opaque JSON> }`; a success payload
                // without a `result` key rejects with `INTERNAL_ERROR`
                // `details.kind = "transport"`.
                match payload.get("result") {
                    Some(result) => spoke_ok(result.clone()),
                    None => internal_error(
                        "transport",
                        format!(
                            "response payload decode failed: payload does not match the {capability_id} success shape"
                        ),
                    ),
                }
            }
        }
    }

    // ── Session lifecycle (hard-private — only `connect_responder` starts) ──

    /// All failure paths that make the session unusable: mark `Closed`, fail
    /// every pending reverse-invoke waiter with `session_closed`, and
    /// release the transport (fire-and-forget).
    fn close_session(&self, reason: &str) {
        {
            let mut state = self.state.lock().expect("state lock");
            if *state == ConnectResponderState::Closed {
                return;
            }
            *state = ConnectResponderState::Closed;
        }

        let entries: Vec<PendingReverseInvoke> = self
            .pending
            .lock()
            .expect("pending lock")
            .drain()
            .map(|(_request_id, entry)| entry)
            .collect();
        for entry in entries {
            entry.timeout_task.abort();
            let _ = entry.tx.try_send(Err(ResponderError::new(
                ResponderErrorKind::SessionClosed,
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

    // ── Handshake (allowlist-first → hello verify → nonce → dial-bound
    // ── responder hello → signed session snapshot) ─────────────────────────

    async fn handshake(&self) -> Result<(), String> {
        let bytes = self
            .transport
            .recv()
            .await
            .map_err(|error| error.to_string())?;
        let hello: ConnectHello = serde_json::from_slice(&bytes)
            .map_err(|_| "expected ConnectHello from client".to_string())?;
        // Allowlist fail-closed check FIRST: an untrusted peer is rejected
        // before any signature work.
        let client_peer_id = hello.peer_id.as_str().to_string();
        if !is_allowlisted(&self.allowlist, &client_peer_id) {
            return Err(format!("peer {client_peer_id} not on allowlist"));
        }
        let client_pubkey = self
            .peer_keys
            .get(&client_peer_id)
            .copied()
            .ok_or_else(|| {
                format!("no preconfigured public key for allowlisted peer {client_peer_id}")
            })?;
        let derived_client_peer_id = derive_peer_id_from_ed25519_pubkey(&client_pubkey);
        verify_hello_ed25519(&client_pubkey, &derived_client_peer_id, &hello, None)
            .map_err(|error| format!("client hello verification failed: {error}"))?;
        if !self
            .nonce_store
            .lock()
            .expect("nonce lock")
            .check_and_record(&client_peer_id, hello.nonce.as_str())
        {
            return Err("nonce replay".to_string());
        }

        let responder_peer_id = derive_peer_id_from_ed25519_pubkey(
            &ed25519_dalek::SigningKey::from_bytes(&self.local_secret)
                .verifying_key()
                .to_bytes(),
        );
        let remote_manifest: HostCapabilityManifest = {
            let wire: ConnectHostCapabilityManifest = hello.host;
            serde_json::from_value(serde_json::to_value(&wire).expect("hello host serializes"))
                .map_err(|error| format!("dialer hello host decode failed: {error}"))?
        };
        let negotiated: Vec<String> = self
            .manifest
            .capabilities
            .iter()
            .filter(|cap| remote_manifest.capabilities.iter().any(|remote| remote == *cap))
            .cloned()
            .collect();

        *self.session.lock().expect("session lock") = Some(ResponderSession {
            session_id: format!("{SESSION_ID_PREFIX}{derived_client_peer_id}"),
            client_pubkey,
            negotiated_capabilities: negotiated.clone(),
            inbound: Mutex::new(InboundSequence::new()),
            outbound: Mutex::new(OutboundSequence::new()),
        });
        *self.remote_manifest.lock().expect("manifest lock") = Some(remote_manifest);

        // Answer with our signed hello (responder role — dial binding): the
        // hello signs the 5-field object incl. `peer_nonce` = the
        // initiator's nonce, so a captured responder hello cannot be
        // replayed into a fresh dial. Then the signed session snapshot.
        let nonce = generate_nonce().map_err(|error| error.to_string())?;
        let hello = sign_hello_ed25519(
            &self.local_secret,
            &nonce,
            &connect_manifest(&self.manifest),
            Some(hello.nonce.as_str()),
        )
        .map_err(|error| format!("responder hello sign failed: {error}"))?;
        let bytes = serde_json::to_vec(&hello).map_err(|error| error.to_string())?;
        self.transport.send(&bytes).await.map_err(|error| error.to_string())?;
        // The session snapshot is signed with the responder identity
        // (`spoke-connect-session-jcs-v1`) — the dialer verifies it against
        // this responder's hello public key. The wire snapshot requires ≥1
        // negotiated capability; the dialer derives its own set from the
        // hellos, so the fallback only covers a degenerate
        // empty-intersection dial (carried-over demo behavior, no
        // authorization impact).
        let snapshot = authenticate_session(
            &self.local_secret,
            &SessionSignInput {
                session_id: format!("{SESSION_ID_PREFIX}{derived_client_peer_id}"),
                initiator_peer_id: derived_client_peer_id,
                responder_peer_id,
                opened_at: chrono::Utc::now(),
                negotiated_capabilities: if negotiated.is_empty() {
                    vec!["spoke-baseline".to_owned()]
                } else {
                    negotiated
                },
                initial_sequence: 0,
            },
            HashMap::new(),
        )
        .map_err(|error| error.to_string())?;
        let bytes = serde_json::to_vec(&snapshot).map_err(|error| error.to_string())?;
        self.transport.send(&bytes).await.map_err(|error| error.to_string())?;
        Ok(())
    }

    // ── Serve loop (request-first classification) ─────────────────────────

    async fn run(self: &Arc<Self>) {
        match self.handshake().await {
            Ok(()) => {}
            Err(error) => {
                // Handshake failure (or responder misconfiguration): fail the
                // peer's pending recv like a connection drop.
                self.close_session(&format!("handshake failed: {error}"));
                return;
            }
        }
        *self.state.lock().expect("state lock") = ConnectResponderState::Established;
        self.run_serve_loop();
    }

    fn run_serve_loop(self: &Arc<Self>) {
        if self.serve_loop_running.swap(true, Ordering::SeqCst) {
            return;
        }
        let responder = Arc::clone(self);
        tokio::spawn(async move {
            responder.serve_loop().await;
            responder.serve_loop_running.store(false, Ordering::SeqCst);
        });
    }

    async fn serve_loop(self: &Arc<Self>) {
        while *self.state.lock().expect("state lock") == ConnectResponderState::Established {
            let bytes = match self.transport.recv().await {
                Ok(bytes) => bytes,
                Err(_) => {
                    // Transport closed — the session is unusable; fail pending
                    // waiters.
                    self.close_session("transport loss");
                    return;
                }
            };
            let doc: Value = match serde_json::from_slice(&bytes) {
                Ok(doc) => doc,
                Err(error) => {
                    // Unparseable inbound: closes the connection per the
                    // carried-over demo semantics — a bare return would leave
                    // the socket open while the responder stops recv'ing, and
                    // the dialer's established session would hang on its next
                    // invoke.
                    self.close_session(&format!("unparseable inbound message: {error}"));
                    return;
                }
            };

            // Classify request shape FIRST (normative `spoke-connect.md`
            // §Request / response classification): an inbound envelope
            // carrying `op` is a `ConnectInvokeRequest` — never a response,
            // even though a reverse request carries the same correlation
            // echo fields + payload as the response success branch.
            if is_connect_invoke_request(&doc) {
                // Gate serialization: await peek → verify → advance inline;
                // the loop reads the next envelope only after this gate
                // completes. Dispatch fires without blocking the loop.
                match self.serve_invoke(&doc).await {
                    Ok(()) => continue,
                    Err(error) => {
                        // Unexpected serving failure (local key
                        // misconfiguration / internal bug): fail closed like
                        // transport loss — a live session must not silently
                        // drop invokes.
                        self.close_session(&format!("invoke serving failed: {error}"));
                        return;
                    }
                }
            }

            if wire_response_correlation(&doc).is_some() {
                self.demux_response(&doc).await;
                continue;
            }

            // Post-handshake stray envelope (hello / session / unknown
            // shape): ignored.
        }
    }

    /// Serve one inbound invoke per the canonical order (frozen §4):
    /// classify (caller) → stray → sequence peek → envelope-auth verify →
    /// advance → gate → handler → signed response. The caller (serve loop)
    /// awaits this method inline so peek → verify → advance are serialized;
    /// dispatch fires without blocking the loop.
    async fn serve_invoke(self: &Arc<Self>, doc: &Value) -> Result<(), ResponderError> {
        match self.run_gate(doc).await? {
            ServeGateResult::Stray => Ok(()),
            ServeGateResult::Denied {
                code,
                message,
                details,
            } => {
                self.send_reverse_error_envelope(doc, &code, &message, details.as_ref())
                    .await?;
                Ok(())
            }
            ServeGateResult::Ok => {
                let doc = doc.clone();
                let responder = Arc::clone(self);
                tokio::spawn(async move {
                    responder.dispatch_invoke(doc).await;
                });
                Ok(())
            }
        }
    }

    /// Gate phase — sequence peek (non-mutating) → envelope-auth verify →
    /// advance. Fail-closed (auth-before-advance, spec §Verify rules): a
    /// forged/tampered/stripped signature answers `auth_failed` and leaves
    /// the inbound counter unchanged. Returns [`ServeGateResult::Stray`] for
    /// stray requests (ignored), a rejection spec for gate failures, or
    /// [`ServeGateResult::Ok`] when dispatch may run.
    async fn run_gate(&self, doc: &Value) -> Result<ServeGateResult, ResponderError> {
        // Extract session material under the guard; never hold a MutexGuard
        // across an await.
        let (session_id, client_pubkey) = {
            let session_guard = self.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                return Ok(ServeGateResult::Stray); // stray — no established session
            };
            (session.session_id.clone(), session.client_pubkey)
        };
        let sequence = match doc.get("sequence") {
            // A present-but-non-numeric `sequence` is a malformed wire
            // request: answer the deny branch (parity with the TS gate,
            // whose strict `InboundSequence.peek` throws on a non-number →
            // `invalid_sequence`) instead of silently ignoring it as `Stray`
            // (deny observability parity — a silent ignore makes the sender
            // wait out its timeout for no answer).
            Some(value) => match value.as_i64() {
                Some(sequence) => sequence,
                None => {
                    return Ok(ServeGateResult::Denied {
                        code: "invalid_sequence".to_owned(),
                        message: format!("inbound sequence {value} is not the next expected"),
                        details: None,
                    });
                }
            },
            None => return Ok(ServeGateResult::Stray),
        };
        // Stray check: a `session_id` bound to a DIFFERENT live session
        // would be ignored — this responder owns one session, so verify owns
        // the session-binding assert (mirroring the adapter).
        // 1. Sequence peek — non-mutating (auth-before-advance, frozen §4).
        let peek_ok = {
            let session_guard = self.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                return Ok(ServeGateResult::Stray);
            };
            let inbound = session.inbound.lock().expect("inbound lock");
            inbound.peek(sequence).is_ok()
        };
        if !peek_ok {
            return Ok(ServeGateResult::Denied {
                code: "invalid_sequence".to_owned(),
                message: format!("inbound sequence {sequence} is not the next expected"),
                details: None,
            });
        }
        // 2. Envelope-auth verify (contract §7 — auth-before-advance): the
        //    request signature is verified over the wire form against the
        //    dialer's hello Ed25519 public key BEFORE the inbound counter
        //    advances.
        if let Err(error) = verify_invoke_request_auth(&client_pubkey, doc, &session_id) {
            return match error.kind() {
                Some(kind) => Ok(ServeGateResult::Denied {
                    code: EnvelopeAuthError::CODE.to_owned(),
                    message: error.to_string(),
                    details: Some(Map::from_iter([(
                        "kind".into(),
                        Value::String(kind.as_str().to_owned()),
                    )])),
                }),
                // Wrong-length key is responder misconfiguration — fail loudly.
                None => Err(ResponderError::new(
                    ResponderErrorKind::EnvelopeAuthInvalid,
                    error.to_string(),
                )),
            };
        }
        // 3. Advance the inbound counter only after envelope-auth verify
        //    passed. The serialized gate makes the advance race-free.
        {
            let session_guard = self.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                return Ok(ServeGateResult::Stray);
            };
            let advance_result = session
                .inbound
                .lock()
                .expect("inbound lock")
                .advance(sequence);
            advance_result.map_err(|error| {
                ResponderError::new(
                    ResponderErrorKind::Transport,
                    format!("inbound sequence advance failed: {error}"),
                )
            })?;
        }
        Ok(ServeGateResult::Ok)
    }

    /// Dispatch phase — runs after the serialized gate; may interleave with
    /// other invokes. Fully contained: a throwing/panicking handler answers
    /// the error branch and never crashes the loop.
    async fn dispatch_invoke(self: &Arc<Self>, doc: Value) {
        if let Err(error) = self.try_dispatch_invoke(&doc).await {
            // Unexpected serving failure (e.g. non-JSON-serializable handler
            // result): answer the error branch so the invoker never hangs,
            // and never crash the loop.
            let _ = self
                .send_reverse_error_envelope(
                    &doc,
                    SpokeRejectCode::InternalError.as_str(),
                    &format!("invoke failed: {error}"),
                    None,
                )
                .await;
        }
    }

    /// Contained dispatch body (see [`ConnectResponder::dispatch_invoke`]).
    async fn try_dispatch_invoke(&self, doc: &Value) -> Result<(), ResponderError> {
        let negotiated = {
            let session_guard = self.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                return Ok(()); // stray — belt-and-braces; the gate checked
            };
            session.negotiated_capabilities.clone()
        };
        let (Some(op), Some(_session_id), Some(_sequence), Some(_request_id)) = (
            doc.get("op").and_then(Value::as_str),
            doc.get("session_id").and_then(Value::as_str),
            doc.get("sequence").and_then(Value::as_i64),
            doc.get("request_id").and_then(Value::as_str),
        ) else {
            return Ok(());
        };
        // Dispatch gate — `dispatch_allowed`-level logic (frozen §3):
        // `tools.*` ops require the op string itself; `port.*` ops require
        // `spoke-baseline` via the product map. Both evaluate against
        // `negotiated_capabilities` (never a raw requirements-map
        // composition, which would deny the self-describing tools family).
        if !self.gate_allows(op, &negotiated) {
            self.send_reverse_error_envelope(
                doc,
                "op_unsupported",
                &format!("op {op} is not authorized by this session"),
                None,
            )
            .await?;
            return Ok(());
        }
        if op.starts_with("tools.") {
            self.dispatch_tool_invoke(doc).await?;
            return Ok(());
        }
        self.dispatch_port_invoke(doc).await?;
        Ok(())
    }

    /// Dispatch gate: core table (incl. `tools.*`) then the port product map.
    fn gate_allows(&self, op: &str, negotiated: &[String]) -> bool {
        if dispatch_allowed(op, negotiated) {
            return true;
        }
        port_op_requirement(op)
            .is_some_and(|required| negotiated.iter().any(|cap| cap == required))
    }

    /// Serve a `tools.*` invoke through the registered handler (or deny).
    async fn dispatch_tool_invoke(&self, doc: &Value) -> Result<(), ResponderError> {
        let Some(op) = doc.get("op").and_then(Value::as_str) else {
            return Ok(());
        };
        // Handler or deny — fail-closed serving (frozen deny matrix): a gate
        // pass with no registered handler answers `op_unsupported`.
        let handler = self
            .tool_handlers
            .lock()
            .expect("tool handlers lock")
            .get(op)
            .cloned();
        let Some(handler) = handler else {
            self.send_reverse_error_envelope(
                doc,
                "op_unsupported",
                &format!("no handler registered for {op}"),
                None,
            )
            .await?;
            return Ok(());
        };
        // The request payload carries the tool arguments as
        // `{ "arguments": <opaque JSON> }` (frozen §4). A non-object
        // arguments field is a malformed provider request — serve `{}`
        // (the structural argument gate is caller-side).
        let arguments = doc
            .get("payload")
            .and_then(|payload| payload.get("arguments"))
            .filter(|arguments| arguments.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        // A panicking handler answers the error branch (catch_unwind
        // containment — the panic is caught at the future-poll boundary so
        // the serve loop never crashes), mirroring the adapter.
        let result = match std::panic::AssertUnwindSafe(handler(arguments))
            .catch_unwind()
            .await
        {
            Ok(result) => result,
            Err(payload) => {
                let message = crate::remote::remote_adapter::panic_payload_message(payload);
                SpokeResult::<Value>::Reject(SpokeReject {
                    code: SpokeRejectCode::InternalError,
                    message,
                    details: None,
                })
            }
        };
        self.send_tool_result(doc, result).await
    }

    /// Send the signed response for a served tool invoke (success
    /// `{ result }` branch, or the error branch via `to_error_envelope`).
    async fn send_tool_result(
        &self,
        doc: &Value,
        result: SpokeResult<Value>,
    ) -> Result<(), ResponderError> {
        let Some(session_id) = doc.get("session_id").and_then(Value::as_str) else {
            return Ok(());
        };
        let Some(sequence) = doc.get("sequence").and_then(Value::as_i64) else {
            return Ok(());
        };
        let Some(request_id) = doc.get("request_id").and_then(Value::as_str) else {
            return Ok(());
        };
        let secret = self.local_secret;
        match result {
            SpokeResult::Ok(value) => {
                // Success branch: `payload = { "result": <opaque JSON> }`
                // (frozen §4).
                let signed = authenticate_invoke_response(
                    &secret,
                    &InvokeResponseSignInput::Success {
                        session_id: session_id.to_owned(),
                        sequence,
                        request_id: request_id.to_owned(),
                        payload: json!({ "result": value }),
                    },
                    HashMap::new(),
                )
                .map_err(|error| {
                    ResponderError::new(
                        ResponderErrorKind::Transport,
                        format!("response sign failed: {error}"),
                    )
                })?;
                self.send_reverse_response(
                    &serde_json::to_value(signed).expect("signed response serializes"),
                );
                Ok(())
            }
            SpokeResult::Reject(reject) => {
                let error: spoke_schemas::connect::connect_invoke_response::ErrorEnvelope =
                    serde_json::from_value(serde_json::to_value(to_error_envelope(&reject))
                        .expect("ops error envelope serializes"))
                    .expect("field-identical error envelope converts");
                let signed = authenticate_invoke_response(
                    &secret,
                    &InvokeResponseSignInput::Error {
                        session_id: session_id.to_owned(),
                        sequence,
                        request_id: request_id.to_owned(),
                        error,
                    },
                    HashMap::new(),
                )
                .map_err(|error| {
                    ResponderError::new(
                        ResponderErrorKind::Transport,
                        format!("response sign failed: {error}"),
                    )
                })?;
                self.send_reverse_response(
                    &serde_json::to_value(signed).expect("signed response serializes"),
                );
                Ok(())
            }
        }
    }

    /// Serve a `port.*` invoke through the D4 catalogue (or dispatch-deny).
    async fn dispatch_port_invoke(&self, doc: &Value) -> Result<(), ResponderError> {
        let Some(op) = doc.get("op").and_then(Value::as_str) else {
            return Ok(());
        };
        let ports = match self.ports.as_ref() {
            Some(ports) => Arc::clone(ports),
            None => {
                // Absent `ports` (documented): the capability gate passes but
                // there is no BaselinePorts to serve — answer the
                // dispatch-deny branch.
                self.send_reverse_error_envelope(
                    doc,
                    "op_unsupported",
                    &format!("no BaselinePorts configured for port op {op}"),
                    None,
                )
                .await?;
                return Ok(());
            }
        };
        let payload = doc
            .get("payload")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let result = dispatch_port_op(op, &payload, ports.as_ref()).await;
        if let SpokeResult::Ok(value) = result {
            // Success payload carries the raw success value `T` (D4), NOT the
            // `{ result }` tool shape — the dialer's `invoke_mapped`
            // validates it.
            self.send_ok_response(doc, value).await?;
        } else {
            let reject = match result {
                SpokeResult::Reject(reject) => reject,
                SpokeResult::Ok(_) => unreachable!("matched above"),
            };
            self.send_reverse_error_envelope(
                doc,
                reject.code.as_str(),
                &reject.message,
                reject.details.as_ref(),
            )
            .await?;
        }
        Ok(())
    }

    // ── response helpers ─────────────────────────────────────────────────────

    /// Send one signed response envelope, fire-and-forget.
    fn send_reverse_response(&self, doc: &Value) {
        let envelope = match serde_json::to_vec(doc) {
            Ok(envelope) => envelope,
            Err(_) => return, // non-JSON-serializable response — drop
        };
        let transport = Arc::clone(&self.transport);
        tokio::spawn(async move {
            let _ = transport.send(&envelope).await;
        });
    }

    /// Sign + send an error-branch response to an inbound invoke. The echo
    /// fields come from the wire document (the request discriminator
    /// guarantees their presence), so reject paths that run before typed
    /// deserialization (sequence gate, envelope-auth verify) can still
    /// answer the sender.
    async fn send_reverse_error_envelope(
        &self,
        doc: &Value,
        code: &str,
        message: &str,
        details: Option<&Map<String, Value>>,
    ) -> Result<(), ResponderError> {
        let Some(session_id) = doc.get("session_id").and_then(Value::as_str) else {
            return Ok(()); // stray — no echo to answer
        };
        // A present-but-non-numeric `sequence` is denied at the gate's
        // sequence-extraction step (parity with the TS gate, which echoes
        // the raw value verbatim); the typed response envelope carries an
        // i64 `sequence`, so echo a wire-impossible sentinel (-1 is below
        // the wire floor of 0) — the deny is still observable and
        // request_id-correlatable, and a well-formed sender never produces
        // a non-numeric sequence.
        let sequence = match doc.get("sequence") {
            Some(value) => value.as_i64().unwrap_or(-1),
            None => return Ok(()), // stray — no echo to answer
        };
        let Some(request_id) = doc.get("request_id").and_then(Value::as_str) else {
            return Ok(());
        };
        let mut error = Map::new();
        error.insert("code".into(), Value::String(code.to_owned()));
        error.insert("message".into(), Value::String(message.to_owned()));
        if let Some(details) = details {
            error.insert("details".into(), Value::Object(details.clone()));
        }
        error.insert("extensions".into(), json!({}));
        let signed = authenticate_invoke_response(
            &self.local_secret,
            &InvokeResponseSignInput::Error {
                session_id: session_id.to_owned(),
                sequence,
                request_id: request_id.to_owned(),
                error: serde_json::from_value(Value::Object(error))
                    .map_err(|error| {
                        ResponderError::new(
                            ResponderErrorKind::Transport,
                            format!("error envelope build failed: {error}"),
                        )
                    })?,
            },
            HashMap::new(),
        )
        .map_err(|error| {
            ResponderError::new(
                ResponderErrorKind::Transport,
                format!("response sign failed: {error}"),
            )
        })?;
        self.send_reverse_response(
            &serde_json::to_value(signed).expect("signed response serializes"),
        );
        Ok(())
    }

    /// Sign + send a success-branch response (raw port value, D4 shape).
    async fn send_ok_response(
        &self,
        doc: &Value,
        payload: Value,
    ) -> Result<(), ResponderError> {
        let Some(session_id) = doc.get("session_id").and_then(Value::as_str) else {
            return Ok(());
        };
        let Some(sequence) = doc.get("sequence").and_then(Value::as_i64) else {
            return Ok(());
        };
        let Some(request_id) = doc.get("request_id").and_then(Value::as_str) else {
            return Ok(());
        };
        let payload = if payload.is_null() { json!({}) } else { payload };
        let signed = authenticate_invoke_response(
            &self.local_secret,
            &InvokeResponseSignInput::Success {
                session_id: session_id.to_owned(),
                sequence,
                request_id: request_id.to_owned(),
                payload,
            },
            HashMap::new(),
        )
        .map_err(|error| {
            ResponderError::new(
                ResponderErrorKind::Transport,
                format!("response sign failed: {error}"),
            )
        })?;
        self.send_reverse_response(
            &serde_json::to_value(signed).expect("signed response serializes"),
        );
        Ok(())
    }

    // ── Reverse-invoke response demux (request_id → waiter) ───────────────

    /// Demux a response envelope to its pending reverse-invoke waiter.
    /// Correlation echo check first (non-mutating wire-position validation),
    /// then envelope-auth verify against the dialer's hello public key. A
    /// forged/tampered response fails closed — only this waiter, never a
    /// session-state mutation.
    async fn demux_response(&self, doc: &Value) {
        let Some(correlation) = wire_response_correlation(doc) else {
            return;
        };
        let request_id = correlation.request_id.clone();
        let entry = self
            .pending
            .lock()
            .expect("pending lock")
            .remove(&request_id);
        let Some(entry) = entry else {
            return; // unknown/duplicate response — dropped
        };
        entry.timeout_task.abort();
        let (client_pubkey, session_id) = {
            let session_guard = self.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                // The session vanished mid-loop (close raced this response):
                // fail every pending waiter and transition to `Closed`.
                drop(session_guard);
                self.close_session("session state missing in serve loop");
                return;
            };
            (session.client_pubkey, session.session_id.clone())
        };
        // Correlation echo check first, then envelope-auth verify (contract
        // §7), then typed deserialization (the signer signed exactly the
        // wire branch, so it cannot fail).
        let outcome = check_response_correlation(&entry.correlation, &correlation)
            .map_err(|_| {
                ResponderError::new(
                    ResponderErrorKind::CorrelationMismatch,
                    "response echo fields do not match the request",
                )
            })
            .and_then(|()| {
                verify_invoke_response_auth(&client_pubkey, doc, &session_id).map_err(|error| {
                    match error.kind() {
                        Some(kind) => ResponderError::new(
                            match kind {
                                crate::core::EnvelopeAuthErrorKind::Missing => {
                                    ResponderErrorKind::EnvelopeAuthMissing
                                }
                                crate::core::EnvelopeAuthErrorKind::Invalid => {
                                    ResponderErrorKind::EnvelopeAuthInvalid
                                }
                                crate::core::EnvelopeAuthErrorKind::SessionUnbound => {
                                    ResponderErrorKind::EnvelopeAuthSessionUnbound
                                }
                            },
                            error.to_string(),
                        ),
                        None => ResponderError::new(
                            ResponderErrorKind::EnvelopeAuthInvalid,
                            error.to_string(),
                        ),
                    }
                })
            })
            .and_then(|()| {
                serde_json::from_value::<ConnectInvokeResponse>(doc.clone()).map_err(|error| {
                    ResponderError::new(
                        ResponderErrorKind::Transport,
                        format!("response decode failed: {error}"),
                    )
                })
            });
        let _ = entry.tx.try_send(outcome);
    }

    // ── Reverse invoke path (outbound counter → sign → send tail → waiter) ──

    /// Send one reverse invoke and resolve with its correlated response
    /// envelope. Errors with [`ResponderError`] on timeout / transport
    /// failure / session close / correlation mismatch / sequence exhaustion.
    ///
    /// Outbound wire-order serialization (contract §5.3 / §10): the send
    /// tail lock is acquired BEFORE the outbound sequence allocation
    /// (mirroring the adapter's lock-first `invoke_op`), so concurrent
    /// reverse invokes reach the wire in exactly the order they allocated.
    /// The waiter is registered BEFORE the tail (its timeout clock starts at
    /// call time), so a timeout that fires while a send is queued behind the
    /// tail is observable as a settled waiter (the deferred-send
    /// poison-close mirror): a waiter that settled while its send was still
    /// queued means the allocated outbound sequence never hit the wire —
    /// close the session (the peer's inbound gate would be stuck at that
    /// sequence) instead of leaving a silently poisoned session or
    /// transmitting late (a duplicate dispatch on the peer).
    async fn invoke_op(
        &self,
        op: &str,
        payload: Value,
    ) -> Result<ConnectInvokeResponse, ResponderError> {
        let state = *self.state.lock().expect("state lock");
        if state != ConnectResponderState::Established {
            return Err(ResponderError::new(
                ResponderErrorKind::SessionClosed,
                format!("connect session is not established (state {state})"),
            ));
        }
        // Waiter registration FIRST (call-time clock, BEFORE the send tail):
        // the request_id + waiter channel + timeout task + a provisional
        // pending entry are created synchronously, so a reverse invoke that
        // queues behind the tail still runs its timeout — a waiter that
        // settles while its send is queued is observable (the deferred-send
        // poison-close mirror; TS `#invokeOp` starts its timer before the
        // send-chain tail wait). The entry's correlation is completed with
        // the allocated sequence once the tail is held below; no response
        // can arrive before the send, so the provisional entry is never
        // demuxed.
        let session_id = {
            let session_guard = self.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                return Err(ResponderError::new(
                    ResponderErrorKind::SessionClosed,
                    "connect session is not established",
                ));
            };
            session.session_id.clone()
        };
        let request_id = generate_request_id()
            .map_err(|error| ResponderError::new(ResponderErrorKind::Transport, error.to_string()))?;
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
                    let _ = tx_timeout.try_send(Err(ResponderError::new(
                        ResponderErrorKind::Timeout,
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
            PendingReverseInvoke {
                correlation: Correlation {
                    session_id: session_id.clone(),
                    sequence: -1, // provisional — completed with the allocated sequence below
                    request_id: request_id.clone(),
                },
                tx: tx.clone(),
                timeout_task,
            },
        );

        // Send-tail serialization (wire-order fix): the tail is acquired
        // BEFORE the outbound allocation, mirroring the adapter's lock-first
        // `invoke_op` — concurrent reverse invokes reach the wire in exactly
        // the order they allocated (contract §5.3 / §10; the dialer's strict
        // inbound gate rejects an out-of-order request).
        let _send_guard = self.send_tail.lock().await;

        // Outbound sequence allocation (synchronous, under the tail).
        let sequence = {
            let session_guard = self.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                self.drop_pending(&request_id);
                return Err(ResponderError::new(
                    ResponderErrorKind::SessionClosed,
                    "connect session is not established",
                ));
            };
            let allocate_result = session.outbound.lock().expect("outbound lock").allocate();
            match allocate_result {
                Ok(sequence) => sequence,
                Err(_) => {
                    drop(session_guard);
                    self.drop_pending(&request_id);
                    // Outbound sequence exhaustion: the session is unusable
                    // (no wrap); close it and fail this invoke with
                    // `sequence_exhausted`.
                    self.close_session("outbound sequence exhausted");
                    return Err(ResponderError::new(
                        ResponderErrorKind::SequenceExhausted,
                        "outbound sequence space exhausted — reopen session",
                    ));
                }
            }
        };
        // The wire request to sign (`spoke-connect-invoke-request-jcs-v1`,
        // envelope-auth contract §2/§3): the signed object covers
        // `{session_id, sequence, request_id, op, payload}`.
        let signed = authenticate_invoke_request(
            &self.local_secret,
            &InvokeRequestSignInput {
                session_id,
                sequence: sequence as i64,
                request_id: request_id.clone(),
                op: op.to_owned(),
                payload,
                auth: None,
            },
            HashMap::new(),
        )
        .map_err(|error| {
            self.drop_pending(&request_id);
            ResponderError::new(
                ResponderErrorKind::Transport,
                format!("envelope auth signing failed: {error}"),
            )
        })?;

        // Complete the waiter's correlation with the allocated sequence (the
        // entry is only demuxed after the send below).
        {
            let mut pending = self.pending.lock().expect("pending lock");
            if let Some(entry) = pending.get_mut(&request_id) {
                entry.correlation.sequence = sequence as i64;
            }
        }

        // Poison check (deferred-send poison-close mirror): a waiter that
        // settled while its send was queued behind the tail means the
        // allocated outbound sequence never hit the wire — close the session
        // and settle with the error the waiter already observed (the
        // timeout/close message is queued on the channel).
        if !self.pending.lock().expect("pending lock").contains_key(&request_id) {
            self.close_session(
                "deferred invoke skipped (settled while queued) — outbound sequence never transmitted",
            );
            return match rx.recv().await {
                Some(result) => result,
                None => Err(ResponderError::new(
                    ResponderErrorKind::SessionClosed,
                    "deferred invoke skipped (settled while queued) — outbound sequence never transmitted",
                )),
            };
        }

        // Encode + send. A synchronous encode failure or an async send
        // failure settles this invoke now — no dead entry waits out the
        // timeout.
        let envelope = match serde_json::to_vec(&signed) {
            Ok(envelope) => envelope,
            Err(error) => {
                self.fail_pending_send(&request_id, &format!("invoke encode failed: {error}"));
                return Err(ResponderError::new(
                    ResponderErrorKind::Transport,
                    format!("invoke encode failed: {error}"),
                ));
            }
        };
        if let Err(error) = self.transport.send(&envelope).await {
            self.fail_pending_send(&request_id, &format!("invoke send failed: {error}"));
            return Err(ResponderError::new(
                ResponderErrorKind::Transport,
                format!("invoke send failed: {error}"),
            ));
        }
        // Wire order is settled — release the send tail so later-allocated
        // invokes can transmit while this one awaits its correlated response.
        drop(_send_guard);

        match rx.recv().await {
            Some(result) => result,
            None => Err(ResponderError::new(
                ResponderErrorKind::Transport,
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
            let _ = entry.tx.try_send(Err(ResponderError::new(
                ResponderErrorKind::Transport,
                message.to_owned(),
            )));
        }
    }

    /// Remove a registered waiter WITHOUT settling (aborting its timeout
    /// task). Used by the pre-send failure paths (allocation / sign) after
    /// the provisional registration — the invoke never reached the wire, so
    /// the caller observes the error directly and the waiter channel is
    /// dropped with this function's caller.
    fn drop_pending(&self, request_id: &str) {
        if let Some(entry) = self
            .pending
            .lock()
            .expect("pending lock")
            .remove(request_id)
        {
            entry.timeout_task.abort();
        }
    }
}

/// Options for [`connect_responder`] (frozen contract §6; mirror of TS
/// `ConnectResponderOptions`).
#[derive(Clone)]
pub struct ConnectResponderOptions {
    /// Message-oriented transport (consumer-provided; loopback ships in-repo).
    pub transport: Arc<dyn Transport>,
    /// This responder's raw Ed25519 seed (hello + envelope-auth identity).
    pub identity: RemoteIdentity,
    /// Host manifest advertised in the responder's signed hello.
    pub manifest: HostCapabilityManifest,
    /// Trusted dialer peer ids (fail-closed allowlist).
    pub allowlist: Vec<String>,
    /// Preconfigured dialer Ed25519 public keys by peer_id. Key distribution
    /// is transport-adapter-owned per the spec; the responder knows its
    /// trusted identities statically. A dialing peer on `allowlist` without
    /// a preconfigured key fails the handshake (fail-closed).
    pub peer_keys: HashMap<String, [u8; 32]>,
    /// Local async `BaselinePorts` served on the remote side via the D4
    /// catalogue. Absent `ports` still answers `port.*` invokes with the
    /// dispatch-deny branch (documented behavior).
    pub ports: Option<Arc<dyn BaselinePorts + Send + Sync>>,
    /// Bounded-wait deadline for each reverse-invoke waiter, ms
    /// (default [`DEFAULT_INVOKE_TIMEOUT_MS`]).
    pub invoke_timeout_ms: Option<u64>,
}

/// Start a connect responder over `transport`: performs the signed hello
/// exchange + session snapshot in the background, then serves invokes. The
/// returned responder is in `Handshaking` until the dialer's hello arrives;
/// a handshake rejection closes the transport so the dial fails fast.
///
/// Divergence note (parity table): the TS factory validates
/// `identity.seed.length === 32` and throws; the Rust `RemoteIdentity` seed
/// is a `[u8; 32]` (type-enforced), so the factory has no error path and
/// returns the responder directly — handshake failures surface via the
/// `state` transition to `Closed` exactly like TS.
pub async fn connect_responder(options: ConnectResponderOptions) -> Arc<ConnectResponder> {
    let responder = Arc::new(ConnectResponder::new(options));
    *responder.state.lock().expect("state lock") = ConnectResponderState::Handshaking;
    let runner = Arc::clone(&responder);
    tokio::spawn(async move {
        runner.run().await;
    });
    responder
}
