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
    auth_rejections: usize,
    dispatch_denials: usize,
    response_order: Vec<i64>,
    auth_seen: bool,
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

struct HostInner {
    transport: Arc<dyn Transport>,
    host_seed: [u8; 32],
    host_manifest: HostCapabilityManifest,
    host_peer_id: String,
    allowlist: Vec<String>,
    adapter: Arc<ToyWorldAdapter>,
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

    /// Test-only: the session's next expected inbound sequence — proves
    /// auth-before-advance (a rejected envelope does not consume the
    /// counter; `0` with no session).
    fn inbound_next_expected(&self) -> u64 {
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
    response_override: Option<Box<dyn Fn(&ConnectInvokeRequest) -> Option<Value> + Send + Sync>>,
    session_peer_ids: Option<(String, String)>,
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
struct DialOptions {
    allowlist: Option<Vec<String>>,
    client_manifest: Option<HostCapabilityManifest>,
    invoke_timeout_ms: Option<u64>,
    host_delay: Option<HostDelay>,
    host_allowlist: Option<Vec<String>>,
    host_response_override:
        Option<Box<dyn Fn(&ConnectInvokeRequest) -> Option<Value> + Send + Sync>>,
    capability_token: Option<CapabilityTokenProof>,
    /// Wrap the client's transport end (wire-level injector fixtures:
    /// tamper / strip signatures on outbound requests or inbound responses).
    /// Mirror of the TS dial's `clientTransport` option.
    client_transport:
        Option<Box<dyn Fn(Arc<dyn Transport>) -> Arc<dyn Transport> + Send + Sync>>,
}

fn seed_host() -> [u8; 32] {
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

// ── Greptile #1 fixture transports (replayed server hello) ────────────────

/// Delegating transport that records every inbound envelope (the view an
/// active transport attacker has after one legitimate dial).
struct RecordingTransport {
    inner: Arc<dyn Transport>,
    captured: Arc<Mutex<Vec<Vec<u8>>>>,
}

#[async_trait]
#[async_trait]
impl Transport for RecordingTransport {
    async fn send(&self, envelope: &[u8]) -> Result<(), TransportError> {
        self.inner.send(envelope).await
    }

    async fn recv(&self) -> Result<Vec<u8>, TransportError> {
        let bytes = self.inner.recv().await?;
        self.captured.lock().expect("captured lock").push(bytes.clone());
        Ok(bytes)
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.inner.close().await
    }
}

/// Wire-level injector transport wrapper (mirror of the TS
/// `tamperOutboundRequests` / `tamperInboundResponses` helpers): mutates
/// envelopes in one direction on the wire — the view an active transport
/// attacker has of the peers' signed envelopes. A `None` mutation returns
/// `None` to pass the envelope through unchanged (the handshake hello /
/// session snapshot traverse untouched so the dial still establishes).
struct TamperTransport {
    inner: Arc<dyn Transport>,
    /// Outbound (client → host) mutation; `None` = pass through.
    outbound: Option<Arc<dyn Fn(Value) -> Option<Value> + Send + Sync>>,
    /// Inbound (host → client) mutation; `None` = pass through.
    inbound: Option<Arc<dyn Fn(Value) -> Option<Value> + Send + Sync>>,
}

#[async_trait]
impl Transport for TamperTransport {
    async fn send(&self, envelope: &[u8]) -> Result<(), TransportError> {
        let Some(mutate) = self.outbound.as_ref() else {
            return self.inner.send(envelope).await;
        };
        let doc: Value = serde_json::from_slice(envelope).map_err(|_| TransportError::Closed)?;
        match mutate(doc) {
            Some(mutated) => {
                let bytes = serde_json::to_vec(&mutated).map_err(|_| TransportError::Closed)?;
                self.inner.send(&bytes).await
            }
            None => self.inner.send(envelope).await,
        }
    }

    async fn recv(&self) -> Result<Vec<u8>, TransportError> {
        let bytes = self.inner.recv().await?;
        let Some(mutate) = self.inbound.as_ref() else {
            return Ok(bytes);
        };
        let doc: Value = serde_json::from_slice(&bytes).map_err(|_| TransportError::Closed)?;
        match mutate(doc) {
            Some(mutated) => {
                Ok(serde_json::to_vec(&mutated).map_err(|_| TransportError::Closed)?)
            }
            None => Ok(bytes),
        }
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.inner.close().await
    }
}

/// Scripted transport that answers like an attacker replaying captured
/// envelopes: server hello, then session snapshot, then "connection closed".
struct ReplayTransport {
    hello: Vec<u8>,
    session: Vec<u8>,
    index: AtomicUsize,
}

#[async_trait]
#[async_trait]
impl Transport for ReplayTransport {
    async fn send(&self, _envelope: &[u8]) -> Result<(), TransportError> {
        Ok(())
    }

    async fn recv(&self) -> Result<Vec<u8>, TransportError> {
        let index = self.index.fetch_add(1, Ordering::SeqCst);
        match index {
            0 => Ok(self.hello.clone()),
            1 => Ok(self.session.clone()),
            _ => Err(TransportError::Closed),
        }
    }

    async fn close(&self) -> Result<(), TransportError> {
        Ok(())
    }
}

/// Scripted transport that answers like a mixed-version OLD responder: it
/// serves a 4-field signed hello (no `peer_nonce`) and nothing else — the
/// new initiator's fail-closed dial must reject at the hello, before the
/// session snapshot is even requested.
struct LegacyHelloTransport {
    hello: Vec<u8>,
}

#[async_trait]
impl Transport for LegacyHelloTransport {
    async fn send(&self, _envelope: &[u8]) -> Result<(), TransportError> {
        Ok(())
    }

    async fn recv(&self) -> Result<Vec<u8>, TransportError> {
        Ok(self.hello.clone())
    }

    async fn close(&self) -> Result<(), TransportError> {
        Ok(())
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
        response_override: None,
        session_peer_ids: None,
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
        response_override: None,
        session_peer_ids: None,
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

#[tokio::test]
async fn maps_correlation_mismatch_to_internal_error_kind_correlation_mismatch() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let mangled = Arc::new(AtomicBool::new(true));
    let mangled_clone = Arc::clone(&mangled);
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            // Same request_id (so the demux still finds the pending waiter)
            // but a wrong sequence echo — a correlation failure (§6 echo
            // rules). Fires once; the retry below uses the real response.
            host_response_override: Some(Box::new(move |request| {
                if !mangled_clone.swap(false, Ordering::SeqCst) {
                    return None;
                }
                Some(json!({
                    "session_id": request.session_id,
                    "sequence": request.sequence + 1,
                    "request_id": request.request_id,
                    "payload": {},
                    "extensions": {},
                }))
            })),
            ..Default::default()
        },
    )
    .await;

    let result = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(result.is_reject());
    assert_eq!(
        reject_kind(&result).as_deref(),
        Some("correlation_mismatch")
    );
    // Mismatch fails only the waiter — the session stays usable.
    assert_eq!(client.state().as_str(), "Established");
    let retry = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(retry.is_ok());

    client.close();
    host.close();
}

#[tokio::test]
async fn rejects_a_replayed_server_hello_before_any_session_is_accepted() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();

    // Dial 1 through a recording transport — the view an active transport
    // attacker has after one legitimate dial (server hello + session
    // snapshot captured at the wire).
    let pair = loopback_transport_pair();
    let captured: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let host = start_loopback_host(LoopbackHostOptions {
        transport: Arc::new(pair.server),
        host_seed: seed_host(),
        host_manifest: manifest("test-host", &["spoke-baseline"]),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_client())],
        adapter: Arc::new(host_adapter),
        delay: Box::new(|_| 0),
        response_override: None,
        session_peer_ids: None,
    })
    .await;
    let client = connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(RecordingTransport {
            inner: Arc::new(pair.client),
            captured: Arc::clone(&captured),
        }),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
        invoke_timeout_ms: Some(2000),
        capability_token: None,
    })
    .await
    .expect("first dial");
    assert_eq!(client.state().as_str(), "Established");
    client.close();
    host.close();
    let captured = captured.lock().expect("captured lock").clone();
    assert!(captured.len() >= 2, "captured hello + session snapshot");
    let replay_hello = captured[0].clone();
    let replay_session = captured[1].clone();

    // Dial 2: replay the captured envelopes through a scripted transport
    // with NO real host on the other end. Without receiver-side nonce
    // single-use this dial would succeed — the signature is genuinely the
    // allowlisted peer's — and the attacker could fabricate a session and
    // answer invokes; the fix rejects the replay before any
    // `ConnectSession` snapshot is accepted.
    let error = match connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(ReplayTransport {
            hello: replay_hello,
            session: replay_session,
            index: AtomicUsize::new(0),
        }),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
        invoke_timeout_ms: Some(2000),
        capability_token: None,
    })
    .await
    {
        Ok(_) => panic!("replayed server hello must be rejected"),
        Err(error) => error,
    };
    assert!(
        matches!(error, RemoteAdapterError::Handshake(ref message) if message.contains("replay") || message.contains("dial binding")),
        "unexpected dial error: {error:?}"
    );
}

#[tokio::test]
async fn replayed_server_hello_is_rejected_across_restart_by_dial_binding() {
    // Greptile P1 scenario, now defeated: a captured responder hello is
    // replayed on a FRESH dial after a "restart" — the process-wide
    // accepted-hello store is reset (in-memory state lost) and the initiator
    // nonce is new. The responder's signed `peer_nonce` (dial 1's initiator
    // nonce) does not match the new initiator nonce, so the dial-binding
    // assert rejects the replay even though the signature is genuinely the
    // allowlisted peer's and the nonce store has forgotten the pair.
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();

    // Dial 1 through a recording transport — capture the responder hello.
    let pair = loopback_transport_pair();
    let captured: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let host = start_loopback_host(LoopbackHostOptions {
        transport: Arc::new(pair.server),
        host_seed: seed_host(),
        host_manifest: manifest("test-host", &["spoke-baseline"]),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_client())],
        adapter: Arc::new(host_adapter),
        delay: Box::new(|_| 0),
        response_override: None,
        session_peer_ids: None,
    })
    .await;
    let client = connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(RecordingTransport {
            inner: Arc::new(pair.client),
            captured: Arc::clone(&captured),
        }),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
        invoke_timeout_ms: Some(2000),
        capability_token: None,
    })
    .await
    .expect("first dial");
    assert_eq!(client.state().as_str(), "Established");
    client.close();
    host.close();
    let captured = captured.lock().expect("captured lock").clone();
    assert!(captured.len() >= 2, "captured hello + session snapshot");
    let replay_hello = captured[0].clone();
    let replay_session = captured[1].clone();

    // "Restart": the in-memory accepted-hello store resets.
    reset_accepted_server_hellos_for_test();

    // Dial 2: fresh initiator nonce, replay the captured responder hello
    // through a scripted transport with NO real host. The receiver-side
    // nonce gate cannot help (store reset) — only the dial-binding assert
    // (signed `peer_nonce` != new initiator nonce) rejects the replay.
    let error = match connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(ReplayTransport {
            hello: replay_hello,
            session: replay_session,
            index: AtomicUsize::new(0),
        }),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
        invoke_timeout_ms: Some(2000),
        capability_token: None,
    })
    .await
    {
        Ok(_) => panic!("replayed server hello must be rejected after a restart"),
        Err(error) => error,
    };
    assert!(
        matches!(error, RemoteAdapterError::Handshake(ref message) if message.contains("dial binding")),
        "unexpected dial error: {error:?}"
    );
}

#[tokio::test]
async fn responder_hello_without_peer_nonce_fails_the_dial_closed() {
    // Mixed-version downgrade: an OLD responder (pre-dial-binding) signs the
    // 4-field initiator object — no `peer_nonce` on the wire, with a
    // genuinely valid signature from the allowlisted host key. The NEW
    // initiator dial expects a responder (it supplies its own nonce), so the
    // missing `peer_nonce` must fail the dial closed — not silently skip
    // the binding assert (fail-open downgrade).
    let legacy_hello = sign_hello_ed25519(
        &seed_host(),
        &host_nonce(),
        &connect_manifest(&manifest("test-host", &["spoke-baseline"])),
        None,
    )
    .expect("sign legacy 4-field hello");
    let legacy_bytes = serde_json::to_vec(&legacy_hello).expect("legacy hello serializes");

    let error = match connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(LegacyHelloTransport {
            hello: legacy_bytes,
        }),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
        invoke_timeout_ms: Some(2000),
        capability_token: None,
    })
    .await
    {
        Ok(_) => panic!("responder hello without peer_nonce must fail the dial"),
        Err(error) => error,
    };
    assert!(
        matches!(&error, RemoteAdapterError::Handshake(message) if message.contains("dial binding")),
        "unexpected dial error: {error:?}"
    );
}

// ── Envelope-auth enforcement (contract §7/§8) ─────────────────────────────

/// Extract the `details.kind` of an `INTERNAL_ERROR` reject (or `None`).
fn reject_kind_of(result: &SpokeResult<KnowledgeEntry>) -> Option<&str> {
    match result {
        SpokeResult::Reject(reject) => reject
            .details
            .as_ref()
            .and_then(|details| details.get("kind"))
            .and_then(Value::as_str),
        SpokeResult::Ok(_) => None,
    }
}

#[tokio::test]
async fn host_rejects_a_wire_tampered_invoke_request_with_auth_failed_and_no_advance_or_dispatch() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let tampered = Arc::new(AtomicBool::new(false));
    let tampered_flag = Arc::clone(&tampered);
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            // One-shot wire-level injector: only the FIRST outbound invoke
            // request's payload is mutated AFTER the signature was computed
            // — the host's envelope-auth verify must reject it.
            client_transport: Some(Box::new(move |client_end| {
                let tampered_flag = Arc::clone(&tampered_flag);
                Arc::new(TamperTransport {
                    inner: client_end,
                    outbound: Some(Arc::new(move |doc| {
                        if doc.get("op").is_some() && !tampered_flag.swap(true, Ordering::SeqCst) {
                            let mut doc = doc;
                            doc["payload"]["tampered"] = json!(true);
                            return Some(doc);
                        }
                        None
                    })),
                    inbound: None,
                })
            })),
            ..Default::default()
        },
    )
    .await;

    // The host answered `auth_failed` (wire code) with the locked
    // `details.kind`; the client maps it to `INTERNAL_ERROR` verbatim.
    let result = client.get_knowledge_entry("kb_tw_mira").await;
    assert_eq!(reject_kind_of(&result), Some("envelope_auth_invalid"));
    assert!(matches!(
        &result,
        SpokeResult::Reject(reject) if reject.code == SpokeRejectCode::InternalError
    ));

    // Auth-before-advance: the forged envelope consumed nothing. The
    // host's inbound counter is still at 0 (the next expected sequence was
    // NOT consumed) and the host's counters prove the forged envelope was
    // neither dispatched nor counted as a sequence rejection. (The client's
    // own outbound counter has moved on — protocol v1 defines no retry —
    // so the session is deliberately not reused here, mirroring the TS
    // twin.)
    let stats = host.stats();
    assert_eq!(host.inbound_next_expected(), 0);
    assert_eq!(stats.auth_rejections, 1);
    assert_eq!(stats.sequence_rejections, 0);
    assert_eq!(stats.invokes_dispatched, 0);

    client.close();
    host.close();
}

#[tokio::test]
async fn host_rejects_an_invoke_request_with_a_stripped_signature_as_auth_failed() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let stripped = Arc::new(AtomicBool::new(false));
    let stripped_flag = Arc::clone(&stripped);
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            // Wire-level injector: strip the signature from the FIRST
            // outbound invoke request — the host must answer `auth_failed`
            // with `envelope_auth_missing` (v2 requires the signature on
            // every post-hello envelope; a missing authenticator is never
            // dispatched).
            client_transport: Some(Box::new(move |client_end| {
                let stripped_flag = Arc::clone(&stripped_flag);
                Arc::new(TamperTransport {
                    inner: client_end,
                    outbound: Some(Arc::new(move |doc| {
                        if doc.get("op").is_some() && !stripped_flag.swap(true, Ordering::SeqCst) {
                            let mut doc = doc;
                            if let Some(object) = doc.as_object_mut() {
                                object.remove("signature");
                            }
                            return Some(doc);
                        }
                        None
                    })),
                    inbound: None,
                })
            })),
            ..Default::default()
        },
    )
    .await;

    let result = client.get_knowledge_entry("kb_tw_mira").await;
    assert_eq!(reject_kind_of(&result), Some("envelope_auth_missing"));
    assert!(matches!(
        &result,
        SpokeResult::Reject(reject) if reject.code == SpokeRejectCode::InternalError
    ));

    // No advance, no dispatch — the host's inbound counter is untouched
    // (next expected still 0) and the stripped envelope was never
    // dispatched nor counted as a sequence rejection.
    let stats = host.stats();
    assert_eq!(host.inbound_next_expected(), 0);
    assert_eq!(stats.auth_rejections, 1);
    assert_eq!(stats.sequence_rejections, 0);
    assert_eq!(stats.invokes_dispatched, 0);

    client.close();
    host.close();
}

#[tokio::test]
async fn rejects_a_wire_tampered_invoke_response_with_envelope_auth_invalid() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let tampered = Arc::new(AtomicBool::new(false));
    let tampered_flag = Arc::clone(&tampered);
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            // One-shot wire-level injector: only the FIRST response's
            // payload is mutated after the host signed it — the client's
            // envelope-auth verify must reject it (fail-closed, only this
            // waiter).
            client_transport: Some(Box::new(move |client_end| {
                let tampered_flag = Arc::clone(&tampered_flag);
                Arc::new(TamperTransport {
                    inner: client_end,
                    outbound: None,
                    inbound: Some(Arc::new(move |doc| {
                        if doc.get("request_id").is_some()
                            && doc.get("payload").is_some()
                            && !tampered_flag.swap(true, Ordering::SeqCst)
                        {
                            let mut doc = doc;
                            doc["payload"]["tampered"] = json!(true);
                            return Some(doc);
                        }
                        None
                    })),
                })
            })),
            ..Default::default()
        },
    )
    .await;

    // The client's `verify_invoke_response_auth` rejects the tampered
    // response with the locked `details.kind` via `INTERNAL_ERROR`.
    let result = client.get_knowledge_entry("kb_tw_mira").await;
    assert_eq!(reject_kind_of(&result), Some("envelope_auth_invalid"));
    assert!(matches!(
        &result,
        SpokeResult::Reject(reject) if reject.code == SpokeRejectCode::InternalError
    ));

    // A forged response fails only this waiter — no session-state mutation:
    // the next invoke round-trips (the host dispatched the first request
    // normally; only the response was tampered with).
    let retry = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(retry.is_ok());
    let stats = host.stats();
    assert_eq!(stats.invokes_dispatched, 2);
    assert_eq!(client.state().as_str(), "Established");

    client.close();
    host.close();
}

#[tokio::test]
async fn rejects_an_invoke_response_with_a_stripped_signature_as_envelope_auth_missing() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let stripped = Arc::new(AtomicBool::new(false));
    let stripped_flag = Arc::clone(&stripped);
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            // Wire-level injector: delete the `signature` field of the FIRST
            // response — the client must fail closed with
            // `envelope_auth_missing` (v2 requires the signature on every
            // response branch).
            client_transport: Some(Box::new(move |client_end| {
                let stripped_flag = Arc::clone(&stripped_flag);
                Arc::new(TamperTransport {
                    inner: client_end,
                    outbound: None,
                    inbound: Some(Arc::new(move |doc| {
                        if doc.get("request_id").is_some()
                            && !stripped_flag.swap(true, Ordering::SeqCst)
                        {
                            let mut doc = doc;
                            if let Some(object) = doc.as_object_mut() {
                                object.remove("signature");
                            }
                            return Some(doc);
                        }
                        None
                    })),
                })
            })),
            ..Default::default()
        },
    )
    .await;

    let result = client.get_knowledge_entry("kb_tw_mira").await;
    assert_eq!(reject_kind_of(&result), Some("envelope_auth_missing"));
    assert!(matches!(
        &result,
        SpokeResult::Reject(reject) if reject.code == SpokeRejectCode::InternalError
    ));

    let retry = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(retry.is_ok());
    assert_eq!(client.state().as_str(), "Established");

    client.close();
    host.close();
}

#[tokio::test]
async fn fails_the_dial_when_the_session_snapshot_signature_is_stripped() {
    // Wire-level injector on the client end: strip the `signature` from the
    // session snapshot — the dial must fail closed at the snapshot verify
    // (`verify_session_auth` runs before typed checks / before establish;
    // contract §7 — a v2 snapshot without a valid authenticator never
    // establishes a session).
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let stripped = Arc::new(AtomicBool::new(false));
    let stripped_flag = Arc::clone(&stripped);
    let pair = loopback_transport_pair();
    let host = start_loopback_host(LoopbackHostOptions {
        transport: Arc::new(pair.server),
        host_seed: seed_host(),
        host_manifest: manifest("test-host", &["spoke-baseline"]),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_client())],
        adapter: Arc::new(host_adapter),
        delay: Box::new(|_| 0),
        response_override: None,
        session_peer_ids: None,
    })
    .await;

    let error = match connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(TamperTransport {
            inner: Arc::new(pair.client),
            outbound: None,
            inbound: Some(Arc::new(move |doc| {
                // Session shape only — the signed server hello (no
                // `session_id`) passes through so the dial reaches the
                // snapshot step.
                if doc.get("session_id").is_some()
                    && doc.get("initiator_peer_id").is_some()
                    && doc.get("request_id").is_none()
                    && !stripped_flag.swap(true, Ordering::SeqCst)
                {
                    let mut doc = doc;
                    if let Some(object) = doc.as_object_mut() {
                        object.remove("signature");
                    }
                    return Some(doc);
                }
                None
            })),
        }),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
        invoke_timeout_ms: Some(2000),
        capability_token: None,
    })
    .await
    {
        Ok(_) => panic!("dial must fail closed on a stripped session snapshot signature"),
        Err(error) => error,
    };
    assert!(
        matches!(&error, RemoteAdapterError::Handshake(message) if message.contains("missing a signature")),
        "unexpected dial error: {error:?}"
    );

    host.close();
}

#[tokio::test]
async fn rejects_a_host_signed_response_with_wrong_session_id_fail_closed() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let fired = Arc::new(AtomicBool::new(false));
    let fired_flag = Arc::clone(&fired);
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            // The host signs its own response envelope — but over a
            // `session_id` that is not bound to the established session
            // (the override is signed by the host after the request passed
            // every inbound gate). The signature verifies against the
            // host's hello key; the response still fails closed.
            //
            // NOTE: the surfaced kind is `correlation_mismatch`, not
            // `envelope_auth_session_unbound`: `session_id` is one of the
            // three correlation echo fields, and the locked check order
            // (mirrored in TS) runs the correlation echo check BEFORE
            // envelope-auth verify — so a wrong `session_id` is caught by
            // the correlation gate first. The session-binding kind is
            // covered end-to-end by
            // `fails_the_dial_when_the_session_snapshot_carries_unbound_peer_ids`
            // and at the core level by `verify_invoke_response_auth` unit
            // tests. Fires once; the retry below uses the real response.
            host_response_override: Some(Box::new(move |request| {
                if !fired_flag.swap(true, Ordering::SeqCst) {
                    return Some(json!({
                        "session_id": "not-the-established-session",
                        "sequence": request.sequence,
                        "request_id": request.request_id,
                        "payload": {},
                        "extensions": {},
                    }));
                }
                None
            })),
            ..Default::default()
        },
    )
    .await;

    // The wrong-session response fails only this waiter — correlation
    // mismatch, fail-closed, no session-state mutation.
    let result = client.get_knowledge_entry("kb_tw_mira").await;
    assert_eq!(reject_kind_of(&result), Some("correlation_mismatch"));
    assert!(matches!(
        &result,
        SpokeResult::Reject(reject) if reject.code == SpokeRejectCode::InternalError
    ));

    // The session stays established and the next invoke round-trips
    // normally.
    let retry = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(retry.is_ok());
    assert_eq!(client.state().as_str(), "Established");

    client.close();
    host.close();
}

#[tokio::test]
async fn fails_the_dial_when_the_session_snapshot_carries_unbound_peer_ids() {
    // The host signs a `ConnectSession` snapshot whose responder peer id is
    // not one of the authenticated hellos' peer ids. The client's
    // `verify_session_auth` verifies the host's signature over the wire
    // form and then fires the step-6 session-binding assert
    // (`envelope_auth_session_unbound`) — the dial fails closed, no
    // adapter instance is created.
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
        response_override: None,
        session_peer_ids: Some((
            derive_peer_id_from_ed25519_pubkey(&pubkey_client()),
            foreign_peer.clone(),
        )),
    })
    .await;

    let error = match connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(pair.client),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![derive_peer_id_from_ed25519_pubkey(&pubkey_host())],
        invoke_timeout_ms: Some(2000),
        capability_token: None,
    })
    .await
    {
        Ok(_) => panic!("unbound-peer-id snapshot must fail the dial"),
        Err(error) => error,
    };
    // `verify_session_auth` step 6 fired `EnvelopeAuthError::SessionUnbound`
    // (host-signed snapshot, binding mismatch) — the message names the
    // unbound peer id and the authenticated hellos.
    assert!(
        matches!(&error, RemoteAdapterError::Handshake(message) if message.contains("session peer ids")),
        "unexpected dial error: {error:?}"
    );

    host.close();
}

/// Wire-level transport wrapper that duplicates the FIRST outbound invoke
/// request (an envelope carrying `op`) — the host then serves two
/// same-sequence envelopes concurrently (one task per envelope, §10
/// concurrency rows), racing peek → verify → advance.
struct DuplicateOnceTransport {
    inner: Arc<dyn Transport>,
    duplicated: AtomicBool,
}

#[async_trait]
impl Transport for DuplicateOnceTransport {
    async fn send(&self, envelope: &[u8]) -> Result<(), TransportError> {
        let doc: Value = serde_json::from_slice(envelope).map_err(|_| TransportError::Closed)?;
        if doc.get("op").is_some() && !self.duplicated.swap(true, Ordering::SeqCst) {
            // Deliver the request twice: both copies hit the host's
            // serve loop and run `handle_invoke` concurrently.
            self.inner.send(envelope).await?;
            return self.inner.send(envelope).await;
        }
        self.inner.send(envelope).await
    }

    async fn recv(&self) -> Result<Vec<u8>, TransportError> {
        self.inner.recv().await
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.inner.close().await
    }
}

#[tokio::test]
async fn concurrent_same_sequence_duplicate_is_rejected_non_fatally() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            // Duplicate the first invoke request on the wire: the host
            // races two same-sequence envelopes through
            // peek → verify → advance. Exactly one wins; the loser must be
            // answered `inbound_sequence_mismatch` (non-fatal, counter
            // increments) — the fixture must NOT panic on the lost race.
            client_transport: Some(Box::new(|client_end| {
                Arc::new(DuplicateOnceTransport {
                    inner: client_end,
                    duplicated: AtomicBool::new(false),
                })
            })),
            ..Default::default()
        },
    )
    .await;

    // The invoke settles exactly once (ok or reject — response order of the
    // payload vs. the mismatch error is not deterministic), the host
    // dispatched exactly one handler, and the duplicate was rejected as a
    // sequence mismatch — never a panic, never a poisoned session.
    let _result = client.get_knowledge_entry("kb_tw_mira").await;
    let stats = host.stats();
    assert_eq!(stats.invokes_dispatched, 1);
    assert_eq!(stats.sequence_rejections, 1);
    assert_eq!(stats.auth_rejections, 0);
    assert_eq!(client.state().as_str(), "Established");

    // The session remains usable: the next invoke round-trips normally.
    let retry = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(retry.is_ok());
    assert_eq!(host.stats().invokes_dispatched, 2);
    assert_eq!(client.state().as_str(), "Established");

    client.close();
    host.close();
}

// ── Fix-wave 1: outbound send serialization + post-verify decode grace ────

/// Wire-level transport wrapper that yields inside `send` before delegating
/// — widens the interleave window at the adapter's `transport.send().await`
/// yield point. Without outbound send serialization, a later-allocated
/// request can reach the wire first and the host's strict inbound sequence
/// gate answers `inbound_sequence_mismatch` (contract §5.3 / §10).
struct YieldingSendTransport {
    inner: Arc<dyn Transport>,
}

#[async_trait]
impl Transport for YieldingSendTransport {
    async fn send(&self, envelope: &[u8]) -> Result<(), TransportError> {
        // Yield before pushing: forces a scheduling point inside the
        // adapter's send so concurrent invokes genuinely race for the wire.
        tokio::task::yield_now().await;
        self.inner.send(envelope).await
    }

    async fn recv(&self) -> Result<Vec<u8>, TransportError> {
        self.inner.recv().await
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.inner.close().await
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_invokes_reach_the_wire_in_allocation_order() {
    const CONCURRENT_INVOKES: i64 = 32;

    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let peer_id_host = derive_peer_id_from_ed25519_pubkey(&pubkey_host());
    let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());

    // Replicates `dial()` with a server-end recording wrapper so the wire
    // order of the client's outbound invoke requests is observable.
    let pair = loopback_transport_pair();
    let captured: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let host = start_loopback_host(LoopbackHostOptions {
        transport: Arc::new(RecordingTransport {
            inner: Arc::new(pair.server),
            captured: Arc::clone(&captured),
        }),
        host_seed: seed_host(),
        host_manifest: manifest("test-host", &["spoke-baseline"]),
        allowlist: vec![peer_id_client.clone()],
        adapter: Arc::new(host_adapter),
        delay: Box::new(|_| 0),
        response_override: None,
        session_peer_ids: None,
    })
    .await;
    let client = connect_remote_adapter(RemoteAdapterOptions {
        // Yield inside every send so concurrent invokes race for the wire
        // position (without serialization the host would see a
        // later-allocated sequence first and reject it).
        transport: Arc::new(YieldingSendTransport {
            inner: Arc::new(pair.client),
        }),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: pubkey_host(),
        allowlist: vec![peer_id_host.clone()],
        invoke_timeout_ms: Some(5000),
        capability_token: None,
    })
    .await
    .expect("dial");

    // Fire all invokes concurrently; each allocates its outbound sequence
    // in call order. Serialized sends must put them on the wire 0..N-1.
    let mut handles = Vec::new();
    for _ in 0..CONCURRENT_INVOKES {
        let client = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            client.get_knowledge_entry("kb_tw_mira").await
        }));
    }
    for handle in handles {
        let result = handle.await.expect("invoke task must not panic");
        assert!(result.is_ok(), "concurrent invoke must succeed: {result:?}");
    }

    // The host accepted and dispatched every request in sequence — no
    // out-of-order rejection, no auth rejection.
    let stats = host.stats();
    assert_eq!(stats.invokes_dispatched, CONCURRENT_INVOKES as usize);
    assert_eq!(stats.sequence_rejections, 0);
    assert_eq!(stats.auth_rejections, 0);

    // Wire witness: the request envelopes the host received carry exactly
    // the monotonic allocation order 0..N-1.
    let captured = captured.lock().expect("captured lock").clone();
    let sequences: Vec<i64> = captured
        .iter()
        .filter_map(|bytes| {
            let doc: Value = serde_json::from_slice(bytes).ok()?;
            doc.get("op").is_some().then(|| doc.get("sequence")?.as_i64())
        })
        .flatten()
        .collect();
    assert_eq!(
        sequences,
        (0..CONCURRENT_INVOKES).collect::<Vec<i64>>(),
        "wire request sequences must be monotonic (allocation order)"
    );

    client.close();
    host.close();
}

#[tokio::test]
async fn host_answers_invalid_request_non_fatally_when_verified_extensions_are_malformed() {
    let host_adapter = ToyWorldAdapter::with_committed_fixtures();
    let mutated = Arc::new(AtomicBool::new(false));
    let mutated_flag = Arc::clone(&mutated);
    let (client, host) = dial(
        host_adapter,
        DialOptions {
            // One-shot wire-level injector: on the FIRST invoke request,
            // replace the (unsigned) `extensions` bag with a value whose
            // key violates the schema pattern `^[a-z][a-z0-9_-]*$`. The
            // signature still verifies (extensions are not in the signed
            // object, contract §3), so the request passes envelope-auth
            // verify and then fails typed deserialization.
            client_transport: Some(Box::new(move |client_end| {
                let mutated_flag = Arc::clone(&mutated_flag);
                Arc::new(TamperTransport {
                    inner: client_end,
                    outbound: Some(Arc::new(move |doc| {
                        if doc.get("op").is_some() && !mutated_flag.swap(true, Ordering::SeqCst)
                        {
                            let mut doc = doc;
                            if let Some(object) = doc.as_object_mut() {
                                object.insert(
                                    "extensions".into(),
                                    json!({ "Bad_Key": { "n": 1 } }),
                                );
                            }
                            Some(doc)
                        } else {
                            None
                        }
                    })),
                    inbound: None,
                })
            })),
            ..Default::default()
        },
    )
    .await;

    // The host must NOT panic on the post-verify decode failure — it
    // answers a non-fatal `invalid_request` error envelope instead.
    let result = client.get_knowledge_entry("kb_tw_mira").await;
    match &result {
        SpokeResult::Reject(reject) => {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(Value::as_str),
                Some("invalid_request")
            );
        }
        SpokeResult::Ok(_) => panic!("malformed unsigned extensions must be rejected"),
    }
    let stats = host.stats();
    assert_eq!(stats.invokes_dispatched, 0);
    assert_eq!(stats.auth_rejections, 0);
    assert_eq!(stats.sequence_rejections, 0);
    // The verified envelope's sequence position was consumed (advance
    // happened before typed deserialization).
    assert_eq!(host.inbound_next_expected(), 1);
    assert_eq!(client.state().as_str(), "Established");

    // The session stays usable: the next invoke round-trips normally.
    let retry = client.get_knowledge_entry("kb_tw_mira").await;
    assert!(retry.is_ok());
    assert_eq!(host.stats().invokes_dispatched, 1);
    assert_eq!(client.state().as_str(), "Established");

    client.close();
    host.close();
}

// ── Multi-peer router loopback proof (contract §9) ────────────────────────

/// Dial a client against a fresh loopback host with a CUSTOM host identity
/// seed + host manifest — the multi-peer proof needs distinct remote peer
/// ids (tie-break) and disjoint capability manifests (routing), which the
/// fixed `dial` fixture cannot express. The client side keeps the standard
/// fixture identity; per-peer session state stays independent (contract §1).
async fn dial_peer(
    host_seed: [u8; 32],
    host_manifest: HostCapabilityManifest,
) -> (Arc<RemoteAdapter>, LoopbackHost) {
    let host_pubkey = SigningKey::from_bytes(&host_seed)
        .verifying_key()
        .to_bytes();
    let peer_id_host = derive_peer_id_from_ed25519_pubkey(&host_pubkey);
    let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());

    let pair = loopback_transport_pair();
    let host = start_loopback_host(LoopbackHostOptions {
        transport: Arc::new(pair.server),
        host_seed,
        host_manifest,
        allowlist: vec![peer_id_client.clone()],
        adapter: Arc::new(ToyWorldAdapter::default()),
        delay: Box::new(|_| 0),
        response_override: None,
        session_peer_ids: None,
    })
    .await;
    let client = connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(pair.client),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest("test-client", &["spoke-baseline"]),
        remote_pubkey: host_pubkey,
        allowlist: vec![peer_id_host],
        invoke_timeout_ms: None,
        capability_token: None,
    })
    .await
    .expect("dial");
    (client, host)
}

#[tokio::test]
async fn multi_peer_router_routes_upsert_and_check_to_the_capable_peer() {
    // Two peers with DISJOINT capabilities: baseline vs l2-computable.
    let (baseline_adapter, baseline_host) =
        dial_peer([0xa1; 32], manifest("host-baseline", &["spoke-baseline"])).await;
    let (computable_adapter, computable_host) =
        dial_peer([0xb2; 32], manifest("host-computable", &["l2-computable"])).await;

    let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
    let baseline_id = router
        .register_peer(baseline_adapter.clone())
        .expect("register baseline");
    let computable_id = router
        .register_peer(computable_adapter.clone())
        .expect("register computable");
    // Registry holds both; selection (not registration order) routes.
    assert_eq!(router.list_peers(), vec![baseline_id, computable_id]);

    // orchestrateUpsert → port.knowledge.get + port.knowledge.put — both
    // baseline ops → the spoke-baseline peer; the l2-computable peer is
    // never touched.
    let request = upsert_request(&[fresh_entry("kb_mpr_upsert", "Multi-Peer Upsert")]);
    let upsert = orchestrate_upsert(&router, request).await;
    assert!(
        upsert.is_ok(),
        "upsert must route to the baseline peer: {upsert:?}"
    );
    assert_eq!(baseline_host.stats().invokes_dispatched, 2);
    assert_eq!(computable_host.stats().invokes_dispatched, 0);
    // The remote write actually landed on the selected peer's host store.
    assert!(baseline_host
        .inner
        .adapter
        .with_store(|store| store.entries.contains_key("kb_mpr_upsert")));

    // orchestrateCheck → listKnowledgeEntries + listTimelineEvents +
    // putFindings (listRules is skipped: the fixture request carries no
    // rule_refs) — all baseline ops → the same peer.
    let checker = |_: CheckRunInput| spoke_ok(Vec::<Finding>::new());
    let check = orchestrate_check(&router, check_request("toy-scope-001"), checker).await;
    assert!(
        check.is_ok(),
        "check must route to the baseline peer: {check:?}"
    );
    assert_eq!(baseline_host.stats().invokes_dispatched, 5);
    assert_eq!(computable_host.stats().invokes_dispatched, 0);

    baseline_adapter.close();
    computable_adapter.close();
    baseline_host.close();
    computable_host.close();
}

#[tokio::test]
async fn multi_peer_router_rejects_no_capable_peer_when_no_peer_has_the_capability() {
    // Only an l2-computable peer registered — every baseline op on the
    // router's six-family surface has no capable peer.
    let (computable_adapter, computable_host) =
        dial_peer([0xb2; 32], manifest("host-computable", &["l2-computable"])).await;

    let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
    router
        .register_peer(computable_adapter.clone())
        .expect("register computable");

    let request = upsert_request(&[fresh_entry("kb_mpr_nomatch", "No Match")]);
    let result = orchestrate_upsert(&router, request).await;
    match &result {
        SpokeResult::Reject(reject) => {
            // §5 locked reject: CAPABILITY_PORT_MISSING + details.kind /
            // wire_code = no_capable_peer — terminal, stable.
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("kind"))
                    .and_then(Value::as_str),
                Some("no_capable_peer")
            );
            assert_eq!(
                reject
                    .details
                    .as_ref()
                    .and_then(|details| details.get("wire_code"))
                    .and_then(Value::as_str),
                Some("no_capable_peer")
            );
        }
        SpokeResult::Ok(_) => panic!("no capable peer must reject"),
    }
    // Terminal: no delegate ran (no wrong-peer fallback).
    assert_eq!(computable_host.stats().invokes_dispatched, 0);

    computable_adapter.close();
    computable_host.close();
}

#[tokio::test]
async fn multi_peer_router_breaks_ties_on_the_lowest_peer_id_across_two_baseline_peers() {
    // Three peers: alpha (baseline-only), beta (baseline + l2-computable),
    // gamma (l2-computable-only). A baseline op has TWO candidates (alpha +
    // beta); the locked §4 tie-break picks the lowest peer_id in UTF-8 byte
    // order — the host ids derive from the seeds, so the expected recipient
    // is computed at runtime rather than hunted for.
    let (alpha_adapter, alpha_host) =
        dial_peer([0xc3; 32], manifest("host-alpha", &["spoke-baseline"])).await;
    let (beta_adapter, beta_host) = dial_peer(
        [0xd4; 32],
        manifest("host-beta", &["spoke-baseline", "l2-computable"]),
    )
    .await;
    let (gamma_adapter, gamma_host) =
        dial_peer([0xe5; 32], manifest("host-gamma", &["l2-computable"])).await;

    let alpha_id = alpha_adapter.remote_peer_id().expect("alpha peer id");
    let beta_id = beta_adapter.remote_peer_id().expect("beta peer id");

    let router = connect_multi_peer_router(MultiPeerRouterOptions::default());
    router
        .register_peer(alpha_adapter.clone())
        .expect("register alpha");
    router
        .register_peer(beta_adapter.clone())
        .expect("register beta");
    router
        .register_peer(gamma_adapter.clone())
        .expect("register gamma");

    let request = upsert_request(&[fresh_entry("kb_mpr_tiebreak", "Tie-Break")]);
    let upsert = orchestrate_upsert(&router, request).await;
    assert!(upsert.is_ok(), "tie-break upsert must route: {upsert:?}");

    let (expected_host, other_host) = if alpha_id < beta_id {
        (&alpha_host, &beta_host)
    } else {
        (&beta_host, &alpha_host)
    };
    // Both baseline port calls (get + put) select the same lowest-id peer.
    assert_eq!(expected_host.stats().invokes_dispatched, 2);
    assert_eq!(other_host.stats().invokes_dispatched, 0);
    // The l2-computable-only peer is never a candidate for baseline ops.
    assert_eq!(gamma_host.stats().invokes_dispatched, 0);

    alpha_adapter.close();
    beta_adapter.close();
    gamma_adapter.close();
    alpha_host.close();
    beta_host.close();
    gamma_host.close();
}
