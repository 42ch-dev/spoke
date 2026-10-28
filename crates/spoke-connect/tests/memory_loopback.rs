//! Loopback test — the memory family served over Connect (`memory.*`).
//!
//! Two faces of the same in-memory provider are exercised:
//!
//! - the **typed boundary**: the host is a `MemoryPort` implementation served
//!   through `MemoryPortService` (the P2 port is the consumed boundary, and
//!   `memory.recall` runs over it);
//! - the **JSON seam**: capture / review ride `RemoteMemoryService` directly,
//!   because their generated success types (`MemoryCaptureResponse`,
//!   `MemoryCandidatesReviewResponse`) are empty-enum unions that cannot
//!   represent a committed receipt — see the note in the report.
//!
//! The provider enforces its own duties, independently of the engine: grant
//! coverage (`spoke_operations::grant_covers`), idempotent replay with
//! `IDEMPOTENCY_CONFLICT` on a changed payload digest, and space / selection /
//! principal filtering before the records reach engine scoring.
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
use serde_json::{json, Value};
use spoke_connect::core::{derive_peer_id_from_ed25519_pubkey, sign_hello_ed25519};
use spoke_connect::remote::{
    connect_responder, loopback_transport_pair, ConnectResponder, ConnectResponderOptions,
    LoopbackTransport, LoopbackTransportPair, MemoryAdmission, MemoryPortService, RemoteIdentity,
    RemoteMemoryService, RemoteServePorts, RemoteServePortsComposite, Transport,
};
use spoke_fixture_toy_world::ToyWorldAdapter;
use spoke_memory::classify::{classify_candidate, CandidateMaterial};
use spoke_memory::recall::{recall as engine_recall, RecallRecord, RecallRequest};
use spoke_memory::ENGINE_POLICY_VERSION;
use spoke_operations::{
    grant_covers, jcs_sha256_hex_excluding, spoke_reject, BaselinePorts, GrantRequirement,
    MemoryPort, SpokeRejectCode, SpokeResult,
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

fn not_served<T>() -> SpokeResult<T> {
    reject(
        SpokeRejectCode::InternalError,
        "not served by the memory loopback host",
    )
}

fn principal_id(principal: &Principal) -> &str {
    principal.principal_id.as_str()
}

/// `§6.10` payload digest over the request object without its idempotency
/// key: the replay comparison is about payload identity, not the key itself.
fn payload_digest(request: &Value) -> String {
    match jcs_sha256_hex_excluding(request, "idempotency_key") {
        SpokeResult::Ok(digest) => digest,
        SpokeResult::Reject(reject) => panic!("request digest: {reject:?}"),
    }
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
        let grant: MemoryGrant = serde_json::from_value(json!({
            "schema_version": 1,
            "grant_id": "grt_loopback",
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "principal_id": principal_id,
            "capabilities": ["capture", "review", "records_write", "recall"],
            "context_ids": contexts,
            "created_at": CAPTURED_AT,
            "revision": 1,
            "extensions": {},
        }))
        .expect("valid grant");
        let store = Store::default();
        store
            .state
            .lock()
            .expect("state lock")
            .grants
            .push(grant);
        Arc::new(store)
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

    fn remember(state: &mut State, namespace: (&str, &str, &str, &str), digest: &str, receipt: &Value) {
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
        let context_id = request["capture_scope"]["context_id"]
            .as_str()
            .map(str::to_string);
        let key = request["idempotency_key"].as_str().unwrap_or_default();
        let digest = payload_digest(&request);
        let mut state = self.state.lock().expect("state lock");
        if let Some(answer) =
            Store::replay(&state, (&space_id, &principal_id, "memory.capture", key), &digest)
        {
            return answer;
        }
        let requirement = GrantRequirement {
            capability: "capture",
            subject_id: Some(&subject_id),
            context_ids: context_id.iter().cloned().collect(),
            reads_shared: false,
            subject_all: false,
        };
        if !Store::authorized(&state, &principal_id, &space_id, &[requirement]) {
            return reject(
                SpokeRejectCode::NotAuthorized,
                format!("no capture grant covers {space_id}/{context_id:?}"),
            );
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
        let space_id = request["space_id"].as_str().unwrap_or_default().to_string();
        let candidate_id = request["candidate_id"].as_str().unwrap_or_default().to_string();
        let expected_revision = request["expected_revision"].as_u64().unwrap_or(0);
        let key = request["idempotency_key"].as_str().unwrap_or_default();
        let digest = payload_digest(&request);
        let mut state = self.state.lock().expect("state lock");
        if let Some(answer) = Store::replay(
            &state,
            (&space_id, &principal_id, "memory.candidates.review", key),
            &digest,
        ) {
            return answer;
        }
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
        let (subject_id, scope, revision, consumed) = {
            let candidate = &state.candidates[index];
            (
                candidate.subject_id.clone(),
                candidate.scope.clone(),
                candidate.revision,
                candidate.consumed,
            )
        };
        let contexts: Vec<String> = scope["context_id"].as_str().map(str::to_string).into_iter().collect();
        let requirements = [
            GrantRequirement {
                capability: "review",
                subject_id: Some(&subject_id),
                context_ids: contexts.clone(),
                reads_shared: false,
                subject_all: false,
            },
            GrantRequirement {
                capability: "records_write",
                subject_id: Some(&subject_id),
                context_ids: contexts,
                reads_shared: false,
                subject_all: false,
            },
        ];
        if !Store::authorized(&state, &principal_id, &space_id, &requirements) {
            return reject(
                SpokeRejectCode::NotAuthorized,
                format!("no review/records_write grant covers candidate {candidate_id}"),
            );
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

// ── The loopback host: JSON service face + typed `MemoryPort` face ────────

struct MemoryHost {
    store: Arc<Store>,
}

#[async_trait]
impl RemoteMemoryService for MemoryHost {
    async fn invoke(&self, peer_id: &str, op: &str, payload: Value) -> SpokeResult<Value> {
        let principal = match principal_for(peer_id) {
            SpokeResult::Ok(principal) => principal,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        match op {
            "memory.capture" => self.store.capture(&principal, payload),
            "memory.candidates.review" => self.store.review(&principal, payload),
            other => reject(
                SpokeRejectCode::InternalError,
                format!("{other} is not served on the JSON memory face"),
            ),
        }
    }
}

/// The typed P2 boundary. `memory.recall` is real; the remaining ops are
/// unserved here because this fixture only needs the recall path over the port
/// (the mutating envelope ops are exercised on the JSON face).
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
        _principal: &Principal,
        _request: MemoryCaptureRequest,
    ) -> SpokeResult<MemoryCaptureResponse> {
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
        principal: &Principal,
        request: MemoryRecallRequest,
    ) -> SpokeResult<MemoryRecallResponse> {
        let payload = serde_json::to_value(request).expect("recall request serializes");
        let value = match self.store.recall(principal, payload) {
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

// ── Wire client (raw signed envelopes) ────────────────────────────────────

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
    let request = sign_invoke_request(seed_client(), session_id, sequence, request_id, op, payload);
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

/// The typed face: the same host served through the `MemoryPort` adapter.
fn typed_face(host: &Arc<MemoryHost>) -> Arc<dyn RemoteMemoryService> {
    let host: Arc<MemoryHost> = Arc::clone(host);
    let port: Arc<dyn MemoryPort + Send + Sync> = host;
    Arc::new(MemoryPortService::new(port, admission()))
}

/// The JSON face: capture / review (their generated success types are not
/// constructible — see the module note).
fn json_face(host: &Arc<MemoryHost>) -> Arc<dyn RemoteMemoryService> {
    Arc::clone(host) as Arc<dyn RemoteMemoryService>
}

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

fn recall_request(context: &str, query: &str, max_items: u64) -> Value {
    json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "selection": { "mode": "selected_contexts", "context_ids": [context] },
        "query": query,
        "budget": { "max_items": max_items },
    })
}

fn granted_host() -> Arc<MemoryHost> {
    let peer_id = derive_peer_id_from_ed25519_pubkey(&pubkey_client());
    Arc::new(MemoryHost {
        store: Store::with_grant(&format!("prn_{peer_id}"), &[CONTEXT, OTHER_CONTEXT]),
    })
}

fn ungranted_host() -> Arc<MemoryHost> {
    Arc::new(MemoryHost {
        store: Arc::new(Store::default()),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn memory_loopback_round_trips_capture_review_and_recall_through_the_port() {
    let host = granted_host();

    // capture + review over the JSON service face.
    let (responder, pair) = start_host(Some(json_face(&host)), &MEMORY_CAPABILITIES).await;
    let manifest = memory_manifest("test-client", &MEMORY_CAPABILITIES);
    let session_id = handshake(&pair.client, &manifest).await;

    let captured = invoke(
        &pair.client,
        &session_id,
        0,
        "memory-capture",
        "memory.capture",
        capture_request("cap-loopback-1", CONTEXT, "Mira told Bo the gate key is hidden under the third barrel."),
    )
    .await;
    let receipt = captured
        .get("payload")
        .unwrap_or_else(|| panic!("capture must answer a payload, got {captured}"));
    assert_eq!(receipt["status"], "committed", "{captured}");
    assert_eq!(receipt["op"], "memory.capture");
    assert_eq!(receipt["idempotency_key"], "cap-loopback-1");
    assert_eq!(
        receipt["requester_principal_id"],
        format!(
            "prn_{}",
            derive_peer_id_from_ed25519_pubkey(&pubkey_client())
        )
    );
    assert_eq!(receipt["result"]["classified"], 1, "{captured}");
    assert!(
        receipt["payload_digest"]
            .as_str()
            .is_some_and(|digest| digest.len() == 64),
        "receipt carries a sha-256 payload digest, got {captured}"
    );
    let candidate_id = receipt["result"]["candidate_ids"][0]
        .as_str()
        .unwrap_or_else(|| panic!("candidate id expected, got {captured}"))
        .to_owned();

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
        review_receipt["result"]["record_ids"].as_array().map(Vec::len),
        Some(2),
        "{reviewed}"
    );
    responder.close();

    // recall over the TYPED boundary (`MemoryPortService` → `MemoryPort`).
    let (responder, pair) = start_host(Some(typed_face(&host)), &MEMORY_CAPABILITIES).await;
    let session_id = handshake(&pair.client, &manifest).await;
    let recalled = invoke(
        &pair.client,
        &session_id,
        0,
        "memory-recall",
        "memory.recall",
        recall_request(CONTEXT, "Where is the gate key?", 8),
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
    // Placement inheritance: a review decision carries no placement field, so
    // the recalled records carry the captured candidate's scope.
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
        1,
        "memory-recall-budget",
        "memory.recall",
        recall_request(CONTEXT, "Where is the gate key?", 1),
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
        2,
        "memory-unknown",
        "memory.archive.purge",
        json!({}),
    )
    .await;
    assert_eq!(unknown["error"]["code"], "op_unsupported", "{unknown}");
    responder.close();
}

#[tokio::test]
async fn memory_loopback_enforces_grants_idempotency_and_selection_isolation() {
    // Denial: a provider state with no grant for the caller.
    let host = ungranted_host();
    let (responder, pair) = start_host(Some(json_face(&host)), &MEMORY_CAPABILITIES).await;
    let manifest = memory_manifest("test-client", &MEMORY_CAPABILITIES);
    let session_id = handshake(&pair.client, &manifest).await;
    let denied = invoke(
        &pair.client,
        &session_id,
        0,
        "capture-unauthorized",
        "memory.capture",
        capture_request("cap-denied", CONTEXT, "Mira told Bo the gate key is hidden under the third barrel."),
    )
    .await;
    assert_eq!(
        denied["error"]["code"], "NOT_AUTHORIZED",
        "an ungranted capture must deny: {denied}"
    );
    assert_eq!(
        host.store.state.lock().expect("state lock").candidates.len(),
        0,
        "a denied capture writes nothing"
    );
    responder.close();

    // The same absence on the read side, over the typed boundary.
    let (responder, pair) = start_host(Some(typed_face(&host)), &MEMORY_CAPABILITIES).await;
    let session_id = handshake(&pair.client, &manifest).await;
    let denied = invoke(
        &pair.client,
        &session_id,
        0,
        "recall-unauthorized",
        "memory.recall",
        recall_request(CONTEXT, "Where is the gate key?", 8),
    )
    .await;
    assert_eq!(
        denied["error"]["code"], "NOT_AUTHORIZED",
        "an ungranted recall must deny: {denied}"
    );
    responder.close();

    // Granted provider: idempotent replay, digest conflict, selection +
    // space isolation.
    let host = granted_host();
    host.store.seed_record(SPACE, OTHER_CONTEXT, "fact", "Bo sold rope at the market.");
    host.store
        .seed_record(OTHER_SPACE, CONTEXT, "fact", "A copy of the run stored in another space.");
    let (responder, pair) = start_host(Some(json_face(&host)), &MEMORY_CAPABILITIES).await;
    let session_id = handshake(&pair.client, &manifest).await;

    let request = capture_request(
        "cap-replay",
        CONTEXT,
        "Mira told Bo the gate key is hidden under the third barrel.",
    );
    let first = invoke(
        &pair.client,
        &session_id,
        0,
        "capture-1",
        "memory.capture",
        request.clone(),
    )
    .await;
    let replay = invoke(&pair.client, &session_id, 1, "capture-2", "memory.capture", request).await;
    assert_eq!(
        replay["payload"]["receipt_id"], first["payload"]["receipt_id"],
        "same key + same payload replays the original receipt: {replay}"
    );
    assert_eq!(
        {
            let state = host.store.state.lock().expect("state lock");
            state.candidates.len()
        },
        1,
        "replay must not double-apply"
    );

    let conflict = invoke(
        &pair.client,
        &session_id,
        2,
        "capture-conflict",
        "memory.capture",
        capture_request("cap-replay", CONTEXT, "A different payload under the same idempotency key."),
    )
    .await;
    assert_eq!(
        conflict["error"]["code"], "IDEMPOTENCY_CONFLICT",
        "same key + different payload digest must conflict: {conflict}"
    );

    // One retained record inside the selected context.
    let candidate_id = first["payload"]["result"]["candidate_ids"][0]
        .as_str()
        .unwrap_or_else(|| panic!("candidate id expected, got {first}"))
        .to_owned();
    let reviewed = invoke(
        &pair.client,
        &session_id,
        3,
        "review-isolation",
        "memory.candidates.review",
        json!({
            "space_id": SPACE,
            "candidate_id": candidate_id,
            "expected_revision": 1,
            "decision": {
                "action": "fragment",
                "fragments": [
                    { "kind": "fact", "content": { "text": "The gate key is hidden under the third barrel." } },
                ],
            },
            "idempotency_key": "review-isolation",
        }),
    )
    .await;
    assert_eq!(
        reviewed["payload"]["result"]["outcome"], "fragmented",
        "{reviewed}"
    );
    responder.close();

    // Isolation: the seeded OTHER_CONTEXT and OTHER_SPACE records are
    // authorized state but outside the selection, so they never appear.
    let (responder, pair) = start_host(Some(typed_face(&host)), &MEMORY_CAPABILITIES).await;
    let session_id = handshake(&pair.client, &manifest).await;
    let recalled = invoke(
        &pair.client,
        &session_id,
        0,
        "recall-isolation",
        "memory.recall",
        recall_request(CONTEXT, "", 8),
    )
    .await;
    let records = recalled["payload"]["records"]
        .as_array()
        .unwrap_or_else(|| panic!("recall must answer records, got {recalled}"));
    assert_eq!(
        records.len(),
        1,
        "only the selected context in this space is authorized input: {recalled}"
    );
    assert_eq!(
        records[0]["scope"],
        json!({ "visibility": "context", "context_id": CONTEXT })
    );
    assert_eq!(recalled["payload"]["total_authorized"], 1);
    responder.close();
}

#[tokio::test]
async fn memory_loopback_denies_unnegotiated_and_unserved_memory_ops() {
    // Known op, capability not negotiated (both manifests omit the flags):
    // `capability_missing` before any handler runs — zero side effects.
    let (responder, pair) = start_host(Some(json_face(&granted_host())), &["spoke-baseline"]).await;
    let session_id =
        handshake(&pair.client, &memory_manifest("test-client", &["spoke-baseline"])).await;
    let denied = invoke(
        &pair.client,
        &session_id,
        0,
        "memory-unnegotiated",
        "memory.capture",
        capture_request("cap-denied", CONTEXT, "Mira told Bo the gate key is hidden under the third barrel."),
    )
    .await;
    assert_eq!(denied["error"]["code"], "capability_missing", "{denied}");
    assert!(
        denied.get("payload").is_none(),
        "a capability deny carries no payload, got {denied}"
    );
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
        capture_request("cap-unserved", CONTEXT, "Mira told Bo the gate key is hidden under the third barrel."),
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
