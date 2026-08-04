//! Loopback interop test — Rust `RemoteAdapter` (client) ↔ a Rust connect
//! host serving an async `ToyWorldAdapter` (server) over the in-repo
//! `loopback_transport_pair` (frozen contract §10 verification checklist;
//! parity with `packages/spoke-connect-ts/tests/remote/remote-adapter.test.ts`).
//!
//! Asserts, per the plan:
//! (a) ENCAPSULATION — the consumer surface is ONLY the async `BaselinePorts`
//!     (+ `connect_remote_adapter`): the client is driven through the port
//!     traits and orchestration entrypoints only; no verification helpers are
//!     reachable on the adapter.
//! (b) DROP-IN — `orchestrate_upsert(remote, req)` / `orchestrate_check(...)`
//!     return the same `SpokeResult` as the local `ToyWorldAdapter` for
//!     identical requests (upsert + conflict-reject + check paths).
//! (c) VERIFICATION RAN — the connect handshake actually happened (host
//!     allowlist + signature + nonce gates; remote hello host cached).
//!
//! Plus the §10 concurrency/error rows: concurrent invokes demuxed by
//! `request_id` with out-of-order responses, invoke timeout, transport close
//! mid-flight, dispatch deny mapping, HostManifestPort cache/proxy, and
//! fail-closed allowlist dials.
//!
//! This file is gated on the `remote-adapter` feature; `cargo test -p
//! spoke-connect` (default features) does not build it.

#![cfg(feature = "remote-adapter")]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ed25519_dalek::SigningKey;
use serde::Serialize;
use serde_json::{json, Value};
use spoke_connect::core::{
    derive_peer_id_from_ed25519_pubkey, ed25519_pubkey_from_peer_id, is_allowlisted,
    required_capability, sign_hello_ed25519, verify_hello_ed25519, CapabilityClaims,
    CapabilityTokenProof, InboundSequence, NonceStore,
};
use spoke_connect::remote::{
    connect_remote_adapter, loopback_transport_pair, RemoteAdapter, RemoteAdapterError,
    RemoteAdapterOptions, RemoteIdentity, Transport,
};
use spoke_fixture_toy_world::ToyWorldAdapter;
use spoke_operations::{
    orchestrate_check, orchestrate_upsert, spoke_ok, spoke_reject, BaselinePorts, CheckRunInput,
    FindingPort, HostManifestPort, KnowledgeEntryPort, RelationPort, RuleQueryPort, ScopeQueryPort,
    SpokeReject, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::connect::connect_hello::HostCapabilityManifest as ConnectHostCapabilityManifest;
use spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequest;
use spoke_schemas::connect::ConnectHello;
use spoke_schemas::connect::ConnectSession;
use spoke_schemas::{
    CheckRequest, Finding, HostCapabilityManifest, KnowledgeEntry, Scope, UpsertRequest,
};

/// Schema-conformant host manifest (mirror of TS `schemaConformantManifest`).
fn manifest(host_id: &str, capabilities: &[&str]) -> HostCapabilityManifest {
    serde_json::from_value(json!({
        "schema_version": 1,
        "host_id": host_id,
        "roles": ["data-store"],
        "capabilities": capabilities,
        "namespaces": ["toy_world"],
        "extensions": {},
    }))
    .expect("valid HostCapabilityManifest")
}

fn connect_manifest(manifest: &HostCapabilityManifest) -> ConnectHostCapabilityManifest {
    serde_json::from_value(serde_json::to_value(manifest).expect("manifest serializes"))
        .expect("field-identical manifest converts")
}

/// Minimal schema-valid provisional KnowledgeEntry for the upsert parity.
fn fresh_entry(entry_id: &str, canonical_name: &str) -> KnowledgeEntry {
    serde_json::from_value(json!({
        "schema_version": 1,
        "entry_id": entry_id,
        "entry_type": "character",
        "canonical_name": canonical_name,
        "status": "provisional",
        "body": { "summary": format!("Upserted over the loopback: {entry_id}") },
        "extensions": {},
    }))
    .expect("valid KnowledgeEntry")
}

fn upsert_request(entries: &[KnowledgeEntry]) -> UpsertRequest {
    serde_json::from_value(json!({ "knowledge_entries": entries })).expect("valid UpsertRequest")
}

fn check_request(scope_id: &str) -> CheckRequest {
    serde_json::from_value(json!({ "scope": { "scope_id": scope_id } }))
        .expect("valid CheckRequest")
}

// ── Loopback host (test-only; mirror of TS `tests/remote/loopback-host.ts`) ─

/// Default product `op_capability_requirements` map for the loopback host
/// (frozen contract §5.1): every baseline `port.*` op requires
/// `spoke-baseline`.
const DEFAULT_PORT_CAPABILITY_REQUIREMENTS: &[(&str, &str)] = &[
    ("port.knowledge.get", "spoke-baseline"),
    ("port.knowledge.put", "spoke-baseline"),
    ("port.relation.get", "spoke-baseline"),
    ("port.relation.put", "spoke-baseline"),
    ("port.scope.list_knowledge_entries", "spoke-baseline"),
    ("port.scope.list_timeline_events", "spoke-baseline"),
    ("port.finding.put", "spoke-baseline"),
    ("port.rule.list", "spoke-baseline"),
    ("port.host.list_peer_manifests", "spoke-baseline"),
];

const SESSION_ID: &str = "test-session-loopback-0001";

/// Host hello nonces: distinct per host instance, above the 16-char wire
/// floor. The client side does not run a nonce replay store, so a counter
/// suffices for the loopback fixture (the client's own dial nonce is the
/// single-use one the host records).
static HOST_NONCE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn host_nonce() -> String {
    let n = HOST_NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("test-host-nonce-{n:04}")
}

#[derive(Debug, Clone, Default)]
struct LoopbackHostStats {
    hellos_verified: usize,
    invokes_dispatched: usize,
    sequence_rejections: usize,
    dispatch_denials: usize,
    response_order: Vec<i64>,
    auth_seen: bool,
}

struct HostSession {
    session_id: String,
    negotiated_capabilities: Vec<String>,
    inbound: Mutex<InboundSequence>,
}

struct HostInner {
    transport: Arc<dyn Transport>,
    host_seed: [u8; 32],
    host_manifest: HostCapabilityManifest,
    host_peer_id: String,
    allowlist: Vec<String>,
    adapter: Arc<ToyWorldAdapter>,
    requirements: HashMap<String, String>,
    delay: Box<dyn Fn(&ConnectInvokeRequest) -> u64 + Send + Sync>,
    session: Mutex<Option<HostSession>>,
    nonce_store: Mutex<NonceStore>,
    stats: Mutex<LoopbackHostStats>,
    closed: AtomicBool,
}

impl HostInner {
    async fn send_envelope(&self, doc: &Value) {
        let bytes = serde_json::to_vec(doc).expect("host envelope serializes");
        // Peer gone — responses are fire-and-forget at the host boundary.
        let _ = self.transport.send(&bytes).await;
    }

    async fn send_error_response(&self, request: &ConnectInvokeRequest, code: &str, message: &str) {
        self.stats
            .lock()
            .expect("stats lock")
            .response_order
            .push(request.sequence);
        self.send_envelope(&json!({
            "session_id": request.session_id,
            "sequence": request.sequence,
            "request_id": request.request_id,
            "error": { "code": code, "message": message, "extensions": {} },
            "extensions": {},
        }))
        .await;
    }

    async fn send_ok_response(&self, request: &ConnectInvokeRequest, payload: Value) {
        self.stats
            .lock()
            .expect("stats lock")
            .response_order
            .push(request.sequence);
        self.send_envelope(&json!({
            "session_id": request.session_id,
            "sequence": request.sequence,
            "request_id": request.request_id,
            "payload": payload,
            "extensions": {},
        }))
        .await;
    }

    async fn send_reject_response(&self, request: &ConnectInvokeRequest, reject: &SpokeReject) {
        self.stats
            .lock()
            .expect("stats lock")
            .response_order
            .push(request.sequence);
        self.send_envelope(&json!({
            "session_id": request.session_id,
            "sequence": request.sequence,
            "request_id": request.request_id,
            "error": {
                "code": reject.code.as_str(),
                "message": reject.message,
                "details": reject.details.clone().unwrap_or_default(),
                "extensions": {},
            },
            "extensions": {},
        }))
        .await;
    }

    async fn handle_invoke(&self, request: ConnectInvokeRequest) {
        // Session gates run under the std Mutex guard — all guard usage is
        // confined to this block so no MutexGuard crosses an await (the
        // handler runs in a spawned Send future).
        let (sequence_ok, negotiated) = {
            let session_guard = self.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                return; // stray request — ignored
            };
            if request.session_id.as_str() != session.session_id {
                return; // stray request — ignored
            }
            let sequence_ok = session
                .inbound
                .lock()
                .expect("inbound lock")
                .advance(request.sequence)
                .is_ok();
            (sequence_ok, session.negotiated_capabilities.clone())
        };
        if request.auth.is_some() {
            self.stats.lock().expect("stats lock").auth_seen = true;
        }

        // 1. Inbound sequence gate — replay/out-of-order fails; no handler
        //    side effect on failure.
        if !sequence_ok {
            self.stats.lock().expect("stats lock").sequence_rejections += 1;
            self.send_error_response(
                &request,
                "inbound_sequence_mismatch",
                &format!(
                    "inbound sequence {} is not the next expected",
                    request.sequence
                ),
            )
            .await;
            return;
        }

        // 2. Dispatch gate — product map first, then the core table; unknown
        //    ops and missing capabilities answer `op_unsupported`.
        let op = request.op.as_str();
        let required = self
            .requirements
            .get(op)
            .map(|value| value.as_str())
            .or_else(|| required_capability(op));
        let allowed =
            required.is_some_and(|requirement| negotiated.iter().any(|cap| cap == requirement));
        if !allowed {
            self.stats.lock().expect("stats lock").dispatch_denials += 1;
            self.send_error_response(
                &request,
                "op_unsupported",
                &format!("op {op} is not authorized by this session"),
            )
            .await;
            return;
        }

        // Optional deterministic delay (out-of-order response fixtures).
        let delay_ms = (self.delay)(&request);
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        if self.closed.load(Ordering::SeqCst) {
            return;
        }

        // 3. Dispatch to the local adapter.
        let result = self.dispatch_op(&request).await;
        self.stats.lock().expect("stats lock").invokes_dispatched += 1;
        match result {
            SpokeResult::Ok(payload) => self.send_ok_response(&request, payload).await,
            SpokeResult::Reject(reject) => self.send_reject_response(&request, &reject).await,
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
                    format!("host response serialize failed: {error}"),
                    None,
                ),
            },
            SpokeResult::Reject(reject) => SpokeResult::Reject(reject),
        }
    }

    /// Deserialize one opaque payload field (defensive — the dispatch gate
    /// already ran; the loopback fixtures always carry valid payloads).
    fn payload_field<T: serde::de::DeserializeOwned>(
        payload: &Value,
        field: &str,
        op: &str,
    ) -> SpokeResult<T> {
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

    async fn dispatch_op(&self, request: &ConnectInvokeRequest) -> SpokeResult<Value> {
        let adapter = self.adapter.as_ref();
        let payload = &request.payload;
        let field =
            |name: &str| Self::payload_field::<Value>(payload, name, request.op.as_str());
        match request.op.as_str() {
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
                Self::map_result(adapter.get_knowledge_entry(&entry_id).await)
            }
            "port.knowledge.put" => {
                let entry = match Self::payload_field::<KnowledgeEntry>(
                    payload,
                    "entry",
                    request.op.as_str(),
                ) {
                    SpokeResult::Ok(value) => value,
                    SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
                };
                let expected = match Self::payload_field::<Option<u64>>(
                    payload,
                    "expected_base_revision",
                    request.op.as_str(),
                ) {
                    SpokeResult::Ok(value) => value,
                    SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
                };
                Self::map_result(adapter.put_knowledge_entry(entry, expected).await)
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
                Self::map_result(adapter.get_relation(&relation_id).await)
            }
            "port.relation.put" => {
                let relation = match Self::payload_field::<spoke_schemas::Relation>(
                    payload,
                    "relation",
                    request.op.as_str(),
                ) {
                    SpokeResult::Ok(value) => value,
                    SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
                };
                let expected = match Self::payload_field::<Option<u64>>(
                    payload,
                    "expected_base_revision",
                    request.op.as_str(),
                ) {
                    SpokeResult::Ok(value) => value,
                    SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
                };
                Self::map_result(adapter.put_relation(relation, expected).await)
            }
            "port.scope.list_knowledge_entries" => {
                let scope =
                    match Self::payload_field::<Scope>(payload, "scope", request.op.as_str()) {
                        SpokeResult::Ok(value) => value,
                        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
                    };
                Self::map_result(adapter.list_knowledge_entries(&scope).await)
            }
            "port.scope.list_timeline_events" => {
                let scope =
                    match Self::payload_field::<Scope>(payload, "scope", request.op.as_str()) {
                        SpokeResult::Ok(value) => value,
                        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
                    };
                Self::map_result(adapter.list_timeline_events(&scope).await)
            }
            "port.finding.put" => {
                let findings = match Self::payload_field::<Vec<Finding>>(
                    payload,
                    "findings",
                    request.op.as_str(),
                ) {
                    SpokeResult::Ok(value) => value,
                    SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
                };
                Self::map_result(adapter.put_findings(findings).await)
            }
            "port.rule.list" => {
                let rule_refs = match Self::payload_field::<Vec<String>>(
                    payload,
                    "rule_refs",
                    request.op.as_str(),
                ) {
                    SpokeResult::Ok(value) => value,
                    SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
                };
                Self::map_result(adapter.list_rules(&rule_refs).await)
            }
            "port.host.list_peer_manifests" => {
                Self::map_result(adapter.list_peer_host_capability_manifests().await)
            }
            _ => SpokeResult::Reject(SpokeReject {
                code: SpokeRejectCode::CapabilityPortMissing,
                message: format!("unimplemented port op {}", request.op.as_str()),
                details: None,
            }),
        }
    }

    /// Signed-hello handshake + serve loop. A hello-gate failure closes the
    /// transport so the client's dial fails fast (TS loopback-host parity).
    async fn handshake_and_serve(self: &Arc<Self>) -> Result<(), String> {
        // Client hello: allowlist → signature → nonce single-use gates.
        let bytes = self
            .transport
            .recv()
            .await
            .map_err(|error| error.to_string())?;
        let hello: ConnectHello = serde_json::from_slice(&bytes)
            .map_err(|_| "expected ConnectHello from client".to_string())?;
        let client_peer_id = hello.peer_id.as_str().to_string();
        if !is_allowlisted(&self.allowlist, &client_peer_id) {
            let _ = self.transport.close().await;
            return Err(format!("peer {client_peer_id} not on allowlist"));
        }
        let client_pubkey = ed25519_pubkey_from_peer_id(&client_peer_id)
            .ok_or_else(|| "client peer id does not carry an Ed25519 key".to_string())?;
        verify_hello_ed25519(&client_pubkey, &client_peer_id, &hello)
            .map_err(|error| format!("client hello verification failed: {error}"))?;
        if !self
            .nonce_store
            .lock()
            .expect("nonce lock")
            .check_and_record(&client_peer_id, hello.nonce.as_str())
        {
            let _ = self.transport.close().await;
            return Err("nonce replay".to_string());
        }
        self.stats.lock().expect("stats lock").hellos_verified += 1;

        // Negotiated capabilities = intersection (same rule as the client).
        let negotiated = self
            .host_manifest
            .capabilities
            .iter()
            .filter(|cap| hello.host.capabilities.iter().any(|remote| remote == *cap))
            .cloned()
            .collect::<Vec<String>>();
        *self.session.lock().expect("session lock") = Some(HostSession {
            session_id: SESSION_ID.to_string(),
            negotiated_capabilities: negotiated.clone(),
            inbound: Mutex::new(InboundSequence::new()),
        });

        // Answer with our signed hello, then the responder-assigned session
        // snapshot. The wire snapshot requires ≥1 negotiated capability; the
        // client derives its own negotiated set from the hellos, so the
        // fallback only covers the degenerate empty-intersection fixture.
        let hello = sign_hello_ed25519(
            &self.host_seed,
            &host_nonce(),
            &connect_manifest(&self.host_manifest),
        )
        .map_err(|error| format!("host hello sign failed: {error}"))?;
        let bytes = serde_json::to_vec(&hello).map_err(|error| error.to_string())?;
        self.transport
            .send(&bytes)
            .await
            .map_err(|error| error.to_string())?;
        let snapshot: ConnectSession = serde_json::from_value(json!({
            "session_id": SESSION_ID,
            "initiator_peer_id": client_peer_id,
            "responder_peer_id": self.host_peer_id,
            "opened_at": "2026-01-01T00:00:00Z",
            "negotiated_capabilities": if negotiated.is_empty() {
                vec!["spoke-baseline".to_string()]
            } else {
                negotiated
            },
            "initial_sequence": 0,
            "extensions": {},
        }))
        .map_err(|error| error.to_string())?;
        let bytes = serde_json::to_vec(&snapshot).map_err(|error| error.to_string())?;
        self.transport
            .send(&bytes)
            .await
            .map_err(|error| error.to_string())?;

        // Serve invokes; each handler runs concurrently (out-of-order
        // fixtures). Transport close ends the loop.
        while !self.closed.load(Ordering::SeqCst) {
            let bytes = match self.transport.recv().await {
                Ok(bytes) => bytes,
                Err(_) => return Ok(()),
            };
            let Ok(request) = serde_json::from_slice::<ConnectInvokeRequest>(&bytes) else {
                continue; // stray envelope — ignored
            };
            let inner = Arc::clone(self);
            tokio::spawn(async move {
                inner.handle_invoke(request).await;
            });
        }
        Ok(())
    }
}

struct LoopbackHost {
    inner: Arc<HostInner>,
}

impl LoopbackHost {
    fn session_id(&self) -> &str {
        SESSION_ID
    }

    fn stats(&self) -> LoopbackHostStats {
        self.inner.stats.lock().expect("stats lock").clone()
    }

    /// Close the connection (fails the client's pending recv / invokes).
    fn close(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        let transport = Arc::clone(&self.inner.transport);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = transport.close().await;
            });
        }
    }
}

struct LoopbackHostOptions {
    transport: Arc<dyn Transport>,
    host_seed: [u8; 32],
    host_manifest: HostCapabilityManifest,
    allowlist: Vec<String>,
    adapter: Arc<ToyWorldAdapter>,
    delay: Box<dyn Fn(&ConnectInvokeRequest) -> u64 + Send + Sync>,
}

async fn start_loopback_host(options: LoopbackHostOptions) -> LoopbackHost {
    let host_pubkey = SigningKey::from_bytes(&options.host_seed)
        .verifying_key()
        .to_bytes();
    let inner = Arc::new(HostInner {
        transport: options.transport,
        host_seed: options.host_seed,
        host_manifest: options.host_manifest,
        host_peer_id: derive_peer_id_from_ed25519_pubkey(&host_pubkey),
        allowlist: options.allowlist,
        adapter: options.adapter,
        requirements: DEFAULT_PORT_CAPABILITY_REQUIREMENTS
            .iter()
            .map(|(op, requirement)| ((*op).to_string(), (*requirement).to_string()))
            .collect(),
        delay: options.delay,
        session: Mutex::new(None),
        nonce_store: Mutex::new(NonceStore::new()),
        stats: Mutex::new(LoopbackHostStats::default()),
        closed: AtomicBool::new(false),
    });
    let serve = Arc::clone(&inner);
    tokio::spawn(async move {
        if let Err(error) = serve.handshake_and_serve().await {
            // Hello-gate failure: close the transport so the client's dial
            // fails fast instead of waiting out its timeout.
            let _ = serve.transport.close().await;
            eprintln!("loopback host stopped: {error}");
        }
    });
    LoopbackHost { inner }
}

// ── Dial fixture ───────────────────────────────────────────────────────────

/// Per-request response delay callback (out-of-order fixtures).
type HostDelay = Box<dyn Fn(&ConnectInvokeRequest) -> u64 + Send + Sync>;

#[derive(Default)]
struct DialOptions {
    allowlist: Option<Vec<String>>,
    client_manifest: Option<HostCapabilityManifest>,
    invoke_timeout_ms: Option<u64>,
    host_delay: Option<HostDelay>,
    host_allowlist: Option<Vec<String>>,
    capability_token: Option<CapabilityTokenProof>,
}

fn seed_host() -> [u8; 32] {
    [0xa0u8; 32]
}

fn seed_client() -> [u8; 32] {
    [0x10u8; 32]
}

fn pubkey_host() -> [u8; 32] {
    SigningKey::from_bytes(&seed_host())
        .verifying_key()
        .to_bytes()
}

fn pubkey_client() -> [u8; 32] {
    SigningKey::from_bytes(&seed_client())
        .verifying_key()
        .to_bytes()
}

/// Dial a client against a fresh loopback host serving `host_adapter`.
async fn dial(
    host_adapter: ToyWorldAdapter,
    options: DialOptions,
) -> (Arc<RemoteAdapter>, LoopbackHost) {
    let peer_id_host = derive_peer_id_from_ed25519_pubkey(&pubkey_host());
    let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());

    let pair = loopback_transport_pair();
    let host = start_loopback_host(LoopbackHostOptions {
        transport: Arc::new(pair.server),
        host_seed: seed_host(),
        host_manifest: manifest("test-host", &["spoke-baseline"]),
        allowlist: options
            .host_allowlist
            .unwrap_or_else(|| vec![peer_id_client.clone()]),
        adapter: Arc::new(host_adapter),
        delay: options.host_delay.unwrap_or_else(|| Box::new(|_| 0)),
    })
    .await;
    let client = connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(pair.client),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: options
            .client_manifest
            .unwrap_or_else(|| manifest("test-client", &["spoke-baseline"])),
        remote_pubkey: pubkey_host(),
        allowlist: options
            .allowlist
            .unwrap_or_else(|| vec![peer_id_host.clone()]),
        invoke_timeout_ms: options.invoke_timeout_ms,
        capability_token: options.capability_token,
    })
    .await
    .expect("dial");
    (client, host)
}

/// Compare two `SpokeResult`s structurally (generated types do not derive
/// `PartialEq`).
fn results_equal<T: std::fmt::Debug>(left: &SpokeResult<T>, right: &SpokeResult<T>) -> bool {
    // Generated wire types do not derive PartialEq; Debug is the derived
    // structural equality surface.
    format!("{left:?}") == format!("{right:?}")
}

/// Extract the `details.kind` of an `INTERNAL_ERROR` reject (or `None`).
fn reject_kind(result: &SpokeResult<KnowledgeEntry>) -> Option<String> {
    match result {
        SpokeResult::Ok(_) => None,
        SpokeResult::Reject(reject) => reject
            .details
            .as_ref()
            .and_then(|details| details.get("kind"))
            .and_then(|kind| kind.as_str())
            .map(|kind| kind.to_string()),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn encapsulates_verification_and_is_drop_in_baseline_ports() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let local_adapter = ToyWorldAdapter::with_committed_fixtures(); // drop-in parity target
    let (client, host) = dial(host_adapter, DialOptions::default()).await;

    // (a) Encapsulation: the consumer surface is ONLY the async
    //     BaselinePorts — the trait-object coercion proves the type surface
    //     compiles, and the test drives the adapter through ports +
    //     orchestration entrypoints only (no hello/nonce/sequence helpers).
    let ports: &dyn BaselinePorts = client.as_ref();
    let _: &dyn KnowledgeEntryPort = ports;

    // (c) Verification ran: both hellos authenticated, remote hello host
    //     cached (the REMOTE "test-host", not the client's "test-client").
    assert_eq!(host.stats().hellos_verified, 1);
    assert_eq!(client.state().as_str(), "Established");
    assert_eq!(client.session_id().as_deref(), Some(host.session_id()));
    assert_eq!(
        client.remote_peer_id().as_deref(),
        Some(derive_peer_id_from_ed25519_pubkey(&pubkey_host()).as_str())
    );
    assert_eq!(
        client
            .remote_manifest()
            .expect("remote manifest")
            .host_id
            .as_str(),
        "test-host"
    );

    // (b) Drop-in parity — upsert path: identical requests produce identical
    //     SpokeResults on the local adapter and the remote one.
    let candidate = fresh_entry("kb_remote_cartographer", "Remote Cartographer");
    let request = upsert_request(&[candidate]);
    let local_upsert = orchestrate_upsert(&local_adapter, request.clone()).await;
    let remote_upsert = orchestrate_upsert(client.as_ref(), request).await;
    assert!(results_equal(&remote_upsert, &local_upsert));
    assert!(remote_upsert.is_ok());
    // The remote write actually landed in the host-side store.
    assert!(host
        .inner
        .adapter
        .with_store(|store| store.entries.contains_key("kb_remote_cartographer")));

    // Drop-in parity — reject path: a conflicting second upsert rejects
    // identically on both sides (error branch → SpokeResult reject).
    let request = upsert_request(&[fresh_entry("kb_remote_cartographer", "Remote Cartographer")]);
    let local_conflict = orchestrate_upsert(&local_adapter, request.clone()).await;
    let remote_conflict = orchestrate_upsert(client.as_ref(), request).await;
    assert!(remote_conflict.is_reject());
    assert!(results_equal(&remote_conflict, &local_conflict));

    // Drop-in parity — check path (listKnowledgeEntries + listTimelineEvents +
    // listRules + putFindings over the wire).
    let checker = |_: CheckRunInput| spoke_ok(Vec::<Finding>::new());
    let local_check =
        orchestrate_check(&local_adapter, check_request("toy-scope-001"), checker).await;
    let remote_check =
        orchestrate_check(client.as_ref(), check_request("toy-scope-001"), checker).await;
    assert!(results_equal(&remote_check, &local_check));
    assert!(remote_check.is_ok());

    client.close();
    host.close();
}

#[tokio::test]
async fn demuxes_concurrent_invokes_with_out_of_order_responses() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    // Deterministic out-of-order fixture: sequence-0 responses are delayed
    // 30ms, so the sequence-1 response arrives first.
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            host_delay: Some(Box::new(
                |request| {
                    if request.sequence == 0 {
                        30
                    } else {
                        0
                    }
                },
            )),
            ..Default::default()
        },
    )
    .await;

    // Both invokes must be in flight concurrently (the delay fixture only
    // works when seq-0 is already parked when seq-1 arrives).
    let (first, second) = tokio::join!(
        client.get_knowledge_entry("kb_tw_mira"), // sequence 0 — delayed
        client.get_knowledge_entry("kb_tw_harbor"), // sequence 1 — fast
    );
    match (&first, &second) {
        (SpokeResult::Ok(mira), SpokeResult::Ok(harbor)) => {
            assert_eq!(mira.entry_id.as_str(), "kb_tw_mira");
            assert_eq!(harbor.entry_id.as_str(), "kb_tw_harbor");
        }
        _ => panic!("both concurrent gets must succeed"),
    }
    // The delayed response landed second: demux delivered to the right
    // waiter despite arrival order.
    let stats = host.stats();
    assert_eq!(stats.response_order, vec![1, 0]);
    assert_eq!(stats.invokes_dispatched, 2);

    client.close();
    host.close();
}

#[tokio::test]
async fn maps_invoke_timeout_to_internal_error_kind_timeout_without_closing_session() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let delay_ms = Arc::new(AtomicU64::new(100));
    let delay_ms_clone = Arc::clone(&delay_ms);
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            invoke_timeout_ms: Some(20),
            host_delay: Some(Box::new(move |_| delay_ms_clone.load(Ordering::Relaxed))),
            ..Default::default()
        },
    )
    .await;

    let result = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(result.is_reject());
    assert_eq!(reject_kind(&result).as_deref(), Some("timeout"));

    // Timeout fails only the waiter — the session stays usable.
    assert_eq!(client.state().as_str(), "Established");
    delay_ms.store(0, Ordering::Relaxed);
    let retry = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(retry.is_ok());

    client.close();
    host.close();
}

#[tokio::test]
async fn fails_pending_invokes_with_session_closed_when_transport_closes_mid_flight() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            host_delay: Some(Box::new(|_| 100)),
            ..Default::default()
        },
    )
    .await;

    // The request is registered + sent synchronously, then the host drops
    // the connection while the response is still delayed.
    let pending = client.get_knowledge_entry("kb_tw_mira");
    host.close();
    let result = pending.await;
    assert!(result.is_reject());
    assert_eq!(reject_kind(&result).as_deref(), Some("session_closed"));
    assert_eq!(client.state().as_str(), "Closed");

    // Subsequent port calls also fail closed with session_closed.
    let after = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(after.is_reject());
    assert_eq!(reject_kind(&after).as_deref(), Some("session_closed"));

    client.close();
    host.close();
}

#[tokio::test]
async fn maps_host_dispatch_denials_to_capability_port_missing() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    // A client manifest without spoke-baseline ⇒ the negotiated set is empty
    // ⇒ the host's dispatch gate denies every port.* op.
    let no_baseline = manifest("test-client", &["l2-computable"]);
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            client_manifest: Some(no_baseline),
            ..Default::default()
        },
    )
    .await;

    let result = client.get_knowledge_entry("kb_tw_mira").await;
    match result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(|code| code.as_str()),
                Some("op_unsupported")
            );
        }
        SpokeResult::Ok(_) => panic!("dispatch denial must reject"),
    }
    let stats = host.stats();
    assert_eq!(stats.dispatch_denials, 1);
    assert_eq!(stats.invokes_dispatched, 0);

    client.close();
    host.close();
}

#[tokio::test]
async fn host_manifest_port_returns_remote_hello_host_from_cache_and_proxies_peer_list() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let (client, host) = dial(host_adapter, DialOptions::default()).await;

    // getHostCapabilityManifest = remote hello host cache — NO invoke.
    let self_manifest = client.get_host_capability_manifest().await;
    match &self_manifest {
        SpokeResult::Ok(manifest) => assert_eq!(manifest.host_id.as_str(), "test-host"),
        SpokeResult::Reject(_) => panic!("cache manifest must succeed"),
    }
    assert_eq!(host.stats().invokes_dispatched, 0);

    // listPeerHostCapabilityManifests = remote proxy (product-seeded peers).
    let peers = client.list_peer_host_capability_manifests().await;
    match &peers {
        SpokeResult::Ok(manifests) => {
            let host_ids: Vec<&str> = manifests.iter().map(|m| m.host_id.as_str()).collect();
            assert_eq!(host_ids, vec!["host_tw_peer"]);
        }
        SpokeResult::Reject(_) => panic!("peer list must succeed"),
    }
    assert_eq!(host.stats().invokes_dispatched, 1);

    client.close();
    host.close();
}

#[tokio::test]
async fn attaches_configured_capability_token_as_auth_on_outbound_invokes() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs();
    let token = spoke_connect::core::issue_capability_token(
        &seed_client(),
        CapabilityClaims {
            iss: derive_peer_id_from_ed25519_pubkey(&pubkey_client()),
            sub: derive_peer_id_from_ed25519_pubkey(&pubkey_client()),
            aud: derive_peer_id_from_ed25519_pubkey(&pubkey_host()),
            capabilities: vec!["spoke-baseline".to_string()],
            exp: now + 3600,
            iat: None,
            jti: None,
        },
        now,
    )
    .expect("token issues");
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            capability_token: Some(token),
            ..Default::default()
        },
    )
    .await;

    let result = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(result.is_ok());
    // The host observed the auth field on the wire (attach path ran).
    assert!(host.stats().auth_seen);

    client.close();
    host.close();
}

#[tokio::test]
async fn fails_dial_fail_closed_when_remote_peer_is_not_on_allowlist() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let pair = loopback_transport_pair();
    let foreign_peer = derive_peer_id_from_ed25519_pubkey(&[0x70u8; 32]);
    let host = start_loopback_host(LoopbackHostOptions {
        transport: Arc::new(pair.server),
        host_seed: seed_host(),
        host_manifest: manifest("test-host", &["spoke-baseline"]),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_client())],
        adapter: Arc::new(host_adapter),
        delay: Box::new(|_| 0),
    })
    .await;

    let error = match connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(pair.client),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![foreign_peer], // wrong peer — fail-closed before any hello
        invoke_timeout_ms: Some(2000),
        capability_token: None,
    })
    .await
    {
        Ok(_) => panic!("dial must fail closed"),
        Err(error) => error,
    };
    assert!(
        matches!(error, RemoteAdapterError::Config(ref message) if message.contains("not on the allowlist"))
    );

    host.close();
}

#[tokio::test]
async fn fails_dial_when_host_rejects_client_hello() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let host_peer_id = derive_peer_id_from_ed25519_pubkey(&pubkey_host());
    let other_peer_id = derive_peer_id_from_ed25519_pubkey(&[0x20u8; 32]);
    let pair = loopback_transport_pair();
    let host = start_loopback_host(LoopbackHostOptions {
        transport: Arc::new(pair.server),
        host_seed: seed_host(),
        host_manifest: manifest("test-host", &["spoke-baseline"]),
        allowlist: vec![other_peer_id], // the real client is NOT allowed
        adapter: Arc::new(host_adapter),
        delay: Box::new(|_| 0),
    })
    .await;

    let error = match connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(pair.client),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![host_peer_id],
        invoke_timeout_ms: Some(2000),
        capability_token: None,
    })
    .await
    {
        Ok(_) => panic!("dial must fail when the host rejects the client hello"),
        Err(error) => error,
    };
    // The host closes the transport on a hello-gate failure, so the client
    // observes a handshake failure (never a half-open adapter).
    assert!(
        matches!(error, RemoteAdapterError::Handshake(_)),
        "unexpected dial error: {error:?}"
    );
    host.close();
}
