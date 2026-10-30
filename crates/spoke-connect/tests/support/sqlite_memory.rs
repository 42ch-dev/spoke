//! SQLite-backed reference `MemoryPort` provider — storage + low-complexity ops
//! (P5 proof infrastructure, sub-round 3a).
//!
//! A **conformance proof artifact**, not a shipped product runtime. This round
//! lands provider-private storage and the first five op groups — `describe`,
//! `spaces.*`, `subjects.*`, `grants.*`, `receipts.get` — with the provider
//! enforcing its own duties: admission and grant coverage, CAS, idempotency with
//! dedup tombstones, freeze and fence fencing, and content-free receipts.
//! `spoke_fixture_memory_engine` supplies policy only.
//!
//! Op groups owned by later rounds answer `CAPABILITY_PORT_MISSING` through
//! [`not_served`] with the op name — an honest capability gap, never a fake
//! success: capture/candidates/records/recall land in 3b, profile/reflections in
//! 3c, transfer in Task 4.
//!
//! Speech conventions follow the working loopback host
//! (`crates/spoke-connect/tests/memory_loopback.rs`): methods take the typed
//! request, convert it to `Value` once, run JSON-based provider logic, and
//! deserialize the response at the boundary. `SpokeResult` is an enum, so the
//! `tri!` macro performs the early-return the `?` operator cannot.
//!
//! Storage (`simplify:` ceiling): provider-private state is one JSON document in
//! SQLite (`memory_state`, single row) plus a `memory_snapshot` table. Each
//! mutation loads the document, applies the duty, and writes it back in one
//! transaction — atomic and durable; a snapshot is that same document, so
//! save/restore is two statements and also rolls back dedup records (a repeated
//! request after a restore is a fresh execution, not a replay). Normalized tables
//! are the production shape if this stops being a proof artifact.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spoke_fixture_memory_engine::classify::{classify_candidate, CandidateMaterial};
use spoke_fixture_memory_engine::recall::{recall as engine_recall, RecallRecord, RecallRequest as EngineRecall};
use spoke_fixture_memory_engine::reflect::{assemble_reflection, basis_digests, ReflectionBasis, SourceRef};
use spoke_fixture_memory_engine::transfer::{
    adopt_grants as transfer_adopt_grants, bundle_digest, chunk_digest, partition_bundle,
    reassemble, staged_report as transfer_staged_report, verify_staging,
};
use spoke_fixture_memory_engine::ENGINE_POLICY_VERSION;
use spoke_operations::{
    is_freeze_allowlisted, jcs_sha256_hex, spoke_reject, validate_grant_issuance,
    validate_memory_candidates_list, validate_memory_candidates_review, validate_memory_capture,
    validate_memory_grants_grant, validate_memory_grants_list, validate_memory_imports_stage,
    validate_memory_recall, validate_memory_records_create, validate_memory_records_list,
    validate_memory_records_update, validate_memory_reflections_generate,
    validate_memory_reflections_list, validate_memory_reflections_state,
    validate_memory_spaces_list, validate_memory_subjects_fence, GrantRequirement, IssuedGrant,
    MemoryPort, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::data::memory_candidate::Disposition;
use spoke_schemas::data::read_selection::ReadSelection;
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

/// Provider-declared limits (`memory.describe`), mirroring research §15.7.
pub const MAX_CONTENT_BYTES: u64 = 65_536;
pub const MAX_CANDIDATES_PER_CAPTURE: u64 = 16;
pub const MAX_PAGE_SIZE: u64 = 100;
pub const EXPORT_CHUNK_BYTES: u64 = 262_144;
pub const IDEMPOTENCY_RETENTION_SECONDS: i64 = 86_400;
pub const RECEIPT_RETENTION_SECONDS: i64 = 604_800;
pub const DEDUP_RETENTION_SECONDS: i64 = 2_592_000;
pub const STAGED_IMPORT_TTL_SECONDS: i64 = 259_200;
pub const EXPORT_RETENTION_SECONDS: i64 = 86_400;

/// Fixed clock origin: repeated runs produce identical timestamps.
pub const CLOCK_ORIGIN: &str = "2026-07-23T06:00:00Z";

fn reject<T>(code: SpokeRejectCode, message: impl Into<String>) -> SpokeResult<T> {
    spoke_reject(code, message, None)
}

fn reject_details<T>(
    code: SpokeRejectCode,
    message: impl Into<String>,
    details: Value,
) -> SpokeResult<T> {
    spoke_reject(code, message, details.as_object().cloned())
}

/// Early return for `SpokeResult`: the `?` operator cannot be used on it.
macro_rules! tri {
    ($expression:expr) => {
        match $expression {
            SpokeResult::Ok(value) => value,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        }
    };
}

fn reply<T: serde::de::DeserializeOwned>(value: Value) -> SpokeResult<T> {
    match serde_json::from_value(value) {
        Ok(response) => SpokeResult::Ok(response),
        Err(error) => reject(
            SpokeRejectCode::InternalError,
            format!("provider response shape: {error}"),
        ),
    }
}

fn wire<T: Serialize>(request: &T) -> Value {
    serde_json::to_value(request).expect("request serializes")
}

fn request_digest(request: &Value) -> SpokeResult<String> {
    jcs_sha256_hex(request)
}

fn identity(principal: &Principal) -> &str {
    principal.principal_id.as_str()
}

/// Page limits are `NonZero<u64>` in the P2 validators.
fn page_limit() -> std::num::NonZero<u64> {
    std::num::NonZero::new(MAX_PAGE_SIZE).expect("max page size is non-zero")
}

fn parse_time(raw: &str) -> Option<DateTime<Utc>> {
    raw.parse::<DateTime<Utc>>().ok()
}

/// Read a string field from a serialized request, defaulting to empty.
fn text_field(value: &Value, field: &str) -> String {
    value[field].as_str().unwrap_or_default().to_string()
}

/// Read a u64 field from a serialized request, defaulting to zero.
fn number_field(value: &Value, field: &str) -> u64 {
    value[field].as_u64().unwrap_or_default()
}

/// Read a string array field from a serialized request.
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

/// Identity translation map over the elements a staged chunk carries: the
/// cooperative provider policy is "reassign, do not rename".
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

/// Placement selection helpers shared by `records.list`, `recall` and the
/// reflection group.
fn selection_contexts(selection: &ReadSelection) -> Vec<String> {
    match selection {
        ReadSelection::SelectedContexts { context_ids, .. } => context_ids.clone(),
        _ => Vec::new(),
    }
}

fn selection_reads_shared(selection: &ReadSelection) -> bool {
    match selection {
        ReadSelection::SharedOnly => true,
        ReadSelection::SelectedContexts { include_shared, .. } => include_shared.unwrap_or(false),
        ReadSelection::SubjectAll => true,
    }
}

fn selection_subject_all(selection: &ReadSelection) -> bool {
    matches!(selection, ReadSelection::SubjectAll)
}

/// Authorized records for a subject + selection + provenance filter; the
/// provider filters (placement, tombstone, provenance), the engine ranks.
fn selected_records(
    space: &Space,
    subject_id: &str,
    selection: &ReadSelection,
    filter: Option<&Value>,
) -> Vec<Value> {
    let mode_contexts = selection_contexts(selection);
    let include_shared = selection_reads_shared(selection);
    space
        .records
        .values()
        .filter(|record| record["subject_id"].as_str() == Some(subject_id))
        .filter(|record| {
            let scope = &record["scope"];
            let shared = scope["visibility"].as_str() == Some("shared");
            match selection {
                ReadSelection::SubjectAll => true,
                ReadSelection::SharedOnly => shared,
                ReadSelection::SelectedContexts { .. } => {
                    (shared && include_shared)
                        || scope["context_id"]
                            .as_str()
                            .map(|context| mode_contexts.iter().any(|wanted| wanted == context))
                            .unwrap_or(false)
                }
            }
        })
        .filter(|record| match filter {
            None => true,
            Some(filter) => {
                let wanted: Vec<String> = filter["source_context_ids"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                record["provenance"]["source_context_ids"]
                    .as_array()
                    .map(|sources| {
                        sources
                            .iter()
                            .filter_map(Value::as_str)
                            .any(|source| wanted.iter().any(|wanted| wanted == source))
                    })
                    .unwrap_or(false)
            }
        })
        .cloned()
        .collect()
}

/// Op groups owned by later sub-rounds: an honest capability gap.
fn not_served<T>(op: &str) -> SpokeResult<T> {
    reject(
        SpokeRejectCode::CapabilityPortMissing,
        format!("{op} is not served by this provider yet (later sub-round)"),
    )
}

/// Deferred work for a pending receipt (reflection generation).
#[derive(Clone, Serialize, Deserialize)]
enum PendingWork {
    Reflection {
        subject_id: String,
        selection: ReadSelection,
        sources: Vec<(String, u64)>,
        profile_revisions: Vec<(String, u64)>,
    },
    /// Receipt whose work already happened (export creation, import
    /// activation); the worker commits it with this content-free result.
    Receipt { result: Value },
}

/// A completed export (`§6.11`/§6.13): the sealed bundle, its chunks and the
/// content-free summary the receipt reports.
#[derive(Clone, Serialize, Deserialize)]
struct ExportState {
    export_id: String,
    subject_id: Option<String>,
    chunks: Vec<Value>,
    chunk_count: u64,
    total_bytes: u64,
    bundle_digest: String,
    counts: Value,
    created_at_seconds: i64,
}

/// A staged import (`§7.31`): durable chunk accumulation plus the verification
/// report produced by the final chunk.
#[derive(Clone, Serialize, Deserialize)]
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
    staged_seconds: i64,
    status: String,
}

/// Dedup namespace entry (`§7.0.5`): `(principal, op, idempotency_key)`.
#[derive(Clone, Serialize, Deserialize)]
struct DedupEntry {
    receipt_id: String,
    digest: String,
    created_at: String,
}

#[derive(Clone, Serialize, Deserialize, Default)]
struct Space {
    space_id: String,
    display_name: Option<String>,
    authority_host_id: String,
    authority_generation: u64,
    frozen: bool,
    revision: u64,
    created_at: String,
    /// Bumped by every mutation; cursors pin it (`MEM-R026`, 3b).
    state_epoch: u64,
    /// Provider policy: kinds `reflections.generate` requires (3c).
    required_profile_kinds: Vec<String>,
    subjects: BTreeMap<String, Value>,
    grants: BTreeMap<String, Value>,
    candidates: BTreeMap<String, Value>,
    records: BTreeMap<String, Value>,
    /// `"<subject_id>\u{1f}<profile_kind>"` → profile document.
    profiles: BTreeMap<String, Value>,
    reflections: BTreeMap<String, Value>,
    /// Pinned record ids per reflection (provider-private; basis carries only
    /// digests on the wire).
    reflection_sources: BTreeMap<String, Vec<String>>,
    /// Reflections activated from a staged import: §7.31 stale
    /// unconditionally, regardless of whether the translated basis reproduces.
    imported_reflections: BTreeSet<String>,
    /// Deferred work a pending receipt will complete on the next observation.
    pending: BTreeMap<String, PendingWork>,
    exports: BTreeMap<String, ExportState>,
    imports: BTreeMap<String, ImportState>,
    receipts: BTreeMap<String, Value>,
    /// Monotonic receipt-id sequence (never rewound by eviction; rides the
    /// document, so snapshots restore it too).
    receipt_sequence: u64,
    dedup: BTreeMap<String, DedupEntry>,
}

impl Space {
    fn pending_receipts(&self) -> usize {
        self.receipts
            .values()
            .filter(|receipt| receipt["status"] == "pending")
            .count()
    }

    fn touch(&mut self) {
        self.state_epoch += 1;
    }

    fn fenced_contexts(&self, subject_id: &str) -> Vec<String> {
        self.subjects
            .get(subject_id)
            .and_then(|subject| subject["fenced_contexts"].as_array())
            .map(|contexts| {
                contexts
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
struct Document {
    spaces: BTreeMap<String, Space>,
    clock_offset_seconds: i64,
}

/// Provider-private SQLite storage.
struct Store {
    connection: Connection,
}

impl Store {
    fn open(path: &Path) -> rusqlite::Result<Self> {
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS memory_state (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 document TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS memory_snapshot (
                 id INTEGER PRIMARY KEY,
                 document TEXT NOT NULL,
                 created_at TEXT NOT NULL
             );",
        )?;
        let store = Self { connection };
        if store.read()?.spaces.is_empty() {
            store.write(&Document::default())?;
        }
        Ok(store)
    }

    fn read(&self) -> rusqlite::Result<Document> {
        let mut statement = self
            .connection
            .prepare("SELECT document FROM memory_state WHERE id = 1")?;
        let mut rows = statement.query([])?;
        match rows.next()? {
            Some(row) => {
                let raw: String = row.get(0)?;
                Ok(serde_json::from_str(&raw).unwrap_or_default())
            }
            None => Ok(Document::default()),
        }
    }

    /// One transaction per mutation: the document swap is atomic and durable.
    fn write(&self, document: &Document) -> rusqlite::Result<()> {
        let raw = serde_json::to_string(document).expect("document serializes");
        self.connection.execute(
            "INSERT INTO memory_state (id, document) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET document = excluded.document",
            params![raw],
        )?;
        Ok(())
    }

    fn write_snapshot(&self, document: &Document, created_at: &str) -> rusqlite::Result<()> {
        let raw = serde_json::to_string(document).expect("document serializes");
        self.connection.execute(
            "INSERT INTO memory_snapshot (document, created_at) VALUES (?1, ?2)",
            params![raw, created_at],
        )?;
        Ok(())
    }

    fn restore_snapshot(&self) -> rusqlite::Result<bool> {
        let mut statement = self
            .connection
            .prepare("SELECT document FROM memory_snapshot ORDER BY id DESC LIMIT 1")?;
        let mut rows = statement.query([])?;
        let Some(row) = rows.next()? else {
            return Ok(false);
        };
        let raw: String = row.get(0)?;
        let document: Document = serde_json::from_str(&raw).unwrap_or_default();
        self.write(&document)?;
        Ok(true)
    }
}

/// SQLite-backed reference provider (sub-round 3a surface).
pub struct SqliteMemoryProvider {
    store: Mutex<Store>,
}

impl SqliteMemoryProvider {
    pub fn open(path: &Path) -> Self {
        Self {
            store: Mutex::new(Store::open(path).expect("open provider database")),
        }
    }

    pub fn in_memory() -> Self {
        Self {
            store: Mutex::new(Store::open(Path::new(":memory:")).expect("open provider database")),
        }
    }

    fn clock(document: &Document) -> DateTime<Utc> {
        let origin: DateTime<Utc> = CLOCK_ORIGIN.parse().expect("clock origin");
        origin + Duration::seconds(document.clock_offset_seconds)
    }

    /// Harness clock control: move the provider clock forward.
    pub fn advance_clock(&self, seconds: i64) {
        let store = self.store.lock().expect("store lock");
        let mut document = store.read().expect("read");
        document.clock_offset_seconds += seconds;
        store.write(&document).expect("write");
    }

    /// Harness snapshot: capture the whole provider state, dedup included.
    pub fn save_snapshot(&self) {
        let store = self.store.lock().expect("store lock");
        let document = store.read().expect("read");
        store
            .write_snapshot(&document, CLOCK_ORIGIN)
            .expect("write snapshot");
    }

    /// Harness snapshot: restore the most recent snapshot (`false` = none taken).
    pub fn restore_snapshot(&self) -> bool {
        let store = self.store.lock().expect("store lock");
        match store.restore_snapshot() {
            Ok(restored) => restored,
            Err(error) => panic!("restore snapshot: {error}"),
        }
    }

    /// Provider provisioning: a space plus its provider policy.
    pub fn provision_space(
        &self,
        space_id: &str,
        authority_host_id: &str,
        required_profile_kinds: &[&str],
    ) {
        let store = self.store.lock().expect("store lock");
        let mut document = store.read().expect("read");
        document
            .spaces
            .entry(space_id.to_string())
            .or_insert_with(|| Space {
                space_id: space_id.to_string(),
                display_name: Some(format!("{space_id} (sqlite reference)")),
                authority_host_id: authority_host_id.to_string(),
                authority_generation: 1,
                frozen: false,
                revision: 1,
                created_at: CLOCK_ORIGIN.to_string(),
                state_epoch: 1,
                required_profile_kinds: required_profile_kinds
                    .iter()
                    .map(|kind| (*kind).to_string())
                    .collect(),
                ..Space::default()
            });
        store.write(&document).expect("write");
    }

    /// Provider provisioning: a subject.
    pub fn provision_subject(&self, space_id: &str, subject_id: &str, kind: Option<(&str, &str)>) {
        let store = self.store.lock().expect("store lock");
        let mut document = store.read().expect("read");
        let space = document
            .spaces
            .get_mut(space_id)
            .unwrap_or_else(|| panic!("space {space_id} not provisioned"));
        space
            .subjects
            .entry(subject_id.to_string())
            .or_insert_with(|| {
                let mut subject = json!({
                    "schema_version": 1,
                    "space_id": space_id,
                    "subject_id": subject_id,
                    "status": "active",
                    "fenced_contexts": [],
                    "created_at": CLOCK_ORIGIN,
                    "revision": 1,
                    "extensions": {},
                });
                if let Some((ns, id)) = kind {
                    subject["kind"] = json!({ "ns": ns, "id": id });
                }
                subject
            });
        store.write(&document).expect("write");
    }

    /// Provider provisioning: a profile document.
    pub fn provision_profile(&self, space_id: &str, profile: Value) {
        let store = self.store.lock().expect("store lock");
        let mut document = store.read().expect("read");
        let space = document
            .spaces
            .get_mut(space_id)
            .unwrap_or_else(|| panic!("space {space_id} not provisioned"));
        let key = Self::profile_key(
            profile["subject_id"].as_str().expect("subject_id"),
            profile["profile_kind"].as_str().expect("profile_kind"),
        );
        space.profiles.insert(key, profile);
        store.write(&document).expect("write");
    }

    /// Provider provisioning: a reflection.
    pub fn provision_reflection(&self, space_id: &str, reflection: Value) {
        let store = self.store.lock().expect("store lock");
        let mut document = store.read().expect("read");
        let space = document
            .spaces
            .get_mut(space_id)
            .unwrap_or_else(|| panic!("space {space_id} not provisioned"));
        let id = reflection["reflection_id"]
            .as_str()
            .expect("reflection_id")
            .to_string();
        space.reflections.insert(id, reflection);
        store.write(&document).expect("write");
    }

    /// Provider provisioning: a grant (the admin bootstrap).
    pub fn provision_grant(&self, space_id: &str, grant: Value) {
        let store = self.store.lock().expect("store lock");
        let mut document = store.read().expect("read");
        let space = document
            .spaces
            .get_mut(space_id)
            .unwrap_or_else(|| panic!("space {space_id} not provisioned"));
        let grant_id = grant["grant_id"]
            .as_str()
            .expect("provisioned grant carries grant_id")
            .to_string();
        space.grants.insert(grant_id, grant);
        store.write(&document).expect("write");
    }

    /// Provider provisioning: a retained record (seeding for recall tests).
    pub fn provision_record(&self, space_id: &str, record: Value) {
        let store = self.store.lock().expect("store lock");
        let mut document = store.read().expect("read");
        let space = document
            .spaces
            .get_mut(space_id)
            .unwrap_or_else(|| panic!("space {space_id} not provisioned"));
        let record_id = record["record_id"]
            .as_str()
            .expect("provisioned record carries record_id")
            .to_string();
        space.records.insert(record_id, record);
        store.write(&document).expect("write");
    }

    /// The provider worker, run separately from any read path (MEM-R011):
    /// completes deferred receipts (reflection generation, async op
    /// results). Idempotent — a no-op when nothing is pending.
    pub fn run_worker(&self) {
        let store = self.store.lock().expect("store lock");
        let mut document = store.read().expect("read state");
        if Self::complete_pending(&mut document) {
            store.write(&document).expect("write state");
        }
    }

    /// Test-support accessor: run the worker for one receipt and return its
    /// JSON. The generated `MemoryReceiptsGetResponse` copy rejects content-free
    /// results that carry nested objects (Task 4 report finding), so the
    /// transfer tests observe receipts at value level.
    pub fn poll_receipt_value(&self, space_id: &str, receipt_id: &str) -> Option<Value> {
        self.run_worker();
        let store = self.store.lock().expect("store lock");
        let document = store.read().expect("read state");
        document
            .spaces
            .get(space_id)?
            .receipts
            .get(receipt_id)
            .cloned()
    }

    /// Test-support accessor: the provider's own chunk JSON for an export, so
    /// digests can be verified against what the provider emitted (the typed
    /// response normalizes optional fields, which would perturb a re-digest).
    pub fn export_chunks(&self, space_id: &str, export_id: &str) -> Option<Vec<Value>> {
        let store = self.store.lock().expect("store lock");
        let document = store.read().expect("read");
        document
            .spaces
            .get(space_id)?
            .exports
            .get(export_id)
            .map(|export| export.chunks.clone())
    }

    fn space<'a>(document: &'a Document, space_id: &str) -> SpokeResult<&'a Space> {
        match document.spaces.get(space_id) {
            Some(space) => SpokeResult::Ok(space),
            None => reject(
                SpokeRejectCode::NotFound,
                format!("space {space_id} not found"),
            ),
        }
    }

    fn space_mut<'a>(document: &'a mut Document, space_id: &str) -> SpokeResult<&'a mut Space> {
        match document.spaces.get_mut(space_id) {
            Some(space) => SpokeResult::Ok(space),
            None => reject(
                SpokeRejectCode::NotFound,
                format!("space {space_id} not found"),
            ),
        }
    }

    /// §6.5 coverage for one requirement.
    fn authorized(
        space: &Space,
        now: DateTime<Utc>,
        principal_id: &str,
        requirement: &GrantRequirement<'_>,
    ) -> bool {
        space.grants.values().any(|grant| {
            match serde_json::from_value::<MemoryGrant>(grant.clone()) {
                Ok(grant) => {
                    grant_covers_wrapper(&grant, principal_id, &space.space_id, now, requirement)
                }
                Err(_) => false,
            }
        })
    }

    fn require(
        space: &Space,
        now: DateTime<Utc>,
        principal_id: &str,
        requirement: &GrantRequirement<'_>,
        op: &str,
    ) -> SpokeResult<()> {
        if Self::authorized(space, now, principal_id, requirement) {
            SpokeResult::Ok(())
        } else {
            reject(
                SpokeRejectCode::NotAuthorized,
                format!(
                    "principal {principal_id} lacks {} coverage for {op}",
                    requirement.capability
                ),
            )
        }
    }

    /// Value-based staging entry point (the trait method and the transfer tests
    /// share it). Kept separate because the generated `MemoryExportChunk` union
    /// cannot currently carry a provider chunk — see the Task 4 report finding.
    pub fn stage_import_value(
        &self,
        principal: &Principal,
        request_value: Value,
    ) -> SpokeResult<Value> {
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let chunk = request_value["chunk"].clone();
        let ordinal = number_field(&chunk, "chunk_ordinal");
        let chunk_count = number_field(&chunk, "chunk_count");
        let export_id = text_field(&chunk, "export_id");
        let conflict_policy = request_value["conflict_policy"]
            .as_str()
            .unwrap_or("fail")
            .to_string();
        let adopt = request_value["adopt_grants"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let key = text_field(&request_value, "idempotency_key");
        let digest = tri!(request_digest(&request_value));
        let receipt = tri!(self.mutate_value(|document| {
            let now = Self::clock(document);
            let clock_seconds = document.clock_offset_seconds;
            let space = tri!(Self::space_mut(document, &space_id));
            for capability in ["transfer", "admin"] {
                let requirement = GrantRequirement {
                    capability,
                    subject_id: None,
                    context_ids: Vec::new(),
                    reads_shared: false,
                    subject_all: false,
                };
                tri!(Self::require(
                    space,
                    now,
                    &principal_id,
                    &requirement,
                    "memory.imports.stage"
                ));
            }
            if Self::replay_conflict(space, &principal_id, "memory.imports.stage", &key, &digest) {
                return reject(
                    SpokeRejectCode::IdempotencyConflict,
                    "idempotency key reused with a different payload",
                );
            }
            if let Some(receipt) =
                Self::replay(space, &principal_id, "memory.imports.stage", &key, &digest)
            {
                return SpokeResult::Ok(receipt);
            }
            let existing = space
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
                        import_id: format!("imp_sqlite_{:04}", space.imports.len() + 1),
                        export_id: export_id.clone(),
                        chunks: Vec::new(),
                        chunk_count: None,
                        conflict_policy,
                        adopt_grants: adopt,
                        subject_renames: BTreeMap::new(),
                        report: None,
                        bundle: None,
                        staged_seconds: clock_seconds,
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
                // Subject collision policy (§7.31).
                let mut renames: BTreeMap<String, String> = BTreeMap::new();
                for subject in bundle["subjects"].as_array().cloned().unwrap_or_default() {
                    let Some(subject_id) = subject["subject_id"].as_str() else {
                        continue;
                    };
                    if !space.subjects.contains_key(subject_id) {
                        continue;
                    }
                    if import.conflict_policy == "rename_subject" {
                        renames.insert(
                            subject_id.to_string(),
                            format!("{subject_id}_imported_{}", space.imports.len() + 1),
                        );
                    } else {
                        return reject(
                            SpokeRejectCode::ImportConflict,
                            format!("subject {subject_id} already exists in the target space"),
                        );
                    }
                }
                import.subject_renames = renames;
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
                    &outcome.maps,
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
            space.imports.insert(import.import_id.clone(), import);
            space.touch();
            let mut receipt =
                Self::receipt(space, now, "memory.imports.stage", &principal_id, &digest, &key);
            receipt["result"] = result;
            Self::remember(
                space,
                now,
                &principal_id,
                "memory.imports.stage",
                &key,
                &digest,
                &receipt,
            );
            SpokeResult::Ok(receipt)
        }));
        reply(receipt)
    }

    /// Monotonic receipt id: shared with [`Self::receipt`], so pending-op
    /// receipts and immediate receipts draw from one sequence that eviction
    /// never rewinds.
    fn next_receipt_id(space: &mut Space) -> String {
        space.receipt_sequence += 1;
        format!("rcpt_sqlite_{:04}", space.receipt_sequence)
    }

    /// Record document created by review outcomes and `records.create`; the
    /// placement is inherited verbatim (MEM-R009).
    fn record_document(
        space_id: &str,
        subject_id: &str,
        record_id: &str,
        kind: &str,
        content: Value,
        scope: Value,
        captured_by: &str,
    ) -> Value {
        json!({
            "schema_version": 1,
            "record_id": record_id,
            "space_id": space_id,
            "subject_id": subject_id,
            "kind": kind,
            "content": content,
            "scope": scope,
            "provenance": {
                "source_context_ids": [],
                "anchors": [],
                "captured_by": captured_by,
                "captured_at": CLOCK_ORIGIN,
            },
            "created_at": CLOCK_ORIGIN,
            "updated_at": CLOCK_ORIGIN,
            "revision": 1,
            "extensions": {},
        })
    }

    /// Fresh receipts stamp the provider's CURRENT clock: after any clock
    /// advance, new receipted work must survive its own retention window.
    /// The id sequence is monotonic in the document (never `receipts.len()`),
    /// so ids stay distinct after eviction — an evicted id is never reissued
    /// to a different request.
    fn receipt(
        space: &mut Space,
        now: DateTime<Utc>,
        op: &str,
        principal_id: &str,
        digest: &str,
        idempotency_key: &str,
    ) -> Value {
        space.receipt_sequence += 1;
        let sequence = space.receipt_sequence;
        let stamp = now.to_rfc3339_opts(SecondsFormat::Secs, true);
        let mut receipt = json!({
            "schema_version": 1,
            "receipt_id": format!("rcpt_sqlite_{sequence:04}"),
            "op": op,
            "space_id": space.space_id,
            "requester_principal_id": principal_id,
            "payload_digest": digest,
            "status": "committed",
            "created_at": stamp,
            "completed_at": stamp,
            "extensions": {},
        });
        if !idempotency_key.is_empty() {
            receipt["idempotency_key"] = json!(idempotency_key);
        }
        receipt
    }

    /// §7.0.5 replay: same key + same digest returns the original receipt; a
    /// different digest is a conflict. Callers run authorization first (MEM-R021).
    fn replay(
        space: &Space,
        principal_id: &str,
        op: &str,
        key: &str,
        digest: &str,
    ) -> Option<Value> {
        if key.is_empty() {
            return None;
        }
        let namespace = format!("{principal_id}\u{1f}{op}\u{1f}{key}");
        space
            .dedup
            .get(&namespace)
            .filter(|entry| entry.digest == digest)
            .and_then(|entry| space.receipts.get(&entry.receipt_id).cloned())
    }

    fn replay_conflict(
        space: &Space,
        principal_id: &str,
        op: &str,
        key: &str,
        digest: &str,
    ) -> bool {
        if key.is_empty() {
            return false;
        }
        let namespace = format!("{principal_id}\u{1f}{op}\u{1f}{key}");
        space
            .dedup
            .get(&namespace)
            .map(|entry| entry.digest != digest)
            .unwrap_or(false)
    }

    /// §7.0.5: the dedup record outlives the receipt body (MEM-R022), so a
    /// same-key replay after eviction answers RECEIPT_EXPIRED without
    /// re-executing the op.
    fn replay_expired(
        space: &Space,
        principal_id: &str,
        op: &str,
        key: &str,
        digest: &str,
    ) -> bool {
        if key.is_empty() {
            return false;
        }
        let namespace = format!("{principal_id}\u{1f}{op}\u{1f}{key}");
        space
            .dedup
            .get(&namespace)
            .map(|entry| entry.digest == digest)
            .unwrap_or(false)
    }

    /// The dedup tombstone carries the receipt's own clock stamp, so
    /// `receipts.get` ages it against the same current clock.
    fn remember(
        space: &mut Space,
        now: DateTime<Utc>,
        principal_id: &str,
        op: &str,
        key: &str,
        digest: &str,
        receipt: &Value,
    ) {
        if key.is_empty() {
            return;
        }
        let namespace = format!("{principal_id}\u{1f}{op}\u{1f}{key}");
        let receipt_id = receipt["receipt_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        space.dedup.insert(
            namespace,
            DedupEntry {
                receipt_id: receipt_id.clone(),
                digest: digest.to_string(),
                created_at: now.to_rfc3339_opts(SecondsFormat::Secs, true),
            },
        );
        space.receipts.insert(receipt_id, receipt.clone());
    }

    fn profile_key(subject_id: &str, profile_kind: &str) -> String {
        format!("{subject_id}\u{1f}{profile_kind}")
    }

    /// Profile revisions for a subject, as `(kind, revision)` pairs — the
    /// `profile_revisions` half of a reflection basis (MEM-R013).
    fn profile_revisions(space: &Space, subject_id: &str) -> Vec<(String, u64)> {
        let prefix = format!("{subject_id}\u{1f}");
        space
            .profiles
            .iter()
            .filter(|(key, _)| key.starts_with(&prefix))
            .filter_map(|(key, profile)| {
                let kind = key.split('\u{1f}').nth(1)?.to_string();
                let revision = profile["revision"].as_u64()?;
                Some((kind, revision))
            })
            .collect()
    }

    fn missing_required_profiles(space: &Space, subject_id: &str) -> Vec<String> {
        space
            .required_profile_kinds
            .iter()
            .filter(|kind| {
                !space
                    .profiles
                    .contains_key(&Self::profile_key(subject_id, kind))
            })
            .cloned()
            .collect()
    }

    /// Current basis digests plus the authorized source pairs.
    fn basis_for(
        space: &Space,
        subject_id: &str,
        selection: &ReadSelection,
        filter: Option<&Value>,
    ) -> (String, Vec<(String, u64)>, Vec<(String, u64)>) {
        let records = selected_records(space, subject_id, selection, filter);
        let sources: Vec<(String, u64)> = records
            .iter()
            .filter_map(|record| {
                Some((
                    record["record_id"].as_str()?.to_string(),
                    record["revision"].as_u64()?,
                ))
            })
            .collect();
        let profiles = Self::profile_revisions(space, subject_id);
        let source_refs: Vec<SourceRef<'_>> = sources
            .iter()
            .map(|(record_id, revision)| SourceRef {
                record_id,
                revision: *revision,
            })
            .collect();
        let profile_refs: Vec<(&str, u64)> = profiles
            .iter()
            .map(|(kind, revision)| (kind.as_str(), *revision))
            .collect();
        let basis = ReflectionBasis {
            selection,
            filter: None,
            sources: &source_refs,
            profile_revisions: &profile_refs,
        };
        let digests = basis_digests(&basis);
        (digests.state_digest, sources, profiles)
    }

    /// `§6.11` logical bundle for an export scope (digest not yet sealed).
    fn export_bundle(
        space: &Space,
        subject_id: Option<&str>,
        include_reflections: bool,
        include_tombstoned: bool,
    ) -> Value {
        let in_scope = |value: &Value| match subject_id {
            None => true,
            Some(subject) => value["subject_id"].as_str() == Some(subject),
        };
        let tombstoned: Vec<String> = space
            .records
            .values()
            .filter(|record| record.get("tombstone").is_some())
            .filter_map(|record| record["record_id"].as_str().map(str::to_string))
            .collect();
        let mut bundle = json!({
            "schema_version": 1,
            "bundle_format_version": 1,
            "memory_protocol_version": 1,
            "export_id": format!("exp_sqlite_{:04}", space.exports.len() + 1),
            "space_id": space.space_id,
            "created_at": CLOCK_ORIGIN,
            "source": {
                "authority_host_id": space.authority_host_id,
                "authority_generation": space.authority_generation,
                "engine_policy_version": ENGINE_POLICY_VERSION,
            },
            "subjects": space
                .subjects
                .values()
                .filter(|subject| in_scope(subject))
                .cloned()
                .collect::<Vec<_>>(),
            "records": space
                .records
                .values()
                .filter(|record| in_scope(record))
                .filter(|record| include_tombstoned || record.get("tombstone").is_none())
                .cloned()
                .collect::<Vec<_>>(),
            "candidates": space
                .candidates
                .values()
                .filter(|candidate| in_scope(candidate))
                .cloned()
                .collect::<Vec<_>>(),
            "profiles": space
                .profiles
                .values()
                .filter(|profile| in_scope(profile))
                .cloned()
                .collect::<Vec<_>>(),
            "receipt_summaries": space
                .receipts
                .values()
                .map(|receipt| {
                    let mut summary = json!({
                        "receipt_id": receipt["receipt_id"],
                        "op": receipt["op"],
                        "requester_principal_ref": receipt["requester_principal_id"],
                        "payload_digest": receipt["payload_digest"],
                        "status": receipt["status"],
                        "created_at": receipt["created_at"],
                    });
                    if let Some(key) = receipt.get("idempotency_key") {
                        summary["idempotency_key"] = key.clone();
                    }
                    summary
                })
                .collect::<Vec<_>>(),
            "grants": space
                .grants
                .values()
                .map(|grant| {
                    let mut summary = json!({
                        "grant_id": grant["grant_id"],
                        "principal_ref": grant["principal_id"],
                        "capabilities": grant["capabilities"],
                        "revision": grant["revision"],
                    });
                    for field in ["subject_id", "context_ids", "shared_access"] {
                        if let Some(value) = grant.get(field) {
                            summary[field] = value.clone();
                        }
                    }
                    summary
                })
                .collect::<Vec<_>>(),
            "extensions": {},
        });
        if include_reflections {
            bundle["reflections"] = json!(space
                .reflections
                .values()
                .filter(|reflection| in_scope(reflection))
                .filter(|reflection| {
                    let id = reflection["reflection_id"].as_str().unwrap_or_default();
                    space
                        .reflection_sources
                        .get(id)
                        .map(|sources| !sources.iter().any(|source| tombstoned.contains(source)))
                        .unwrap_or(true)
                })
                .cloned()
                .collect::<Vec<_>>());
        }
        // The typed Connect round-trip drops explicit empty arrays (the
        // generated chunk types skip serializing them), which would break the
        // sealed chunk digests over the wire. The sealed bundle therefore
        // omits empty arrays: absence is §6.13-valid, and reassembly
        // reproduces the sealed form exactly.
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

    fn latest_reflection<'a>(space: &'a Space, subject_id: &str) -> Option<&'a Value> {
        space
            .reflections
            .values()
            .filter(|reflection| reflection["subject_id"].as_str() == Some(subject_id))
            .max_by(|left, right| {
                left["created_at"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["created_at"].as_str().unwrap_or_default())
                    .then_with(|| {
                        left["reflection_id"]
                            .as_str()
                            .unwrap_or_default()
                            .cmp(right["reflection_id"].as_str().unwrap_or_default())
                    })
            })
    }

    /// `§6.9` staleness at read time: the current basis digest for the pinned
    /// selection differs from the reflection's pinned `basis.state_digest`.
    /// Imported reflections are the §7.31 exception: stale unconditionally,
    /// even when the translated basis reproduces exactly.
    fn reflection_stale(space: &Space, subject_id: &str, reflection: &Value) -> bool {
        let reflection_id = reflection["reflection_id"].as_str().unwrap_or_default();
        if space.imported_reflections.contains(reflection_id) {
            return true;
        }
        let Ok(selection) =
            serde_json::from_value::<ReadSelection>(reflection["selection"].clone())
        else {
            return false;
        };
        let filter = reflection
            .get("filter")
            .filter(|value| !value.is_null())
            .cloned();
        let (current, _sources, _profiles) =
            Self::basis_for(space, subject_id, &selection, filter.as_ref());
        current
            != reflection["basis"]["state_digest"]
                .as_str()
                .unwrap_or_default()
    }

    /// MEM-R026 binding: principal, space, subject, selection/filter, the
    /// ranking query, snapshot marker (state epoch) and
    /// `engine_policy_version`. The offset is the pagination position, never
    /// part of the binding.
    fn cursor_binding(
        space: &Space,
        principal_id: &str,
        subject_id: &str,
        request: &Value,
    ) -> Option<String> {
        let bound = json!({
            "principal_id": principal_id,
            "space_id": space.space_id,
            "subject_id": subject_id,
            "selection": request["selection"].clone(),
            "filter": request.get("filter").cloned().unwrap_or(Value::Null),
            // The query ranks the list, so a continuation must resume the
            // same ranking, not the same filter over a new one.
            "query": request.get("query").cloned().unwrap_or(Value::Null),
            "state_epoch": space.state_epoch,
            "engine_policy_version": ENGINE_POLICY_VERSION,
        });
        match jcs_sha256_hex(&bound) {
            SpokeResult::Ok(digest) => Some(digest),
            SpokeResult::Reject(_) => None,
        }
    }

    /// Opaque cursor token carrying the pagination offset plus the MEM-R026
    /// binding (the epoch prefix is the same snapshot marker, cheap to check).
    fn cursor_token(
        space: &Space,
        principal_id: &str,
        subject_id: &str,
        request: &Value,
        offset: usize,
    ) -> Option<String> {
        let binding = Self::cursor_binding(space, principal_id, subject_id, request)?;
        Some(format!("c{}:{}:{}", space.state_epoch, offset, binding))
    }

    /// Cursor offset, or `None` when any bound input moved (invalidated cursor).
    fn cursor_offset(
        space: &Space,
        principal_id: &str,
        subject_id: &str,
        request: &Value,
        cursor: &str,
    ) -> Option<usize> {
        let (epoch, rest) = cursor.split_once(':')?;
        let (offset, binding) = rest.split_once(':')?;
        let epoch: u64 = epoch.strip_prefix('c')?.parse().ok()?;
        if epoch != space.state_epoch {
            return None;
        }
        if binding != Self::cursor_binding(space, principal_id, subject_id, request)? {
            return None;
        }
        offset.parse().ok()
    }

    /// Admin-shaped requirement: space-wide admin coverage.
    fn admin_requirement<'a>() -> GrantRequirement<'a> {
        GrantRequirement {
            capability: "admin",
            subject_id: None,
            context_ids: Vec::new(),
            reads_shared: false,
            subject_all: false,
        }
    }

    /// Mutation helper for state-only updates: writes the document on success.
    fn mutate_unit(
        &self,
        apply: impl FnOnce(&mut Document) -> SpokeResult<()>,
    ) -> SpokeResult<Document> {
        let store = self.store.lock().expect("store lock");
        let mut document = store.read().expect("read state");
        Self::evict_receipts(&mut document);
        match apply(&mut document) {
            SpokeResult::Ok(()) => {
                store.write(&document).expect("write state");
                SpokeResult::Ok(document)
            }
            SpokeResult::Reject(reject) => SpokeResult::Reject(reject),
        }
    }

    /// Mutation helper for ops that return a value: writes on success, then
    /// hands the value back (the document is provider-private).
    fn mutate_value<T>(
        &self,
        apply: impl FnOnce(&mut Document) -> SpokeResult<T>,
    ) -> SpokeResult<T> {
        let store = self.store.lock().expect("store lock");
        let mut document = store.read().expect("read state");
        Self::evict_receipts(&mut document);
        match apply(&mut document) {
            SpokeResult::Ok(value) => {
                store.write(&document).expect("write state");
                SpokeResult::Ok(value)
            }
            SpokeResult::Reject(reject) => SpokeResult::Reject(reject),
        }
    }

    fn inspect<T>(&self, answer: impl FnOnce(&Document) -> SpokeResult<T>) -> SpokeResult<T> {
        let store = self.store.lock().expect("store lock");
        let mut document = store.read().expect("read state");
        Self::evict_receipts(&mut document);
        store.write(&document).expect("write state");
        answer(&document)
    }

    /// Evict receipt bodies past `receipt_retention_seconds`; dedup tombstones
    /// outlive the bodies (MEM-R022), so a replay of an evicted op answers
    /// `RECEIPT_EXPIRED`.
    fn evict_receipts(document: &mut Document) {
        let now = Self::clock(document);
        for space in document.spaces.values_mut() {
            let stale: Vec<String> = space
                .receipts
                .iter()
                .filter(|(_, receipt)| {
                    parse_time(receipt["created_at"].as_str().unwrap_or(CLOCK_ORIGIN))
                        .map(|created| now - created > Duration::seconds(RECEIPT_RETENTION_SECONDS))
                        .unwrap_or(false)
                })
                .map(|(id, _)| id.clone())
                .collect();
            for id in stale {
                space.receipts.remove(&id);
            }
        }
    }

    /// Provider worker: complete deferred receipts (reflection generation).
    /// Returns `true` when the document changed. The publish-pinned rule is
    /// structural here: the reflection is assembled from the *pinned* basis, so
    /// a basis that moved meanwhile still publishes and reads stale.
    fn complete_pending(document: &mut Document) -> bool {
        let mut completed = false;
        for space in document.spaces.values_mut() {
            let work: Vec<(String, PendingWork)> = space
                .pending
                .iter()
                .map(|(receipt_id, item)| (receipt_id.clone(), item.clone()))
                .collect();
            for (receipt_id, item) in work {
                if let PendingWork::Receipt { result } = item {
                    if let Some(receipt) = space.receipts.get_mut(&receipt_id) {
                        receipt["status"] = json!("committed");
                        receipt["completed_at"] = json!(CLOCK_ORIGIN);
                        receipt["result"] = result;
                    }
                    space.pending.remove(&receipt_id);
                    space.touch();
                    completed = true;
                    continue;
                }
                let PendingWork::Reflection {
                    subject_id,
                    selection,
                    sources,
                    profile_revisions,
                } = item
                else {
                    continue;
                };
                let source_refs: Vec<SourceRef<'_>> = sources
                    .iter()
                    .map(|(record_id, revision)| SourceRef {
                        record_id,
                        revision: *revision,
                    })
                    .collect();
                let profile_refs: Vec<(&str, u64)> = profile_revisions
                    .iter()
                    .map(|(kind, revision)| (kind.as_str(), *revision))
                    .collect();
                let basis = ReflectionBasis {
                    selection: &selection,
                    filter: None,
                    sources: &source_refs,
                    profile_revisions: &profile_refs,
                };
                let draft = assemble_reflection(&basis);
                let reflection_id = format!("refl_sqlite_{:04}", space.reflections.len() + 1);
                let reflection = json!({
                    "schema_version": 1,
                    "reflection_id": reflection_id,
                    "space_id": space.space_id,
                    "subject_id": subject_id,
                    "selection": serde_json::to_value(&selection).unwrap_or(Value::Null),
                    "basis": {
                        "source_set_digest": draft.source_set_digest,
                        "record_count": draft.record_count,
                        "profile_revisions": profile_revisions
                            .iter()
                            .cloned()
                            .collect::<BTreeMap<String, u64>>(),
                        "state_digest": draft.state_digest,
                    },
                    "body": { "format": draft.format, "body": draft.body },
                    "stale": false,
                    "generator": { "engine_policy_version": ENGINE_POLICY_VERSION },
                    "created_at": CLOCK_ORIGIN,
                    "revision": 1,
                    "extensions": {},
                });
                space.reflection_sources.insert(
                    reflection_id.clone(),
                    sources.iter().map(|(id, _)| id.clone()).collect(),
                );
                space.reflections.insert(reflection_id.clone(), reflection);
                if let Some(receipt) = space.receipts.get_mut(&receipt_id) {
                    receipt["status"] = json!("committed");
                    receipt["completed_at"] = json!(CLOCK_ORIGIN);
                    receipt["result"] = json!({ "reflection_id": reflection_id });
                }
                space.pending.remove(&receipt_id);
                space.touch();
                completed = true;
            }
        }
        completed
    }
}

/// `grant_covers` is re-exported through `spoke_operations`; this wrapper keeps
/// the call site readable next to the JSON-shaped state.
fn grant_covers_wrapper(
    grant: &MemoryGrant,
    principal_id: &str,
    space_id: &str,
    now: DateTime<Utc>,
    requirement: &GrantRequirement<'_>,
) -> bool {
    spoke_operations::grant_covers(grant, principal_id, space_id, now, requirement)
}

#[async_trait]
impl MemoryPort for SqliteMemoryProvider {
    // ── describe ──────────────────────────────────────────────────────────
    async fn describe(
        &self,
        _principal: &Principal,
        _request: MemoryDescribeRequest,
    ) -> SpokeResult<MemoryDescribeResponse> {
        reply(json!({
            "memory_protocol_version": 1,
            "capabilities": ["spoke-memory"],
            "limits": {
                "max_content_bytes": MAX_CONTENT_BYTES,
                "max_candidates_per_capture": MAX_CANDIDATES_PER_CAPTURE,
                "max_page_size": MAX_PAGE_SIZE,
                "export_chunk_bytes": EXPORT_CHUNK_BYTES,
                "idempotency_retention_seconds": IDEMPOTENCY_RETENTION_SECONDS,
                "receipt_retention_seconds": RECEIPT_RETENTION_SECONDS,
                "dedup_retention_seconds": DEDUP_RETENTION_SECONDS,
                "staged_import_ttl_seconds": STAGED_IMPORT_TTL_SECONDS,
                "export_retention_seconds": EXPORT_RETENTION_SECONDS,
            },
            "required_profile_kinds": ["personality"],
            "engine_policy_version": ENGINE_POLICY_VERSION,
        }))
    }

    // ── spaces ────────────────────────────────────────────────────────────
    async fn list_spaces(
        &self,
        principal: &Principal,
        request: MemorySpacesListRequest,
    ) -> SpokeResult<MemorySpacesListResponse> {
        let request_value = wire(&request);
        if let SpokeResult::Reject(reject) = validate_memory_spaces_list(&request, page_limit()) {
            return SpokeResult::Reject(reject);
        }
        let principal_id = identity(principal).to_string();
        let limit = request_value["limit"].as_u64().unwrap_or(MAX_PAGE_SIZE) as usize;
        self.inspect(|document| {
            let now = Self::clock(document);
            let mut spaces: Vec<Value> = document
                .spaces
                .values()
                .filter(|space| {
                    space.grants.values().any(|grant| {
                        if grant["principal_id"] != principal_id.as_str() {
                            return false;
                        }
                        match grant.get("expires_at").and_then(Value::as_str) {
                            Some(expires_at) => {
                                parse_time(expires_at).map(|at| at > now).unwrap_or(false)
                            }
                            None => true,
                        }
                    })
                })
                .map(|space| {
                    json!({
                        "schema_version": 1,
                        "space_id": space.space_id,
                        "display_name": space.display_name,
                        "authority_host_id": space.authority_host_id,
                        "authority_generation": space.authority_generation,
                        "status": if space.frozen { "frozen" } else { "active" },
                        "pending_receipts": space.pending_receipts(),
                        "created_at": space.created_at,
                        "revision": space.revision,
                        "extensions": {},
                    })
                })
                .collect();
            spaces.sort_by(|left, right| {
                left["space_id"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["space_id"].as_str().unwrap_or_default())
            });
            spaces.truncate(limit);
            reply(json!({ "spaces": spaces }))
        })
    }

    async fn freeze_space(
        &self,
        principal: &Principal,
        request: MemorySpacesFreezeRequest,
    ) -> SpokeResult<MemorySpacesFreezeResponse> {
        self.set_frozen(principal, wire(&request), true)
    }

    async fn unfreeze_space(
        &self,
        principal: &Principal,
        request: MemorySpacesUnfreezeRequest,
    ) -> SpokeResult<MemorySpacesUnfreezeResponse> {
        self.set_frozen(principal, wire(&request), false)
    }

    // ── subjects ──────────────────────────────────────────────────────────
    async fn ensure_subject(
        &self,
        principal: &Principal,
        request: MemorySubjectsEnsureRequest,
    ) -> SpokeResult<MemorySubjectsEnsureResponse> {
        let request_value = wire(&request);
        let principal_id = identity(principal).to_string();
        let space_id = request_value["space_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let subject_id = request_value["subject_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        // `memory.subjects.ensure` is exempt from the idempotency-key echo.
        let key = String::new();
        let digest = tri!(request_digest(&request_value));
        let receipt = tri!(self.mutate_value(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space_mut(document, &space_id));
            let requirement = GrantRequirement {
                capability: "capture",
                subject_id: Some(&subject_id),
                context_ids: Vec::new(),
                reads_shared: false,
                subject_all: false,
            };
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.subjects.ensure"
            ));
            if let Some(receipt) = Self::replay(
                space,
                &principal_id,
                "memory.subjects.ensure",
                &key,
                &digest,
            ) {
                return SpokeResult::Ok(receipt);
            }
            let created = !space.subjects.contains_key(&subject_id);
            space.subjects.entry(subject_id.clone()).or_insert_with(|| {
                let mut subject = json!({
                    "schema_version": 1,
                    "space_id": space_id,
                    "subject_id": subject_id,
                    "status": "active",
                    "fenced_contexts": [],
                    "created_at": CLOCK_ORIGIN,
                    "revision": 1,
                    "extensions": {},
                });
                if let Some(kind) = request_value.get("kind") {
                    subject["kind"] = kind.clone();
                }
                subject
            });
            space.touch();
            let mut receipt = Self::receipt(
                space,
                now,
                "memory.subjects.ensure",
                &principal_id,
                &digest,
                &key,
            );
            let subject = space
                .subjects
                .get(&subject_id)
                .cloned()
                .unwrap_or(Value::Null);
            receipt["result"] = json!({
                "subject_id": subject_id,
                "created": created,
                "status": subject["status"],
                "revision": subject["revision"],
            });
            Self::remember(
                space,
                now,
                &principal_id,
                "memory.subjects.ensure",
                &key,
                &digest,
                &receipt,
            );
            SpokeResult::Ok(receipt)
        }));
        reply(receipt)
    }

    async fn get_subject(
        &self,
        principal: &Principal,
        request: MemorySubjectsGetRequest,
    ) -> SpokeResult<MemorySubjectsGetResponse> {
        let request_value = wire(&request);
        let principal_id = identity(principal).to_string();
        let space_id = request_value["space_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let subject_id = request_value["subject_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        self.inspect(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space(document, &space_id));
            let requirement = GrantRequirement {
                capability: "capture",
                subject_id: Some(&subject_id),
                context_ids: Vec::new(),
                reads_shared: false,
                subject_all: false,
            };
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.subjects.get"
            ));
            match space.subjects.get(&subject_id) {
                Some(subject) => reply(json!({ "subject": subject })),
                None => reject(SpokeRejectCode::NotFound, "subject not found"),
            }
        })
    }

    async fn fence_subject(
        &self,
        principal: &Principal,
        request: MemorySubjectsFenceRequest,
    ) -> SpokeResult<MemorySubjectsFenceResponse> {
        let request_value = wire(&request);
        if let SpokeResult::Reject(reject) = validate_memory_subjects_fence(&request) {
            return SpokeResult::Reject(reject);
        }
        let principal_id = identity(principal).to_string();
        let space_id = request_value["space_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let subject_id = request_value["subject_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let key = request_value["idempotency_key"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let expected_revision = request_value["expected_revision"]
            .as_u64()
            .unwrap_or_default();
        let action = request_value["action"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let target = request_value["target"].clone();
        let digest = tri!(request_digest(&request_value));
        let receipt = tri!(self.mutate_value(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space_mut(document, &space_id));
            let requirement = GrantRequirement {
                capability: "admin",
                subject_id: Some(&subject_id),
                context_ids: Vec::new(),
                reads_shared: false,
                subject_all: false,
            };
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.subjects.fence"
            ));
            if Self::replay_conflict(space, &principal_id, "memory.subjects.fence", &key, &digest) {
                return reject(
                    SpokeRejectCode::IdempotencyConflict,
                    "idempotency key reused with a different payload",
                );
            }
            if let Some(receipt) =
                Self::replay(space, &principal_id, "memory.subjects.fence", &key, &digest)
            {
                return SpokeResult::Ok(receipt);
            }
            let Some(subject) = space.subjects.get(&subject_id).cloned() else {
                return reject(SpokeRejectCode::NotFound, "subject not found");
            };
            let revision = subject["revision"].as_u64().unwrap_or_default();
            if revision != expected_revision {
                return reject(
                    SpokeRejectCode::RevisionConflict,
                    format!("subject revision is {revision}"),
                );
            }
            let mut updated = subject;
            match target["type"].as_str() {
                Some("context") => {
                    let context_id = target["context_id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    let mut fenced: Vec<String> = updated["fenced_contexts"]
                        .as_array()
                        .map(|contexts| {
                            contexts
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default();
                    fenced.retain(|context| context != &context_id);
                    if action == "fence" {
                        fenced.push(context_id);
                    }
                    updated["fenced_contexts"] = json!(fenced);
                }
                _ => {
                    updated["status"] = json!(match action.as_str() {
                        "archive" => "archived",
                        "fence" => "fenced",
                        _ => "active",
                    });
                }
            }
            updated["revision"] = json!(revision + 1);
            space.subjects.insert(subject_id.clone(), updated);
            space.touch();
            let subject = space
                .subjects
                .get(&subject_id)
                .cloned()
                .unwrap_or(Value::Null);
            let mut receipt =
                Self::receipt(space, now, "memory.subjects.fence", &principal_id, &digest, &key);
            receipt["result"] = json!({
                "subject_id": subject_id,
                "status": subject["status"],
                "fenced_contexts": subject["fenced_contexts"],
                "revision": subject["revision"],
            });
            Self::remember(
                space,
                now,
                &principal_id,
                "memory.subjects.fence",
                &key,
                &digest,
                &receipt,
            );
            SpokeResult::Ok(receipt)
        }));
        reply(receipt)
    }

    // ── grants ────────────────────────────────────────────────────────────
    async fn list_grants(
        &self,
        principal: &Principal,
        request: MemoryGrantsListRequest,
    ) -> SpokeResult<MemoryGrantsListResponse> {
        let request_value = wire(&request);
        if let SpokeResult::Reject(reject) = validate_memory_grants_list(&request, page_limit()) {
            return SpokeResult::Reject(reject);
        }
        let principal_id = identity(principal).to_string();
        let space_id = request_value["space_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let limit = request_value["limit"].as_u64().unwrap_or(MAX_PAGE_SIZE) as usize;
        self.inspect(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space(document, &space_id));
            let requirement = Self::admin_requirement();
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.grants.list"
            ));
            let mut grants: Vec<Value> = space.grants.values().cloned().collect();
            grants.sort_by(|left, right| {
                left["grant_id"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["grant_id"].as_str().unwrap_or_default())
            });
            grants.truncate(limit);
            reply(json!({ "grants": grants }))
        })
    }

    async fn grant(
        &self,
        principal: &Principal,
        request: MemoryGrantsGrantRequest,
    ) -> SpokeResult<MemoryGrantsGrantResponse> {
        let request_value = wire(&request);
        if let SpokeResult::Reject(reject) = validate_memory_grants_grant(&request) {
            return SpokeResult::Reject(reject);
        }
        let principal_id = identity(principal).to_string();
        let space_id = request_value["space_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let key = request_value["idempotency_key"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let digest = tri!(request_digest(&request_value));
        let receipt = tri!(self.mutate_value(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space_mut(document, &space_id));
            if Self::replay_conflict(space, &principal_id, "memory.grants.grant", &key, &digest) {
                return reject(
                    SpokeRejectCode::IdempotencyConflict,
                    "idempotency key reused with a different payload",
                );
            }
            if let Some(receipt) =
                Self::replay(space, &principal_id, "memory.grants.grant", &key, &digest)
            {
                return SpokeResult::Ok(receipt);
            }
            // §7.9 non-amplification against the granter's own admin grants.
            let granter_grants: Vec<MemoryGrant> = space
                .grants
                .values()
                .filter(|grant| grant["principal_id"] == principal_id.as_str())
                .filter_map(|grant| serde_json::from_value::<MemoryGrant>(grant.clone()).ok())
                .collect();
            let capability_strings: Vec<String> = request_value["capabilities"]
                .as_array()
                .map(|capabilities| {
                    capabilities
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let capabilities: Vec<&str> = capability_strings.iter().map(String::as_str).collect();
            let context_ids: Vec<String> = request_value["context_ids"]
                .as_array()
                .map(|contexts| {
                    contexts
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let issued = IssuedGrant {
                capabilities: &capabilities,
                subject_id: request_value["subject_id"].as_str(),
                context_ids: &context_ids,
                shared_access: request_value["shared_access"].as_bool().unwrap_or(false),
                expires_at: request_value["expires_at"].as_str().and_then(parse_time),
            };
            match validate_grant_issuance(&granter_grants, &principal_id, &space_id, now, &issued) {
                SpokeResult::Ok(()) => {}
                SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
            }
            let grant_id = format!("grt_sqlite_{:04}", space.grants.len() + 1);
            let mut issued_grant = json!({
                "schema_version": 1,
                "grant_id": grant_id,
                "space_id": space_id,
                "principal_id": request_value["principal_id"],
                "capabilities": request_value["capabilities"],
                "created_at": CLOCK_ORIGIN,
                "revision": 1,
                "extensions": {},
            });
            if let Some(subject_id) = request_value["subject_id"].as_str() {
                issued_grant["subject_id"] = json!(subject_id);
            }
            if request_value["context_ids"].is_array() {
                issued_grant["context_ids"] = request_value["context_ids"].clone();
                if let Some(shared_access) = request_value["shared_access"].as_bool() {
                    issued_grant["shared_access"] = json!(shared_access);
                }
            }
            if let Some(expires_at) = request_value["expires_at"].as_str() {
                issued_grant["expires_at"] = json!(expires_at);
            }
            space.grants.insert(grant_id.clone(), issued_grant);
            space.touch();
            let mut receipt =
                Self::receipt(space, now, "memory.grants.grant", &principal_id, &digest, &key);
            receipt["result"] = json!({ "grant_id": grant_id, "revision": 1 });
            Self::remember(
                space,
                now,
                &principal_id,
                "memory.grants.grant",
                &key,
                &digest,
                &receipt,
            );
            SpokeResult::Ok(receipt)
        }));
        reply(receipt)
    }

    async fn revoke_grant(
        &self,
        principal: &Principal,
        request: MemoryGrantsRevokeRequest,
    ) -> SpokeResult<MemoryGrantsRevokeResponse> {
        let request_value = wire(&request);
        let principal_id = identity(principal).to_string();
        let space_id = request_value["space_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let grant_id = request_value["grant_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let key = request_value["idempotency_key"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let expected_revision = request_value["expected_revision"]
            .as_u64()
            .unwrap_or_default();
        let digest = tri!(request_digest(&request_value));
        let receipt = tri!(self.mutate_value(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space_mut(document, &space_id));
            // Revocation stays available inside a freeze (§8.2).
            let requirement = Self::admin_requirement();
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.grants.revoke"
            ));
            if Self::replay_conflict(space, &principal_id, "memory.grants.revoke", &key, &digest) {
                return reject(
                    SpokeRejectCode::IdempotencyConflict,
                    "idempotency key reused with a different payload",
                );
            }
            if let Some(receipt) =
                Self::replay(space, &principal_id, "memory.grants.revoke", &key, &digest)
            {
                return SpokeResult::Ok(receipt);
            }
            let Some(grant) = space.grants.get(&grant_id).cloned() else {
                return reject(SpokeRejectCode::NotFound, "grant not found");
            };
            let revision = grant["revision"].as_u64().unwrap_or_default();
            if revision != expected_revision {
                return reject(
                    SpokeRejectCode::RevisionConflict,
                    format!("grant revision is {revision}"),
                );
            }
            let mut revoked = grant;
            revoked["revoked_at"] = json!(CLOCK_ORIGIN);
            revoked["revision"] = json!(revision + 1);
            space.grants.insert(grant_id.clone(), revoked);
            space.touch();
            let mut receipt =
                Self::receipt(space, now, "memory.grants.revoke", &principal_id, &digest, &key);
            receipt["result"] = json!({
                "grant_id": grant_id,
                "revoked": true,
                "revision": revision + 1,
            });
            Self::remember(
                space,
                now,
                &principal_id,
                "memory.grants.revoke",
                &key,
                &digest,
                &receipt,
            );
            SpokeResult::Ok(receipt)
        }));
        reply(receipt)
    }

    // ── receipts ──────────────────────────────────────────────────────────
    async fn get_receipt(
        &self,
        principal: &Principal,
        request: MemoryReceiptsGetRequest,
    ) -> SpokeResult<MemoryReceiptsGetResponse> {
        let request_value = wire(&request);
        let principal_id = identity(principal).to_string();
        let space_id = request_value["space_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let receipt_id = request_value["receipt_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        // MEM-R011: reads never write. Polling observes the receipt as
        // stored; the provider worker settles pending work separately
        // (`run_worker`), so polling alone cannot change stored state.
        self.inspect(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space(document, &space_id));
            let Some(receipt) = space.receipts.get(&receipt_id).cloned() else {
                // Body evicted? The dedup tombstone outlives it (MEM-R022).
                let tombstone = space
                    .dedup
                    .values()
                    .find(|entry| entry.receipt_id == receipt_id);
                if let Some(entry) = tombstone {
                    let created = parse_time(&entry.created_at).unwrap_or(now);
                    if now - created > Duration::seconds(RECEIPT_RETENTION_SECONDS) {
                        return reject_details(
                            SpokeRejectCode::ReceiptExpired,
                            "receipt body evicted; recovered from the dedup tombstone",
                            json!({ "last_known_status": "committed" }),
                        );
                    }
                }
                return reject(SpokeRejectCode::NotFound, "receipt not found");
            };
            if receipt["requester_principal_id"] != principal_id.as_str() {
                let requirement = Self::admin_requirement();
                tri!(Self::require(
                    space,
                    now,
                    &principal_id,
                    &requirement,
                    "memory.receipts.get"
                ));
            }
            let created =
                parse_time(receipt["created_at"].as_str().unwrap_or(CLOCK_ORIGIN)).unwrap_or(now);
            if now - created > Duration::seconds(RECEIPT_RETENTION_SECONDS) {
                return reject_details(
                    SpokeRejectCode::ReceiptExpired,
                    "receipt body evicted",
                    json!({ "last_known_status": receipt["status"].clone() }),
                );
            }
            reply(json!({ "receipt": receipt }))
        })
    }

    // ── op groups owned by later sub-rounds ───────────────────────────────
    async fn capture(
        &self,
        principal: &Principal,
        request: MemoryCaptureRequest,
    ) -> SpokeResult<MemoryCaptureResponse> {
        let request_value = wire(&request);
        if let SpokeResult::Reject(reject) =
            validate_memory_capture(&request, MAX_CONTENT_BYTES as usize)
        {
            return SpokeResult::Reject(reject);
        }
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let subject_id = text_field(&request_value, "subject_id");
        let scope = request_value["capture_scope"].clone();
        let shared = scope["visibility"].as_str() == Some("shared");
        let contexts: Vec<String> = scope["context_id"]
            .as_str()
            .map(str::to_string)
            .into_iter()
            .collect();
        let key = text_field(&request_value, "idempotency_key");
        let digest = tri!(request_digest(&request_value));
        let receipt = tri!(self.mutate_value(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space_mut(document, &space_id));
            // Capture is outside the §8.2 maintenance allowlist.
            if space.frozen {
                return reject(
                    SpokeRejectCode::SpaceFrozen,
                    format!("space {} is frozen", space.space_id),
                );
            }
            let requirement = GrantRequirement {
                capability: "capture",
                subject_id: Some(&subject_id),
                context_ids: contexts.clone(),
                reads_shared: shared,
                subject_all: false,
            };
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.capture"
            ));
            if contexts
                .iter()
                .any(|context| space.fenced_contexts(&subject_id).contains(context))
            {
                return reject(
                    SpokeRejectCode::SubjectFenced,
                    format!("capture_scope is fenced for {subject_id}"),
                );
            }
            if Self::replay_conflict(space, &principal_id, "memory.capture", &key, &digest) {
                return reject(
                    SpokeRejectCode::IdempotencyConflict,
                    "idempotency key reused with a different payload",
                );
            }
            if let Some(receipt) =
                Self::replay(space, &principal_id, "memory.capture", &key, &digest)
            {
                return SpokeResult::Ok(receipt);
            }
            if Self::replay_expired(space, &principal_id, "memory.capture", &key, &digest) {
                return reject_details(
                    SpokeRejectCode::ReceiptExpired,
                    "receipt body evicted; dedup tombstone retained (MEM-R022)",
                    json!({ "last_known_status": "committed" }),
                );
            }
            let content = request_value["content"].clone();
            let text = content["text"].as_str().unwrap_or_default().to_string();
            let kind = request_value["kind_hint"]
                .as_str()
                .unwrap_or("note")
                .to_string();
            let classification = classify_candidate(&CandidateMaterial {
                kind: &kind,
                text: &text,
                has_structured_content: content.get("structured").is_some(),
                occurred_at: None,
                source_context_count: request_value["source_context_ids"]
                    .as_array()
                    .map_or(0, Vec::len),
                anchor_count: request_value["anchors"].as_array().map_or(0, Vec::len),
            });
            let mut candidate_ids: Vec<String> = Vec::new();
            if classification.disposition != Disposition::Drop {
                let candidate_id = format!("cand_sqlite_{:04}", space.candidates.len() + 1);
                space.candidates.insert(
                    candidate_id.clone(),
                    json!({
                        "schema_version": 1,
                        "candidate_id": candidate_id,
                        "space_id": space_id,
                        "subject_id": subject_id,
                        "capture_scope": scope,
                        "proposed": { "kind": kind, "content": content },
                        "provenance": {
                            "source_context_ids": request_value["source_context_ids"]
                                .as_array()
                                .cloned()
                                .unwrap_or_default(),
                            "anchors": request_value["anchors"]
                                .as_array()
                                .cloned()
                                .unwrap_or_default(),
                            "captured_by": principal_id,
                            "captured_at": CLOCK_ORIGIN,
                        },
                        "classification": serde_json::to_value(&classification)
                            .unwrap_or(Value::Null),
                        "status": "pending",
                        "occurred_at": request_value["occurred_at"].clone(),
                        "created_at": CLOCK_ORIGIN,
                        "revision": 1,
                        "extensions": {},
                    }),
                );
                candidate_ids.push(candidate_id);
            }
            let classified = candidate_ids.len();
            space.touch();
            let mut receipt = Self::receipt(space, now, "memory.capture", &principal_id, &digest, &key);
            receipt["result"] = json!({
                "candidate_ids": candidate_ids,
                "classified": classified,
            });
            Self::remember(
                space,
                now,
                &principal_id,
                "memory.capture",
                &key,
                &digest,
                &receipt,
            );
            SpokeResult::Ok(receipt)
        }));
        reply(receipt)
    }

    async fn list_candidates(
        &self,
        principal: &Principal,
        request: MemoryCandidatesListRequest,
    ) -> SpokeResult<MemoryCandidatesListResponse> {
        let request_value = wire(&request);
        if let SpokeResult::Reject(reject) = validate_memory_candidates_list(&request, page_limit())
        {
            return SpokeResult::Reject(reject);
        }
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let subject_id = text_field(&request_value, "subject_id");
        let status = request_value["status"].as_str().map(str::to_string);
        let limit = request_value["limit"].as_u64().unwrap_or(MAX_PAGE_SIZE) as usize;
        self.inspect(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space(document, &space_id));
            let requirement = GrantRequirement {
                capability: "review",
                subject_id: Some(&subject_id),
                context_ids: Vec::new(),
                reads_shared: false,
                subject_all: false,
            };
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.candidates.list"
            ));
            let mut candidates: Vec<Value> = space
                .candidates
                .values()
                .filter(|candidate| candidate["subject_id"] == subject_id.as_str())
                .filter(|candidate| match status.as_deref() {
                    None => true,
                    Some(status) => candidate["status"] == status,
                })
                .cloned()
                .collect();
            candidates.sort_by(|left, right| {
                left["candidate_id"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["candidate_id"].as_str().unwrap_or_default())
            });
            candidates.truncate(limit);
            reply(json!({ "candidates": candidates }))
        })
    }

    async fn get_candidate(
        &self,
        principal: &Principal,
        request: MemoryCandidatesGetRequest,
    ) -> SpokeResult<MemoryCandidatesGetResponse> {
        let request_value = wire(&request);
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let candidate_id = text_field(&request_value, "candidate_id");
        self.inspect(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space(document, &space_id));
            let Some(candidate) = space.candidates.get(&candidate_id).cloned() else {
                return reject(SpokeRejectCode::NotFound, "candidate not found");
            };
            let subject_id = text_field(&candidate, "subject_id");
            let requirement = GrantRequirement {
                capability: "review",
                subject_id: Some(&subject_id),
                context_ids: Vec::new(),
                reads_shared: false,
                subject_all: false,
            };
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.candidates.get"
            ));
            reply(json!({ "candidate": candidate }))
        })
    }

    async fn dismiss_candidate(
        &self,
        principal: &Principal,
        request: MemoryCandidatesDismissRequest,
    ) -> SpokeResult<MemoryCandidatesDismissResponse> {
        let request_value = wire(&request);
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let candidate_id = text_field(&request_value, "candidate_id");
        let key = text_field(&request_value, "idempotency_key");
        let expected_revision = number_field(&request_value, "expected_revision");
        let digest = tri!(request_digest(&request_value));
        let receipt = tri!(self.mutate_value(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space_mut(document, &space_id));
            if space.frozen {
                return reject(
                    SpokeRejectCode::SpaceFrozen,
                    format!("space {} is frozen", space.space_id),
                );
            }
            let Some(candidate) = space.candidates.get(&candidate_id).cloned() else {
                return reject(SpokeRejectCode::NotFound, "candidate not found");
            };
            let subject_id = text_field(&candidate, "subject_id");
            let scope = candidate["capture_scope"].clone();
            let shared = scope["visibility"].as_str() == Some("shared");
            let contexts: Vec<String> = scope["context_id"]
                .as_str()
                .map(str::to_string)
                .into_iter()
                .collect();
            let review = GrantRequirement {
                capability: "review",
                subject_id: Some(&subject_id),
                context_ids: contexts.clone(),
                reads_shared: shared,
                subject_all: false,
            };
            let write = GrantRequirement {
                capability: "records_write",
                subject_id: Some(&subject_id),
                context_ids: contexts,
                reads_shared: shared,
                subject_all: false,
            };
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &review,
                "memory.candidates.dismiss"
            ));
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &write,
                "memory.candidates.dismiss"
            ));
            if Self::replay_conflict(
                space,
                &principal_id,
                "memory.candidates.dismiss",
                &key,
                &digest,
            ) {
                return reject(
                    SpokeRejectCode::IdempotencyConflict,
                    "idempotency key reused with a different payload",
                );
            }
            if let Some(receipt) = Self::replay(
                space,
                &principal_id,
                "memory.candidates.dismiss",
                &key,
                &digest,
            ) {
                return SpokeResult::Ok(receipt);
            }
            if candidate["status"] == "consumed" {
                return reject(
                    SpokeRejectCode::CandidateConsumed,
                    "candidate already consumed",
                );
            }
            let revision = number_field(&candidate, "revision");
            if revision != expected_revision {
                return reject(
                    SpokeRejectCode::RevisionConflict,
                    format!("candidate revision is {revision}"),
                );
            }
            // Allocate the receipt ONCE and write its id into consumed_by —
            // never allocate separately from the receipt constructor.
            let mut receipt = Self::receipt(
                space,
                now,
                "memory.candidates.dismiss",
                &principal_id,
                &digest,
                &key,
            );
            let receipt_id = receipt["receipt_id"].clone();
            let content_digest = match jcs_sha256_hex(&candidate["proposed"]["content"]) {
                SpokeResult::Ok(digest) => digest,
                SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
            };
            let mut consumed = candidate;
            consumed["status"] = json!("consumed");
            consumed["revision"] = json!(revision + 1);
            consumed["content_digest"] = json!(content_digest);
            consumed["proposed"] = json!({ "kind": consumed["proposed"]["kind"].clone() });
            consumed["consumed_by"] = json!({
                "op": "memory.candidates.dismiss",
                "receipt_id": receipt_id,
                "outcome": "dismissed",
                "record_ids": [],
            });
            if let Some(object) = consumed.as_object_mut() {
                object.remove("expires_at");
            }
            space.candidates.insert(candidate_id.clone(), consumed);
            space.touch();
            receipt["result"] = json!({ "candidate_id": candidate_id, "outcome": "dismissed" });
            Self::remember(
                space,
                now,
                &principal_id,
                "memory.candidates.dismiss",
                &key,
                &digest,
                &receipt,
            );
            SpokeResult::Ok(receipt)
        }));
        reply(receipt)
    }

    async fn review_candidate(
        &self,
        principal: &Principal,
        request: MemoryCandidatesReviewRequest,
    ) -> SpokeResult<MemoryCandidatesReviewResponse> {
        let request_value = wire(&request);
        if let SpokeResult::Reject(reject) = validate_memory_candidates_review(&request) {
            return SpokeResult::Reject(reject);
        }
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let candidate_id = text_field(&request_value, "candidate_id");
        let key = text_field(&request_value, "idempotency_key");
        let expected_revision = number_field(&request_value, "expected_revision");
        let digest = tri!(request_digest(&request_value));
        let receipt = tri!(self.mutate_value(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space_mut(document, &space_id));
            if space.frozen {
                return reject(
                    SpokeRejectCode::SpaceFrozen,
                    format!("space {} is frozen", space.space_id),
                );
            }
            let Some(candidate) = space.candidates.get(&candidate_id).cloned() else {
                return reject(SpokeRejectCode::NotFound, "candidate not found");
            };
            let subject_id = text_field(&candidate, "subject_id");
            let scope = candidate["capture_scope"].clone();
            let shared = scope["visibility"].as_str() == Some("shared");
            let contexts: Vec<String> = scope["context_id"]
                .as_str()
                .map(str::to_string)
                .into_iter()
                .collect();
            // Review outcomes inherit the admitted placement; a fenced scope is
            // refused before any state changes (MEM-R024).
            if contexts
                .iter()
                .any(|context| space.fenced_contexts(&subject_id).contains(context))
            {
                return reject(
                    SpokeRejectCode::SubjectFenced,
                    "candidate capture_scope is fenced for this subject",
                );
            }
            let review = GrantRequirement {
                capability: "review",
                subject_id: Some(&subject_id),
                context_ids: contexts.clone(),
                reads_shared: shared,
                subject_all: false,
            };
            let write = GrantRequirement {
                capability: "records_write",
                subject_id: Some(&subject_id),
                context_ids: contexts,
                reads_shared: shared,
                subject_all: false,
            };
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &review,
                "memory.candidates.review"
            ));
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &write,
                "memory.candidates.review"
            ));
            if Self::replay_conflict(
                space,
                &principal_id,
                "memory.candidates.review",
                &key,
                &digest,
            ) {
                return reject(
                    SpokeRejectCode::IdempotencyConflict,
                    "idempotency key reused with a different payload",
                );
            }
            if let Some(receipt) = Self::replay(
                space,
                &principal_id,
                "memory.candidates.review",
                &key,
                &digest,
            ) {
                return SpokeResult::Ok(receipt);
            }
            if candidate["status"] == "consumed" {
                return reject(
                    SpokeRejectCode::CandidateConsumed,
                    "candidate already consumed",
                );
            }
            let revision = number_field(&candidate, "revision");
            if revision != expected_revision {
                return reject(
                    SpokeRejectCode::RevisionConflict,
                    format!("candidate revision is {revision}"),
                );
            }
            let proposed = candidate["proposed"].clone();
            let decision = request_value["decision"].clone();
            let action = decision["action"].as_str().unwrap_or("drop").to_string();
            // Allocate the receipt ONCE and write its id into consumed_by —
            // never allocate separately from the receipt constructor.
            let mut receipt = Self::receipt(
                space,
                now,
                "memory.candidates.review",
                &principal_id,
                &digest,
                &key,
            );
            let receipt_id = receipt["receipt_id"].clone();
            let mut record_ids: Vec<String> = Vec::new();
            match action.as_str() {
                "promote" => {
                    let kind = decision["kind"]
                        .as_str()
                        .or_else(|| proposed["kind"].as_str())
                        .unwrap_or("note")
                        .to_string();
                    let content = if decision.get("content_override").is_some() {
                        decision["content_override"].clone()
                    } else {
                        proposed["content"].clone()
                    };
                    let record_id = format!("rec_sqlite_{:04}", space.records.len() + 1);
                    space.records.insert(
                        record_id.clone(),
                        Self::record_document(
                            &space_id,
                            &subject_id,
                            &record_id,
                            &kind,
                            content,
                            scope.clone(),
                            &principal_id,
                        ),
                    );
                    record_ids.push(record_id);
                }
                "fragment" => {
                    for (index, fragment) in decision["fragments"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .enumerate()
                    {
                        let record_id =
                            format!("rec_sqlite_{:04}", space.records.len() + 1 + index);
                        space.records.insert(
                            record_id.clone(),
                            Self::record_document(
                                &space_id,
                                &subject_id,
                                &record_id,
                                fragment["kind"].as_str().unwrap_or("note"),
                                fragment["content"].clone(),
                                scope.clone(),
                                &principal_id,
                            ),
                        );
                        record_ids.push(record_id);
                    }
                }
                _ => {}
            }
            let content_digest = match jcs_sha256_hex(&proposed["content"]) {
                SpokeResult::Ok(digest) => digest,
                SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
            };
            let outcome = match action.as_str() {
                "promote" => "promoted",
                "fragment" => "fragmented",
                _ => "dropped",
            };
            let mut consumed = candidate;
            consumed["status"] = json!("consumed");
            consumed["revision"] = json!(revision + 1);
            consumed["content_digest"] = json!(content_digest);
            consumed["proposed"] = json!({ "kind": proposed["kind"].clone() });
            consumed["consumed_by"] = json!({
                "op": "memory.candidates.review",
                "receipt_id": receipt_id,
                "outcome": outcome,
                "record_ids": record_ids,
            });
            if let Some(object) = consumed.as_object_mut() {
                object.remove("expires_at");
            }
            space.candidates.insert(candidate_id, consumed);
            space.touch();
            receipt["result"] = json!({ "outcome": outcome, "record_ids": record_ids });
            Self::remember(
                space,
                now,
                &principal_id,
                "memory.candidates.review",
                &key,
                &digest,
                &receipt,
            );
            SpokeResult::Ok(receipt)
        }));
        reply(receipt)
    }

    async fn create_record(
        &self,
        principal: &Principal,
        request: MemoryRecordsCreateRequest,
    ) -> SpokeResult<MemoryRecordsCreateResponse> {
        let request_value = wire(&request);
        if let SpokeResult::Reject(reject) =
            validate_memory_records_create(&request, MAX_CONTENT_BYTES as usize)
        {
            return SpokeResult::Reject(reject);
        }
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let subject_id = text_field(&request_value, "subject_id");
        let scope = request_value["scope"].clone();
        let shared = scope["visibility"].as_str() == Some("shared");
        let contexts: Vec<String> = scope["context_id"]
            .as_str()
            .map(str::to_string)
            .into_iter()
            .collect();
        let key = text_field(&request_value, "idempotency_key");
        let digest = tri!(request_digest(&request_value));
        let receipt = tri!(self.mutate_value(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space_mut(document, &space_id));
            if space.frozen {
                return reject(
                    SpokeRejectCode::SpaceFrozen,
                    format!("space {} is frozen", space.space_id),
                );
            }
            let requirement = GrantRequirement {
                capability: "records_write",
                subject_id: Some(&subject_id),
                context_ids: contexts.clone(),
                reads_shared: shared,
                subject_all: false,
            };
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.records.create"
            ));
            if contexts
                .iter()
                .any(|context| space.fenced_contexts(&subject_id).contains(context))
            {
                return reject(
                    SpokeRejectCode::SubjectFenced,
                    "placement context is fenced",
                );
            }
            if Self::replay_conflict(space, &principal_id, "memory.records.create", &key, &digest) {
                return reject(
                    SpokeRejectCode::IdempotencyConflict,
                    "idempotency key reused with a different payload",
                );
            }
            if let Some(receipt) =
                Self::replay(space, &principal_id, "memory.records.create", &key, &digest)
            {
                return SpokeResult::Ok(receipt);
            }
            let record_id = format!("rec_sqlite_{:04}", space.records.len() + 1);
            space.records.insert(
                record_id.clone(),
                Self::record_document(
                    &space_id,
                    &subject_id,
                    &record_id,
                    request_value["kind"].as_str().unwrap_or("note"),
                    request_value["content"].clone(),
                    scope,
                    &principal_id,
                ),
            );
            space.touch();
            let mut receipt =
                Self::receipt(space, now, "memory.records.create", &principal_id, &digest, &key);
            receipt["result"] = json!({ "record_id": record_id, "revision": 1 });
            Self::remember(
                space,
                now,
                &principal_id,
                "memory.records.create",
                &key,
                &digest,
                &receipt,
            );
            SpokeResult::Ok(receipt)
        }));
        reply(receipt)
    }
    async fn get_record(
        &self,
        principal: &Principal,
        request: MemoryRecordsGetRequest,
    ) -> SpokeResult<MemoryRecordsGetResponse> {
        let request_value = wire(&request);
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let record_id = text_field(&request_value, "record_id");
        self.inspect(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space(document, &space_id));
            let Some(record) = space.records.get(&record_id).cloned() else {
                return reject(SpokeRejectCode::NotFound, "record not found");
            };
            let subject_id = text_field(&record, "subject_id");
            let scope = &record["scope"];
            let requirement = GrantRequirement {
                capability: "records_read",
                subject_id: Some(&subject_id),
                context_ids: scope["context_id"]
                    .as_str()
                    .map(str::to_string)
                    .into_iter()
                    .collect(),
                reads_shared: scope["visibility"].as_str() == Some("shared"),
                subject_all: false,
            };
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.records.get"
            ));
            reply(json!({ "record": record }))
        })
    }
    async fn update_record(
        &self,
        principal: &Principal,
        request: MemoryRecordsUpdateRequest,
    ) -> SpokeResult<MemoryRecordsUpdateResponse> {
        let request_value = wire(&request);
        if let SpokeResult::Reject(reject) =
            validate_memory_records_update(&request, MAX_CONTENT_BYTES as usize)
        {
            return SpokeResult::Reject(reject);
        }
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let record_id = text_field(&request_value, "record_id");
        let key = text_field(&request_value, "idempotency_key");
        let expected_revision = number_field(&request_value, "expected_revision");
        let digest = tri!(request_digest(&request_value));
        let receipt = tri!(self.mutate_value(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space_mut(document, &space_id));
            if space.frozen {
                return reject(
                    SpokeRejectCode::SpaceFrozen,
                    format!("space {} is frozen", space.space_id),
                );
            }
            let Some(record) = space.records.get(&record_id).cloned() else {
                return reject(SpokeRejectCode::NotFound, "record not found");
            };
            let subject_id = text_field(&record, "subject_id");
            let scope = &record["scope"];
            let requirement = GrantRequirement {
                capability: "records_write",
                subject_id: Some(&subject_id),
                context_ids: scope["context_id"]
                    .as_str()
                    .map(str::to_string)
                    .into_iter()
                    .collect(),
                reads_shared: scope["visibility"].as_str() == Some("shared"),
                subject_all: false,
            };
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.records.update"
            ));
            if Self::replay_conflict(space, &principal_id, "memory.records.update", &key, &digest) {
                return reject(
                    SpokeRejectCode::IdempotencyConflict,
                    "idempotency key reused with a different payload",
                );
            }
            if let Some(receipt) =
                Self::replay(space, &principal_id, "memory.records.update", &key, &digest)
            {
                return SpokeResult::Ok(receipt);
            }
            let revision = number_field(&record, "revision");
            if revision != expected_revision {
                return reject(
                    SpokeRejectCode::RevisionConflict,
                    format!("record revision is {revision}"),
                );
            }
            let mut updated = record;
            if let Some(content) = request_value.get("content") {
                updated["content"] = content.clone();
            }
            if let Some(kind) = request_value.get("kind") {
                updated["kind"] = kind.clone();
            }
            updated["revision"] = json!(revision + 1);
            updated["updated_at"] = json!(CLOCK_ORIGIN);
            space.records.insert(record_id.clone(), updated);
            space.touch();
            let mut receipt =
                Self::receipt(space, now, "memory.records.update", &principal_id, &digest, &key);
            receipt["result"] = json!({ "record_id": record_id, "revision": revision + 1 });
            Self::remember(
                space,
                now,
                &principal_id,
                "memory.records.update",
                &key,
                &digest,
                &receipt,
            );
            SpokeResult::Ok(receipt)
        }));
        reply(receipt)
    }
    async fn delete_record(
        &self,
        principal: &Principal,
        request: MemoryRecordsDeleteRequest,
    ) -> SpokeResult<MemoryRecordsDeleteResponse> {
        let request_value = wire(&request);
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let record_id = text_field(&request_value, "record_id");
        let key = text_field(&request_value, "idempotency_key");
        let expected_revision = number_field(&request_value, "expected_revision");
        let digest = tri!(request_digest(&request_value));
        let receipt = tri!(self.mutate_value(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space_mut(document, &space_id));
            if space.frozen {
                return reject(
                    SpokeRejectCode::SpaceFrozen,
                    format!("space {} is frozen", space.space_id),
                );
            }
            let Some(record) = space.records.get(&record_id).cloned() else {
                return reject(SpokeRejectCode::NotFound, "record not found");
            };
            let subject_id = text_field(&record, "subject_id");
            let requirement = GrantRequirement {
                capability: "records_write",
                subject_id: Some(&subject_id),
                context_ids: Vec::new(),
                reads_shared: false,
                subject_all: false,
            };
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.records.delete"
            ));
            if Self::replay_conflict(space, &principal_id, "memory.records.delete", &key, &digest) {
                return reject(
                    SpokeRejectCode::IdempotencyConflict,
                    "idempotency key reused with a different payload",
                );
            }
            if let Some(receipt) =
                Self::replay(space, &principal_id, "memory.records.delete", &key, &digest)
            {
                return SpokeResult::Ok(receipt);
            }
            let revision = number_field(&record, "revision");
            if revision != expected_revision {
                return reject(
                    SpokeRejectCode::RevisionConflict,
                    format!("record revision is {revision}"),
                );
            }
            // MEM-R014: content scrubbed, placement kept for audit.
            let mut tombstoned = record;
            tombstoned["content"] = json!({ "text": "" });
            tombstoned["revision"] = json!(revision + 1);
            tombstoned["updated_at"] = json!(CLOCK_ORIGIN);
            tombstoned["tombstone"] = json!({
                "deleted_at": CLOCK_ORIGIN,
                "deleted_by": principal_id,
            });
            space.records.insert(record_id.clone(), tombstoned);
            space.touch();
            let mut receipt =
                Self::receipt(space, now, "memory.records.delete", &principal_id, &digest, &key);
            receipt["result"] = json!({ "record_id": record_id, "tombstoned": true });
            Self::remember(
                space,
                now,
                &principal_id,
                "memory.records.delete",
                &key,
                &digest,
                &receipt,
            );
            SpokeResult::Ok(receipt)
        }));
        reply(receipt)
    }
    async fn list_records(
        &self,
        principal: &Principal,
        request: MemoryRecordsListRequest,
    ) -> SpokeResult<MemoryRecordsListResponse> {
        let request_value = wire(&request);
        if let SpokeResult::Reject(reject) = validate_memory_records_list(&request, page_limit()) {
            return SpokeResult::Reject(reject);
        }
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let subject_id = text_field(&request_value, "subject_id");
        let selection: ReadSelection =
            match serde_json::from_value(request_value["selection"].clone()) {
                Ok(selection) => selection,
                Err(error) => {
                    return reject(SpokeRejectCode::InvalidInput, format!("selection: {error}"))
                }
            };
        let requirement = GrantRequirement {
            capability: "records_read",
            subject_id: Some(&subject_id),
            context_ids: selection_contexts(&selection),
            reads_shared: selection_reads_shared(&selection),
            subject_all: selection_subject_all(&selection),
        };
        let filter = request_value.get("filter").cloned();
        let include_tombstoned = request_value["include_tombstoned"]
            .as_bool()
            .unwrap_or(false);
        let kinds = string_list_field(&request_value, "kinds");
        let limit = request_value["limit"].as_u64().unwrap_or(MAX_PAGE_SIZE) as usize;
        self.inspect(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space(document, &space_id));
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.records.list"
            ));
            let mut records = selected_records(space, &subject_id, &selection, filter.as_ref());
            if !include_tombstoned {
                records.retain(|record| record.get("tombstone").is_none());
            }
            if !kinds.is_empty() {
                records.retain(|record| {
                    record["kind"]
                        .as_str()
                        .map(|kind| kinds.iter().any(|wanted| wanted == kind))
                        .unwrap_or(false)
                });
            }
            records.sort_by(|left, right| {
                left["record_id"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["record_id"].as_str().unwrap_or_default())
            });
            let total_authorized = records.len();
            records.truncate(limit);
            reply(json!({ "records": records, "total_authorized": total_authorized }))
        })
    }
    async fn promote_record(
        &self,
        principal: &Principal,
        request: MemoryRecordsPromoteRequest,
    ) -> SpokeResult<MemoryRecordsPromoteResponse> {
        let request_value = wire(&request);
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let record_id = text_field(&request_value, "record_id");
        let key = text_field(&request_value, "idempotency_key");
        let expected_revision = number_field(&request_value, "expected_revision");
        let scope = request_value["scope"].clone();
        let digest = tri!(request_digest(&request_value));
        let receipt = tri!(self.mutate_value(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space_mut(document, &space_id));
            if space.frozen {
                return reject(
                    SpokeRejectCode::SpaceFrozen,
                    format!("space {} is frozen", space.space_id),
                );
            }
            let Some(record) = space.records.get(&record_id).cloned() else {
                return reject(SpokeRejectCode::NotFound, "record not found");
            };
            let subject_id = text_field(&record, "subject_id");
            let requirement = GrantRequirement {
                capability: "promote",
                subject_id: Some(&subject_id),
                context_ids: Vec::new(),
                reads_shared: true,
                subject_all: false,
            };
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.records.promote"
            ));
            if Self::replay_conflict(
                space,
                &principal_id,
                "memory.records.promote",
                &key,
                &digest,
            ) {
                return reject(
                    SpokeRejectCode::IdempotencyConflict,
                    "idempotency key reused with a different payload",
                );
            }
            if let Some(receipt) = Self::replay(
                space,
                &principal_id,
                "memory.records.promote",
                &key,
                &digest,
            ) {
                return SpokeResult::Ok(receipt);
            }
            let revision = number_field(&record, "revision");
            if revision != expected_revision {
                return reject(
                    SpokeRejectCode::RevisionConflict,
                    format!("record revision is {revision}"),
                );
            }
            let mut promoted = record;
            promoted["scope"] = scope.clone();
            promoted["revision"] = json!(revision + 1);
            promoted["updated_at"] = json!(CLOCK_ORIGIN);
            space.records.insert(record_id.clone(), promoted);
            space.touch();
            let mut receipt = Self::receipt(
                space,
                now,
                "memory.records.promote",
                &principal_id,
                &digest,
                &key,
            );
            receipt["result"] = json!({
                "record_id": record_id,
                "revision": revision + 1,
                "scope": scope,
            });
            Self::remember(
                space,
                now,
                &principal_id,
                "memory.records.promote",
                &key,
                &digest,
                &receipt,
            );
            SpokeResult::Ok(receipt)
        }));
        reply(receipt)
    }
    async fn recall(
        &self,
        principal: &Principal,
        request: MemoryRecallRequest,
    ) -> SpokeResult<MemoryRecallResponse> {
        let request_value = wire(&request);
        if let SpokeResult::Reject(reject) = validate_memory_recall(&request, page_limit()) {
            return SpokeResult::Reject(reject);
        }
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let subject_id = text_field(&request_value, "subject_id");
        let selection: ReadSelection =
            match serde_json::from_value(request_value["selection"].clone()) {
                Ok(selection) => selection,
                Err(error) => {
                    return reject(SpokeRejectCode::InvalidInput, format!("selection: {error}"))
                }
            };
        let requirement = GrantRequirement {
            capability: "recall",
            subject_id: Some(&subject_id),
            context_ids: selection_contexts(&selection),
            reads_shared: selection_reads_shared(&selection),
            subject_all: selection_subject_all(&selection),
        };
        let filter = request_value.get("filter").cloned();
        let query = request_value["query"].as_str().map(str::to_string);
        let max_items = number_field(&request_value, "budget") as usize;
        let max_bytes = request_value["budget"]["max_bytes"]
            .as_u64()
            .map(|bytes| bytes as usize);
        let max_items = if max_items == 0 {
            request_value["budget"]["max_items"]
                .as_u64()
                .unwrap_or(MAX_PAGE_SIZE) as usize
        } else {
            max_items
        };
        let cursor = request_value["cursor"].as_str().map(str::to_string);
        self.inspect(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space(document, &space_id));
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.recall"
            ));
            let offset = match cursor.as_deref() {
                None => 0,
                Some(cursor) => match Self::cursor_offset(
                    space,
                    &principal_id,
                    &subject_id,
                    &request_value,
                    cursor,
                ) {
                    Some(offset) => offset,
                    None => {
                        return reject(
                            SpokeRejectCode::CursorInvalid,
                            "cursor was invalidated by a state or grant change",
                        )
                    }
                },
            };
            let authorized = selected_records(space, &subject_id, &selection, filter.as_ref());
            let created_at: DateTime<Utc> = CLOCK_ORIGIN.parse().expect("clock origin parses");
            let engine_inputs: Vec<RecallRecord> = authorized
                .iter()
                .filter_map(|record| {
                    let content: Content =
                        serde_json::from_value(record["content"].clone()).ok()?;
                    Some(RecallRecord {
                        record_id: record["record_id"].as_str()?.to_string(),
                        kind: record["kind"].as_str()?.to_string(),
                        content,
                        created_at,
                    })
                })
                .collect();
            let page = match engine_recall(
                &engine_inputs,
                &EngineRecall {
                    query: query.as_deref(),
                    // MEM-R026: the cursor offset drops the leading ranked
                    // records BEFORE the page budget, so page 2+ is the next
                    // window of the same ranking (applying it after the
                    // budget made every continuation page empty).
                    offset,
                    max_items,
                    max_bytes,
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
            let total_authorized = page.total_authorized;
            let mut records: Vec<Value> = Vec::new();
            for ranked in &page.records {
                if let Some(record) = authorized
                    .iter()
                    .find(|record| record["record_id"].as_str() == Some(ranked.record_id.as_str()))
                {
                    records.push(record.clone());
                }
            }
            let returned = records.len();
            let consumed = offset + returned;
            let mut response = json!({
                "records": records,
                "total_authorized": total_authorized,
                "truncated": page.truncated,
                "engine_policy_version": ENGINE_POLICY_VERSION,
            });
            if let Some(overflow) = page.overflow_record_id {
                response["overflow_record_id"] = json!(overflow);
            }
            if page.truncated {
                let Some(token) =
                    Self::cursor_token(space, &principal_id, &subject_id, &request_value, consumed)
                else {
                    return reject(
                        SpokeRejectCode::InternalError,
                        "recall cursor binding digest failed",
                    );
                };
                response["next_cursor"] = json!(token);
            }
            reply(response)
        })
    }

    async fn get_profile(
        &self,
        principal: &Principal,
        request: MemoryProfileGetRequest,
    ) -> SpokeResult<MemoryProfileGetResponse> {
        let request_value = wire(&request);
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let subject_id = text_field(&request_value, "subject_id");
        let kinds = string_list_field(&request_value, "profile_kinds");
        self.inspect(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space(document, &space_id));
            let requirement = GrantRequirement {
                capability: "records_read",
                subject_id: Some(&subject_id),
                context_ids: Vec::new(),
                reads_shared: false,
                subject_all: false,
            };
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.profile.get"
            ));
            let prefix = format!("{subject_id}\u{1f}");
            let mut profiles: Vec<Value> = space
                .profiles
                .iter()
                .filter(|(key, _)| key.starts_with(&prefix))
                .filter(|(key, _)| {
                    kinds.is_empty()
                        || key
                            .split('\u{1f}')
                            .nth(1)
                            .map(|kind| kinds.iter().any(|wanted| wanted == kind))
                            .unwrap_or(false)
                })
                .map(|(_, profile)| profile.clone())
                .collect();
            profiles.sort_by(|left, right| {
                left["profile_kind"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["profile_kind"].as_str().unwrap_or_default())
            });
            // Honest empty: an absent kind is an empty list, never an error (§4.2).
            reply(json!({ "profiles": profiles }))
        })
    }

    async fn update_profile(
        &self,
        principal: &Principal,
        request: MemoryProfileUpdateRequest,
    ) -> SpokeResult<MemoryProfileUpdateResponse> {
        let request_value = wire(&request);
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let subject_id = text_field(&request_value, "subject_id");
        let profile_kind = text_field(&request_value, "profile_kind");
        let key = text_field(&request_value, "idempotency_key");
        let expected_revision = request_value["expected_revision"].as_u64();
        let digest = tri!(request_digest(&request_value));
        let receipt = tri!(self.mutate_value(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space_mut(document, &space_id));
            if space.frozen {
                return reject(
                    SpokeRejectCode::SpaceFrozen,
                    format!("space {} is frozen", space.space_id),
                );
            }
            let requirement = GrantRequirement {
                capability: "profile_write",
                subject_id: Some(&subject_id),
                context_ids: Vec::new(),
                reads_shared: true,
                subject_all: false,
            };
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.profile.update"
            ));
            if Self::replay_conflict(space, &principal_id, "memory.profile.update", &key, &digest) {
                return reject(
                    SpokeRejectCode::IdempotencyConflict,
                    "idempotency key reused with a different payload",
                );
            }
            if let Some(receipt) =
                Self::replay(space, &principal_id, "memory.profile.update", &key, &digest)
            {
                return SpokeResult::Ok(receipt);
            }
            let profile_key = Self::profile_key(&subject_id, &profile_kind);
            let existing = space.profiles.get(&profile_key).cloned();
            let revision = existing
                .as_ref()
                .and_then(|profile| profile["revision"].as_u64())
                .unwrap_or(0);
            match expected_revision {
                None if existing.is_some() => {
                    return reject(
                        SpokeRejectCode::RevisionConflict,
                        "profile exists; create-only update cannot overwrite it",
                    )
                }
                Some(expected) if expected != revision => {
                    return reject(
                        SpokeRejectCode::RevisionConflict,
                        format!("profile revision is {revision}"),
                    )
                }
                _ => {}
            }
            let next_revision = revision + 1;
            let profile = json!({
                "schema_version": 1,
                "space_id": space_id,
                "subject_id": subject_id,
                "profile_kind": profile_kind,
                "document": request_value["document"].clone(),
                "updated_by": principal_id,
                "created_at": existing
                    .as_ref()
                    .map(|profile| profile["created_at"].clone())
                    .filter(|value| !value.is_null())
                    .unwrap_or_else(|| json!(CLOCK_ORIGIN)),
                "updated_at": CLOCK_ORIGIN,
                "revision": next_revision,
                "extensions": {},
            });
            space.profiles.insert(profile_key, profile);
            // MEM-R013: the basis includes profile revisions, so every pinned
            // reflection now reads stale — computed at read time.
            space.touch();
            let mut receipt =
                Self::receipt(space, now, "memory.profile.update", &principal_id, &digest, &key);
            receipt["result"] = json!({
                "profile_kind": profile_kind,
                "revision": next_revision,
            });
            Self::remember(
                space,
                now,
                &principal_id,
                "memory.profile.update",
                &key,
                &digest,
                &receipt,
            );
            SpokeResult::Ok(receipt)
        }));
        reply(receipt)
    }

    async fn get_reflection_state(
        &self,
        principal: &Principal,
        request: MemoryReflectionsStateRequest,
    ) -> SpokeResult<MemoryReflectionsStateResponse> {
        let request_value = wire(&request);
        if let SpokeResult::Reject(reject) = validate_memory_reflections_state(&request) {
            return SpokeResult::Reject(reject);
        }
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let subject_id = text_field(&request_value, "subject_id");
        let selection: ReadSelection =
            match serde_json::from_value(request_value["selection"].clone()) {
                Ok(selection) => selection,
                Err(error) => {
                    return reject(SpokeRejectCode::InvalidInput, format!("selection: {error}"))
                }
            };
        let filter = request_value.get("filter").cloned();
        let requirement = GrantRequirement {
            capability: "reflect",
            subject_id: Some(&subject_id),
            context_ids: selection_contexts(&selection),
            reads_shared: selection_reads_shared(&selection),
            subject_all: selection_subject_all(&selection),
        };
        self.inspect(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space(document, &space_id));
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.reflections.state"
            ));
            let missing = Self::missing_required_profiles(space, &subject_id);
            let (state_digest, sources, profiles) =
                Self::basis_for(space, &subject_id, &selection, filter.as_ref());
            let stale = Self::latest_reflection(space, &subject_id)
                .map(|reflection| Self::reflection_stale(space, &subject_id, reflection))
                .unwrap_or(false);
            let mut response = json!({
                "state_digest": state_digest,
                "basis": {
                    "record_count": sources.len(),
                    "profile_kinds": profiles
                        .iter()
                        .map(|(kind, _)| kind.clone())
                        .collect::<Vec<_>>(),
                },
                "stale": stale,
                "missing_required_profile_kinds": missing,
                "data_sufficiency": if sources.is_empty() { "insufficient" } else { "sufficient" },
            });
            if let Some(reflection) = Self::latest_reflection(space, &subject_id) {
                response["last_reflection_id"] = reflection["reflection_id"].clone();
            }
            reply(response)
        })
    }

    async fn list_reflections(
        &self,
        principal: &Principal,
        request: MemoryReflectionsListRequest,
    ) -> SpokeResult<MemoryReflectionsListResponse> {
        let request_value = wire(&request);
        if let SpokeResult::Reject(reject) =
            validate_memory_reflections_list(&request, page_limit())
        {
            return SpokeResult::Reject(reject);
        }
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let subject_id = text_field(&request_value, "subject_id");
        let limit = request_value["limit"].as_u64().unwrap_or(MAX_PAGE_SIZE) as usize;
        let requirement = GrantRequirement {
            capability: "reflect",
            subject_id: Some(&subject_id),
            context_ids: Vec::new(),
            reads_shared: false,
            subject_all: false,
        };
        self.inspect(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space(document, &space_id));
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.reflections.list"
            ));
            let tombstoned: Vec<String> = space
                .records
                .values()
                .filter(|record| record.get("tombstone").is_some())
                .filter_map(|record| record["record_id"].as_str().map(str::to_string))
                .collect();
            let mut reflections: Vec<Value> = space
                .reflections
                .values()
                .filter(|reflection| reflection["subject_id"].as_str() == Some(&subject_id))
                // Reflections pinned to a tombstoned record are excluded (§6.9).
                .filter(|reflection| {
                    let id = reflection["reflection_id"].as_str().unwrap_or_default();
                    space
                        .reflection_sources
                        .get(id)
                        .map(|sources| !sources.iter().any(|source| tombstoned.contains(source)))
                        .unwrap_or(true)
                })
                .map(|reflection| {
                    let mut reflection = reflection.clone();
                    reflection["stale"] =
                        json!(Self::reflection_stale(space, &subject_id, &reflection));
                    reflection
                })
                .collect();
            reflections.sort_by(|left, right| {
                right["created_at"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(left["created_at"].as_str().unwrap_or_default())
                    .then_with(|| {
                        left["reflection_id"]
                            .as_str()
                            .unwrap_or_default()
                            .cmp(right["reflection_id"].as_str().unwrap_or_default())
                    })
            });
            reflections.truncate(limit);
            reply(json!({ "reflections": reflections }))
        })
    }

    async fn generate_reflection(
        &self,
        principal: &Principal,
        request: MemoryReflectionsGenerateRequest,
    ) -> SpokeResult<MemoryReflectionsGenerateResponse> {
        let request_value = wire(&request);
        if let SpokeResult::Reject(reject) = validate_memory_reflections_generate(&request) {
            return SpokeResult::Reject(reject);
        }
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let subject_id = text_field(&request_value, "subject_id");
        let key = text_field(&request_value, "idempotency_key");
        let expected_state_digest = text_field(&request_value, "expected_state_digest");
        let selection: ReadSelection =
            match serde_json::from_value(request_value["selection"].clone()) {
                Ok(selection) => selection,
                Err(error) => {
                    return reject(SpokeRejectCode::InvalidInput, format!("selection: {error}"))
                }
            };
        let filter = request_value.get("filter").cloned();
        let digest = tri!(request_digest(&request_value));
        let receipt = tri!(self.mutate_value(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space_mut(document, &space_id));
            if space.frozen {
                return reject(
                    SpokeRejectCode::SpaceFrozen,
                    format!("space {} is frozen", space.space_id),
                );
            }
            let requirement = GrantRequirement {
                capability: "reflect",
                subject_id: Some(&subject_id),
                context_ids: selection_contexts(&selection),
                reads_shared: selection_reads_shared(&selection),
                subject_all: selection_subject_all(&selection),
            };
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.reflections.generate"
            ));
            if Self::replay_conflict(
                space,
                &principal_id,
                "memory.reflections.generate",
                &key,
                &digest,
            ) {
                return reject(
                    SpokeRejectCode::IdempotencyConflict,
                    "idempotency key reused with a different payload",
                );
            }
            if let Some(receipt) = Self::replay(
                space,
                &principal_id,
                "memory.reflections.generate",
                &key,
                &digest,
            ) {
                return SpokeResult::Ok(receipt);
            }
            // Admission, fail-closed (§7.27).
            let missing = Self::missing_required_profiles(space, &subject_id);
            if !missing.is_empty() {
                return reject_details(
                    SpokeRejectCode::ProfileIncomplete,
                    format!("required profile kinds absent: {missing:?}"),
                    json!({ "missing": missing }),
                );
            }
            let (state_digest, sources, profiles) =
                Self::basis_for(space, &subject_id, &selection, filter.as_ref());
            if sources.is_empty() {
                return reject(
                    SpokeRejectCode::InsufficientData,
                    "no authorized source records for the pinned selection",
                );
            }
            if state_digest != expected_state_digest {
                return reject(
                    SpokeRejectCode::RevisionConflict,
                    "expected_state_digest does not match the current basis",
                );
            }
            // Deferred generation: the receipt is pending until the provider
            // worker completes it from the *pinned* basis (publish-pinned).
            let receipt_id = Self::next_receipt_id(space);
            let receipt = json!({
                "schema_version": 1,
                "receipt_id": receipt_id,
                "op": "memory.reflections.generate",
                "space_id": space_id,
                "requester_principal_id": principal_id,
                "idempotency_key": key.clone(),
                "payload_digest": digest.clone(),
                "status": "pending",
                "created_at": now.to_rfc3339_opts(SecondsFormat::Secs, true),
                "extensions": {},
            });
            space.pending.insert(
                receipt_id.clone(),
                PendingWork::Reflection {
                    subject_id: subject_id.clone(),
                    selection: selection.clone(),
                    sources,
                    profile_revisions: profiles,
                },
            );
            space.receipts.insert(receipt_id, receipt.clone());
            Self::remember(
                space,
                now,
                &principal_id,
                "memory.reflections.generate",
                &key,
                &digest,
                &receipt,
            );
            SpokeResult::Ok(receipt)
        }));
        reply(receipt)
    }

    async fn create_export(
        &self,
        principal: &Principal,
        request: MemoryExportsCreateRequest,
    ) -> SpokeResult<MemoryExportsCreateResponse> {
        let request_value = wire(&request);
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let subject_id = request_value["subject_id"].as_str().map(str::to_string);
        let selection: Option<ReadSelection> = request_value
            .get("selection")
            .and_then(|value| serde_json::from_value(value.clone()).ok());
        let include_reflections = request_value["include_reflections"]
            .as_bool()
            .unwrap_or(false);
        let include_tombstoned = request_value["include_tombstoned"]
            .as_bool()
            .unwrap_or(false);
        let require_quiescent = request_value["require_quiescent"]
            .as_bool()
            .unwrap_or(false);
        let key = text_field(&request_value, "idempotency_key");
        let digest = tri!(request_digest(&request_value));
        let receipt = tri!(self.mutate_value(|document| {
            let now = Self::clock(document);
            let clock_seconds = document.clock_offset_seconds;
            let space = tri!(Self::space_mut(document, &space_id));
            let requirement = GrantRequirement {
                capability: "transfer",
                subject_id: subject_id.as_deref(),
                context_ids: selection
                    .as_ref()
                    .map(selection_contexts)
                    .unwrap_or_default(),
                reads_shared: selection
                    .as_ref()
                    .map(selection_reads_shared)
                    .unwrap_or(subject_id.is_none()),
                subject_all: selection
                    .as_ref()
                    .map(selection_subject_all)
                    .unwrap_or(false),
            };
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.exports.create"
            ));
            if Self::replay_conflict(space, &principal_id, "memory.exports.create", &key, &digest) {
                return reject(
                    SpokeRejectCode::IdempotencyConflict,
                    "idempotency key reused with a different payload",
                );
            }
            if let Some(receipt) =
                Self::replay(space, &principal_id, "memory.exports.create", &key, &digest)
            {
                return SpokeResult::Ok(receipt);
            }
            // Drain barrier (§7.29): quiescent exports need a frozen, drained
            // space. Export itself stays on the freeze allowlist.
            if require_quiescent && (!space.frozen || space.pending_receipts() > 0) {
                return reject_details(
                    SpokeRejectCode::InvalidInput,
                    "require_quiescent needs a frozen space with no pending receipts",
                    json!({ "reason": "space_not_drained" }),
                );
            }
            let bundle = Self::export_bundle(
                space,
                subject_id.as_deref(),
                include_reflections,
                include_tombstoned,
            );
            let bundle_digest_value = match bundle_digest(&bundle) {
                Ok(digest) => digest,
                Err(error) => {
                    return reject(
                        SpokeRejectCode::InternalError,
                        format!("bundle digest: {error}"),
                    )
                }
            };
            let mut sealed = bundle.clone();
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
            let counts = Self::bundle_counts(&sealed);
            let export_id = sealed["export_id"].as_str().unwrap_or_default().to_string();
            space.exports.insert(
                export_id.clone(),
                ExportState {
                    export_id: export_id.clone(),
                    subject_id: subject_id.clone(),
                    chunks,
                    chunk_count,
                    total_bytes,
                    bundle_digest: bundle_digest_value.clone(),
                    counts: counts.clone(),
                    created_at_seconds: clock_seconds,
                },
            );
            space.touch();
            let receipt_id = Self::next_receipt_id(space);
            let receipt = json!({
                "schema_version": 1,
                "receipt_id": receipt_id,
                "op": "memory.exports.create",
                "space_id": space_id,
                "requester_principal_id": principal_id,
                "idempotency_key": key.clone(),
                "payload_digest": digest.clone(),
                "status": "pending",
                "created_at": now.to_rfc3339_opts(SecondsFormat::Secs, true),
                "extensions": {},
            });
            space.pending.insert(
                receipt_id.clone(),
                PendingWork::Receipt {
                    result: json!({
                        "export_id": export_id,
                        "chunk_count": chunk_count,
                        "total_bytes": total_bytes,
                        "bundle_digest": bundle_digest_value,
                        "counts": counts,
                    }),
                },
            );
            space.receipts.insert(receipt_id, receipt.clone());
            Self::remember(
                space,
                now,
                &principal_id,
                "memory.exports.create",
                &key,
                &digest,
                &receipt,
            );
            SpokeResult::Ok(receipt)
        }));
        reply(receipt)
    }
    async fn get_export(
        &self,
        principal: &Principal,
        request: MemoryExportsGetRequest,
    ) -> SpokeResult<MemoryExportsGetResponse> {
        let request_value = wire(&request);
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let export_id = text_field(&request_value, "export_id");
        let ordinal = number_field(&request_value, "chunk_ordinal") as usize;
        self.inspect(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space(document, &space_id));
            let Some(export) = space.exports.get(&export_id).cloned() else {
                return reject(SpokeRejectCode::NotFound, "export not found");
            };
            let requirement = GrantRequirement {
                capability: "transfer",
                subject_id: export.subject_id.as_deref(),
                context_ids: Vec::new(),
                reads_shared: true,
                subject_all: false,
            };
            tri!(Self::require(
                space,
                now,
                &principal_id,
                &requirement,
                "memory.exports.get"
            ));
            if document.clock_offset_seconds - export.created_at_seconds > EXPORT_RETENTION_SECONDS
            {
                return reject(
                    SpokeRejectCode::ExportExpired,
                    "export is past export_retention_seconds",
                );
            }
            if ordinal as u64 >= export.chunk_count {
                return reject(
                    SpokeRejectCode::InvalidInput,
                    format!("chunk_ordinal {ordinal} is outside the export"),
                );
            }
            reply(json!({ "chunk": export.chunks[ordinal] }))
        })
    }
    async fn stage_import(
        &self,
        principal: &Principal,
        request: MemoryImportsStageRequest,
    ) -> SpokeResult<MemoryImportsStageResponse> {
        let request_value = wire(&request);
        if let SpokeResult::Reject(reject) = validate_memory_imports_stage(&request) {
            return SpokeResult::Reject(reject);
        }
        let receipt = tri!(self.stage_import_value(principal, request_value));
        reply(receipt)
    }
    async fn activate_import(
        &self,
        principal: &Principal,
        request: MemoryImportsActivateRequest,
    ) -> SpokeResult<MemoryImportsActivateResponse> {
        let request_value = wire(&request);
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let import_id = text_field(&request_value, "import_id");
        let key = text_field(&request_value, "idempotency_key");
        let digest = tri!(request_digest(&request_value));
        let receipt = tri!(self.mutate_value(|document| {
            let now = Self::clock(document);
            let clock_seconds = document.clock_offset_seconds;
            let space = tri!(Self::space_mut(document, &space_id));
            for capability in ["transfer", "admin"] {
                let requirement = GrantRequirement {
                    capability,
                    subject_id: None,
                    context_ids: Vec::new(),
                    reads_shared: false,
                    subject_all: false,
                };
                tri!(Self::require(
                    space,
                    now,
                    &principal_id,
                    &requirement,
                    "memory.imports.activate"
                ));
            }
            if Self::replay_conflict(
                space,
                &principal_id,
                "memory.imports.activate",
                &key,
                &digest,
            ) {
                return reject(
                    SpokeRejectCode::IdempotencyConflict,
                    "idempotency key reused with a different payload",
                );
            }
            if let Some(receipt) = Self::replay(
                space,
                &principal_id,
                "memory.imports.activate",
                &key,
                &digest,
            ) {
                return SpokeResult::Ok(receipt);
            }
            let Some(import) = space.imports.get(&import_id).cloned() else {
                return reject(SpokeRejectCode::NotFound, "import not found");
            };
            if import.status != "staged" {
                return reject(
                    SpokeRejectCode::ImportConflict,
                    format!("import is {} and cannot be activated", import.status),
                );
            }
            if clock_seconds - import.staged_seconds > STAGED_IMPORT_TTL_SECONDS {
                return reject(
                    SpokeRejectCode::ImportConflict,
                    "staged import expired (staged_import_ttl_seconds)",
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
            // so target ids equal source ids and revisions start at 1.
            let mut subjects = 0usize;
            for subject in bundle["subjects"].as_array().cloned().unwrap_or_default() {
                let source = text_field(&subject, "subject_id");
                let target = rename(&source);
                let mut subject = subject;
                subject["space_id"] = json!(space_id);
                subject["subject_id"] = json!(target);
                subject["revision"] = json!(1);
                space.subjects.insert(target, subject);
                subjects += 1;
            }
            let mut records = 0usize;
            for record in bundle["records"].as_array().cloned().unwrap_or_default() {
                let mut record = record;
                let subject = rename(&text_field(&record, "subject_id"));
                record["space_id"] = json!(space_id);
                record["subject_id"] = json!(subject);
                record["revision"] = json!(1);
                space
                    .records
                    .insert(text_field(&record, "record_id"), record);
                records += 1;
            }
            let mut candidates = 0usize;
            for candidate in bundle["candidates"].as_array().cloned().unwrap_or_default() {
                let mut candidate = candidate;
                candidate["space_id"] = json!(space_id);
                let subject = rename(&text_field(&candidate, "subject_id"));
                candidate["subject_id"] = json!(subject);
                if candidate["status"] == "pending" {
                    candidate["revision"] = json!(1);
                }
                space
                    .candidates
                    .insert(text_field(&candidate, "candidate_id"), candidate);
                candidates += 1;
            }
            let mut profiles = 0usize;
            for profile in bundle["profiles"].as_array().cloned().unwrap_or_default() {
                let mut profile = profile;
                profile["space_id"] = json!(space_id);
                let subject = rename(&text_field(&profile, "subject_id"));
                profile["subject_id"] = json!(subject);
                profile["revision"] = json!(1);
                let key = Self::profile_key(&subject, &text_field(&profile, "profile_kind"));
                space.profiles.insert(key, profile);
                profiles += 1;
            }
            let mut reflections = 0usize;
            for reflection in bundle["reflections"]
                .as_array()
                .cloned()
                .unwrap_or_default()
            {
                let mut reflection = reflection;
                reflection["space_id"] = json!(space_id);
                let subject = rename(&text_field(&reflection, "subject_id"));
                reflection["subject_id"] = json!(subject);
                reflection["revision"] = json!(1);
                // §7.31: imported reflections are stale unconditionally. The
                // stored flag is not enough: read-time staleness recomputes
                // the basis, and a cooperative provider reproduces the
                // translated basis exactly — the marker keeps imported
                // reflections stale regardless.
                reflection["stale"] = json!(true);
                let reflection_id = text_field(&reflection, "reflection_id");
                space.imported_reflections.insert(reflection_id.clone());
                space.reflections.insert(reflection_id, reflection);
                reflections += 1;
            }
            // Explicit-only grant adoption, non-amplified against the stager.
            let granter_grants: Vec<MemoryGrant> = space
                .grants
                .values()
                .filter(|grant| grant["principal_id"] == principal_id.as_str())
                .filter_map(|grant| serde_json::from_value(grant.clone()).ok())
                .collect();
            let summaries: Vec<Value> = bundle["grants"].as_array().cloned().unwrap_or_default();
            let mut adopted = 0usize;
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
                let target_principal = text_field(requested, "principal_id");
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
                match validate_grant_issuance(
                    &granter_grants,
                    &principal_id,
                    &space_id,
                    now,
                    &issued,
                ) {
                    SpokeResult::Ok(()) => {}
                    SpokeResult::Reject(_) => {
                        return reject(
                            SpokeRejectCode::ImportConflict,
                            format!("adopting grant {grant_id} would amplify coverage"),
                        )
                    }
                }
                // Adoption keeps the source grant id: the staged bundle names
                // the grant (§7.31 adopts exactly that grant), and the target
                // must hold the grant under the same id.
                let mut grant = json!({
                    "schema_version": 1,
                    "grant_id": grant_id,
                    "space_id": space_id,
                    "principal_id": target_principal,
                    "capabilities": summary["capabilities"],
                    "created_at": CLOCK_ORIGIN,
                    "revision": 1,
                    "extensions": {},
                });
                for field in ["subject_id", "context_ids", "shared_access"] {
                    if let Some(value) = summary.get(field) {
                        grant[field] = value.clone();
                    }
                }
                space.grants.insert(text_field(&grant, "grant_id"), grant);
                adopted += 1;
            }
            let authority_generation = space.authority_generation + 1;
            space.authority_generation = authority_generation;
            if let Some(import) = space.imports.get_mut(&import_id) {
                import.status = "activated".to_string();
            }
            space.touch();
            let counts = import
                .report
                .as_ref()
                .map(|report| report["counts"].clone())
                .unwrap_or_else(|| json!({}));
            let receipt_id = Self::next_receipt_id(space);
            let receipt = json!({
                "schema_version": 1,
                "receipt_id": receipt_id,
                "op": "memory.imports.activate",
                "space_id": space_id,
                "requester_principal_id": principal_id,
                "idempotency_key": key.clone(),
                "payload_digest": digest.clone(),
                "status": "pending",
                "created_at": now.to_rfc3339_opts(SecondsFormat::Secs, true),
                "extensions": {},
            });
            space.pending.insert(
                receipt_id.clone(),
                PendingWork::Receipt {
                    // §7.32: the completion result is exactly
                    // {import_id, activated, authority_generation}; adoption
                    // totals live in the staged report.
                    result: json!({
                        "import_id": import_id,
                        "activated": counts,
                        "authority_generation": authority_generation,
                    }),
                },
            );
            space.receipts.insert(receipt_id, receipt.clone());
            Self::remember(
                space,
                now,
                &principal_id,
                "memory.imports.activate",
                &key,
                &digest,
                &receipt,
            );
            SpokeResult::Ok(receipt)
        }));
        reply(receipt)
    }
    async fn discard_import(
        &self,
        principal: &Principal,
        request: MemoryImportsDiscardRequest,
    ) -> SpokeResult<MemoryImportsDiscardResponse> {
        let request_value = wire(&request);
        let principal_id = identity(principal).to_string();
        let space_id = text_field(&request_value, "space_id");
        let import_id = text_field(&request_value, "import_id");
        let key = text_field(&request_value, "idempotency_key");
        let digest = tri!(request_digest(&request_value));
        let receipt = tri!(self.mutate_value(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space_mut(document, &space_id));
            for capability in ["transfer", "admin"] {
                let requirement = GrantRequirement {
                    capability,
                    subject_id: None,
                    context_ids: Vec::new(),
                    reads_shared: false,
                    subject_all: false,
                };
                tri!(Self::require(
                    space,
                    now,
                    &principal_id,
                    &requirement,
                    "memory.imports.discard"
                ));
            }
            if Self::replay_conflict(
                space,
                &principal_id,
                "memory.imports.discard",
                &key,
                &digest,
            ) {
                return reject(
                    SpokeRejectCode::IdempotencyConflict,
                    "idempotency key reused with a different payload",
                );
            }
            if let Some(receipt) = Self::replay(
                space,
                &principal_id,
                "memory.imports.discard",
                &key,
                &digest,
            ) {
                return SpokeResult::Ok(receipt);
            }
            let Some(import) = space.imports.get_mut(&import_id) else {
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
            space.touch();
            let mut receipt = Self::receipt(
                space,
                now,
                "memory.imports.discard",
                &principal_id,
                &digest,
                &key,
            );
            receipt["result"] = json!({ "import_id": import_id, "discarded": true });
            Self::remember(
                space,
                now,
                &principal_id,
                "memory.imports.discard",
                &key,
                &digest,
                &receipt,
            );
            SpokeResult::Ok(receipt)
        }));
        reply(receipt)
    }
}

impl SqliteMemoryProvider {
    /// `memory.spaces.freeze` / `unfreeze`: admin, CAS on the space revision.
    fn set_frozen<T: serde::de::DeserializeOwned>(
        &self,
        principal: &Principal,
        request_value: Value,
        frozen: bool,
    ) -> SpokeResult<T> {
        let principal_id = identity(principal).to_string();
        let space_id = request_value["space_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let key = request_value["idempotency_key"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let expected_revision = request_value["expected_revision"]
            .as_u64()
            .unwrap_or_default();
        let digest = tri!(request_digest(&request_value));
        let op = if frozen {
            "memory.spaces.freeze"
        } else {
            "memory.spaces.unfreeze"
        };
        let receipt = tri!(self.mutate_value(|document| {
            let now = Self::clock(document);
            let space = tri!(Self::space_mut(document, &space_id));
            let requirement = Self::admin_requirement();
            tri!(Self::require(space, now, &principal_id, &requirement, op));
            if Self::replay_conflict(space, &principal_id, op, &key, &digest) {
                return reject(
                    SpokeRejectCode::IdempotencyConflict,
                    "idempotency key reused with a different payload",
                );
            }
            if let Some(receipt) = Self::replay(space, &principal_id, op, &key, &digest) {
                return SpokeResult::Ok(receipt);
            }
            if space.revision != expected_revision {
                return reject(
                    SpokeRejectCode::RevisionConflict,
                    format!("space revision is {}", space.revision),
                );
            }
            space.frozen = frozen;
            space.revision += 1;
            space.touch();
            let mut receipt = Self::receipt(space, now, op, &principal_id, &digest, &key);
            receipt["result"] = json!({
                "status": if frozen { "frozen" } else { "active" },
                "pending_receipts": space.pending_receipts(),
                "revision": space.revision,
            });
            Self::remember(space, now, &principal_id, op, &key, &digest, &receipt);
            SpokeResult::Ok(receipt)
        }));
        reply(receipt)
    }
}
