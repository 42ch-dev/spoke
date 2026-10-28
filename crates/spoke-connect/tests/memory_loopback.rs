//! Loopback test — the memory family served over Connect (`memory.*`).
//!
//! A raw signed-wire client drives `connect_responder` serving the
//! `RemoteMemoryService` face over an in-memory provider that uses the
//! reference `spoke-memory` engine for candidate classification and recall
//! ranking/budget.
//!
//! Asserts the Task 2 contract: one capture → review → recall round trip
//! yields the committed receipt, placement inherited from the captured
//! candidate at review, engine-ranked and budgeted recall, and the
//! fail-closed dispatch rows — a known-but-unnegotiated memory op answers
//! `capability_missing`, an unknown `memory.*` op stays `op_unsupported`, and
//! a host declared with `spoke-memory` but without the service face denies
//! `op_unsupported` too.
//!
//! Gated on the `remote-adapter` feature; `cargo test -p spoke-connect`
//! (default features) does not build it.

#![cfg(feature = "remote-adapter")]

#[path = "common/loopback_oracle.rs"]
mod loopback_oracle;
use loopback_oracle::{
    connect_manifest, pubkey_client, seed_client, seed_host, sign_envelope,
};

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use spoke_connect::core::{derive_peer_id_from_ed25519_pubkey, sign_hello_ed25519};
use spoke_connect::remote::{
    connect_responder, loopback_transport_pair, ConnectResponder, ConnectResponderOptions,
    LoopbackTransport, LoopbackTransportPair, RemoteIdentity, RemoteMemoryService,
    RemoteServePorts, RemoteServePortsComposite, Transport,
};
use spoke_fixture_toy_world::ToyWorldAdapter;
use spoke_memory::classify::{classify_candidate, CandidateMaterial};
use spoke_memory::recall::{recall as engine_recall, RecallRecord, RecallRequest};
use spoke_memory::ENGINE_POLICY_VERSION;
use spoke_operations::{
    jcs_sha256_hex, spoke_reject, BaselinePorts, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::connect::{ConnectHello, ConnectSession};
use spoke_schemas::data::memory_candidate::Disposition;
use spoke_schemas::data::retained_record::Content;
use spoke_schemas::{
    HostCapabilityManifest, MemoryCandidatesReviewRequest, MemoryCaptureRequest,
    MemoryRecallRequest,
};

const SPACE: &str = "spc_local_default";
const SUBJECT: &str = "nexus:character:mira";
const CONTEXT: &str = "bnd_mira_harbor_run_7";
const CAPTURED_AT: &str = "2026-10-28T09:20:00Z";

/// Memory capabilities negotiated by both ends of the loopback session.
const MEMORY_CAPABILITIES: [&str; 4] = [
    "spoke-baseline",
    "spoke-memory",
    "spoke-memory-reflection",
    "spoke-memory-transfer",
];

// ── In-memory provider behind `RemoteMemoryService` ───────────────────────

#[derive(Default)]
struct MemoryHost {
    state: Mutex<HostState>,
}

#[derive(Default)]
struct HostState {
    candidates: Vec<Candidate>,
    records: Vec<StoredRecord>,
    issued_receipts: usize,
}

struct Candidate {
    id: String,
    revision: u64,
    consumed: bool,
    space_id: String,
    subject_id: String,
    /// Immutable admitted placement (§6.7) — inherited by review outcomes.
    scope: Value,
}

struct StoredRecord {
    id: String,
    space_id: String,
    subject_id: String,
    kind: String,
    text: String,
    scope: Value,
    captured_by: String,
}

fn reject<T>(code: SpokeRejectCode, message: impl Into<String>) -> SpokeResult<T> {
    spoke_reject(code, message, None)
}

fn decode<T: DeserializeOwned>(op: &str, request: &Value) -> SpokeResult<T> {
    match serde_json::from_value(request.clone()) {
        Ok(value) => SpokeResult::Ok(value),
        Err(error) => reject(
            SpokeRejectCode::InvalidInput,
            format!("invalid {op} payload: {error}"),
        ),
    }
}

impl MemoryHost {
    fn receipt(
        &self,
        op: &str,
        request: &Value,
        space_id: &str,
        principal: &str,
        result: Value,
    ) -> Value {
        let receipt_id = {
            let mut state = self.state.lock().expect("state lock");
            state.issued_receipts += 1;
            format!("rcpt_{:04}", state.issued_receipts)
        };
        let mut receipt = json!({
            "schema_version": 1,
            "receipt_id": receipt_id,
            "op": op,
            "space_id": space_id,
            "requester_principal_id": principal,
            "payload_digest": match jcs_sha256_hex(request) {
                SpokeResult::Ok(digest) => digest,
                SpokeResult::Reject(reject) => panic!("request digest: {reject:?}"),
            },
            "status": "committed",
            "result": result,
            "created_at": CAPTURED_AT,
            "completed_at": CAPTURED_AT,
            "extensions": {},
        });
        if let Some(key) = request.get("idempotency_key") {
            receipt["idempotency_key"] = key.clone();
        }
        receipt
    }

    fn record_wire(&self, record: &StoredRecord) -> Value {
        json!({
            "schema_version": 1,
            "record_id": record.id,
            "space_id": record.space_id,
            "subject_id": record.subject_id,
            "kind": record.kind,
            "content": { "text": record.text },
            "scope": record.scope,
            "provenance": {
                "source_context_ids": [CONTEXT],
                "anchors": [],
                "captured_by": record.captured_by,
                "captured_at": CAPTURED_AT,
            },
            "created_at": CAPTURED_AT,
            "updated_at": CAPTURED_AT,
            "revision": 1,
            "extensions": {},
        })
    }

    fn capture(&self, principal: &str, request: Value) -> SpokeResult<Value> {
        let _: MemoryCaptureRequest = match decode("memory.capture", &request) {
            SpokeResult::Ok(typed) => typed,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        let text = request["content"]["text"].as_str().unwrap_or_default();
        let kind = request
            .get("kind_hint")
            .and_then(Value::as_str)
            .unwrap_or("note");
        let classification = classify_candidate(&CandidateMaterial {
            kind,
            text,
            has_structured_content: request["content"].get("structured").is_some(),
            occurred_at: None,
            source_context_count: request
                .get("source_context_ids")
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
            anchor_count: request
                .get("anchors")
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
        });
        let mut candidate_ids = Vec::new();
        if classification.disposition != Disposition::Drop {
            let mut state = self.state.lock().expect("state lock");
            let id = format!("cand_{:04}", state.candidates.len() + 1);
            state.candidates.push(Candidate {
                id: id.clone(),
                revision: 1,
                consumed: false,
                space_id: request["space_id"].as_str().unwrap_or_default().to_string(),
                subject_id: request["subject_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                scope: request["capture_scope"].clone(),
            });
            candidate_ids.push(id);
        }
        let space_id = request["space_id"].as_str().unwrap_or_default();
        SpokeResult::Ok(self.receipt(
            "memory.capture",
            &request,
            space_id,
            principal,
            json!({ "candidate_ids": candidate_ids, "classified": candidate_ids.len() }),
        ))
    }

    fn review(&self, principal: &str, request: Value) -> SpokeResult<Value> {
        let _: MemoryCandidatesReviewRequest = match decode("memory.candidates.review", &request) {
            SpokeResult::Ok(typed) => typed,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        let candidate_id = request["candidate_id"].as_str().unwrap_or_default();
        let expected_revision = request["expected_revision"].as_u64().unwrap_or(0);
        let action = request["decision"]["action"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let fragments = request["decision"]["fragments"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        let mut state = self.state.lock().expect("state lock");
        let Some(index) = state
            .candidates
            .iter()
            .position(|candidate| candidate.id == candidate_id)
        else {
            return reject(
                SpokeRejectCode::NotFound,
                format!("candidate {candidate_id} not found"),
            );
        };
        let (space_id, subject_id, scope, revision, consumed) = {
            let candidate = &state.candidates[index];
            (
                candidate.space_id.clone(),
                candidate.subject_id.clone(),
                candidate.scope.clone(),
                candidate.revision,
                candidate.consumed,
            )
        };
        if consumed {
            return reject(
                SpokeRejectCode::CandidateConsumed,
                format!("candidate {candidate_id} is already consumed"),
            );
        }
        if revision != expected_revision {
            return reject(
                SpokeRejectCode::RevisionConflict,
                format!("candidate {candidate_id} revision is {revision}"),
            );
        }
        // Review outcomes inherit the candidate's admitted placement: no
        // placement field exists in a decision (§7.15).
        let mut record_ids = Vec::new();
        if action == "fragment" {
            for fragment in &fragments {
                let id = format!("rec_{:04}", state.records.len() + 1);
                state.records.push(StoredRecord {
                    id: id.clone(),
                    space_id: space_id.clone(),
                    subject_id: subject_id.clone(),
                    kind: fragment["kind"].as_str().unwrap_or("note").to_string(),
                    text: fragment["content"]["text"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    scope: scope.clone(),
                    captured_by: principal.to_string(),
                });
                record_ids.push(id);
            }
        }
        {
            let candidate = &mut state.candidates[index];
            candidate.consumed = true;
            candidate.revision += 1;
        }
        drop(state);

        let outcome = match action.as_str() {
            "fragment" => "fragmented",
            "promote" => "promoted",
            _ => "dropped",
        };
        SpokeResult::Ok(self.receipt(
            "memory.candidates.review",
            &request,
            &space_id,
            principal,
            json!({ "outcome": outcome, "record_ids": record_ids }),
        ))
    }

    fn recall(&self, request: Value) -> SpokeResult<Value> {
        let _: MemoryRecallRequest = match decode("memory.recall", &request) {
            SpokeResult::Ok(typed) => typed,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        let subject_id = request["subject_id"].as_str().unwrap_or_default();
        let created_at = CAPTURED_AT.parse().expect("timestamp parses");
        let max_items = request["budget"]["max_items"].as_u64().unwrap_or(8) as usize;
        let max_bytes = request["budget"]["max_bytes"]
            .as_u64()
            .map(|bytes| bytes as usize);
        let query = request.get("query").and_then(Value::as_str);

        let state = self.state.lock().expect("state lock");
        let stored: Vec<&StoredRecord> = state
            .records
            .iter()
            .filter(|record| record.subject_id == subject_id)
            .collect();
        let inputs: Vec<RecallRecord> = stored
            .iter()
            .map(|record| RecallRecord {
                record_id: record.id.clone(),
                kind: record.kind.clone(),
                content: Content {
                    text: record.text.clone(),
                    structured: None,
                },
                created_at,
            })
            .collect();
        let page = engine_recall(
            &inputs,
            &RecallRequest {
                query,
                max_items,
                max_bytes,
            },
        );
        let records: Vec<Value> = page
            .records
            .iter()
            .map(|ranked| {
                let record = stored
                    .iter()
                    .find(|record| record.id == ranked.record_id)
                    .expect("ranked record is stored");
                self.record_wire(record)
            })
            .collect();
        SpokeResult::Ok(json!({
            "records": records,
            "total_authorized": page.total_authorized,
            "truncated": page.truncated,
            "engine_policy_version": ENGINE_POLICY_VERSION,
        }))
    }
}

#[async_trait]
impl RemoteMemoryService for MemoryHost {
    async fn invoke(&self, peer_id: &str, op: &str, payload: Value) -> SpokeResult<Value> {
        // The principal is derived from the authenticated peer, never the
        // payload (MEM-R003).
        let principal = format!("prn_{peer_id}");
        match op {
            "memory.capture" => self.capture(&principal, payload),
            "memory.candidates.review" => self.review(&principal, payload),
            "memory.recall" => self.recall(payload),
            other => reject(
                SpokeRejectCode::InternalError,
                format!("{other} is not served by the memory loopback host"),
            ),
        }
    }
}

// ── Wire client (raw signed envelopes) ────────────────────────────────────

fn memory_manifest(host_id: &str, capabilities: &[&str]) -> HostCapabilityManifest {
    serde_json::from_value(json!({
        "schema_version": 1,
        "host_id": host_id,
        "roles": ["data-store"],
        "capabilities": capabilities,
        "namespaces": [],
        "extensions": {},
    }))
    .expect("valid manifest")
}

fn sign_invoke_request(
    seed: [u8; 32],
    session_id: &str,
    sequence: i64,
    request_id: &str,
    op: &str,
    payload: Value,
) -> Value {
    let signed_object = json!({
        "session_id": session_id,
        "sequence": sequence,
        "request_id": request_id,
        "op": op,
        "payload": payload,
    });
    let signature = sign_envelope(&seed, &signed_object);
    let mut wire = signed_object
        .as_object()
        .expect("signed object is an object")
        .clone();
    wire.insert("extensions".into(), json!({}));
    wire.insert("signature".into(), json!(signature));
    Value::Object(wire)
}

async fn handshake(client: &LoopbackTransport, manifest: &HostCapabilityManifest) -> String {
    let nonce = "memory-loopback-nonce-0000".to_owned();
    let hello = sign_hello_ed25519(&seed_client(), &nonce, &connect_manifest(manifest), None)
        .expect("initiator hello sign");
    client
        .send(&serde_json::to_vec(&hello).expect("hello bytes"))
        .await
        .expect("hello send");
    let _responder_hello: ConnectHello =
        serde_json::from_slice(&client.recv().await.expect("responder hello recv"))
            .expect("responder hello decode");
    let snapshot = client.recv().await.expect("snapshot recv");
    let session: ConnectSession = serde_json::from_slice(&snapshot).expect("session decode");
    session.session_id.to_string()
}

async fn invoke(
    client: &LoopbackTransport,
    session_id: &str,
    sequence: i64,
    request_id: &str,
    op: &str,
    payload: Value,
) -> Value {
    let request = sign_invoke_request(
        seed_client(),
        session_id,
        sequence,
        request_id,
        op,
        payload,
    );
    client
        .send(&serde_json::to_vec(&request).expect("request bytes"))
        .await
        .expect("request send");
    serde_json::from_slice(&client.recv().await.expect("response recv")).expect("response decode")
}

/// Start a responder whose ports face carries `memory` (when given) and whose
/// manifest advertises `capabilities`.
async fn start_host(
    memory: Option<Arc<dyn RemoteMemoryService>>,
    capabilities: &[&str],
) -> (Arc<ConnectResponder>, LoopbackTransportPair) {
    let pair = loopback_transport_pair();
    let peer_id_client = derive_peer_id_from_ed25519_pubkey(&pubkey_client());
    let baseline: Arc<dyn BaselinePorts + Send + Sync> =
        Arc::new(ToyWorldAdapter::with_committed_fixtures());
    let mut ports = RemoteServePortsComposite::new(baseline, None, None);
    if let Some(memory) = memory {
        ports = ports.with_memory(memory);
    }
    let ports: Arc<dyn RemoteServePorts + Send + Sync> = Arc::new(ports);
    let responder = connect_responder(ConnectResponderOptions {
        transport: Arc::new(pair.server.clone()),
        identity: RemoteIdentity { seed: seed_host() },
        manifest: memory_manifest("memory-loopback-host", capabilities),
        allowlist: vec![peer_id_client.clone()],
        peer_keys: HashMap::from([(peer_id_client, pubkey_client())]),
        ports: Some(ports),
        invoke_timeout_ms: None,
    })
    .await;
    (responder, pair)
}

fn capture_request(idempotency_key: &str) -> Value {
    json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "content": { "text": "Mira told Bo the gate key is hidden under the third barrel." },
        "capture_scope": { "visibility": "context", "context_id": CONTEXT },
        "source_context_ids": [CONTEXT],
        "kind_hint": "fact",
        "idempotency_key": idempotency_key,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn memory_loopback_round_trips_capture_review_and_recall() {
    let host = Arc::new(MemoryHost::default());
    let (responder, pair) =
        start_host(Some(host), &MEMORY_CAPABILITIES).await;
    let session_id = handshake(&pair.client, &memory_manifest("test-client", &MEMORY_CAPABILITIES)).await;

    // capture → committed receipt echoing the idempotency key.
    let captured = invoke(
        &pair.client,
        &session_id,
        0,
        "memory-capture",
        "memory.capture",
        capture_request("cap-loopback-1"),
    )
    .await;
    let receipt = captured.get("payload").unwrap_or_else(|| {
        panic!("capture must answer a payload, got {captured}");
    });
    assert_eq!(receipt["status"], "committed", "{captured}");
    assert_eq!(receipt["op"], "memory.capture");
    assert_eq!(receipt["idempotency_key"], "cap-loopback-1");
    assert_eq!(receipt["requester_principal_id"], format!("prn_{}", derive_peer_id_from_ed25519_pubkey(&pubkey_client())));
    assert_eq!(receipt["result"]["classified"], 1, "{captured}");
    let candidate_id = receipt["result"]["candidate_ids"][0]
        .as_str()
        .unwrap_or_else(|| panic!("candidate id expected, got {captured}"))
        .to_owned();
    assert!(
        receipt["payload_digest"]
            .as_str()
            .is_some_and(|digest| digest.len() == 64),
        "receipt carries a sha-256 payload digest, got {captured}"
    );

    // review → one fact + one event fragment; the receipt is content-free.
    let reviewed = invoke(
        &pair.client,
        &session_id,
        1,
        "memory-review",
        "memory.candidates.review",
        json!({
            "space_id": SPACE,
            "candidate_id": candidate_id,
            "expected_revision": 1,
            "decision": {
                "action": "fragment",
                "fragments": [
                    { "kind": "fact", "content": { "text": "The gate key is hidden under the third barrel." } },
                    { "kind": "event", "content": { "text": "Mira revealed the key location to Bo." } },
                ],
            },
            "idempotency_key": "review-cand-1",
        }),
    )
    .await;
    let review_receipt = reviewed
        .get("payload")
        .unwrap_or_else(|| panic!("review must answer a payload, got {reviewed}"));
    assert_eq!(review_receipt["status"], "committed", "{reviewed}");
    assert_eq!(review_receipt["result"]["outcome"], "fragmented", "{reviewed}");
    assert_eq!(
        review_receipt["result"]["record_ids"]
            .as_array()
            .map(Vec::len),
        Some(2),
        "{reviewed}"
    );

    // recall → engine-ranked page; placement inherited from the candidate.
    let recalled = invoke(
        &pair.client,
        &session_id,
        2,
        "memory-recall",
        "memory.recall",
        json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": { "mode": "selected_contexts", "context_ids": [CONTEXT] },
            "query": "Where is the gate key?",
            "budget": { "max_items": 8 },
        }),
    )
    .await;
    let page = recalled
        .get("payload")
        .unwrap_or_else(|| panic!("recall must answer a payload, got {recalled}"));
    let records = page["records"].as_array().expect("records array");
    assert_eq!(records.len(), 2, "{recalled}");
    assert_eq!(page["total_authorized"], 2);
    assert_eq!(page["truncated"], false);
    assert_eq!(page["engine_policy_version"], ENGINE_POLICY_VERSION);
    // Ranking: the fact fragment matches the query tokens and outranks the
    // event fragment (§9 policy: query overlap then kind weight).
    assert_eq!(records[0]["kind"], "fact", "{recalled}");
    assert_eq!(records[1]["kind"], "event", "{recalled}");
    // Placement inheritance: no placement field exists in a review decision,
    // so the recalled records carry the captured candidate's scope.
    for record in records {
        assert_eq!(
            record["scope"],
            json!({ "visibility": "context", "context_id": CONTEXT }),
            "review outcomes inherit the candidate placement, got {record}"
        );
    }

    // Budget: a one-item cap truncates the same ranking.
    let truncated = invoke(
        &pair.client,
        &session_id,
        3,
        "memory-recall-budget",
        "memory.recall",
        json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": { "mode": "selected_contexts", "context_ids": [CONTEXT] },
            "query": "Where is the gate key?",
            "budget": { "max_items": 1 },
        }),
    )
    .await;
    let page = truncated
        .get("payload")
        .unwrap_or_else(|| panic!("budgeted recall must answer a payload, got {truncated}"));
    assert_eq!(page["records"].as_array().map(Vec::len), Some(1), "{truncated}");
    assert_eq!(page["truncated"], true);
    assert_eq!(page["total_authorized"], 2);

    // An unknown `memory.*` op has no dispatch row: fail-closed
    // `op_unsupported`, never a handler call.
    let unknown = invoke(
        &pair.client,
        &session_id,
        4,
        "memory-unknown",
        "memory.archive.purge",
        json!({}),
    )
    .await;
    assert_eq!(unknown["error"]["code"], "op_unsupported", "{unknown}");

    responder.close();
}

#[tokio::test]
async fn memory_loopback_denies_unnegotiated_and_unserved_memory_ops() {
    let host = Arc::new(MemoryHost::default());

    // Known op, capability not negotiated (both manifests omit the flags):
    // `capability_missing` before any handler runs — zero side effects.
    let (responder, pair) = start_host(Some(host.clone()), &["spoke-baseline"]).await;
    let session_id =
        handshake(&pair.client, &memory_manifest("test-client", &["spoke-baseline"])).await;
    let denied = invoke(
        &pair.client,
        &session_id,
        0,
        "memory-unnegotiated",
        "memory.capture",
        capture_request("cap-denied"),
    )
    .await;
    assert_eq!(denied["error"]["code"], "capability_missing", "{denied}");
    assert!(
        denied.get("payload").is_none(),
        "a capability deny carries no payload, got {denied}"
    );
    // Zero side effects: no candidate was admitted by the denied capture.
    assert_eq!(host.state.lock().expect("state lock").candidates.len(), 0);
    responder.close();

    // Capability negotiated but the host provides no memory face: the
    // dispatch-deny branch, `op_unsupported`.
    let (responder, pair) = start_host(None, &MEMORY_CAPABILITIES).await;
    let session_id =
        handshake(&pair.client, &memory_manifest("test-client", &MEMORY_CAPABILITIES)).await;
    let unserved = invoke(
        &pair.client,
        &session_id,
        0,
        "memory-unserved",
        "memory.capture",
        capture_request("cap-unserved"),
    )
    .await;
    assert_eq!(unserved["error"]["code"], "op_unsupported", "{unserved}");
    assert!(
        unserved["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("no memory service configured"),
        "{unserved}"
    );
    responder.close();
}
