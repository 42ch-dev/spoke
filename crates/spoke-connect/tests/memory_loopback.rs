//! Loopback test — the memory family end-to-end over Connect (`memory.*`).
//!
//! The Rust **client** is the typed `MemoryPort` adapter on `RemoteAdapter`
//! (`crates/spoke-connect/src/remote/remote_adapter.rs`); the **host** is an
//! in-memory `MemoryPort` provider served through `MemoryPortService`. Both
//! directions therefore ride the typed P2 boundary — no JSON-face shortcut.
//!
//! The provider enforces its own duties, independently of the engine: grant
//! coverage (`spoke_operations::grant_covers`), idempotent replay with
//! `IDEMPOTENCY_CONFLICT` on a changed payload digest, space addressing on
//! review, and space / selection / principal filtering before the records
//! reach engine scoring.
//!
//! The dispatch-deny rows and the unknown-op fallback are asserted on the raw
//! wire (the typed client cannot express an unknown op).
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
use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};
use spoke_connect::core::{derive_peer_id_from_ed25519_pubkey, sign_hello_ed25519};
use spoke_connect::remote::{
    connect_remote_adapter, connect_responder, loopback_transport_pair, ConnectResponder,
    ConnectResponderOptions, LoopbackTransport, LoopbackTransportPair, MemoryAdmission,
    MemoryPortService, RemoteAdapter, RemoteAdapterOptions, RemoteIdentity, RemoteMemoryService,
    RemoteServePorts, RemoteServePortsComposite, Transport,
};
use spoke_fixture_toy_world::ToyWorldAdapter;
use spoke_memory::classify::{classify_candidate, CandidateMaterial};
use spoke_memory::recall::{recall as engine_recall, RecallRecord, RecallRequest};
use spoke_memory::ENGINE_POLICY_VERSION;
use spoke_operations::{
    grant_covers, jcs_sha256_hex, spoke_reject, BaselinePorts, GrantRequirement, MemoryPort,
    SpokeReject, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::connect::{ConnectHello, ConnectSession};
use spoke_schemas::data::memory_candidate::Disposition;
use spoke_schemas::data::retained_record::Content;
use spoke_schemas::{
    HostCapabilityManifest, MemoryCandidatesDismissRequest, MemoryCandidatesDismissResponse,
    MemoryCandidatesGetRequest, MemoryCandidatesGetResponse, MemoryCandidatesListRequest,
    MemoryCandidatesListResponse, MemoryCandidatesReviewRequest, MemoryCandidatesReviewResponse,
    MemoryCaptureRequest, MemoryCaptureResponse, MemoryDescribeRequest, MemoryDescribeResponse,
    MemoryExportsCreateRequest, MemoryExportsCreateResponse, MemoryExportsGetRequest,
    MemoryExportsGetResponse, MemoryGrant, MemoryGrantsGrantRequest, MemoryGrantsGrantResponse,
    MemoryGrantsListRequest, MemoryGrantsListResponse, MemoryGrantsRevokeRequest,
    MemoryGrantsRevokeResponse, MemoryImportsActivateRequest, MemoryImportsActivateResponse,
    MemoryImportsDiscardRequest, MemoryImportsDiscardResponse, MemoryImportsStageRequest,
    MemoryImportsStageResponse, MemoryProfileGetRequest, MemoryProfileGetResponse,
    MemoryProfileUpdateRequest, MemoryProfileUpdateResponse, MemoryRecallRequest,
    MemoryRecallResponse, MemoryReceiptsGetRequest, MemoryReceiptsGetResponse,
    MemoryRecordsCreateRequest, MemoryRecordsCreateResponse, MemoryRecordsDeleteRequest,
    MemoryRecordsDeleteResponse, MemoryRecordsGetRequest, MemoryRecordsGetResponse,
    MemoryRecordsListRequest, MemoryRecordsListResponse, MemoryRecordsPromoteRequest,
    MemoryRecordsPromoteResponse, MemoryRecordsUpdateRequest, MemoryRecordsUpdateResponse,
    MemoryReflectionsGenerateRequest, MemoryReflectionsGenerateResponse,
    MemoryReflectionsListRequest, MemoryReflectionsListResponse, MemoryReflectionsStateRequest,
    MemoryReflectionsStateResponse, MemorySpacesFreezeRequest, MemorySpacesFreezeResponse,
    MemorySpacesListRequest, MemorySpacesListResponse, MemorySpacesUnfreezeRequest,
    MemorySpacesUnfreezeResponse, MemorySubjectsEnsureRequest, MemorySubjectsEnsureResponse,
    MemorySubjectsFenceRequest, MemorySubjectsFenceResponse, MemorySubjectsGetRequest,
    MemorySubjectsGetResponse, Principal,
};

const SPACE: &str = "spc_local_default";
const OTHER_SPACE: &str = "spc_other";
const SUBJECT: &str = "nexus:character:mira";
const CONTEXT: &str = "bnd_mira_harbor_run_7";
const OTHER_CONTEXT: &str = "bnd_bo_market_run_2";
const CAPTURED_AT: &str = "2026-10-28T09:20:00Z";

/// Memory capabilities negotiated by both ends of the loopback session.
const MEMORY_CAPABILITIES: [&str; 4] = [
    "spoke-baseline",
    "spoke-memory",
    "spoke-memory-reflection",
    "spoke-memory-transfer",
];

fn reject<T>(code: SpokeRejectCode, message: impl Into<String>) -> SpokeResult<T> {
    spoke_reject(code, message, None)
}

fn not_served<T: std::fmt::Debug>() -> SpokeResult<T> {
    reject(
        SpokeRejectCode::InternalError,
        "not served by the memory loopback host",
    )
}

fn principal_id(principal: &Principal) -> &str {
    principal.principal_id.as_str()
}

/// `§6.1` payload digest: SHA-256 over the JCS canonicalization of the whole
/// request payload object, idempotency key included. The dedup namespace
/// already binds `(space, principal, op, key)`, so the digest never excludes
/// the key.
fn payload_digest(request: &Value) -> String {
    match jcs_sha256_hex(request) {
        SpokeResult::Ok(digest) => digest,
        SpokeResult::Reject(reject) => panic!("request digest: {reject:?}"),
    }
}

fn expect_ok<T>(result: SpokeResult<T>) -> T {
    match result {
        SpokeResult::Ok(value) => value,
        SpokeResult::Reject(reject) => panic!("expected success, got {reject:?}"),
    }
}

fn expect_reject<T: std::fmt::Debug>(result: SpokeResult<T>) -> SpokeReject {
    match result {
        SpokeResult::Ok(value) => panic!("expected a rejection, got {value:?}"),
        SpokeResult::Reject(reject) => reject,
    }
}

fn wire<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("wire value serializes")
}

fn reject_wire_code(reject: &SpokeReject) -> Option<String> {
    reject
        .details
        .as_ref()
        .and_then(|details| details.get("wire_code"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

// ── In-memory provider: storage + provider duties ─────────────────────────

#[derive(Default)]
struct Store {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    grants: Vec<MemoryGrant>,
    candidates: Vec<Candidate>,
    records: Vec<StoredRecord>,
    receipts: Vec<IssuedReceipt>,
    issued_receipts: usize,
    issued_records: usize,
}

struct Candidate {
    id: String,
    revision: u64,
    consumed: bool,
    /// The candidate lives in the space that admitted the capture; review
    /// authorization and the addressed space are checked against it.
    space_id: String,
    subject_id: String,
    /// Immutable admitted placement (§6.7) — inherited by review outcomes.
    scope: Value,
}

struct StoredRecord {
    space_id: String,
    subject_id: String,
    record_id: String,
    kind: String,
    text: String,
    scope: Value,
    captured_by: String,
}

struct IssuedReceipt {
    space_id: String,
    principal_id: String,
    op: String,
    idempotency_key: String,
    digest: String,
    receipt: Value,
}

impl Store {
    fn with_grant(principal_id: &str, contexts: &[&str]) -> Arc<Self> {
        let store = Store::default();
        store.add_grant(
            principal_id,
            SPACE,
            Some(SUBJECT),
            &["capture", "review", "records_write", "recall"],
            contexts,
            false,
        );
        Arc::new(store)
    }

    /// Seed a grant. `contexts` empty = unrestricted within the space;
    /// `shared_access` is only meaningful on a restricted grant (§6.5).
    fn add_grant(
        &self,
        principal_id: &str,
        space: &str,
        subject: Option<&str>,
        capabilities: &[&str],
        contexts: &[&str],
        shared_access: bool,
    ) {
        let mut state = self.state.lock().expect("state lock");
        let mut grant = json!({
            "schema_version": 1,
            "grant_id": format!("grt_{:02}", state.grants.len() + 1),
            "space_id": space,
            "principal_id": principal_id,
            "capabilities": capabilities,
            "context_ids": contexts,
            "created_at": CAPTURED_AT,
            "revision": 1,
            "extensions": {},
        });
        if let Some(subject) = subject {
            grant["subject_id"] = json!(subject);
        }
        if shared_access {
            grant["shared_access"] = json!(true);
        }
        let grant: MemoryGrant = serde_json::from_value(grant).expect("valid grant");
        state.grants.push(grant);
    }

    /// Seed a pending candidate the provider owns (`None` = shared placement).
    fn seed_candidate(&self, context: Option<&str>) -> String {
        let mut state = self.state.lock().expect("state lock");
        let id = format!("cand_seed_{:02}", state.candidates.len() + 1);
        let scope = context.map_or_else(
            || json!({ "visibility": "shared" }),
            |context| json!({ "visibility": "context", "context_id": context }),
        );
        state.candidates.push(Candidate {
            id: id.clone(),
            revision: 1,
            consumed: false,
            space_id: SPACE.to_string(),
            subject_id: SUBJECT.to_string(),
            scope,
        });
        id
    }

    fn seed_record(&self, space: &str, context: &str, kind: &str, text: &str) {
        let mut state = self.state.lock().expect("state lock");
        state.issued_records += 1;
        let record_id = format!("rec_seed_{:04}", state.issued_records);
        let scope = if context.is_empty() {
            json!({ "visibility": "shared" })
        } else {
            json!({ "visibility": "context", "context_id": context })
        };
        state.records.push(StoredRecord {
            space_id: space.to_string(),
            subject_id: SUBJECT.to_string(),
            record_id,
            kind: kind.to_string(),
            text: text.to_string(),
            scope,
            captured_by: "prn_seed".to_string(),
        });
    }

    fn receipt(
        state: &mut State,
        op: &str,
        space_id: &str,
        principal_id: &str,
        request: &Value,
        result: Value,
    ) -> Value {
        state.issued_receipts += 1;
        json!({
            "schema_version": 1,
            "receipt_id": format!("rcpt_{:04}", state.issued_receipts),
            "op": op,
            "space_id": space_id,
            "requester_principal_id": principal_id,
            "idempotency_key": request["idempotency_key"].clone(),
            "payload_digest": payload_digest(request),
            "status": "committed",
            "result": result,
            "created_at": CAPTURED_AT,
            "completed_at": CAPTURED_AT,
            "extensions": {},
        })
    }

    /// `§7.0.5` dedup namespace `(space, principal, op, key)`: the same key +
    /// the same payload digest replays the original receipt without executing
    /// again; a different digest is `IDEMPOTENCY_CONFLICT`.
    fn replay(
        state: &State,
        namespace: (&str, &str, &str, &str),
        digest: &str,
    ) -> Option<SpokeResult<Value>> {
        let (space_id, principal_id, op, key) = namespace;
        if key.is_empty() {
            return None;
        }
        let issued = state.receipts.iter().find(|issued| {
            issued.space_id == space_id
                && issued.principal_id == principal_id
                && issued.op == op
                && issued.idempotency_key == key
        });
        let issued = issued?;
        if issued.digest == digest {
            Some(SpokeResult::Ok(issued.receipt.clone()))
        } else {
            Some(reject(
                SpokeRejectCode::IdempotencyConflict,
                format!("idempotency key {key} was used with a different payload"),
            ))
        }
    }

    fn remember(
        state: &mut State,
        namespace: (&str, &str, &str, &str),
        digest: &str,
        receipt: &Value,
    ) {
        let (space_id, principal_id, op, key) = namespace;
        if key.is_empty() {
            return;
        }
        state.receipts.push(IssuedReceipt {
            space_id: space_id.to_string(),
            principal_id: principal_id.to_string(),
            op: op.to_string(),
            idempotency_key: key.to_string(),
            digest: digest.to_string(),
            receipt: receipt.clone(),
        });
    }

    fn has_receipt(&self, op: &str, key: &str) -> bool {
        if key.is_empty() {
            return false;
        }
        self.state
            .lock()
            .expect("state lock")
            .receipts
            .iter()
            .any(|issued| issued.op == op && issued.idempotency_key == key)
    }

    fn authorized(
        state: &State,
        principal_id: &str,
        space_id: &str,
        requirements: &[GrantRequirement<'_>],
    ) -> bool {
        let now = Utc::now();
        requirements.iter().all(|requirement| {
            state
                .grants
                .iter()
                .any(|grant| grant_covers(grant, principal_id, space_id, now, requirement))
        })
    }

    fn capture(&self, principal: &Principal, request: Value) -> SpokeResult<Value> {
        let principal_id = principal_id(principal).to_string();
        let space_id = request["space_id"].as_str().unwrap_or_default().to_string();
        let subject_id = request["subject_id"].as_str().unwrap_or_default().to_string();
        let scope = &request["capture_scope"];
        let shared = scope["visibility"].as_str() == Some("shared");
        let context_id = scope["context_id"].as_str().map(str::to_string);
        let key = request["idempotency_key"].as_str().unwrap_or_default();
        let digest = payload_digest(&request);
        let mut state = self.state.lock().expect("state lock");
        // Authorization first: a replay must never disclose a receipt to a
        // caller who is no longer covered (MEM-R021).
        let requirement = GrantRequirement {
            capability: "capture",
            subject_id: Some(&subject_id),
            context_ids: context_id.iter().cloned().collect(),
            reads_shared: shared,
            subject_all: false,
        };
        if !Store::authorized(&state, &principal_id, &space_id, &[requirement]) {
            return reject(
                SpokeRejectCode::NotAuthorized,
                format!("no capture grant covers {space_id}/{context_id:?} shared={shared}"),
            );
        }
        if let Some(answer) =
            Store::replay(&state, (&space_id, &principal_id, "memory.capture", key), &digest)
        {
            return answer;
        }
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
            let id = format!("cand_{:04}", state.candidates.len() + 1);
            state.candidates.push(Candidate {
                id: id.clone(),
                revision: 1,
                consumed: false,
                space_id: space_id.clone(),
                subject_id: subject_id.clone(),
                scope: request["capture_scope"].clone(),
            });
            candidate_ids.push(id);
        }
        let receipt = Store::receipt(
            &mut state,
            "memory.capture",
            &space_id,
            &principal_id,
            &request,
            json!({ "candidate_ids": candidate_ids, "classified": candidate_ids.len() }),
        );
        Store::remember(
            &mut state,
            (&space_id, &principal_id, "memory.capture", key),
            &digest,
            &receipt,
        );
        SpokeResult::Ok(receipt)
    }

    fn review(&self, principal: &Principal, request: Value) -> SpokeResult<Value> {
        let principal_id = principal_id(principal).to_string();
        let candidate_id = request["candidate_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let expected_revision = request["expected_revision"].as_u64().unwrap_or(0);
        let key = request["idempotency_key"].as_str().unwrap_or_default();
        let digest = payload_digest(&request);
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
        // §7.0.1/§7.15: the addressed space is part of the operation identity.
        // A request naming another space never silently resolves to this
        // candidate — it is rejected before any state changes.
        let addressed_space = request["space_id"].as_str().unwrap_or_default();
        if addressed_space != space_id {
            return reject(
                SpokeRejectCode::NotAuthorized,
                format!(
                    "review addresses space {addressed_space} but candidate {candidate_id} lives in {space_id}"
                ),
            );
        }
        // Authorization runs against the candidate's space (never the request's).
        let shared = scope["visibility"].as_str() == Some("shared");
        let contexts: Vec<String> = scope["context_id"]
            .as_str()
            .map(str::to_string)
            .into_iter()
            .collect();
        let requirements = [
            GrantRequirement {
                capability: "review",
                subject_id: Some(&subject_id),
                context_ids: contexts.clone(),
                reads_shared: shared,
                subject_all: false,
            },
            GrantRequirement {
                capability: "records_write",
                subject_id: Some(&subject_id),
                context_ids: contexts,
                reads_shared: shared,
                subject_all: false,
            },
        ];
        if !Store::authorized(&state, &principal_id, &space_id, &requirements) {
            return reject(
                SpokeRejectCode::NotAuthorized,
                format!("no review/records_write grant covers candidate {candidate_id} in {space_id}"),
            );
        }
        // Replay after the authorization recheck (MEM-R021): a caller who lost
        // coverage never gets the original receipt back.
        if let Some(answer) = Store::replay(
            &state,
            (&space_id, &principal_id, "memory.candidates.review", key),
            &digest,
        ) {
            return answer;
        }
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
        let action = request["decision"]["action"].as_str().unwrap_or_default();
        let fragments = request["decision"]["fragments"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut record_ids = Vec::new();
        if action == "fragment" {
            for fragment in &fragments {
                state.issued_records += 1;
                let record_id = format!("rec_{:04}", state.issued_records);
                state.records.push(StoredRecord {
                    space_id: space_id.clone(),
                    subject_id: subject_id.clone(),
                    record_id: record_id.clone(),
                    kind: fragment["kind"].as_str().unwrap_or("note").to_string(),
                    text: fragment["content"]["text"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    scope: scope.clone(),
                    captured_by: principal_id.clone(),
                });
                record_ids.push(record_id);
            }
        }
        {
            let candidate = &mut state.candidates[index];
            candidate.consumed = true;
            candidate.revision += 1;
        }
        let outcome = match action {
            "fragment" => "fragmented",
            "promote" => "promoted",
            _ => "dropped",
        };
        let receipt = Store::receipt(
            &mut state,
            "memory.candidates.review",
            &space_id,
            &principal_id,
            &request,
            json!({ "outcome": outcome, "record_ids": record_ids }),
        );
        Store::remember(
            &mut state,
            (
                &space_id,
                &principal_id,
                "memory.candidates.review",
                key,
            ),
            &digest,
            &receipt,
        );
        SpokeResult::Ok(receipt)
    }

    fn recall(&self, principal: &Principal, request: Value) -> SpokeResult<Value> {
        let principal_id = principal_id(principal).to_string();
        let space_id = request["space_id"].as_str().unwrap_or_default().to_string();
        let subject_id = request["subject_id"].as_str().unwrap_or_default().to_string();
        let selection = &request["selection"];
        let mode = selection["mode"].as_str().unwrap_or_default().to_string();
        let context_ids: Vec<String> = selection["context_ids"]
            .as_array()
            .map(|contexts| {
                contexts
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let include_shared = selection["include_shared"].as_bool().unwrap_or(false);
        let state = self.state.lock().expect("state lock");
        let requirement = GrantRequirement {
            capability: "recall",
            subject_id: Some(&subject_id),
            context_ids: context_ids.clone(),
            reads_shared: include_shared || mode == "shared_only",
            subject_all: mode == "subject_all",
        };
        if !Store::authorized(&state, &principal_id, &space_id, &[requirement]) {
            return reject(
                SpokeRejectCode::NotAuthorized,
                format!("no recall grant covers {space_id} selection {mode}"),
            );
        }
        // Authorized input set: space + subject + placement selection, before
        // any ranking (§9: the provider filters, the engine ranks).
        let selected: Vec<&StoredRecord> = state
            .records
            .iter()
            .filter(|record| record.space_id == space_id && record.subject_id == subject_id)
            .filter(|record| placement_selected(record, &mode, &context_ids, include_shared))
            .collect();
        let created_at = CAPTURED_AT.parse().expect("timestamp parses");
        let inputs: Vec<RecallRecord> = selected
            .iter()
            .map(|record| RecallRecord {
                record_id: record.record_id.clone(),
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
                query: request.get("query").and_then(Value::as_str),
                max_items: request["budget"]["max_items"].as_u64().unwrap_or(8) as usize,
                max_bytes: request["budget"]["max_bytes"]
                    .as_u64()
                    .map(|bytes| bytes as usize),
            },
        );
        let records: Vec<Value> = page
            .records
            .iter()
            .map(|ranked| {
                let record = selected
                    .iter()
                    .find(|record| record.record_id == ranked.record_id)
                    .expect("ranked record is authorized");
                json!({
                    "schema_version": 1,
                    "record_id": record.record_id,
                    "space_id": record.space_id,
                    "subject_id": record.subject_id,
                    "kind": record.kind,
                    "content": { "text": record.text },
                    "scope": record.scope,
                    "provenance": {
                        "source_context_ids": [],
                        "anchors": [],
                        "captured_by": record.captured_by,
                        "captured_at": CAPTURED_AT,
                    },
                    "created_at": CAPTURED_AT,
                    "updated_at": CAPTURED_AT,
                    "revision": 1,
                    "extensions": {},
                })
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

fn placement_selected(
    record: &StoredRecord,
    mode: &str,
    context_ids: &[String],
    include_shared: bool,
) -> bool {
    let shared = record.scope["visibility"].as_str() == Some("shared");
    match mode {
        "subject_all" => true,
        "shared_only" => shared,
        "selected_contexts" => {
            (shared && include_shared)
                || record
                    .scope
                    .get("context_id")
                    .and_then(Value::as_str)
                    .is_some_and(|context| context_ids.iter().any(|ids| ids == context))
        }
        _ => false,
    }
}

// ── The loopback host: a typed `MemoryPort` provider ──────────────────────

struct MemoryHost {
    store: Arc<Store>,
}

/// The provider answers the three ops the scenarios exercise; every other op
/// is unserved by this fixture.
#[async_trait]
impl MemoryPort for MemoryHost {
    async fn describe(
        &self,
        _principal: &Principal,
        _request: MemoryDescribeRequest,
    ) -> SpokeResult<MemoryDescribeResponse> {
        not_served()
    }

    async fn list_spaces(
        &self,
        _principal: &Principal,
        _request: MemorySpacesListRequest,
    ) -> SpokeResult<MemorySpacesListResponse> {
        not_served()
    }

    async fn freeze_space(
        &self,
        _principal: &Principal,
        _request: MemorySpacesFreezeRequest,
    ) -> SpokeResult<MemorySpacesFreezeResponse> {
        not_served()
    }

    async fn unfreeze_space(
        &self,
        _principal: &Principal,
        _request: MemorySpacesUnfreezeRequest,
    ) -> SpokeResult<MemorySpacesUnfreezeResponse> {
        not_served()
    }

    async fn ensure_subject(
        &self,
        _principal: &Principal,
        _request: MemorySubjectsEnsureRequest,
    ) -> SpokeResult<MemorySubjectsEnsureResponse> {
        not_served()
    }

    async fn get_subject(
        &self,
        _principal: &Principal,
        _request: MemorySubjectsGetRequest,
    ) -> SpokeResult<MemorySubjectsGetResponse> {
        not_served()
    }

    async fn fence_subject(
        &self,
        _principal: &Principal,
        _request: MemorySubjectsFenceRequest,
    ) -> SpokeResult<MemorySubjectsFenceResponse> {
        not_served()
    }

    async fn list_grants(
        &self,
        _principal: &Principal,
        _request: MemoryGrantsListRequest,
    ) -> SpokeResult<MemoryGrantsListResponse> {
        not_served()
    }

    async fn grant(
        &self,
        _principal: &Principal,
        _request: MemoryGrantsGrantRequest,
    ) -> SpokeResult<MemoryGrantsGrantResponse> {
        not_served()
    }

    async fn revoke_grant(
        &self,
        _principal: &Principal,
        _request: MemoryGrantsRevokeRequest,
    ) -> SpokeResult<MemoryGrantsRevokeResponse> {
        not_served()
    }

    async fn capture(
        &self,
        principal: &Principal,
        request: MemoryCaptureRequest,
    ) -> SpokeResult<MemoryCaptureResponse> {
        let value = match self.store.capture(principal, wire(&request)) {
            SpokeResult::Ok(value) => value,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        match serde_json::from_value(value) {
            Ok(response) => SpokeResult::Ok(response),
            Err(error) => reject(
                SpokeRejectCode::InternalError,
                format!("capture response decode failed: {error}"),
            ),
        }
    }

    async fn list_candidates(
        &self,
        _principal: &Principal,
        _request: MemoryCandidatesListRequest,
    ) -> SpokeResult<MemoryCandidatesListResponse> {
        not_served()
    }

    async fn get_candidate(
        &self,
        _principal: &Principal,
        _request: MemoryCandidatesGetRequest,
    ) -> SpokeResult<MemoryCandidatesGetResponse> {
        not_served()
    }

    async fn dismiss_candidate(
        &self,
        _principal: &Principal,
        _request: MemoryCandidatesDismissRequest,
    ) -> SpokeResult<MemoryCandidatesDismissResponse> {
        not_served()
    }

    async fn review_candidate(
        &self,
        principal: &Principal,
        request: MemoryCandidatesReviewRequest,
    ) -> SpokeResult<MemoryCandidatesReviewResponse> {
        let value = match self.store.review(principal, wire(&request)) {
            SpokeResult::Ok(value) => value,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        match serde_json::from_value(value) {
            Ok(response) => SpokeResult::Ok(response),
            Err(error) => reject(
                SpokeRejectCode::InternalError,
                format!("review response decode failed: {error}"),
            ),
        }
    }

    async fn create_record(
        &self,
        _principal: &Principal,
        _request: MemoryRecordsCreateRequest,
    ) -> SpokeResult<MemoryRecordsCreateResponse> {
        not_served()
    }

    async fn get_record(
        &self,
        _principal: &Principal,
        _request: MemoryRecordsGetRequest,
    ) -> SpokeResult<MemoryRecordsGetResponse> {
        not_served()
    }

    async fn update_record(
        &self,
        _principal: &Principal,
        _request: MemoryRecordsUpdateRequest,
    ) -> SpokeResult<MemoryRecordsUpdateResponse> {
        not_served()
    }

    async fn delete_record(
        &self,
        _principal: &Principal,
        _request: MemoryRecordsDeleteRequest,
    ) -> SpokeResult<MemoryRecordsDeleteResponse> {
        not_served()
    }

    async fn list_records(
        &self,
        _principal: &Principal,
        _request: MemoryRecordsListRequest,
    ) -> SpokeResult<MemoryRecordsListResponse> {
        not_served()
    }

    async fn promote_record(
        &self,
        _principal: &Principal,
        _request: MemoryRecordsPromoteRequest,
    ) -> SpokeResult<MemoryRecordsPromoteResponse> {
        not_served()
    }

    async fn recall(
        &self,
        principal: &Principal,
        request: MemoryRecallRequest,
    ) -> SpokeResult<MemoryRecallResponse> {
        let value = match self.store.recall(principal, wire(&request)) {
            SpokeResult::Ok(value) => value,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        match serde_json::from_value(value) {
            Ok(response) => SpokeResult::Ok(response),
            Err(error) => reject(
                SpokeRejectCode::InternalError,
                format!("recall response decode failed: {error}"),
            ),
        }
    }

    async fn get_profile(
        &self,
        _principal: &Principal,
        _request: MemoryProfileGetRequest,
    ) -> SpokeResult<MemoryProfileGetResponse> {
        not_served()
    }

    async fn update_profile(
        &self,
        _principal: &Principal,
        _request: MemoryProfileUpdateRequest,
    ) -> SpokeResult<MemoryProfileUpdateResponse> {
        not_served()
    }

    async fn get_reflection_state(
        &self,
        _principal: &Principal,
        _request: MemoryReflectionsStateRequest,
    ) -> SpokeResult<MemoryReflectionsStateResponse> {
        not_served()
    }

    async fn list_reflections(
        &self,
        _principal: &Principal,
        _request: MemoryReflectionsListRequest,
    ) -> SpokeResult<MemoryReflectionsListResponse> {
        not_served()
    }

    async fn generate_reflection(
        &self,
        _principal: &Principal,
        _request: MemoryReflectionsGenerateRequest,
    ) -> SpokeResult<MemoryReflectionsGenerateResponse> {
        not_served()
    }

    async fn get_receipt(
        &self,
        _principal: &Principal,
        _request: MemoryReceiptsGetRequest,
    ) -> SpokeResult<MemoryReceiptsGetResponse> {
        not_served()
    }

    async fn create_export(
        &self,
        _principal: &Principal,
        _request: MemoryExportsCreateRequest,
    ) -> SpokeResult<MemoryExportsCreateResponse> {
        not_served()
    }

    async fn get_export(
        &self,
        _principal: &Principal,
        _request: MemoryExportsGetRequest,
    ) -> SpokeResult<MemoryExportsGetResponse> {
        not_served()
    }

    async fn stage_import(
        &self,
        _principal: &Principal,
        _request: MemoryImportsStageRequest,
    ) -> SpokeResult<MemoryImportsStageResponse> {
        not_served()
    }

    async fn activate_import(
        &self,
        _principal: &Principal,
        _request: MemoryImportsActivateRequest,
    ) -> SpokeResult<MemoryImportsActivateResponse> {
        not_served()
    }

    async fn discard_import(
        &self,
        _principal: &Principal,
        _request: MemoryImportsDiscardRequest,
    ) -> SpokeResult<MemoryImportsDiscardResponse> {
        not_served()
    }
}

// ── Session plumbing ──────────────────────────────────────────────────────

fn principal_for(peer_id: &str) -> SpokeResult<Principal> {
    match serde_json::from_value(json!({
        "principal_id": format!("prn_{peer_id}"),
        "extensions": {},
    })) {
        Ok(principal) => SpokeResult::Ok(principal),
        Err(error) => reject(
            SpokeRejectCode::InternalError,
            format!("principal decode failed: {error}"),
        ),
    }
}

fn admission() -> MemoryAdmission {
    Arc::new(|peer_id: &str| principal_for(peer_id))
}

/// The serving face: the typed `MemoryPort` adapter (the P2 boundary).
fn memory_face(host: &Arc<MemoryHost>) -> Arc<dyn spoke_connect::remote::RemoteMemoryService> {
    let host: Arc<MemoryHost> = Arc::clone(host);
    let port: Arc<dyn MemoryPort + Send + Sync> = host;
    Arc::new(MemoryPortService::new(port, admission()))
}

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

/// Start a responder whose ports face carries the memory service (when given)
/// and whose manifest advertises `capabilities`.
async fn start_host(
    memory: Option<Arc<dyn spoke_connect::remote::RemoteMemoryService>>,
    capabilities: &[&str],
) -> (Arc<ConnectResponder>, LoopbackTransportPair) {
    start_host_with_budget(memory, capabilities, None).await
}

async fn start_host_with_budget(
    memory: Option<Arc<dyn spoke_connect::remote::RemoteMemoryService>>,
    capabilities: &[&str],
    serve_budget_ms: Option<u64>,
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
        invoke_timeout_ms: serve_budget_ms,
    })
    .await;
    (responder, pair)
}

/// Dial the typed client over the loopback pair.
async fn dial(pair: LoopbackTransportPair, manifest: &HostCapabilityManifest) -> Arc<RemoteAdapter> {
    let peer_id_host = derive_peer_id_from_ed25519_pubkey(&pubkey_host());
    let _: Arc<dyn Transport> = Arc::new(pair.client.clone());
    connect_remote_adapter(RemoteAdapterOptions {
        transport: Arc::new(pair.client),
        local_identity: RemoteIdentity {
            seed: seed_client(),
        },
        local_manifest: manifest.clone(),
        remote_pubkey: pubkey_host(),
        allowlist: vec![peer_id_host],
        invoke_timeout_ms: None,
        capability_token: None,
    })
    .await
    .expect("dial")
}

fn pubkey_host() -> [u8; 32] {
    ed25519_dalek::SigningKey::from_bytes(&seed_host())
        .verifying_key()
        .to_bytes()
}

/// The local admission context of the dialing peer. Over the wire the host
/// derives the principal from the authenticated peer, so this value is never
/// transmitted (MEM-R003).
fn client_principal() -> Principal {
    expect_ok(principal_for(&derive_peer_id_from_ed25519_pubkey(
        &pubkey_client(),
    )))
}

// ── Typed request fixtures ────────────────────────────────────────────────

fn capture_request(idempotency_key: &str, context: &str, text: &str) -> Value {
    json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "content": { "text": text },
        "capture_scope": { "visibility": "context", "context_id": context },
        "source_context_ids": [context],
        "kind_hint": "fact",
        "idempotency_key": idempotency_key,
    })
}

fn shared_capture_request(idempotency_key: &str, text: &str) -> Value {
    json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "content": { "text": text },
        "capture_scope": { "visibility": "shared" },
        "kind_hint": "fact",
        "idempotency_key": idempotency_key,
    })
}

fn review_request(space_id: &str, candidate_id: &str, key: &str, text: &str) -> Value {
    json!({
        "space_id": space_id,
        "candidate_id": candidate_id,
        "expected_revision": 1,
        "decision": {
            "action": "fragment",
            "fragments": [{ "kind": "fact", "content": { "text": text } }],
        },
        "idempotency_key": key,
    })
}

fn fragmented_review_request(
    space_id: &str,
    candidate_id: &str,
    key: &str,
    fragments: &[(&str, &str)],
) -> Value {
    let fragments: Vec<Value> = fragments
        .iter()
        .map(|(kind, text)| json!({ "kind": kind, "content": { "text": text } }))
        .collect();
    json!({
        "space_id": space_id,
        "candidate_id": candidate_id,
        "expected_revision": 1,
        "decision": { "action": "fragment", "fragments": fragments },
        "idempotency_key": key,
    })
}

fn recall_request(context: &str, query: &str, max_items: u64) -> Value {
    json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "selection": { "mode": "selected_contexts", "context_ids": [context] },
        "query": query,
        "budget": { "max_items": max_items },
    })
}

fn shared_recall_request(max_items: u64) -> Value {
    json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "selection": { "mode": "shared_only" },
        "budget": { "max_items": max_items },
    })
}

fn typed_capture(request: Value) -> MemoryCaptureRequest {
    serde_json::from_value(request).expect("valid memory.capture request")
}

fn typed_review(request: Value) -> MemoryCandidatesReviewRequest {
    serde_json::from_value(request).expect("valid memory.candidates.review request")
}

fn typed_recall(request: Value) -> MemoryRecallRequest {
    serde_json::from_value(request).expect("valid memory.recall request")
}

// ── Raw-wire client (unknown-op fallback only) ────────────────────────────

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

async fn raw_invoke(
    client: &LoopbackTransport,
    session_id: &str,
    sequence: i64,
    request_id: &str,
    op: &str,
    payload: Value,
) -> Value {
    let request = sign_invoke_request(seed_client(), session_id, sequence, request_id, op, payload);
    client
        .send(&serde_json::to_vec(&request).expect("request bytes"))
        .await
        .expect("request send");
    serde_json::from_slice(&client.recv().await.expect("response recv")).expect("response decode")
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn memory_loopback_round_trips_capture_review_and_recall_through_the_typed_port() {
    let host = granted_host();
    let (responder, pair) = start_host(Some(memory_face(&host)), &MEMORY_CAPABILITIES).await;
    let manifest = memory_manifest("test-client", &MEMORY_CAPABILITIES);
    let client = dial(pair, &manifest).await;
    let principal = client_principal();

    let request = capture_request(
        "cap-loopback-1",
        CONTEXT,
        "Mira told Bo the gate key is hidden under the third barrel.",
    );
    let captured = expect_ok(
        client
            .capture(&principal, typed_capture(request.clone()))
            .await,
    );
    let receipt = wire(&captured);
    assert_eq!(receipt["status"], "committed", "{receipt}");
    assert_eq!(receipt["op"], "memory.capture");
    assert_eq!(receipt["idempotency_key"], "cap-loopback-1");
    // The provider admits the authenticated dialing peer as the requester; the
    // payload never carries a principal (MEM-R003).
    assert_eq!(
        receipt["requester_principal_id"],
        format!(
            "prn_{}",
            derive_peer_id_from_ed25519_pubkey(&pubkey_client())
        ),
        "the receipt names the authenticated peer, got {receipt}"
    );
    assert!(
        request.get("requester_principal_id").is_none(),
        "the capture payload carries no principal"
    );
    assert_eq!(receipt["result"]["classified"], 1, "{receipt}");
    // §6.1: the digest is SHA-256 over the JCS canonicalization of the whole
    // request object (idempotency_key included) — recomputed here
    // independently of the fixture's receipt builder.
    let expected_digest = match jcs_sha256_hex(&request) {
        SpokeResult::Ok(digest) => digest,
        SpokeResult::Reject(reject) => panic!("request digest: {reject:?}"),
    };
    assert_eq!(
        receipt["payload_digest"], expected_digest,
        "receipt payload_digest must be the §6.1 JCS digest of the request, got {receipt}"
    );
    let candidate_id = receipt["result"]["candidate_ids"][0]
        .as_str()
        .unwrap_or_else(|| panic!("candidate id expected, got {receipt}"))
        .to_owned();

    let reviewed = expect_ok(
        client
            .review_candidate(
                &principal,
                typed_review(fragmented_review_request(
                    SPACE,
                    &candidate_id,
                    "review-cand-1",
                    &[
                        (
                            "fact",
                            "The gate key is hidden under the third barrel.",
                        ),
                        ("event", "Mira revealed the key location to Bo."),
                    ],
                )),
            )
            .await,
    );
    let review_receipt = wire(&reviewed);
    assert_eq!(review_receipt["status"], "committed", "{review_receipt}");
    assert_eq!(
        review_receipt["result"]["outcome"], "fragmented",
        "{review_receipt}"
    );
    assert_eq!(
        review_receipt["result"]["record_ids"]
            .as_array()
            .map(Vec::len),
        Some(2),
        "{review_receipt}"
    );

    let recalled = expect_ok(
        client
            .recall(
                &principal,
                typed_recall(recall_request(CONTEXT, "Where is the gate key?", 8)),
            )
            .await,
    );
    let page = wire(&recalled);
    let records = page["records"].as_array().expect("records array");
    assert_eq!(records.len(), 2, "{page}");
    assert_eq!(page["total_authorized"], 2);
    assert_eq!(page["truncated"], false);
    assert_eq!(page["engine_policy_version"], ENGINE_POLICY_VERSION);
    // Ranking (§9 policy: query overlap then kind weight): the fact fragment
    // matches the query tokens and outranks the event fragment.
    assert_eq!(records[0]["kind"], "fact", "{page}");
    assert_eq!(records[1]["kind"], "event", "{page}");
    // Placement inheritance: a review decision carries no placement field, so
    // the recalled records carry the captured candidate's scope.
    for record in records {
        assert_eq!(
            record["scope"],
            json!({ "visibility": "context", "context_id": CONTEXT }),
            "review outcomes inherit the candidate placement, got {record}"
        );
    }

    // Budget: a one-item cap truncates the same ranking and keeps the totals.
    let truncated = expect_ok(
        client
            .recall(
                &principal,
                typed_recall(recall_request(CONTEXT, "Where is the gate key?", 1)),
            )
            .await,
    );
    let page = wire(&truncated);
    let records = page["records"].as_array().expect("records array");
    assert_eq!(records.len(), 1, "{page}");
    assert_eq!(page["truncated"], true);
    assert_eq!(page["total_authorized"], 2);
    assert_eq!(records[0]["kind"], "fact", "{page}");

    responder.close();
    client.close();
}

#[tokio::test]
async fn memory_loopback_enforces_grants_idempotency_and_selection_isolation() {
    // Denial: a provider state with no grant for the caller.
    let host = ungranted_host();
    let (responder, pair) = start_host(Some(memory_face(&host)), &MEMORY_CAPABILITIES).await;
    let manifest = memory_manifest("test-client", &MEMORY_CAPABILITIES);
    let client = dial(pair, &manifest).await;
    let principal = client_principal();

    let denied = expect_reject(
        client
            .capture(
                &principal,
                typed_capture(capture_request(
                    "cap-denied",
                    CONTEXT,
                    "Mira told Bo the gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(
        denied.code,
        SpokeRejectCode::NotAuthorized,
        "an ungranted capture must deny: {denied:?}"
    );
    let denied = expect_reject(
        client
            .recall(
                &principal,
                typed_recall(recall_request(CONTEXT, "Where is the gate key?", 8)),
            )
            .await,
    );
    assert_eq!(
        denied.code,
        SpokeRejectCode::NotAuthorized,
        "an ungranted recall must deny: {denied:?}"
    );
    {
        let state = host.store.state.lock().expect("state lock");
        assert_eq!(state.candidates.len(), 0, "denied captures write nothing");
        assert_eq!(state.receipts.len(), 0, "denied ops issue no receipt");
        assert_eq!(state.records.len(), 0);
    }
    responder.close();
    client.close();

    // Granted provider: idempotent replay, digest conflict, selection + space
    // isolation.
    let host = granted_host();
    host.store
        .seed_record(SPACE, OTHER_CONTEXT, "fact", "Bo sold rope at the market.");
    host.store.seed_record(
        OTHER_SPACE,
        CONTEXT,
        "fact",
        "A copy of the run stored in another space.",
    );
    let (responder, pair) = start_host(Some(memory_face(&host)), &MEMORY_CAPABILITIES).await;
    let client = dial(pair, &manifest).await;
    let principal = client_principal();

    let request = capture_request(
        "cap-replay",
        CONTEXT,
        "Mira told Bo the gate key is hidden under the third barrel.",
    );
    let first = expect_ok(
        client
            .capture(&principal, typed_capture(request.clone()))
            .await,
    );
    let replay = expect_ok(client.capture(&principal, typed_capture(request.clone())).await);
    assert_eq!(
        wire(&replay)["receipt_id"],
        wire(&first)["receipt_id"],
        "same key + same payload replays the original receipt"
    );
    assert_eq!(
        host.store.state.lock().expect("state lock").candidates.len(),
        1,
        "replay must not double-apply"
    );

    let conflict = expect_reject(
        client
            .capture(
                &principal,
                typed_capture(capture_request(
                    "cap-replay",
                    CONTEXT,
                    "A different payload under the same idempotency key.",
                )),
            )
            .await,
    );
    assert_eq!(
        conflict.code,
        SpokeRejectCode::IdempotencyConflict,
        "same key + different payload digest must conflict: {conflict:?}"
    );

    // One retained record inside the selected context.
    let candidate_id = wire(&first)["result"]["candidate_ids"][0]
        .as_str()
        .unwrap_or_else(|| panic!("candidate id expected, got {:?}", wire(&first)))
        .to_owned();
    expect_ok(
        client
            .review_candidate(
                &principal,
                typed_review(review_request(
                    SPACE,
                    &candidate_id,
                    "review-isolation",
                    "The gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );

    // Isolation: the seeded OTHER_CONTEXT and OTHER_SPACE records are
    // authorized state but outside the selection, so they never appear.
    let recalled = expect_ok(
        client
            .recall(&principal, typed_recall(recall_request(CONTEXT, "", 8)))
            .await,
    );
    let page = wire(&recalled);
    let records = page["records"]
        .as_array()
        .unwrap_or_else(|| panic!("recall must answer records, got {page}"));
    assert_eq!(
        records.len(),
        1,
        "only the selected context in this space is authorized input: {page}"
    );
    assert_eq!(
        records[0]["scope"],
        json!({ "visibility": "context", "context_id": CONTEXT })
    );
    assert_eq!(page["total_authorized"], 1);
    responder.close();
    client.close();
}

#[tokio::test]
async fn memory_loopback_rejects_mismatched_review_space_and_rechecks_on_replay() {
    let host = Arc::new(MemoryHost {
        store: Arc::new(Store::default()),
    });
    let principal_id = format!("prn_{}", derive_peer_id_from_ed25519_pubkey(&pubkey_client()));
    host.store.add_grant(
        &principal_id,
        SPACE,
        Some(SUBJECT),
        &["capture", "records_write"],
        &[CONTEXT],
        false,
    );
    host.store.add_grant(
        &principal_id,
        OTHER_SPACE,
        Some(SUBJECT),
        &["review", "records_write"],
        &[CONTEXT],
        false,
    );

    let manifest = memory_manifest("test-client", &MEMORY_CAPABILITIES);
    let (responder, pair) = start_host(Some(memory_face(&host)), &MEMORY_CAPABILITIES).await;
    let client = dial(pair, &manifest).await;
    let principal = client_principal();

    let captured = expect_ok(
        client
            .capture(
                &principal,
                typed_capture(capture_request(
                    "cap-cross-space",
                    CONTEXT,
                    "Mira told Bo the gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    let candidate_id = wire(&captured)["result"]["candidate_ids"][0]
        .as_str()
        .expect("candidate id")
        .to_owned();
    let receipts_after_capture = host.store.state.lock().expect("state lock").receipts.len();

    // Addressing another space is a different operation (§7.0.1/§7.15).
    let cross_space = expect_reject(
        client
            .review_candidate(
                &principal,
                typed_review(review_request(
                    OTHER_SPACE,
                    &candidate_id,
                    "review-cross-space",
                    "The gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(
        cross_space.code,
        SpokeRejectCode::NotAuthorized,
        "a review addressing another space must be rejected: {cross_space:?}"
    );
    {
        let state = host.store.state.lock().expect("state lock");
        assert!(!state.candidates[0].consumed, "denied review consumes nothing");
        assert_eq!(state.records.len(), 0, "denied review writes no record");
        assert_eq!(
            state.receipts.len(),
            receipts_after_capture,
            "denied review issues no receipt"
        );
    }

    // The mismatch is rejected on addressing alone.
    host.store.add_grant(
        &principal_id,
        SPACE,
        Some(SUBJECT),
        &["review"],
        &[CONTEXT],
        false,
    );
    let still_mismatched = expect_reject(
        client
            .review_candidate(
                &principal,
                typed_review(review_request(
                    OTHER_SPACE,
                    &candidate_id,
                    "review-space-mismatch",
                    "The gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(
        still_mismatched.code,
        SpokeRejectCode::NotAuthorized,
        "the addressed space is part of the operation identity: {still_mismatched:?}"
    );
    assert!(
        !host.store.state.lock().expect("state lock").candidates[0].consumed,
        "a mismatched address consumes nothing"
    );

    // Positive control: the request names the candidate's own space.
    let reviewed = expect_ok(
        client
            .review_candidate(
                &principal,
                typed_review(review_request(
                    SPACE,
                    &candidate_id,
                    "review-in-space",
                    "The gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    let receipt = wire(&reviewed);
    assert_eq!(receipt["status"], "committed", "{receipt}");
    assert_eq!(receipt["space_id"], SPACE, "{receipt}");
    {
        let state = host.store.state.lock().expect("state lock");
        assert!(state.candidates[0].consumed, "the addressed review consumes");
        assert_eq!(state.records.len(), 1);
        assert_eq!(state.records[0].space_id, SPACE);
    }

    // A second mutating op, issued before coverage is withdrawn.
    let capture_body = capture_request(
        "cap-revoked",
        CONTEXT,
        "Mira told Bo the gate key is hidden under the third barrel.",
    );
    expect_ok(
        client
            .capture(&principal, typed_capture(capture_body.clone()))
            .await,
    );

    // Replay after coverage is withdrawn: the original receipt is never
    // disclosed to a caller who no longer has coverage (MEM-R021).
    host.store.state.lock().expect("state lock").grants.clear();
    let issued_before_replay = host.store.state.lock().expect("state lock").receipts.len();
    let replayed_review = expect_reject(
        client
            .review_candidate(
                &principal,
                typed_review(review_request(
                    SPACE,
                    &candidate_id,
                    "review-in-space",
                    "The gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(
        replayed_review.code,
        SpokeRejectCode::NotAuthorized,
        "review replay without current coverage must deny, not disclose: {replayed_review:?}"
    );
    let replayed_capture = expect_reject(
        client
            .capture(&principal, typed_capture(capture_body))
            .await,
    );
    assert_eq!(
        replayed_capture.code,
        SpokeRejectCode::NotAuthorized,
        "capture replay without current coverage must deny, not disclose: {replayed_capture:?}"
    );
    assert_eq!(
        host.store.state.lock().expect("state lock").receipts.len(),
        issued_before_replay,
        "denied replays issue no new receipt"
    );
    responder.close();
    client.close();
}

#[tokio::test]
async fn memory_loopback_enforces_shared_placement_coverage() {
    let host = Arc::new(MemoryHost {
        store: Arc::new(Store::default()),
    });
    let principal_id = format!("prn_{}", derive_peer_id_from_ed25519_pubkey(&pubkey_client()));
    // Restricted grant: non-empty context coverage, no shared access.
    host.store.add_grant(
        &principal_id,
        SPACE,
        Some(SUBJECT),
        &["capture", "review", "records_write", "recall"],
        &[CONTEXT],
        false,
    );
    let shared_candidate = host.store.seed_candidate(None);

    let manifest = memory_manifest("test-client", &MEMORY_CAPABILITIES);
    let (responder, pair) = start_host(Some(memory_face(&host)), &MEMORY_CAPABILITIES).await;
    let client = dial(pair, &manifest).await;
    let principal = client_principal();

    // Shared placement is denied on capture, review and read without
    // `shared_access`.
    let denied_capture = expect_reject(
        client
            .capture(
                &principal,
                typed_capture(shared_capture_request(
                    "cap-shared",
                    "Mira told Bo the gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(
        denied_capture.code,
        SpokeRejectCode::NotAuthorized,
        "a shared placement needs shared access: {denied_capture:?}"
    );
    let denied_review = expect_reject(
        client
            .review_candidate(
                &principal,
                typed_review(review_request(
                    SPACE,
                    &shared_candidate,
                    "review-shared-denied",
                    "The gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(
        denied_review.code,
        SpokeRejectCode::NotAuthorized,
        "reviewing shared placement needs shared access: {denied_review:?}"
    );
    assert!(
        !host.store.state.lock().expect("state lock").candidates[0].consumed,
        "denied shared review consumes nothing"
    );
    let denied_recall = expect_reject(
        client
            .recall(&principal, typed_recall(shared_recall_request(8)))
            .await,
    );
    assert_eq!(
        denied_recall.code,
        SpokeRejectCode::NotAuthorized,
        "a shared read needs shared access: {denied_recall:?}"
    );

    // Corrected positive control: a restricted grant with non-empty context
    // coverage AND shared access.
    host.store.add_grant(
        &principal_id,
        SPACE,
        Some(SUBJECT),
        &["capture", "review", "records_write", "recall"],
        &[CONTEXT],
        true,
    );
    let captured = expect_ok(
        client
            .capture(
                &principal,
                typed_capture(shared_capture_request(
                    "cap-shared",
                    "Mira told Bo the gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(wire(&captured)["status"], "committed");
    let reviewed = expect_ok(
        client
            .review_candidate(
                &principal,
                typed_review(review_request(
                    SPACE,
                    &shared_candidate,
                    "review-shared",
                    "The gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(wire(&reviewed)["result"]["outcome"], "fragmented");

    let recalled = expect_ok(
        client
            .recall(&principal, typed_recall(shared_recall_request(8)))
            .await,
    );
    let page = wire(&recalled);
    let records = page["records"]
        .as_array()
        .unwrap_or_else(|| panic!("shared recall must answer records, got {page}"));
    assert_eq!(records.len(), 1, "{page}");
    assert_eq!(records[0]["scope"], json!({ "visibility": "shared" }));
    responder.close();
    client.close();
}

#[tokio::test]
async fn memory_loopback_denies_unnegotiated_unserved_and_unknown_memory_ops() {
    // (a) Known op, capability not negotiated: `capability_missing` before any
    // handler runs — zero side effects. The host outlives the denied invoke so
    // its state can be inspected.
    let host = granted_host();
    let (responder, pair) = start_host(Some(memory_face(&host)), &["spoke-baseline"]).await;
    let manifest = memory_manifest("test-client", &["spoke-baseline"]);
    let client = dial(pair, &manifest).await;
    let denied = expect_reject(
        client
            .capture(
                &client_principal(),
                typed_capture(capture_request(
                    "cap-denied",
                    CONTEXT,
                    "Mira told Bo the gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(
        denied.code,
        SpokeRejectCode::CapabilityPortMissing,
        "an unnegotiated memory op is a capability deny: {denied:?}"
    );
    assert_eq!(
        reject_wire_code(&denied).as_deref(),
        Some("capability_missing"),
        "{denied:?}"
    );
    assert_eq!(
        denied
            .details
            .as_ref()
            .and_then(|details| details.get("retry_class"))
            .and_then(Value::as_str),
        Some("terminal"),
        "§14.2: a memory-boundary deny is classified terminal: {denied:?}"
    );
    {
        let state = host.store.state.lock().expect("state lock");
        assert_eq!(state.candidates.len(), 0, "a capability deny admits no candidate");
        assert_eq!(state.receipts.len(), 0, "a capability deny issues no receipt");
        assert_eq!(state.records.len(), 0, "a capability deny writes no record");
    }
    responder.close();
    client.close();

    // (b) Capability negotiated but the host provides no memory face: the
    // dispatch-deny branch, `op_unsupported`.
    let (responder, pair) = start_host(None, &MEMORY_CAPABILITIES).await;
    let client = dial(pair, &manifest_with(&MEMORY_CAPABILITIES)).await;
    let unserved = expect_reject(
        client
            .capture(
                &client_principal(),
                typed_capture(capture_request(
                    "cap-unserved",
                    CONTEXT,
                    "Mira told Bo the gate key is hidden under the third barrel.",
                )),
            )
            .await,
    );
    assert_eq!(unserved.code, SpokeRejectCode::CapabilityPortMissing);
    assert_eq!(
        reject_wire_code(&unserved).as_deref(),
        Some("op_unsupported"),
        "{unserved:?}"
    );
    assert!(
        unserved.message.contains("no memory service configured"),
        "{unserved:?}"
    );
    assert_eq!(
        unserved
            .details
            .as_ref()
            .and_then(|details| details.get("retry_class"))
            .and_then(Value::as_str),
        Some("terminal"),
        "§14.2: an absent memory face is classified terminal: {unserved:?}"
    );
    responder.close();
    client.close();

    // (c) Unknown `memory.*` op: no dispatch row, fail-closed `op_unsupported`.
    // The typed client cannot express an unknown op, so this rides the raw
    // wire (the client above moved the transport).
    let (responder, pair) = start_host(Some(memory_face(&granted_host())), &MEMORY_CAPABILITIES).await;
    let raw_manifest = memory_manifest("test-client", &MEMORY_CAPABILITIES);
    let session_id = handshake(&pair.client, &raw_manifest).await;
    let unknown = raw_invoke(
        &pair.client,
        &session_id,
        0,
        "memory-unknown",
        "memory.archive.purge",
        json!({}),
    )
    .await;
    assert_eq!(unknown["error"]["code"], "op_unsupported", "{unknown}");
    responder.close();
}

fn manifest_with(capabilities: &[&str]) -> HostCapabilityManifest {
    memory_manifest("test-client", capabilities)
}

fn granted_store() -> Arc<Store> {
    let peer_id = derive_peer_id_from_ed25519_pubkey(&pubkey_client());
    Store::with_grant(&format!("prn_{peer_id}"), &[CONTEXT, OTHER_CONTEXT])
}

fn granted_host() -> Arc<MemoryHost> {
    Arc::new(MemoryHost {
        store: granted_store(),
    })
}

fn ungranted_host() -> Arc<MemoryHost> {
    Arc::new(MemoryHost {
        store: Arc::new(Store::default()),
    })
}

// ── Golden vectors driven through the client adapter ──────────────────────

/// Echoes the shared golden vector for every `memory.*` op it is asked for and
/// asserts the client encoded exactly the vector's request object.
struct VectorHost {
    vectors: Arc<Vec<Value>>,
}

#[async_trait]
impl spoke_connect::remote::RemoteMemoryService for VectorHost {
    async fn invoke(&self, _peer_id: &str, op: &str, payload: Value) -> SpokeResult<Value> {
        let vector = self
            .vectors
            .iter()
            .find(|vector| vector["op"] == op)
            .unwrap_or_else(|| panic!("no golden vector for {op}"));
        assert_eq!(
            payload, vector["request"],
            "{op} client payload drifted from the golden vector"
        );
        SpokeResult::Ok(vector["response"].clone())
    }
}

fn req<T: serde::de::DeserializeOwned>(vector: &Value) -> T {
    serde_json::from_value(vector["request"].clone())
        .unwrap_or_else(|error| panic!("{} request does not decode: {error}", vector["op"]))
}

#[tokio::test]
async fn memory_loopback_golden_vectors_round_trip_through_the_client_adapter() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/memory-invoke-golden.json"))
        .expect("golden fixture parses");
    let vectors = fixture["vectors"]
        .as_array()
        .expect("fixture carries vectors")
        .clone();
    assert!(vectors.len() >= 8, "the shared vector set is representative");

    let host = Arc::new(VectorHost {
        vectors: Arc::new(vectors.clone()),
    });
    let (responder, pair) = start_host(Some(host), &MEMORY_CAPABILITIES).await;
    let client = dial(pair, &memory_manifest("test-client", &MEMORY_CAPABILITIES)).await;
    let principal = client_principal();

    for vector in &vectors {
        let op = vector["op"].as_str().expect("vector op");
        let response = match op {
            "memory.describe" => wire(&expect_ok(
                client.describe(&principal, req::<MemoryDescribeRequest>(vector)).await,
            )),
            "memory.capture" => wire(&expect_ok(
                client.capture(&principal, req::<MemoryCaptureRequest>(vector)).await,
            )),
            "memory.candidates.review" => wire(&expect_ok(
                client
                    .review_candidate(&principal, req::<MemoryCandidatesReviewRequest>(vector))
                    .await,
            )),
            "memory.recall" => wire(&expect_ok(
                client.recall(&principal, req::<MemoryRecallRequest>(vector)).await,
            )),
            "memory.profile.update" => wire(&expect_ok(
                client
                    .update_profile(&principal, req::<MemoryProfileUpdateRequest>(vector))
                    .await,
            )),
            "memory.exports.create" => wire(&expect_ok(
                client
                    .create_export(&principal, req::<MemoryExportsCreateRequest>(vector))
                    .await,
            )),
            "memory.exports.get" => wire(&expect_ok(
                client
                    .get_export(&principal, req::<MemoryExportsGetRequest>(vector))
                    .await,
            )),
            "memory.imports.stage" => wire(&expect_ok(
                client
                    .stage_import(&principal, req::<MemoryImportsStageRequest>(vector))
                    .await,
            )),
            "memory.receipts.get" => wire(&expect_ok(
                client
                    .get_receipt(&principal, req::<MemoryReceiptsGetRequest>(vector))
                    .await,
            )),
            other => panic!("vector {other} has no adapter arm"),
        };
        assert_eq!(
            response, vector["response"],
            "{op} response drifted from the golden vector"
        );
    }

    responder.close();
    client.close();
}


#[test]
fn memory_port_service_classifies_a_malformed_payload_as_terminal() {
    let port: Arc<dyn MemoryPort + Send + Sync> = granted_host();
    let service = MemoryPortService::new(port, admission());
    let outcome = futures::executor::block_on(service.invoke(
        "peer-invalid",
        "memory.capture",
        json!({ "space_id": 1 }),
    ));
    let reject = expect_reject(outcome);
    assert_eq!(reject.code, SpokeRejectCode::InvalidInput, "{reject:?}");
    assert_eq!(
        reject
            .details
            .as_ref()
            .and_then(|details| details.get("retry_class"))
            .and_then(Value::as_str),
        Some("terminal"),
        "§14.2: a memory-boundary validation failure is terminal: {reject:?}"
    );
}

/// Provider fixture whose mutation commits asynchronously: the responder's
/// budget expires while the commit is still in flight, which is exactly the
/// uncertain-outcome case (the transport cannot know whether it committed).
struct DelayedHost {
    store: Arc<Store>,
}

#[async_trait]
impl MemoryPort for DelayedHost {
    async fn describe(
        &self,
        _principal: &Principal,
        _request: MemoryDescribeRequest,
    ) -> SpokeResult<MemoryDescribeResponse> {
        not_served()
    }

    async fn list_spaces(
        &self,
        _principal: &Principal,
        _request: MemorySpacesListRequest,
    ) -> SpokeResult<MemorySpacesListResponse> {
        not_served()
    }

    async fn freeze_space(
        &self,
        _principal: &Principal,
        _request: MemorySpacesFreezeRequest,
    ) -> SpokeResult<MemorySpacesFreezeResponse> {
        not_served()
    }

    async fn unfreeze_space(
        &self,
        _principal: &Principal,
        _request: MemorySpacesUnfreezeRequest,
    ) -> SpokeResult<MemorySpacesUnfreezeResponse> {
        not_served()
    }

    async fn ensure_subject(
        &self,
        _principal: &Principal,
        _request: MemorySubjectsEnsureRequest,
    ) -> SpokeResult<MemorySubjectsEnsureResponse> {
        not_served()
    }

    async fn get_subject(
        &self,
        _principal: &Principal,
        _request: MemorySubjectsGetRequest,
    ) -> SpokeResult<MemorySubjectsGetResponse> {
        not_served()
    }

    async fn fence_subject(
        &self,
        _principal: &Principal,
        _request: MemorySubjectsFenceRequest,
    ) -> SpokeResult<MemorySubjectsFenceResponse> {
        not_served()
    }

    async fn list_grants(
        &self,
        _principal: &Principal,
        _request: MemoryGrantsListRequest,
    ) -> SpokeResult<MemoryGrantsListResponse> {
        not_served()
    }

    async fn grant(
        &self,
        _principal: &Principal,
        _request: MemoryGrantsGrantRequest,
    ) -> SpokeResult<MemoryGrantsGrantResponse> {
        not_served()
    }

    async fn revoke_grant(
        &self,
        _principal: &Principal,
        _request: MemoryGrantsRevokeRequest,
    ) -> SpokeResult<MemoryGrantsRevokeResponse> {
        not_served()
    }

    async fn capture(
        &self,
        principal: &Principal,
        request: MemoryCaptureRequest,
    ) -> SpokeResult<MemoryCaptureResponse> {
        let payload = wire(&request);
        let key = payload["idempotency_key"].as_str().unwrap_or_default();
        // A replay of an already-committed key answers from provider state
        // immediately (MEM-R008: replay never re-executes).
        if self.store.has_receipt("memory.capture", key) {
            return match self.store.capture(principal, payload) {
                SpokeResult::Ok(value) => match serde_json::from_value(value) {
                    Ok(response) => SpokeResult::Ok(response),
                    Err(error) => reject(
                        SpokeRejectCode::InternalError,
                        format!("capture response decode failed: {error}"),
                    ),
                },
                SpokeResult::Reject(reject) => SpokeResult::Reject(reject),
            };
        }
        let store = Arc::clone(&self.store);
        let principal = principal.clone();
        // Detached commit: the responder's budget may expire first, and the
        // transport has no way to know the outcome.
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            let _ = store.capture(&principal, payload);
        });
        // Outlive the budget; this wait is dropped when the responder expires.
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        not_served()
    }

    async fn list_candidates(
        &self,
        _principal: &Principal,
        _request: MemoryCandidatesListRequest,
    ) -> SpokeResult<MemoryCandidatesListResponse> {
        not_served()
    }

    async fn get_candidate(
        &self,
        _principal: &Principal,
        _request: MemoryCandidatesGetRequest,
    ) -> SpokeResult<MemoryCandidatesGetResponse> {
        not_served()
    }

    async fn dismiss_candidate(
        &self,
        _principal: &Principal,
        _request: MemoryCandidatesDismissRequest,
    ) -> SpokeResult<MemoryCandidatesDismissResponse> {
        not_served()
    }

    async fn review_candidate(
        &self,
        _principal: &Principal,
        _request: MemoryCandidatesReviewRequest,
    ) -> SpokeResult<MemoryCandidatesReviewResponse> {
        not_served()
    }

    async fn create_record(
        &self,
        _principal: &Principal,
        _request: MemoryRecordsCreateRequest,
    ) -> SpokeResult<MemoryRecordsCreateResponse> {
        not_served()
    }

    async fn get_record(
        &self,
        _principal: &Principal,
        _request: MemoryRecordsGetRequest,
    ) -> SpokeResult<MemoryRecordsGetResponse> {
        not_served()
    }

    async fn update_record(
        &self,
        _principal: &Principal,
        _request: MemoryRecordsUpdateRequest,
    ) -> SpokeResult<MemoryRecordsUpdateResponse> {
        not_served()
    }

    async fn delete_record(
        &self,
        _principal: &Principal,
        _request: MemoryRecordsDeleteRequest,
    ) -> SpokeResult<MemoryRecordsDeleteResponse> {
        not_served()
    }

    async fn list_records(
        &self,
        _principal: &Principal,
        _request: MemoryRecordsListRequest,
    ) -> SpokeResult<MemoryRecordsListResponse> {
        not_served()
    }

    async fn promote_record(
        &self,
        _principal: &Principal,
        _request: MemoryRecordsPromoteRequest,
    ) -> SpokeResult<MemoryRecordsPromoteResponse> {
        not_served()
    }

    async fn recall(
        &self,
        _principal: &Principal,
        _request: MemoryRecallRequest,
    ) -> SpokeResult<MemoryRecallResponse> {
        not_served()
    }

    async fn get_profile(
        &self,
        _principal: &Principal,
        _request: MemoryProfileGetRequest,
    ) -> SpokeResult<MemoryProfileGetResponse> {
        not_served()
    }

    async fn update_profile(
        &self,
        _principal: &Principal,
        _request: MemoryProfileUpdateRequest,
    ) -> SpokeResult<MemoryProfileUpdateResponse> {
        not_served()
    }

    async fn get_reflection_state(
        &self,
        _principal: &Principal,
        _request: MemoryReflectionsStateRequest,
    ) -> SpokeResult<MemoryReflectionsStateResponse> {
        not_served()
    }

    async fn list_reflections(
        &self,
        _principal: &Principal,
        _request: MemoryReflectionsListRequest,
    ) -> SpokeResult<MemoryReflectionsListResponse> {
        not_served()
    }

    async fn generate_reflection(
        &self,
        _principal: &Principal,
        _request: MemoryReflectionsGenerateRequest,
    ) -> SpokeResult<MemoryReflectionsGenerateResponse> {
        not_served()
    }

    async fn get_receipt(
        &self,
        _principal: &Principal,
        _request: MemoryReceiptsGetRequest,
    ) -> SpokeResult<MemoryReceiptsGetResponse> {
        not_served()
    }

    async fn create_export(
        &self,
        _principal: &Principal,
        _request: MemoryExportsCreateRequest,
    ) -> SpokeResult<MemoryExportsCreateResponse> {
        not_served()
    }

    async fn get_export(
        &self,
        _principal: &Principal,
        _request: MemoryExportsGetRequest,
    ) -> SpokeResult<MemoryExportsGetResponse> {
        not_served()
    }

    async fn stage_import(
        &self,
        _principal: &Principal,
        _request: MemoryImportsStageRequest,
    ) -> SpokeResult<MemoryImportsStageResponse> {
        not_served()
    }

    async fn activate_import(
        &self,
        _principal: &Principal,
        _request: MemoryImportsActivateRequest,
    ) -> SpokeResult<MemoryImportsActivateResponse> {
        not_served()
    }

    async fn discard_import(
        &self,
        _principal: &Principal,
        _request: MemoryImportsDiscardRequest,
    ) -> SpokeResult<MemoryImportsDiscardResponse> {
        not_served()
    }
}

#[tokio::test]
async fn memory_loopback_reports_an_uncertain_outcome_and_recovers_on_replay() {
    let store = granted_store();
    let delayed_port: Arc<dyn MemoryPort + Send + Sync> = Arc::new(DelayedHost {
        store: Arc::clone(&store),
    });
    let delayed: Arc<dyn RemoteMemoryService> =
        Arc::new(MemoryPortService::new(delayed_port, admission()));
    // Short responder budget: it expires while the provider's commit is still
    // in flight.
    let (responder, pair) =
        start_host_with_budget(Some(delayed), &MEMORY_CAPABILITIES, Some(50)).await;
    let client = dial(pair, &memory_manifest("test-client", &MEMORY_CAPABILITIES)).await;
    let principal = client_principal();
    let request = capture_request(
        "cap-uncertain",
        CONTEXT,
        "Mira told Bo the gate key is hidden under the third barrel.",
    );

    let timed_out = expect_reject(
        client
            .capture(&principal, typed_capture(request.clone()))
            .await,
    );
    assert_eq!(
        timed_out.code,
        SpokeRejectCode::OutcomeUncertain,
        "§14.3: an expired memory budget after invocation is uncertain, not internal: {timed_out:?}"
    );
    assert_eq!(
        timed_out
            .details
            .as_ref()
            .and_then(|details| details.get("retry_class"))
            .and_then(Value::as_str),
        Some("uncertain"),
        "{timed_out:?}"
    );

    // Let the provider's detached commit land, then recover via the same key.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let recovered = expect_ok(client.capture(&principal, typed_capture(request)).await);
    let receipt = wire(&recovered);
    assert_eq!(receipt["status"], "committed", "{receipt}");
    assert_eq!(receipt["idempotency_key"], "cap-uncertain", "{receipt}");
    assert_eq!(
        store.state.lock().expect("state lock").receipts.len(),
        1,
        "the replay reuses the committed receipt instead of double-applying"
    );

    responder.close();
    client.close();
}
