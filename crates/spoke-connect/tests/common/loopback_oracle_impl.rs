// Shared loopback host oracle impl (`loopback_oracle_impl.rs`); wrappers in `tests/common/loopback_oracle.rs` and `src/test_support/mod.rs`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::Serialize;
use serde_json::{json, Value};
use spoke_connect::core::{
    derive_peer_id_from_ed25519_pubkey, ed25519_pubkey_from_peer_id, is_allowlisted,
    required_capability, sign_hello_ed25519, verify_hello_ed25519, CapabilityClaims,
    CapabilityTokenProof, CoreInvokeError, InboundSequence, NonceStore,
};
use spoke_connect::remote::{
    connect_multi_peer_router, connect_remote_adapter, loopback_transport_pair,
    reset_accepted_server_hellos_for_test, MultiPeerRouterOptions, RemoteAdapter,
    RemoteAdapterError, RemoteAdapterOptions, RemoteIdentity, Transport, TransportError,
};
#[cfg(test)]
use spoke_fixture_toy_world::ToyWorldAdapter;
use spoke_operations::{
    orchestrate_check, orchestrate_upsert, spoke_ok, spoke_reject, BaselinePorts, CheckRunInput,
    FindingPort, HostManifestPort, KnowledgeEntryPort, RelationPort, RuleQueryPort, ScopeQueryPort,
    SpokeReject, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::connect::connect_hello::HostCapabilityManifest as ConnectHostCapabilityManifest;
use spoke_schemas::host_capability_manifest::HostCapabilityManifestExtensionsKey;
use spoke_schemas::connect::connect_invoke_request::ConnectInvokeRequest;
use spoke_schemas::connect::ConnectHello;
use spoke_schemas::connect::ConnectSession;
use spoke_schemas::{
    CheckRequest, Finding, HostCapabilityManifest, KnowledgeEntry, Scope, UpsertRequest,
};

/// Schema-conformant host manifest (mirror of TS `schemaConformantManifest`).
pub fn manifest(host_id: &str, capabilities: &[&str]) -> HostCapabilityManifest {
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

pub fn connect_manifest(manifest: &HostCapabilityManifest) -> ConnectHostCapabilityManifest {
    serde_json::from_value(serde_json::to_value(manifest).expect("manifest serializes"))
        .expect("field-identical manifest converts")
}

/// Minimal schema-valid provisional KnowledgeEntry for the upsert parity.
pub fn fresh_entry(entry_id: &str, canonical_name: &str) -> KnowledgeEntry {
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

pub fn upsert_request(entries: &[KnowledgeEntry]) -> UpsertRequest {
    serde_json::from_value(json!({ "knowledge_entries": entries })).expect("valid UpsertRequest")
}

pub fn check_request(scope_id: &str) -> CheckRequest {
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

pub fn host_nonce() -> String {
    let n = HOST_NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("test-host-nonce-{n:04}")
}

#[derive(Debug, Clone, Default)]
pub struct LoopbackHostStats {
    pub hellos_verified: usize,
    pub invokes_dispatched: usize,
    pub sequence_rejections: usize,
    pub auth_rejections: usize,
    pub dispatch_denials: usize,
    pub response_order: Vec<i64>,
    pub auth_seen: bool,
}

/// What the serialized inbound gate passes to the dispatch phase: the
/// session material the handler needs, extracted under the gate so the
/// dispatch phase can run concurrently.
struct GateOutcome {
    session_id: String,
    sequence: i64,
    request_id: String,
    negotiated_capabilities: Vec<String>,
}

struct HostSession {
    session_id: String,
    /// The client's hello Ed25519 public key (the key that verified the
    /// client's hello at establish) — inbound invoke request signatures are
    /// verified against it (envelope-auth contract §4/§7).
    client_pubkey: [u8; 32],
    negotiated_capabilities: Vec<String>,
    inbound: Mutex<InboundSequence>,
}

pub struct HostInner {
    transport: Arc<dyn Transport>,
    host_seed: [u8; 32],
    host_manifest: HostCapabilityManifest,
    host_peer_id: String,
    allowlist: Vec<String>,
    pub adapter: Arc<dyn BaselinePorts + Send + Sync>,
    requirements: HashMap<String, String>,
    delay: Box<dyn Fn(&ConnectInvokeRequest) -> u64 + Send + Sync>,
    /// Test-only: when a request maps to `Some(envelope)`, that envelope is
    /// sent verbatim instead of the host's real response (malformed-response
    /// fixtures, e.g. corrupted echo fields).
    response_override: Option<Box<dyn Fn(&ConnectInvokeRequest) -> Option<Value> + Send + Sync>>,
    /// Test-only: sign the session snapshot with these (initiator,
    /// responder) peer ids instead of the authenticated hellos' ids
    /// (session-binding fixtures — `envelope_auth_session_unbound`).
    session_peer_ids: Option<(String, String)>,
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

    /// Send a signed invoke-response envelope (schema-v2 requires the
    /// `signature` on every response branch). `branch` is the exact
    /// response branch object (`payload` success XOR `error`) — the same
    /// object becomes the signed object, mirroring the contract's
    /// branch-exact signed field set (`{session_id, sequence, request_id}`
    /// + branch). The echo fields come from the wire document, so reject
    /// paths that run before typed deserialization (sequence gate,
    /// envelope-auth verify) can still answer the sender.
    async fn send_signed_response(
        &self,
        session_id: &str,
        sequence: i64,
        request_id: &str,
        branch: Value,
    ) {
        self.stats
            .lock()
            .expect("stats lock")
            .response_order
            .push(sequence);
        let branch = branch.as_object().expect("branch is an object").clone();
        let mut signed = serde_json::Map::new();
        signed.insert("session_id".into(), json!(session_id));
        signed.insert("sequence".into(), json!(sequence));
        signed.insert("request_id".into(), json!(request_id));
        signed.extend(branch.clone());
        let signature = sign_envelope(&self.host_seed, &serde_json::Value::Object(signed));
        let mut wire = serde_json::Map::new();
        wire.insert("session_id".into(), json!(session_id));
        wire.insert("sequence".into(), json!(sequence));
        wire.insert("request_id".into(), json!(request_id));
        wire.extend(branch);
        wire.insert("extensions".into(), json!({}));
        wire.insert("signature".into(), json!(signature));
        self.send_envelope(&serde_json::Value::Object(wire)).await;
    }

    async fn send_error_response(
        &self,
        session_id: &str,
        sequence: i64,
        request_id: &str,
        code: &str,
        message: &str,
    ) {
        self.send_signed_response(
            session_id,
            sequence,
            request_id,
            json!({ "error": { "code": code, "message": message, "extensions": {} } }),
        )
        .await;
    }

    /// Answer with a signed `auth_failed` error branch carrying the locked
    /// `details.kind` (envelope-auth contract §8) for a request that failed
    /// envelope verification — no advance, no dispatch, no handler side
    /// effect.
    async fn send_auth_failed_response(
        &self,
        session_id: &str,
        sequence: i64,
        request_id: &str,
        error: &HostEnvelopeAuthError,
    ) {
        // The locked machine kind (contract §8): every wire rejection
        // carries its `details.kind`; the local key-misuse case (unreachable
        // here — the key is the client's verified hello key) carries none
        // and is encoded without `details.kind`.
        let mut error_obj = serde_json::Map::new();
        error_obj.insert("code".into(), json!("auth_failed"));
        error_obj.insert("message".into(), json!(error.message));
        if let Some(kind) = error.kind {
            error_obj.insert("details".into(), json!({ "kind": kind }));
        }
        error_obj.insert("extensions".into(), json!({}));
        self.send_signed_response(
            session_id,
            sequence,
            request_id,
            json!({ "error": Value::Object(error_obj) }),
        )
        .await;
    }

    async fn send_ok_response(
        &self,
        session_id: &str,
        sequence: i64,
        request_id: &str,
        payload: Value,
    ) {
        self.send_signed_response(
            session_id,
            sequence,
            request_id,
            json!({ "payload": payload }),
        )
        .await;
    }

    async fn send_reject_response(
        &self,
        session_id: &str,
        sequence: i64,
        request_id: &str,
        reject: &SpokeReject,
    ) {
        self.send_signed_response(
            session_id,
            sequence,
            request_id,
            json!({
                "error": {
                    "code": reject.code.as_str(),
                    "message": reject.message,
                    "details": reject.details.clone().unwrap_or_default(),
                    "extensions": {},
                },
            }),
        )
        .await;
    }

    /// Serialized inbound gate (mirror of the TS loopback host's
    /// `gateTail`). The serve loop calls this INLINE, so gates run in
    /// arrival (wire) order: peek → verify → advance complete per request
    /// before the next envelope's gate — a concurrent invoke can never
    /// peek against the pre-advance counter and be mis-rejected as
    /// `inbound_sequence_mismatch` (the async verify / rejection responses
    /// would otherwise interleave across spawned handlers). Gate rejections
    /// are answered here; `None` for stray envelopes and rejections,
    /// `Some(outcome)` when the invoke may dispatch.
    async fn run_gate(&self, wire: &Value) -> Option<GateOutcome> {
        // Session gates run under the std Mutex guard — all guard usage is
        // confined to this block so no MutexGuard crosses an await.
        let Some(session_id) = wire.get("session_id").and_then(Value::as_str) else {
            return None; // stray envelope — ignored
        };
        let Some(sequence) = wire.get("sequence").and_then(Value::as_i64) else {
            return None; // stray envelope — ignored
        };
        let Some(request_id) = wire
            .get("request_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return None; // stray envelope — ignored
        };
        let (session_id, client_pubkey, negotiated) = {
            let session_guard = self.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                return None; // stray request — ignored
            };
            if session_id != session.session_id {
                return None; // stray request — ignored
            }
            (
                session.session_id.clone(),
                session.client_pubkey,
                session.negotiated_capabilities.clone(),
            )
        };
        if wire.get("auth").is_some() {
            self.stats.lock().expect("stats lock").auth_seen = true;
        }

        // 1. Inbound sequence gate — non-mutating `peek` (auth-before-
        //    advance, contract §7): the wire position is validated WITHOUT
        //    consuming it, so a bogus-signature envelope cannot desync the
        //    session. A replay / out-of-order sequence fails here with no
        //    handler side effect.
        let peek_ok = {
            let session_guard = self.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                return None;
            };
            let inbound = session.inbound.lock().expect("inbound lock");
            inbound.peek(sequence).is_ok()
        };
        if !peek_ok {
            self.stats.lock().expect("stats lock").sequence_rejections += 1;
            self.send_error_response(
                &session_id,
                sequence,
                &request_id,
                "inbound_sequence_mismatch",
                &format!("inbound sequence {sequence} is not the next expected"),
            )
            .await;
            return None;
        }

        // 2. Envelope-auth verify (contract §7 — auth-before-advance): the
        //    request signature is verified over the wire form against the
        //    client's hello Ed25519 public key BEFORE the inbound counter
        //    advances. A forged / tampered / missing signature is answered
        //    `auth_failed` carrying the locked `details.kind`, and the
        //    session state is left untouched — no advance, no dispatch, no
        //    handler side effect.
        if let Err(error) = self.verify_invoke_request_auth(wire, &session_id, &client_pubkey) {
            self.stats.lock().expect("stats lock").auth_rejections += 1;
            self.send_auth_failed_response(&session_id, sequence, &request_id, &error)
                .await;
            return None;
        }

        // 3. Advance the inbound counter — `peek` validated the position
        //    above. The serialized gate makes the advance race-free; the
        //    rejection below is retained as defense-in-depth for a
        //    concurrent same-sequence injection (the fixture may hand two
        //    envelopes the same sequence). A lost race is non-fatal: answer
        //    the same `inbound_sequence_mismatch` rejection as the peek
        //    gate (counter increments, no advance, no dispatch) — never
        //    panic.
        let advance_rejected = {
            let session_guard = self.session.lock().expect("session lock");
            let Some(session) = session_guard.as_ref() else {
                return None;
            };
            let advance_result = session
                .inbound
                .lock()
                .expect("inbound lock")
                .advance(sequence);
            matches!(
                advance_result,
                // `advance` reports only `InboundSequenceMismatch` (see
                // `core::InboundSequence::advance`); a duplicate that lost
                // the race hits exactly this variant.
                Err(CoreInvokeError::InboundSequenceMismatch { .. })
            )
        };
        if advance_rejected {
            self.stats.lock().expect("stats lock").sequence_rejections += 1;
            self.send_error_response(
                &session_id,
                sequence,
                &request_id,
                "inbound_sequence_mismatch",
                &format!("inbound sequence {sequence} is not the next expected"),
            )
            .await;
            return None;
        }

        Some(GateOutcome {
            session_id,
            sequence,
            request_id,
            negotiated_capabilities: negotiated,
        })
    }

    /// Dispatch phase — runs concurrently per request (after the
    /// serialized gate): typed deserialize (graceful — never panic on wire
    /// input), dispatch gate, deterministic delay, response override,
    /// local dispatch.
    async fn handle_invoke(&self, wire: Value, outcome: GateOutcome) {
        let GateOutcome {
            session_id,
            sequence,
            request_id,
            negotiated_capabilities: negotiated,
        } = outcome;

        // The signature verified ⇒ the wire doc is a well-formed
        // `ConnectInvokeRequest` (the signer signed exactly the wire
        // fields); typed deserialization is expected to succeed but is NOT
        // infallible on wire input: `extensions` are unsigned per contract
        // §3, so a validly-signed envelope can carry an extension key
        // outside `^[a-z][a-z0-9_-]*$` or a non-object extension value that
        // fails the typed `ExtensionMap` deserialize (stripping them after
        // verify changes the typed-deserialize path). A decode failure
        // after a valid verify is a non-fatal protocol input error: answer
        // an error envelope — the verified envelope's sequence position is
        // already consumed — and never panic (mirrors the P3 advance-race
        // handling).
        let request: ConnectInvokeRequest = match serde_json::from_value(wire) {
            Ok(request) => request,
            Err(error) => {
                self.send_error_response(
                    &session_id,
                    sequence,
                    &request_id,
                    "invalid_request",
                    &format!("verified request does not deserialize: {error}"),
                )
                .await;
                return;
            }
        };

        // 4. Dispatch gate — product map first, then the core table; unknown
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
                &session_id,
                sequence,
                &request_id,
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

        // Test-only response override: replace the envelope the host would
        // send (malformed-response fixtures). The request already passed the
        // gates, so the client has a pending waiter to exercise against.
        // The override is still a schema-v2 response envelope — it carries
        // the host's signature over its own (possibly mangled) wire fields.
        if let Some(override_fn) = self.response_override.as_ref() {
            if let Some(envelope) = override_fn(&request) {
                self.stats
                    .lock()
                    .expect("stats lock")
                    .response_order
                    .push(sequence);
                let mut envelope = envelope;
                let mut signed = envelope
                    .as_object()
                    .expect("override envelope is an object")
                    .clone();
                signed.remove("extensions");
                signed.remove("signature");
                let signature =
                    sign_envelope(&self.host_seed, &serde_json::Value::Object(signed));
                envelope
                    .as_object_mut()
                    .expect("override envelope is an object")
                    .insert("signature".into(), json!(signature));
                self.send_envelope(&envelope).await;
                return;
            }
        }

        // 5. Dispatch to the local adapter.
        let result = self.dispatch_op(&request).await;
        self.stats.lock().expect("stats lock").invokes_dispatched += 1;
        match result {
            SpokeResult::Ok(payload) => {
                self.send_ok_response(&session_id, sequence, &request_id, payload)
                    .await
            }
            SpokeResult::Reject(reject) => {
                self.send_reject_response(&session_id, sequence, &request_id, &reject)
                    .await
            }
        }
    }

    /// Verify an inbound invoke request's envelope signature (contract §7
    /// steps 1–6) over the wire form, against the client's hello Ed25519
    /// public key (the key that verified the client's hello at establish —
    /// a session can only exist after that verify, so the
    /// signer-is-session-peer binding is the key itself). Mirrors
    /// `node.rs::verify_inbound_invoke_auth`. The inbound counter is NOT
    /// touched here (auth-before-advance): the caller advances only after
    /// this returns `Ok`.
    fn verify_invoke_request_auth(
        &self,
        wire: &Value,
        session_id: &str,
        client_pubkey: &[u8; 32],
    ) -> Result<(), HostEnvelopeAuthError> {
        verify_invoke_request_envelope(wire, session_id, client_pubkey)
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
        verify_hello_ed25519(&client_pubkey, &client_peer_id, &hello, None)
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
            client_pubkey,
            negotiated_capabilities: negotiated.clone(),
            inbound: Mutex::new(InboundSequence::new()),
        });

        // Answer with our signed hello, then the responder-assigned session
        // snapshot. The responder hello binds the dial: it signs the 5-field
        // object including `peer_nonce` = the initiator's nonce (dial
        // binding), so a replayed responder hello cannot re-enter a fresh
        // dial. The wire snapshot requires ≥1 negotiated capability; the
        // client derives its own negotiated set from the hellos, so the
        // fallback only covers the degenerate empty-intersection fixture.
        let hello = sign_hello_ed25519(
            &self.host_seed,
            &host_nonce(),
            &connect_manifest(&self.host_manifest),
            Some(hello.nonce.as_str()),
        )
        .map_err(|error| format!("host hello sign failed: {error}"))?;
        let bytes = serde_json::to_vec(&hello).map_err(|error| error.to_string())?;
        self.transport
            .send(&bytes)
            .await
            .map_err(|error| error.to_string())?;
        let snapshot: ConnectSession = {
            // The wire snapshot requires ≥1 negotiated capability; the
            // client derives its own negotiated set from the hellos, so the
            // fallback only covers the degenerate empty-intersection fixture.
            let wire_caps = if negotiated.is_empty() {
                vec!["spoke-baseline".to_string()]
            } else {
                negotiated
            };
            // Test-only session-binding fixture: sign the snapshot over
            // altered peer ids so the client's `verify_session_auth` step-6
            // binding assert fires `envelope_auth_session_unbound`.
            let (initiator_peer_id, responder_peer_id) = self
                .session_peer_ids
                .clone()
                .unwrap_or_else(|| (client_peer_id.clone(), self.host_peer_id.clone()));
            let signed = json!({
                "session_id": SESSION_ID,
                "initiator_peer_id": initiator_peer_id,
                "responder_peer_id": responder_peer_id,
                "opened_at": "2026-01-01T00:00:00Z",
                "negotiated_capabilities": wire_caps,
                "initial_sequence": 0,
            });
            // Schema-v2: the session snapshot carries the host's
            // `spoke-connect-session-jcs-v1` signature (the client's typed
            // deserialization requires it).
            let signature = sign_envelope(&self.host_seed, &signed);
            let mut wire = signed.as_object().expect("session object").clone();
            wire.insert("extensions".into(), json!({}));
            wire.insert("signature".into(), json!(signature));
            serde_json::from_value(serde_json::Value::Object(wire))
                .map_err(|error| error.to_string())?
        };
        let bytes = serde_json::to_vec(&snapshot).map_err(|error| error.to_string())?;
        self.transport
            .send(&bytes)
            .await
            .map_err(|error| error.to_string())?;

        // Serve invokes. The inbound gate (peek → verify → advance) runs
        // INLINE in this loop, so gates execute in arrival (wire) order —
        // a concurrent invoke can never peek against the pre-advance
        // counter and be mis-rejected as `inbound_sequence_mismatch`
        // (mirror of the TS loopback host's `gateTail`). Only the dispatch
        // phase is spawned, so handlers interleave (out-of-order
        // fixtures). Transport close ends the loop. The raw wire document
        // is handed to the gate so envelope-auth verify runs on the wire
        // form BEFORE typed deserialization (contract §7 / §1 — a missing
        // or invalid signature is rejected with its locked `details.kind`
        // instead of a generic serde error).
        while !self.closed.load(Ordering::SeqCst) {
            let bytes = match self.transport.recv().await {
                Ok(bytes) => bytes,
                Err(_) => return Ok(()),
            };
            let Ok(wire) = serde_json::from_slice::<Value>(&bytes) else {
                continue; // stray envelope — ignored
            };
            let Some(outcome) = self.run_gate(&wire).await else {
                continue; // stray / gate-rejected — already answered
            };
            let inner = Arc::clone(self);
            tokio::spawn(async move {
                inner.handle_invoke(wire, outcome).await;
            });
        }
        Ok(())
    }
}

pub struct LoopbackHost {
    pub inner: Arc<HostInner>,
}

impl LoopbackHost {
    pub fn session_id(&self) -> &str {
        SESSION_ID
    }

    pub fn stats(&self) -> LoopbackHostStats {
        self.inner.stats.lock().expect("stats lock").clone()
    }

    /// Test-only: the session's next expected inbound sequence — proves
    /// auth-before-advance (a rejected envelope does not consume the
    /// counter; `0` with no session).
    pub fn inbound_next_expected(&self) -> u64 {
        self.inner
            .session
            .lock()
            .expect("session lock")
            .as_ref()
            .map(|session| {
                session
                    .inbound
                    .lock()
                    .expect("inbound lock")
                    .next_expected()
            })
            .unwrap_or(0)
    }

    /// Close the connection (fails the client's pending recv / invokes).
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

pub struct LoopbackHostOptions {
    pub transport: Arc<dyn Transport>,
    pub host_seed: [u8; 32],
    pub host_manifest: HostCapabilityManifest,
    pub allowlist: Vec<String>,
    pub adapter: Arc<dyn BaselinePorts + Send + Sync>,
    pub delay: Box<dyn Fn(&ConnectInvokeRequest) -> u64 + Send + Sync>,
    pub response_override: Option<Box<dyn Fn(&ConnectInvokeRequest) -> Option<Value> + Send + Sync>>,
    pub session_peer_ids: Option<(String, String)>,
}

pub async fn start_loopback_host(options: LoopbackHostOptions) -> LoopbackHost {
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
        response_override: options.response_override,
        session_peer_ids: options.session_peer_ids,
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
pub struct DialOptions {
    pub allowlist: Option<Vec<String>>,
    pub client_manifest: Option<HostCapabilityManifest>,
    pub invoke_timeout_ms: Option<u64>,
    pub host_delay: Option<HostDelay>,
    pub host_allowlist: Option<Vec<String>>,
    pub host_response_override:
        Option<Box<dyn Fn(&ConnectInvokeRequest) -> Option<Value> + Send + Sync>>,
    pub capability_token: Option<CapabilityTokenProof>,
    /// Wrap the client's transport end (wire-level injector fixtures:
    /// tamper / strip signatures on outbound requests or inbound responses).
    /// Mirror of the TS dial's `clientTransport` option.
    pub client_transport:
        Option<Box<dyn Fn(Arc<dyn Transport>) -> Arc<dyn Transport> + Send + Sync>>,
}

pub fn seed_host() -> [u8; 32] {
    [0xa0u8; 32]
}

/// Sign the exact signed-object of a post-hello envelope with the host's
/// hello-identity seed: RFC 8785 JCS canonicalize → Ed25519 sign → canonical
/// base64url (exactly 86 chars). Mirrors the core `authenticate_*`
/// construction (envelope-auth contract §3/§5) so the loopback host emits
/// the schema-v2 wire form (`signature` required on every post-hello
/// envelope) the client's typed deserialization demands.
fn sign_envelope(secret: &[u8; 32], signed_object: &impl Serialize) -> String {
    let jcs_bytes = serde_jcs::to_vec(signed_object).expect("signed object JCS-canonicalizes");
    let signature = SigningKey::from_bytes(secret).sign(&jcs_bytes);
    URL_SAFE_NO_PAD.encode(signature.to_bytes())
}

/// Envelope-auth rejection surfaced by the loopback host's verify — the
/// locked `details.kind` + message for the wire `auth_failed` answer
/// (contract §8). Mirror of the core `EnvelopeAuthError` (that type is
/// `pub(crate)` — encapsulation HARD, contract §9 — and unreachable from
/// this integration test; the locked kinds + wire code are the contract's
/// surface).
#[derive(Debug)]
struct HostEnvelopeAuthError {
    /// The locked `details.kind` (`envelope_auth_missing` /
    /// `envelope_auth_invalid` / `envelope_auth_session_unbound`); `None`
    /// for the local key-misuse case, which carries no wire kind.
    kind: Option<&'static str>,
    message: String,
}

impl HostEnvelopeAuthError {
    fn missing(message: impl Into<String>) -> Self {
        Self {
            kind: Some("envelope_auth_missing"),
            message: message.into(),
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self {
            kind: Some("envelope_auth_invalid"),
            message: message.into(),
        }
    }

    fn session_unbound(message: impl Into<String>) -> Self {
        Self {
            kind: Some("envelope_auth_session_unbound"),
            message: message.into(),
        }
    }
}

/// Verify an inbound invoke request's envelope signature (contract §7 steps
/// 1–6) over the wire form, against the client's hello Ed25519 public key.
/// Mirrors the core `verify_invoke_request_auth` step-for-step — the core
/// helper is `pub(crate)` (encapsulation HARD, contract §9) and unreachable
/// from this integration test, so the loopback double re-derives the same
/// 7-step check; the core helper itself is pinned by golden vectors.
fn verify_invoke_request_envelope(
    wire: &Value,
    session_id: &str,
    client_pubkey: &[u8; 32],
) -> Result<(), HostEnvelopeAuthError> {
    // Steps 1–2 (locked order): `signature` present and non-empty
    // (`envelope_auth_missing`), then canonical base64url round-trip of
    // exactly 64 bytes (`envelope_auth_invalid`).
    let signature_field = wire.get("signature");
    let Some(Value::String(signature)) = signature_field else {
        return Err(HostEnvelopeAuthError::missing(
            "envelope is missing a signature",
        ));
    };
    if signature.is_empty() {
        return Err(HostEnvelopeAuthError::missing(
            "envelope is missing a signature",
        ));
    }
    let raw = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| HostEnvelopeAuthError::invalid("signature is not valid base64url"))?;
    if URL_SAFE_NO_PAD.encode(&raw) != signature.as_str() {
        return Err(HostEnvelopeAuthError::invalid(
            "signature is not canonical base64url (no padding)",
        ));
    }
    let raw: [u8; 64] = raw.try_into().map_err(|_| {
        HostEnvelopeAuthError::invalid("signature is not 64 bytes (86-char base64url expected)")
    })?;

    // Step 3: signed-object construction — exact keys (unknown key =
    // field-set drift ⇒ `envelope_auth_invalid`; missing signed field ⇒
    // `envelope_auth_invalid`); `auth` is trust-affecting and bound into
    // the signed object when present on the wire.
    let object = wire
        .as_object()
        .ok_or_else(|| HostEnvelopeAuthError::invalid("signed envelope is not a JSON object"))?;
    const WIRE_KEYS: &[&str] = &[
        "session_id",
        "sequence",
        "request_id",
        "op",
        "payload",
        "auth",
        "signature",
        "extensions",
    ];
    const SIGNED_KEYS: &[&str] = &["session_id", "sequence", "request_id", "op", "payload"];
    for key in object.keys() {
        if !WIRE_KEYS.contains(&key.as_str()) {
            return Err(HostEnvelopeAuthError::invalid(format!(
                "unknown key {key} in signed envelope (field-set drift)"
            )));
        }
    }
    for key in SIGNED_KEYS {
        if !object.contains_key(*key) {
            return Err(HostEnvelopeAuthError::invalid(format!(
                "missing signed field {key}"
            )));
        }
    }
    let mut signed = serde_json::Map::new();
    for key in SIGNED_KEYS {
        signed.insert((*key).to_string(), object[*key].clone());
    }
    if let Some(auth) = object.get("auth") {
        signed.insert("auth".to_string(), auth.clone());
    }

    // Steps 4–5: JCS-canonicalize the signed object and Ed25519-verify the
    // decoded signature against the client's hello key.
    let verifying_key = VerifyingKey::from_bytes(client_pubkey)
        .map_err(|_| HostEnvelopeAuthError::invalid("invalid Ed25519 public key"))?;
    let bytes = serde_jcs::to_vec(&serde_json::Value::Object(signed)).map_err(|error| {
        HostEnvelopeAuthError::invalid(format!("signed object is not JSON-serializable: {error}"))
    })?;
    let signature = Signature::from_bytes(&raw);
    if verifying_key.verify(&bytes, &signature).is_err() {
        return Err(HostEnvelopeAuthError::invalid("signature does not verify"));
    }

    // Step 6: session binding — the envelope's `session_id` must equal the
    // bound session (the signer-is-session-peer binding is the key itself,
    // since the session only exists after the client's hello verified).
    let wire_session_id = wire.get("session_id").and_then(Value::as_str);
    if wire_session_id != Some(session_id) {
        let wire_session_id = wire_session_id.unwrap_or("<non-string>");
        return Err(HostEnvelopeAuthError::session_unbound(format!(
            "session_id {wire_session_id} is not bound to session {session_id}"
        )));
    }
    Ok(())
}

pub fn seed_client() -> [u8; 32] {
    [0x10u8; 32]
}

pub fn pubkey_host() -> [u8; 32] {
    SigningKey::from_bytes(&seed_host())
        .verifying_key()
        .to_bytes()
}

pub fn pubkey_client() -> [u8; 32] {
    SigningKey::from_bytes(&seed_client())
        .verifying_key()
        .to_bytes()
}

/// Dial a client against a fresh loopback host serving `host_adapter`.
#[cfg(test)]
pub async fn dial(
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
        response_override: options.host_response_override,
        session_peer_ids: None,
    })
    .await;
    let client = connect_remote_adapter(RemoteAdapterOptions {
        transport: match options.client_transport {
            Some(wrap) => wrap(Arc::new(pair.client)),
            None => Arc::new(pair.client),
        },
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
