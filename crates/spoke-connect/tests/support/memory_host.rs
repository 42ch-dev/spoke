//! Shared loopback memory host: the in-memory `MemoryPort` provider extracted
//! from `memory_loopback.rs`, with the §7.29–§7.33 transfer surface ported
//! from the SQLite reference provider (`support/sqlite_memory.rs`).
//!
//! Pure-move baseline: storage, grant coverage, idempotent replay, capture /
//! review / recall duties — unchanged from the loopback fixture. Transfer
//! additions: `memory.exports.create/get` (§6.13 chunking via
//! `spoke_memory::transfer` + the §7.29 drain barrier), `memory.imports.stage`
//! (§7.31 identity remap, revisions from 1, source `engine_policy_version`,
//! explicit-only `adopt_grants`, staged report), `memory.imports.activate`
//! (atomic, `authority_generation++`), `memory.imports.discard`, and
//! `memory.receipts.get` (the fixture commits synchronously, so a stored
//! receipt is terminal by definition; non-requesters need space admin).
//!
//! The five transfer ops stay on the §8.2 freeze allowlist: while the store is
//! frozen, capture/review reject `SpaceFrozen` and the transfer ops still run.
//! The fixture freezes via [`Store::set_frozen`] (it does not serve
//! `memory.spaces.freeze`).
//!
//! Included via `#[path]` by the integration tests that need it; each test
//! crate compiles its own copy.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};
use spoke_memory::classify::{classify_candidate, CandidateMaterial};
use spoke_memory::recall::{recall as engine_recall, RecallRecord, RecallRequest};
use spoke_memory::transfer::{
    adopt_grants as transfer_adopt_grants, build_translation_maps, bundle_digest,
    partition_bundle, staged_report as transfer_staged_report, verify_staging,
};
use spoke_memory::ENGINE_POLICY_VERSION;
use spoke_operations::{
    grant_covers, jcs_sha256_hex, spoke_reject, validate_grant_issuance,
    validate_memory_imports_stage, GrantRequirement, IssuedGrant, MemoryPort, SpokeReject,
    SpokeRejectCode, SpokeResult,
};
use spoke_schemas::data::memory_candidate::Disposition;
use spoke_schemas::data::retained_record::Content;
use spoke_schemas::{
    MemoryCandidatesDismissRequest, MemoryCandidatesDismissResponse, MemoryCandidatesGetRequest,
    MemoryCandidatesGetResponse, MemoryCandidatesListRequest, MemoryCandidatesListResponse,
    MemoryCandidatesReviewRequest, MemoryCandidatesReviewResponse, MemoryCaptureRequest,
    MemoryCaptureResponse, MemoryDescribeRequest, MemoryDescribeResponse,
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

/// Fixture defaults the seeded store state shares with the loopback
/// scenarios (`memory_loopback.rs` re-exports these as `SPACE` / `SUBJECT`).
pub const DEFAULT_SPACE: &str = "spc_local_default";
pub const DEFAULT_SUBJECT: &str = "nexus:character:mira";

const CAPTURED_AT: &str = "2026-10-28T09:20:00Z";

/// §6.13 chunking budget; same default as the SQLite reference provider.
const EXPORT_CHUNK_BYTES: u64 = 262_144;

pub fn reject<T>(code: SpokeRejectCode, message: impl Into<String>) -> SpokeResult<T> {
    spoke_reject(code, message, None)
}

fn reject_details<T>(
    code: SpokeRejectCode,
    message: impl Into<String>,
    details: Value,
) -> SpokeResult<T> {
    spoke_reject(
        code,
        message,
        Some(details.as_object().expect("details object").clone()),
    )
}

pub fn not_served<T: std::fmt::Debug>() -> SpokeResult<T> {
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

pub fn wire<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("wire value serializes")
}

// ── In-memory provider: storage + provider duties ─────────────────────────

pub struct Store {
    pub state: Mutex<State>,
}

impl Default for Store {
    fn default() -> Self {
        Store {
            state: Mutex::new(State::default()),
        }
    }
}

pub struct State {
    pub grants: Vec<MemoryGrant>,
    pub candidates: Vec<Candidate>,
    pub records: Vec<StoredRecord>,
    pub receipts: Vec<IssuedReceipt>,
    issued_receipts: usize,
    issued_records: usize,
    /// §8.2: while frozen the store admits only the maintenance allowlist.
    pub frozen: bool,
    /// §12.5: bumped by every import activation.
    pub authority_generation: u64,
    exports: BTreeMap<String, ExportState>,
    imports: BTreeMap<String, ImportState>,
}

impl Default for State {
    fn default() -> Self {
        State {
            grants: Vec::new(),
            candidates: Vec::new(),
            records: Vec::new(),
            receipts: Vec::new(),
            issued_receipts: 0,
            issued_records: 0,
            frozen: false,
            // The reference provider provisions spaces at generation 1.
            authority_generation: 1,
            exports: BTreeMap::new(),
            imports: BTreeMap::new(),
        }
    }
}

pub struct Candidate {
    pub id: String,
    pub revision: u64,
    pub consumed: bool,
    /// Consumption record (§6.8 `consumed_by`) set by review outcomes;
    /// exported verbatim so consumed candidates stay schema-valid.
    pub consumed_by: Option<Value>,
    /// The candidate lives in the space that admitted the capture; review
    /// authorization and the addressed space are checked against it.
    pub space_id: String,
    pub subject_id: String,
    /// Immutable admitted placement (§6.7) — inherited by review outcomes.
    pub scope: Value,
}

pub struct StoredRecord {
    pub space_id: String,
    pub subject_id: String,
    pub record_id: String,
    pub kind: String,
    pub text: String,
    pub scope: Value,
    pub captured_by: String,
}

pub struct IssuedReceipt {
    pub space_id: String,
    pub principal_id: String,
    pub op: String,
    pub idempotency_key: String,
    pub digest: String,
    pub receipt: Value,
}

/// A completed export (§6.11/§6.13): the sealed bundle's chunks plus the
/// subject coverage they were exported under.
#[derive(Clone)]
struct ExportState {
    subject_id: Option<String>,
    chunks: Vec<Value>,
    chunk_count: u64,
}

/// A staged import (§7.31): durable chunk accumulation plus the verification
/// report produced by the final chunk.
#[derive(Clone)]
struct ImportState {
    import_id: String,
    export_id: String,
    chunks: Vec<Value>,
    chunk_count: Option<u64>,
    conflict_policy: String,
    adopt_grants: Vec<Value>,
    subject_renames: BTreeMap<String, String>,
    report: Option<Value>,
    bundle: Option<Value>,
    status: String,
}

impl Store {
    pub fn with_grant(principal_id: &str, contexts: &[&str]) -> Arc<Self> {
        let store = Store::default();
        store.add_grant(
            principal_id,
            DEFAULT_SPACE,
            Some(DEFAULT_SUBJECT),
            &["capture", "review", "records_write", "recall"],
            contexts,
            false,
        );
        Arc::new(store)
    }

    /// Seed a grant. `contexts` empty = unrestricted within the space;
    /// `shared_access` is only meaningful on a restricted grant (§6.5).
    pub fn add_grant(
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

    /// Freeze toggle for the fixture (it does not serve `memory.spaces.freeze`).
    // Test-support surface: unused by includers that do not exercise freezes.
    #[allow(dead_code)]
    pub fn set_frozen(&self, frozen: bool) {
        self.state.lock().expect("state lock").frozen = frozen;
    }

    /// Seed a pending candidate the provider owns (`None` = shared placement).
    pub fn seed_candidate(&self, context: Option<&str>) -> String {
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
            consumed_by: None,
            space_id: DEFAULT_SPACE.to_string(),
            subject_id: DEFAULT_SUBJECT.to_string(),
            scope,
        });
        id
    }

    pub fn seed_record(&self, space: &str, context: &str, kind: &str, text: &str) {
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
            subject_id: DEFAULT_SUBJECT.to_string(),
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

    pub fn has_receipt(&self, op: &str, key: &str) -> bool {
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

    /// `memory.receipts.get`: the fixture commits synchronously, so a stored
    /// receipt is terminal by definition. Non-requesters read a receipt only
    /// with space admin (§7.11 receipt-read authorization), mirroring the
    /// SQLite reference provider.
    pub fn get_receipt_value(&self, principal: &Principal, request: Value) -> SpokeResult<Value> {
        let principal_id = principal_id(principal).to_string();
        let space_id = request["space_id"].as_str().unwrap_or_default().to_string();
        let receipt_id = request["receipt_id"].as_str().unwrap_or_default().to_string();
        let state = self.state.lock().expect("state lock");
        let Some(issued) = state
            .receipts
            .iter()
            .find(|issued| issued.space_id == space_id && issued.receipt["receipt_id"] == receipt_id)
        else {
            return reject(
                SpokeRejectCode::NotFound,
                format!("receipt {receipt_id} not found in {space_id}"),
            );
        };
        if issued.principal_id != principal_id {
            let requirement = GrantRequirement {
                capability: "admin",
                subject_id: None,
                context_ids: Vec::new(),
                reads_shared: false,
                subject_all: false,
            };
            if !Store::authorized(&state, &principal_id, &space_id, &[requirement]) {
                return reject(
                    SpokeRejectCode::NotAuthorized,
                    format!("no admin grant covers receipt reads in {space_id}"),
                );
            }
        }
        SpokeResult::Ok(json!({ "receipt": issued.receipt }))
    }

    /// Raw export chunks as stored (the typed `exports.get` response
    /// normalizes optional fields, so re-digests must use these values).
    #[allow(dead_code)]
    pub fn export_chunks(&self, export_id: &str) -> Option<Vec<Value>> {
        self.state
            .lock()
            .expect("state lock")
            .exports
            .get(export_id)
            .map(|export| export.chunks.clone())
    }

    #[allow(dead_code)]
    pub fn import_status(&self, import_id: &str) -> Option<String> {
        self.state
            .lock()
            .expect("state lock")
            .imports
            .get(import_id)
            .map(|import| import.status.clone())
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

    /// §8.2 maintenance gate for mutating ops outside the freeze allowlist.
    fn frozen_guard(state: &State, space_id: &str) -> Option<SpokeReject> {
        if state.frozen {
            return Some(expect_reject_value(reject::<()>(
                SpokeRejectCode::SpaceFrozen,
                format!("space {space_id} is frozen"),
            )));
        }
        None
    }

    pub fn capture(&self, principal: &Principal, request: Value) -> SpokeResult<Value> {
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
        // Capture is outside the §8.2 maintenance allowlist.
        if let Some(frozen) = Store::frozen_guard(&state, &space_id) {
            return SpokeResult::Reject(frozen);
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
                consumed_by: None,
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
        // Review is outside the §8.2 maintenance allowlist.
        if let Some(frozen) = Store::frozen_guard(&state, &space_id) {
            return SpokeResult::Reject(frozen);
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
        {
            let candidate = &mut state.candidates[index];
            candidate.consumed = true;
            candidate.revision += 1;
            candidate.consumed_by = Some(json!({
                "op": "memory.candidates.review",
                "receipt_id": receipt["receipt_id"],
                "outcome": outcome,
                "record_ids": receipt["result"]["record_ids"],
            }));
        }
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
        let page = match engine_recall(
            &inputs,
            &RecallRequest {
                // The loopback host serves no cursor surface (documented
                // serving gap), so the offset is always zero.
                offset: 0,
                query: request.get("query").and_then(Value::as_str),
                max_items: request["budget"]["max_items"].as_u64().unwrap_or(8) as usize,
                max_bytes: request["budget"]["max_bytes"]
                    .as_u64()
                    .map(|bytes| bytes as usize),
            },
        ) {
            Ok(page) => page,
            Err(error) => {
                return reject(
                    SpokeRejectCode::InvalidInput,
                    format!("recall content: {error}"),
                )
            }
        };
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

    // ── Transfer surface (§7.29–§7.33) ────────────────────────────────────

    /// `memory.exports.create`: transfer-grant scoped export, sealed and
    /// chunked per §6.13. `require_quiescent` enforces the §7.29 drain
    /// barrier (frozen + drained; this fixture commits synchronously, so
    /// drained ⇔ frozen).
    fn create_export(&self, principal: &Principal, request: Value) -> SpokeResult<Value> {
        let principal_id = principal_id(principal).to_string();
        let space_id = request["space_id"].as_str().unwrap_or_default().to_string();
        let subject_id = request["subject_id"].as_str().map(str::to_string);
        let require_quiescent = request["require_quiescent"].as_bool().unwrap_or(false);
        let selection = &request["selection"];
        let key = request["idempotency_key"].as_str().unwrap_or_default();
        let digest = payload_digest(&request);
        let mut state = self.state.lock().expect("state lock");
        let requirement = GrantRequirement {
            capability: "transfer",
            subject_id: subject_id.as_deref(),
            context_ids: string_list_field(selection, "context_ids"),
            reads_shared: selection["include_shared"].as_bool().unwrap_or(false)
                || selection["mode"].as_str() == Some("shared_only")
                || subject_id.is_none(),
            subject_all: selection["mode"].as_str() == Some("subject_all"),
        };
        if !Store::authorized(&state, &principal_id, &space_id, &[requirement]) {
            return reject(
                SpokeRejectCode::NotAuthorized,
                format!("no transfer grant covers {space_id}"),
            );
        }
        if let Some(answer) = Store::replay(
            &state,
            (
                &space_id,
                &principal_id,
                "memory.exports.create",
                key,
            ),
            &digest,
        ) {
            return answer;
        }
        // Drain barrier (§7.29): quiescent exports need a frozen, drained
        // space. Export itself stays on the freeze allowlist.
        if require_quiescent && !state.frozen {
            return reject_details(
                SpokeRejectCode::InvalidInput,
                "require_quiescent needs a frozen space with no pending receipts",
                json!({ "reason": "space_not_drained" }),
            );
        }
        let bundle = Store::export_bundle(&state, &space_id, subject_id.as_deref());
        let bundle_digest_value = match bundle_digest(&bundle) {
            Ok(digest) => digest,
            Err(error) => {
                return reject(
                    SpokeRejectCode::InternalError,
                    format!("bundle digest: {error}"),
                )
            }
        };
        let mut sealed = bundle;
        sealed["digest"] = json!(bundle_digest_value);
        let chunks = match partition_bundle(&sealed, EXPORT_CHUNK_BYTES as usize) {
            Ok(chunks) => chunks,
            Err(error) => {
                return reject(
                    SpokeRejectCode::InternalError,
                    format!("partition bundle: {error}"),
                )
            }
        };
        let chunk_count = chunks.len() as u64;
        let total_bytes: u64 = chunks
            .iter()
            .map(|chunk| chunk["bytes"].as_u64().unwrap_or(0))
            .sum();
        let counts = Store::bundle_counts(&sealed);
        let export_id = sealed["export_id"].as_str().unwrap_or_default().to_string();
        state.exports.insert(
            export_id.clone(),
            ExportState {
                subject_id: subject_id.clone(),
                chunks,
                chunk_count,
            },
        );
        let receipt = Store::receipt(
            &mut state,
            "memory.exports.create",
            &space_id,
            &principal_id,
            &request,
            json!({
                "export_id": export_id,
                "chunk_count": chunk_count,
                "total_bytes": total_bytes,
                "bundle_digest": bundle_digest_value,
                "counts": counts,
            }),
        );
        Store::remember(
            &mut state,
            (
                &space_id,
                &principal_id,
                "memory.exports.create",
                key,
            ),
            &digest,
            &receipt,
        );
        SpokeResult::Ok(receipt)
    }

    /// `memory.exports.get`: one chunk by ordinal; transfer grant with the
    /// export's coverage.
    fn get_export(&self, principal: &Principal, request: Value) -> SpokeResult<Value> {
        let principal_id = principal_id(principal).to_string();
        let space_id = request["space_id"].as_str().unwrap_or_default().to_string();
        let export_id = request["export_id"].as_str().unwrap_or_default().to_string();
        let ordinal = request["chunk_ordinal"].as_u64().unwrap_or(0) as usize;
        let state = self.state.lock().expect("state lock");
        let Some(export) = state.exports.get(&export_id).cloned() else {
            return reject(SpokeRejectCode::NotFound, "export not found");
        };
        let requirement = GrantRequirement {
            capability: "transfer",
            subject_id: export.subject_id.as_deref(),
            context_ids: Vec::new(),
            reads_shared: true,
            subject_all: false,
        };
        if !Store::authorized(&state, &principal_id, &space_id, &[requirement]) {
            return reject(
                SpokeRejectCode::NotAuthorized,
                format!("no transfer grant covers export {export_id} in {space_id}"),
            );
        }
        if ordinal as u64 >= export.chunk_count {
            return reject(
                SpokeRejectCode::InvalidInput,
                format!("chunk_ordinal {ordinal} is outside the export"),
            );
        }
        SpokeResult::Ok(json!({ "chunk": export.chunks[ordinal] }))
    }

    /// `memory.imports.stage`: transfer + admin; append-only chunk
    /// accumulation; verification and the staged report run on the final
    /// chunk (§7.31).
    ///
    /// Value-based entry point: the generated `MemoryImportsStageRequest`
    /// chunk types re-serialize with normalized optional fields (empty arrays
    /// dropped), so a typed chunk round-trip is not byte-faithful and the
    /// §6.13 digests no longer verify — same limitation the SQLite reference
    /// provider documents. The trait method validates the typed request, then
    /// staging runs on the raw wire value.
    pub fn stage_import_value(
        &self,
        principal: &Principal,
        request: Value,
    ) -> SpokeResult<Value> {
        let principal_id = principal_id(principal).to_string();
        let space_id = request["space_id"].as_str().unwrap_or_default().to_string();
        let chunk = request["chunk"].clone();
        let ordinal = chunk["chunk_ordinal"].as_u64().unwrap_or(0);
        let chunk_count = chunk["chunk_count"].as_u64().unwrap_or(0);
        let export_id = chunk["export_id"].as_str().unwrap_or_default().to_string();
        let conflict_policy = request["conflict_policy"]
            .as_str()
            .unwrap_or("fail")
            .to_string();
        let adopt = request["adopt_grants"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let key = request["idempotency_key"].as_str().unwrap_or_default();
        let digest = payload_digest(&request);
        let mut state = self.state.lock().expect("state lock");
        for capability in ["transfer", "admin"] {
            let requirement = GrantRequirement {
                capability,
                subject_id: None,
                context_ids: Vec::new(),
                reads_shared: false,
                subject_all: false,
            };
            if !Store::authorized(&state, &principal_id, &space_id, &[requirement]) {
                return reject(
                    SpokeRejectCode::NotAuthorized,
                    format!("no {capability} grant covers {space_id}"),
                );
            }
        }
        if let Some(answer) = Store::replay(
            &state,
            (
                &space_id,
                &principal_id,
                "memory.imports.stage",
                key,
            ),
            &digest,
        ) {
            return answer;
        }
        let existing = state
            .imports
            .values()
            .find(|import| {
                import.export_id == export_id
                    && import.status == "staged"
                    && import.chunk_count.is_none_or(|count| count == chunk_count)
            })
            .cloned();
        let mut import = match existing {
            Some(import) => import,
            None => {
                if ordinal != 0 {
                    return reject(
                        SpokeRejectCode::InvalidInput,
                        "a staged import starts at chunk ordinal 0",
                    );
                }
                ImportState {
                    import_id: format!("imp_{:04}", state.imports.len() + 1),
                    export_id: export_id.clone(),
                    chunks: Vec::new(),
                    chunk_count: None,
                    conflict_policy,
                    adopt_grants: adopt,
                    subject_renames: BTreeMap::new(),
                    report: None,
                    bundle: None,
                    status: "staged".to_string(),
                }
            }
        };
        if ordinal != import.chunks.len() as u64 {
            return reject(
                SpokeRejectCode::InvalidInput,
                format!(
                    "chunk ordinal {ordinal} out of order; expected {}",
                    import.chunks.len()
                ),
            );
        }
        import.chunks.push(chunk);
        import.chunk_count = Some(chunk_count);
        let received = import.chunks.len() as u64;
        let mut result = json!({
            "import_id": import.import_id.clone(),
            "chunks_received": received,
        });
        if received == chunk_count {
            // Verification runs on the final chunk (§7.31); failures reject
            // IMPORT_CONFLICT and leave nothing staged.
            let outcome = verify_staging(
                &import.chunks,
                &import.subject_renames,
                &identity_map(&import.chunks, "records", "record_id"),
                &identity_map(&import.chunks, "candidates", "candidate_id"),
            );
            let verified =
                outcome.verification.bundle_digest_ok && outcome.verification.errors.is_empty();
            if !verified {
                return reject_details(
                    SpokeRejectCode::ImportConflict,
                    "staged import verification failed",
                    json!({ "errors": outcome.verification.errors }),
                );
            }
            let Some(bundle) = outcome.bundle.clone() else {
                return reject(SpokeRejectCode::ImportConflict, "import produced no bundle");
            };
            // Subject collision policy (§7.31): on an empty target nothing
            // collides and ids pass through unchanged.
            let mut renames: BTreeMap<String, String> = BTreeMap::new();
            for subject in bundle["subjects"].as_array().cloned().unwrap_or_default() {
                let Some(subject_id) = subject["subject_id"].as_str() else {
                    continue;
                };
                if !Store::subject_exists(&state, &space_id, subject_id) {
                    continue;
                }
                if import.conflict_policy == "rename_subject" {
                    renames.insert(
                        subject_id.to_string(),
                        format!("{subject_id}_imported_{}", state.imports.len() + 1),
                    );
                } else {
                    return reject(
                        SpokeRejectCode::ImportConflict,
                        format!("subject {subject_id} already exists in the target space"),
                    );
                }
            }
            import.subject_renames = renames;
            // The report maps must reflect the renames chosen above, so the
            // translation maps are rebuilt against the final rename set
            // (verification already passed on the raw chunks).
            let (maps, _) = build_translation_maps(
                &bundle,
                &import.subject_renames,
                &identity_map(&import.chunks, "records", "record_id"),
                &identity_map(&import.chunks, "candidates", "candidate_id"),
            );
            let adopt_ids: Vec<&str> = import
                .adopt_grants
                .iter()
                .filter_map(|grant| grant["grant_id"].as_str())
                .collect();
            let (adopted, dropped) = match transfer_adopt_grants(&bundle, &adopt_ids) {
                Ok(value) => value,
                Err(error) => {
                    return reject(
                        SpokeRejectCode::ImportConflict,
                        format!("grant adoption: {error}"),
                    )
                }
            };
            let report = match transfer_staged_report(
                &bundle,
                &maps,
                adopted.len(),
                dropped,
                &outcome.verification,
            ) {
                Ok(report) => report,
                Err(error) => {
                    return reject(
                        SpokeRejectCode::ImportConflict,
                        format!("staged report: {error}"),
                    )
                }
            };
            result["staged_report"] = report.clone();
            import.report = Some(report);
            import.bundle = Some(bundle);
        }
        state.imports.insert(import.import_id.clone(), import);
        let receipt = Store::receipt(
            &mut state,
            "memory.imports.stage",
            &space_id,
            &principal_id,
            &request,
            result,
        );
        Store::remember(
            &mut state,
            (
                &space_id,
                &principal_id,
                "memory.imports.stage",
                key,
            ),
            &digest,
            &receipt,
        );
        SpokeResult::Ok(receipt)
    }

    /// `memory.imports.activate`: transfer + admin; applies the staged bundle
    /// in one critical section (atomic by construction), adopts explicitly
    /// listed grants non-amplified, and bumps the authority generation
    /// (§7.32, §12.5).
    fn activate_import(&self, principal: &Principal, request: Value) -> SpokeResult<Value> {
        let principal_id = principal_id(principal).to_string();
        let space_id = request["space_id"].as_str().unwrap_or_default().to_string();
        let import_id = request["import_id"].as_str().unwrap_or_default().to_string();
        let key = request["idempotency_key"].as_str().unwrap_or_default();
        let digest = payload_digest(&request);
        let mut state = self.state.lock().expect("state lock");
        for capability in ["transfer", "admin"] {
            let requirement = GrantRequirement {
                capability,
                subject_id: None,
                context_ids: Vec::new(),
                reads_shared: false,
                subject_all: false,
            };
            if !Store::authorized(&state, &principal_id, &space_id, &[requirement]) {
                return reject(
                    SpokeRejectCode::NotAuthorized,
                    format!("no {capability} grant covers {space_id}"),
                );
            }
        }
        if let Some(answer) = Store::replay(
            &state,
            (
                &space_id,
                &principal_id,
                "memory.imports.activate",
                key,
            ),
            &digest,
        ) {
            return answer;
        }
        let Some(import) = state.imports.get(&import_id).cloned() else {
            return reject(SpokeRejectCode::NotFound, "import not found");
        };
        if import.status != "staged" {
            return reject(
                SpokeRejectCode::ImportConflict,
                format!("import is {} and cannot be activated", import.status),
            );
        }
        let Some(bundle) = import.bundle.clone() else {
            return reject(
                SpokeRejectCode::ImportConflict,
                "import has no verified bundle",
            );
        };
        let rename = |source: &str| {
            import
                .subject_renames
                .get(source)
                .cloned()
                .unwrap_or_else(|| source.to_string())
        };
        // Identity remap: §7.31 requires reassignment, not a change of value,
        // so target ids equal source ids and revisions start at 1. Mutations
        // stage in locals and commit only after every check passes, so a
        // rejection (e.g. grant amplification) applies nothing (§7.32).
        let mut new_records: Vec<StoredRecord> = Vec::new();
        for record in bundle["records"].as_array().cloned().unwrap_or_default() {
            new_records.push(StoredRecord {
                space_id: space_id.clone(),
                subject_id: rename(record["subject_id"].as_str().unwrap_or_default()),
                record_id: record["record_id"].as_str().unwrap_or_default().to_string(),
                kind: record["kind"].as_str().unwrap_or("note").to_string(),
                text: record["content"]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                scope: record["scope"].clone(),
                captured_by: record["provenance"]["captured_by"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            });
        }
        let mut new_candidates: Vec<Candidate> = Vec::new();
        for candidate in bundle["candidates"].as_array().cloned().unwrap_or_default() {
            let pending = candidate["status"].as_str() == Some("pending");
            new_candidates.push(Candidate {
                id: candidate["candidate_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                revision: if pending {
                    1
                } else {
                    candidate["revision"].as_u64().unwrap_or(1)
                },
                consumed: !pending,
                consumed_by: candidate.get("consumed_by").cloned(),
                space_id: space_id.clone(),
                subject_id: rename(candidate["subject_id"].as_str().unwrap_or_default()),
                scope: candidate["capture_scope"].clone(),
            });
        }
        // Explicit-only grant adoption, non-amplified against the stager.
        let granter_grants: Vec<MemoryGrant> = state
            .grants
            .iter()
            .filter(|grant| {
                let value = wire(grant);
                value["principal_id"].as_str() == Some(principal_id.as_str())
                    && value["space_id"].as_str() == Some(space_id.as_str())
            })
            .cloned()
            .collect();
        let summaries: Vec<Value> = bundle["grants"].as_array().cloned().unwrap_or_default();
        let now = Utc::now();
        let mut new_grants: Vec<MemoryGrant> = Vec::new();
        for requested in &import.adopt_grants {
            let Some(grant_id) = requested["grant_id"].as_str() else {
                continue;
            };
            let Some(summary) = summaries
                .iter()
                .find(|summary| summary["grant_id"].as_str() == Some(grant_id))
            else {
                return reject(
                    SpokeRejectCode::ImportConflict,
                    format!("grant {grant_id} is not in the bundle"),
                );
            };
            let target_principal = requested["principal_id"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let capability_strings: Vec<String> = summary["capabilities"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let capabilities: Vec<&str> =
                capability_strings.iter().map(String::as_str).collect();
            let context_ids: Vec<String> = summary["context_ids"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let issued = IssuedGrant {
                capabilities: &capabilities,
                subject_id: summary["subject_id"].as_str(),
                context_ids: &context_ids,
                shared_access: summary["shared_access"].as_bool().unwrap_or(false),
                expires_at: None,
            };
            match validate_grant_issuance(&granter_grants, &principal_id, &space_id, now, &issued)
            {
                SpokeResult::Ok(()) => {}
                SpokeResult::Reject(_) => {
                    return reject(
                        SpokeRejectCode::ImportConflict,
                        format!("adopting grant {grant_id} would amplify coverage"),
                    )
                }
            }
            let mut grant = json!({
                "schema_version": 1,
                "grant_id": format!("grt_{:02}", state.grants.len() + new_grants.len() + 1),
                "space_id": space_id,
                "principal_id": target_principal,
                "capabilities": summary["capabilities"],
                "created_at": CAPTURED_AT,
                "revision": 1,
                "extensions": {},
            });
            for field in ["subject_id", "context_ids", "shared_access"] {
                if let Some(value) = summary.get(field) {
                    grant[field] = value.clone();
                }
            }
            let grant: MemoryGrant = serde_json::from_value(grant).expect("valid grant");
            new_grants.push(grant);
        }
        // All checks passed: commit the activation in one critical section.
        state.records.append(&mut new_records);
        state.candidates.append(&mut new_candidates);
        state.grants.append(&mut new_grants);
        let authority_generation = state.authority_generation + 1;
        state.authority_generation = authority_generation;
        if let Some(import) = state.imports.get_mut(&import_id) {
            import.status = "activated".to_string();
        }
        let counts = import
            .report
            .as_ref()
            .map(|report| report["counts"].clone())
            .unwrap_or_else(|| json!({}));
        let receipt = Store::receipt(
            &mut state,
            "memory.imports.activate",
            &space_id,
            &principal_id,
            &request,
            // §7.32: the completion result is exactly {import_id, activated,
            // authority_generation}; adoption totals live in the staged report.
            json!({
                "import_id": import_id,
                "activated": counts,
                "authority_generation": authority_generation,
            }),
        );
        Store::remember(
            &mut state,
            (
                &space_id,
                &principal_id,
                "memory.imports.activate",
                key,
            ),
            &digest,
            &receipt,
        );
        SpokeResult::Ok(receipt)
    }

    /// `memory.imports.discard`: transfer + admin; pre-activation rollback
    /// only (§7.33).
    fn discard_import(&self, principal: &Principal, request: Value) -> SpokeResult<Value> {
        let principal_id = principal_id(principal).to_string();
        let space_id = request["space_id"].as_str().unwrap_or_default().to_string();
        let import_id = request["import_id"].as_str().unwrap_or_default().to_string();
        let key = request["idempotency_key"].as_str().unwrap_or_default();
        let digest = payload_digest(&request);
        let mut state = self.state.lock().expect("state lock");
        for capability in ["transfer", "admin"] {
            let requirement = GrantRequirement {
                capability,
                subject_id: None,
                context_ids: Vec::new(),
                reads_shared: false,
                subject_all: false,
            };
            if !Store::authorized(&state, &principal_id, &space_id, &[requirement]) {
                return reject(
                    SpokeRejectCode::NotAuthorized,
                    format!("no {capability} grant covers {space_id}"),
                );
            }
        }
        if let Some(answer) = Store::replay(
            &state,
            (
                &space_id,
                &principal_id,
                "memory.imports.discard",
                key,
            ),
            &digest,
        ) {
            return answer;
        }
        let Some(import) = state.imports.get_mut(&import_id) else {
            return reject(SpokeRejectCode::NotFound, "import not found");
        };
        if import.status != "staged" {
            return reject(
                SpokeRejectCode::ImportConflict,
                "discard is only valid before activation",
            );
        }
        import.status = "discarded".to_string();
        import.chunks.clear();
        import.bundle = None;
        import.report = None;
        let receipt = Store::receipt(
            &mut state,
            "memory.imports.discard",
            &space_id,
            &principal_id,
            &request,
            json!({ "import_id": import_id, "discarded": true }),
        );
        Store::remember(
            &mut state,
            (
                &space_id,
                &principal_id,
                "memory.imports.discard",
                key,
            ),
            &digest,
            &receipt,
        );
        SpokeResult::Ok(receipt)
    }

    /// The export source bundle: full record/candidate documents plus
    /// content-free grant/receipt summaries, scoped to the space (and subject
    /// when given). The source reports this engine's policy version so the
    /// target can check declared policy on activation.
    fn export_bundle(state: &State, space_id: &str, subject_id: Option<&str>) -> Value {
        let in_scope = |subject: &str| subject_id.is_none_or(|wanted| wanted == subject);
        // The support store models subjects implicitly: one synthesized
        // document per subject that holds state in this space.
        let mut subject_ids: BTreeSet<String> = BTreeSet::new();
        for record in &state.records {
            if record.space_id == space_id && in_scope(&record.subject_id) {
                subject_ids.insert(record.subject_id.clone());
            }
        }
        for candidate in &state.candidates {
            if candidate.space_id == space_id && in_scope(&candidate.subject_id) {
                subject_ids.insert(candidate.subject_id.clone());
            }
        }
        let subjects: Vec<Value> = subject_ids
            .iter()
            .map(|id| {
                json!({
                    "schema_version": 1,
                    "subject_id": id,
                    "space_id": space_id,
                    "status": "active",
                    "fenced_contexts": [],
                    "created_at": CAPTURED_AT,
                    "revision": 1,
                    "extensions": {},
                })
            })
            .collect();
        let records: Vec<Value> = state
            .records
            .iter()
            .filter(|record| record.space_id == space_id && in_scope(&record.subject_id))
            .map(|record| {
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
        // The fixture retains placement and consumption state only, so the
        // exported proposed content is empty; `content_digest` is computed
        // over exactly the proposed content emitted here (MEM-R025).
        let candidates: Vec<Value> = state
            .candidates
            .iter()
            .filter(|candidate| candidate.space_id == space_id && in_scope(&candidate.subject_id))
            .map(|candidate| {
                let proposed = json!({ "kind": "note", "content": { "text": "" } });
                let content_digest = match jcs_sha256_hex(&proposed["content"]) {
                    SpokeResult::Ok(digest) => digest,
                    SpokeResult::Reject(reject) => {
                        panic!("proposed content digest: {reject:?}")
                    }
                };
                let mut document = json!({
                    "schema_version": 1,
                    "candidate_id": candidate.id,
                    "space_id": candidate.space_id,
                    "subject_id": candidate.subject_id,
                    "capture_scope": candidate.scope,
                    "classification": {
                        "disposition": "promote",
                        "rationale_codes": [],
                    },
                    "provenance": {
                        "source_context_ids": [],
                        "anchors": [],
                        "captured_by": "prn_host",
                        "captured_at": CAPTURED_AT,
                    },
                    "created_at": CAPTURED_AT,
                    "revision": candidate.revision,
                    "extensions": {},
                });
                if candidate.consumed {
                    let Some(consumed_by) = &candidate.consumed_by else {
                        panic!("consumed candidate {} has no consumed_by", candidate.id);
                    };
                    document["status"] = json!("consumed");
                    document["consumed_by"] = consumed_by.clone();
                    document["content_digest"] = json!(content_digest);
                } else {
                    document["status"] = json!("pending");
                    document["proposed"] = proposed;
                }
                document
            })
            .collect();
        let receipt_summaries: Vec<Value> = state
            .receipts
            .iter()
            .filter(|issued| issued.space_id == space_id)
            .map(|issued| {
                let mut summary = json!({
                    "receipt_id": issued.receipt["receipt_id"],
                    "op": issued.op,
                    "requester_principal_ref": issued.principal_id,
                    "payload_digest": issued.digest,
                    "status": issued.receipt["status"],
                    "created_at": issued.receipt["created_at"],
                });
                if !issued.idempotency_key.is_empty() {
                    summary["idempotency_key"] = json!(issued.idempotency_key);
                }
                summary
            })
            .collect();
        let grants: Vec<Value> = state
            .grants
            .iter()
            .filter(|grant| wire(grant)["space_id"].as_str() == Some(space_id))
            .map(|grant| {
                let grant = wire(grant);
                let mut summary = json!({
                    "grant_id": grant["grant_id"],
                    "principal_ref": grant["principal_id"],
                    "capabilities": grant["capabilities"],
                    "revision": grant["revision"],
                });
                for field in ["subject_id", "context_ids", "shared_access"] {
                    // The generated chunk types drop explicit empty arrays on
                    // the wire, so summaries must not carry them (the sealed
                    // digest is verified over the served form).
                    if field == "context_ids"
                        && grant[field].as_array().is_some_and(Vec::is_empty)
                    {
                        continue;
                    }
                    if let Some(value) = grant.get(field) {
                        summary[field] = value.clone();
                    }
                }
                summary
            })
            .collect();
        let mut bundle = json!({
            "schema_version": 1,
            "bundle_format_version": 1,
            "memory_protocol_version": 1,
            "export_id": format!("exp_mem_{:04}", state.exports.len() + 1),
            "space_id": space_id,
            "created_at": CAPTURED_AT,
            "source": {
                "authority_host_id": "memory-loopback-host",
                "authority_generation": state.authority_generation,
                "engine_policy_version": ENGINE_POLICY_VERSION,
            },
            "subjects": subjects,
            "records": records,
            "candidates": candidates,
            "receipt_summaries": receipt_summaries,
            "grants": grants,
            "extensions": {},
        });
        // The typed Connect round-trip drops explicit empty arrays (the
        // generated chunk types skip serializing them), which would break the
        // sealed chunk digests over the wire. The sealed bundle therefore
        // omits empty arrays: absence is §6.13-valid, and reassembly
        // reproduces the sealed form exactly. (Same normalization the SQLite
        // reference provider documents.)
        for field in [
            "subjects",
            "records",
            "candidates",
            "profiles",
            "reflections",
            "receipt_summaries",
            "grants",
        ] {
            if bundle[field].as_array().is_some_and(Vec::is_empty) {
                bundle
                    .as_object_mut()
                    .expect("bundle object")
                    .remove(field);
            }
        }
        bundle
    }

    fn bundle_counts(bundle: &Value) -> Value {
        let count = |field: &str| {
            bundle
                .get(field)
                .and_then(Value::as_array)
                .map_or(0, Vec::len)
        };
        json!({
            "subjects": count("subjects"),
            "records": count("records"),
            "candidates": count("candidates"),
            "profiles": count("profiles"),
            "reflections": count("reflections"),
            "receipt_summaries": count("receipt_summaries"),
        })
    }

    /// Subject existence for the §7.31 collision policy: a subject exists
    /// when the space holds any of its records or candidates.
    fn subject_exists(state: &State, space_id: &str, subject_id: &str) -> bool {
        state
            .records
            .iter()
            .any(|record| record.space_id == space_id && record.subject_id == subject_id)
            || state
                .candidates
                .iter()
                .any(|candidate| candidate.space_id == space_id && candidate.subject_id == subject_id)
    }
}

fn expect_reject_value<T: std::fmt::Debug>(result: SpokeResult<T>) -> SpokeReject {
    match result {
        SpokeResult::Ok(value) => panic!("expected a rejection, got {value:?}"),
        SpokeResult::Reject(reject) => reject,
    }
}

/// Identity translation map over the elements a staged chunk carries: the
/// cooperative provider policy is "reassign, do not rename" — target ids
/// equal source ids on empty targets.
fn identity_map(chunks: &[Value], field: &str, id_field: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for chunk in chunks {
        for item in chunk[field].as_array().cloned().unwrap_or_default() {
            if let Some(id) = item[id_field].as_str() {
                map.insert(id.to_string(), id.to_string());
            }
        }
    }
    map
}

fn string_list_field(value: &Value, field: &str) -> Vec<String> {
    value[field]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
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

pub struct MemoryHost {
    pub store: Arc<Store>,
}

/// The provider answers capture / review / recall and the five transfer ops;
/// every other op is unserved by this fixture.
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
        principal: &Principal,
        request: MemoryReceiptsGetRequest,
    ) -> SpokeResult<MemoryReceiptsGetResponse> {
        let value = match self.store.get_receipt_value(principal, wire(&request)) {
            SpokeResult::Ok(value) => value,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        match serde_json::from_value(value) {
            Ok(response) => SpokeResult::Ok(response),
            Err(error) => reject(
                SpokeRejectCode::InternalError,
                format!("receipts.get response decode failed: {error}"),
            ),
        }
    }

    async fn create_export(
        &self,
        principal: &Principal,
        request: MemoryExportsCreateRequest,
    ) -> SpokeResult<MemoryExportsCreateResponse> {
        let value = match self.store.create_export(principal, wire(&request)) {
            SpokeResult::Ok(value) => value,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        match serde_json::from_value(value) {
            Ok(response) => SpokeResult::Ok(response),
            Err(error) => reject(
                SpokeRejectCode::InternalError,
                format!("exports.create response decode failed: {error}"),
            ),
        }
    }

    async fn get_export(
        &self,
        principal: &Principal,
        request: MemoryExportsGetRequest,
    ) -> SpokeResult<MemoryExportsGetResponse> {
        let value = match self.store.get_export(principal, wire(&request)) {
            SpokeResult::Ok(value) => value,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        match serde_json::from_value(value) {
            Ok(response) => SpokeResult::Ok(response),
            Err(error) => reject(
                SpokeRejectCode::InternalError,
                format!("exports.get response decode failed: {error}"),
            ),
        }
    }

    async fn stage_import(
        &self,
        principal: &Principal,
        request: MemoryImportsStageRequest,
    ) -> SpokeResult<MemoryImportsStageResponse> {
        if let SpokeResult::Reject(reject) = validate_memory_imports_stage(&request) {
            return SpokeResult::Reject(reject);
        }
        let value = match self.store.stage_import_value(principal, wire(&request)) {
            SpokeResult::Ok(value) => value,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        match serde_json::from_value(value) {
            Ok(response) => SpokeResult::Ok(response),
            Err(error) => reject(
                SpokeRejectCode::InternalError,
                format!("imports.stage response decode failed: {error}"),
            ),
        }
    }

    async fn activate_import(
        &self,
        principal: &Principal,
        request: MemoryImportsActivateRequest,
    ) -> SpokeResult<MemoryImportsActivateResponse> {
        let value = match self.store.activate_import(principal, wire(&request)) {
            SpokeResult::Ok(value) => value,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        match serde_json::from_value(value) {
            Ok(response) => SpokeResult::Ok(response),
            Err(error) => reject(
                SpokeRejectCode::InternalError,
                format!("imports.activate response decode failed: {error}"),
            ),
        }
    }

    async fn discard_import(
        &self,
        principal: &Principal,
        request: MemoryImportsDiscardRequest,
    ) -> SpokeResult<MemoryImportsDiscardResponse> {
        let value = match self.store.discard_import(principal, wire(&request)) {
            SpokeResult::Ok(value) => value,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        match serde_json::from_value(value) {
            Ok(response) => SpokeResult::Ok(response),
            Err(error) => reject(
                SpokeRejectCode::InternalError,
                format!("imports.discard response decode failed: {error}"),
            ),
        }
    }
}
