//! SQLite-backed reference `MemoryPort` provider — storage + low-complexity ops
//! (P5 proof infrastructure, sub-round 3a).
//!
//! A **conformance proof artifact**, not a shipped product runtime. This round
//! lands provider-private storage and the first five op groups — `describe`,
//! `spaces.*`, `subjects.*`, `grants.*`, `receipts.get` — with the provider
//! enforcing its own duties: admission and grant coverage, CAS, idempotency with
//! dedup tombstones, freeze and fence fencing, and content-free receipts.
//! `spoke_memory` supplies policy only.
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

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use spoke_memory::classify::{classify_candidate, CandidateMaterial};
use spoke_memory::recall::{recall as engine_recall, RecallRecord, RecallRequest as EngineRecall};
use spoke_memory::ENGINE_POLICY_VERSION;
use spoke_operations::{
    jcs_sha256_hex, spoke_reject, validate_grant_issuance, validate_memory_candidates_list,
    validate_memory_candidates_review, validate_memory_capture, validate_memory_grants_grant,
    validate_memory_grants_list, validate_memory_recall, validate_memory_records_create,
    validate_memory_records_list, validate_memory_records_update, validate_memory_spaces_list,
    validate_memory_subjects_fence, GrantRequirement, IssuedGrant, MemoryPort, SpokeRejectCode,
    SpokeResult,
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
    receipts: BTreeMap<String, Value>,
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

    fn next_receipt_id(space: &Space) -> String {
        format!("rcpt_sqlite_{:04}", space.receipts.len() + 1)
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

    fn receipt(
        space: &Space,
        op: &str,
        principal_id: &str,
        digest: &str,
        idempotency_key: &str,
    ) -> Value {
        let sequence = space.receipts.len() + 1;
        let mut receipt = json!({
            "schema_version": 1,
            "receipt_id": format!("rcpt_sqlite_{sequence:04}"),
            "op": op,
            "space_id": space.space_id,
            "requester_principal_id": principal_id,
            "payload_digest": digest,
            "status": "committed",
            "created_at": CLOCK_ORIGIN,
            "completed_at": CLOCK_ORIGIN,
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

    fn remember(
        space: &mut Space,
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
                created_at: CLOCK_ORIGIN.to_string(),
            },
        );
        space.receipts.insert(receipt_id, receipt.clone());
    }

    /// Opaque cursor token pinned to the space's state epoch (MEM-R026).
    fn cursor_token(space: &Space, offset: usize) -> String {
        format!("c{}:{}", space.state_epoch, offset)
    }

    /// Cursor offset, or `None` when the epoch moved (invalidated cursor).
    fn cursor_offset(space: &Space, cursor: &str) -> Option<usize> {
        let (epoch, offset) = cursor.split_once(':')?;
        let epoch: u64 = epoch.strip_prefix('c')?.parse().ok()?;
        if epoch != space.state_epoch {
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
        let document = store.read().expect("read state");
        answer(&document)
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
                Self::receipt(space, "memory.subjects.fence", &principal_id, &digest, &key);
            receipt["result"] = json!({
                "subject_id": subject_id,
                "status": subject["status"],
                "fenced_contexts": subject["fenced_contexts"],
                "revision": subject["revision"],
            });
            Self::remember(
                space,
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
                Self::receipt(space, "memory.grants.grant", &principal_id, &digest, &key);
            receipt["result"] = json!({ "grant_id": grant_id, "revision": 1 });
            Self::remember(
                space,
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
                Self::receipt(space, "memory.grants.revoke", &principal_id, &digest, &key);
            receipt["result"] = json!({
                "grant_id": grant_id,
                "revoked": true,
                "revision": revision + 1,
            });
            Self::remember(
                space,
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
            let mut receipt = Self::receipt(space, "memory.capture", &principal_id, &digest, &key);
            receipt["result"] = json!({
                "candidate_ids": candidate_ids,
                "classified": classified,
            });
            Self::remember(
                space,
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
            let receipt_id = Self::next_receipt_id(space);
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
            let mut receipt = Self::receipt(
                space,
                "memory.candidates.dismiss",
                &principal_id,
                &digest,
                &key,
            );
            receipt["result"] = json!({ "candidate_id": candidate_id, "outcome": "dismissed" });
            Self::remember(
                space,
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
            let receipt_id = Self::next_receipt_id(space);
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
            let mut receipt = Self::receipt(
                space,
                "memory.candidates.review",
                &principal_id,
                &digest,
                &key,
            );
            receipt["result"] = json!({ "outcome": outcome, "record_ids": record_ids });
            Self::remember(
                space,
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
                Self::receipt(space, "memory.records.create", &principal_id, &digest, &key);
            receipt["result"] = json!({ "record_id": record_id, "revision": 1 });
            Self::remember(
                space,
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
                Self::receipt(space, "memory.records.update", &principal_id, &digest, &key);
            receipt["result"] = json!({ "record_id": record_id, "revision": revision + 1 });
            Self::remember(
                space,
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
                Self::receipt(space, "memory.records.delete", &principal_id, &digest, &key);
            receipt["result"] = json!({ "record_id": record_id, "tombstoned": true });
            Self::remember(
                space,
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
                Some(cursor) => match Self::cursor_offset(space, cursor) {
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
            let page = engine_recall(
                &engine_inputs,
                &EngineRecall {
                    query: query.as_deref(),
                    max_items,
                    max_bytes,
                },
            );
            let total_authorized = page.total_authorized;
            let skipped = offset.min(total_authorized);
            let mut records: Vec<Value> = Vec::new();
            for ranked in page.records.iter().skip(skipped) {
                if let Some(record) = authorized
                    .iter()
                    .find(|record| record["record_id"].as_str() == Some(ranked.record_id.as_str()))
                {
                    records.push(record.clone());
                }
            }
            let returned = records.len();
            let mut response = json!({
                "records": records,
                "total_authorized": total_authorized,
                "truncated": page.truncated || skipped + returned < total_authorized,
                "engine_policy_version": ENGINE_POLICY_VERSION,
            });
            if let Some(overflow) = page.overflow_record_id {
                response["overflow_record_id"] = json!(overflow);
            }
            if skipped + returned < total_authorized {
                response["next_cursor"] = json!(Self::cursor_token(space, skipped + returned));
            }
            reply(response)
        })
    }

    async fn get_profile(
        &self,
        _principal: &Principal,
        _request: MemoryProfileGetRequest,
    ) -> SpokeResult<MemoryProfileGetResponse> {
        not_served("memory.profile.get")
    }

    async fn update_profile(
        &self,
        _principal: &Principal,
        _request: MemoryProfileUpdateRequest,
    ) -> SpokeResult<MemoryProfileUpdateResponse> {
        not_served("memory.profile.update")
    }

    async fn get_reflection_state(
        &self,
        _principal: &Principal,
        _request: MemoryReflectionsStateRequest,
    ) -> SpokeResult<MemoryReflectionsStateResponse> {
        not_served("memory.reflections.state")
    }

    async fn list_reflections(
        &self,
        _principal: &Principal,
        _request: MemoryReflectionsListRequest,
    ) -> SpokeResult<MemoryReflectionsListResponse> {
        not_served("memory.reflections.list")
    }

    async fn generate_reflection(
        &self,
        _principal: &Principal,
        _request: MemoryReflectionsGenerateRequest,
    ) -> SpokeResult<MemoryReflectionsGenerateResponse> {
        not_served("memory.reflections.generate")
    }

    async fn create_export(
        &self,
        _principal: &Principal,
        _request: MemoryExportsCreateRequest,
    ) -> SpokeResult<MemoryExportsCreateResponse> {
        not_served("memory.exports.create")
    }

    async fn get_export(
        &self,
        _principal: &Principal,
        _request: MemoryExportsGetRequest,
    ) -> SpokeResult<MemoryExportsGetResponse> {
        not_served("memory.exports.get")
    }

    async fn stage_import(
        &self,
        _principal: &Principal,
        _request: MemoryImportsStageRequest,
    ) -> SpokeResult<MemoryImportsStageResponse> {
        not_served("memory.imports.stage")
    }

    async fn activate_import(
        &self,
        _principal: &Principal,
        _request: MemoryImportsActivateRequest,
    ) -> SpokeResult<MemoryImportsActivateResponse> {
        not_served("memory.imports.activate")
    }

    async fn discard_import(
        &self,
        _principal: &Principal,
        _request: MemoryImportsDiscardRequest,
    ) -> SpokeResult<MemoryImportsDiscardResponse> {
        not_served("memory.imports.discard")
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
            let mut receipt = Self::receipt(space, op, &principal_id, &digest, &key);
            receipt["result"] = json!({
                "status": if frozen { "frozen" } else { "active" },
                "pending_receipts": space.pending_receipts(),
                "revision": space.revision,
            });
            Self::remember(space, &principal_id, op, &key, &digest, &receipt);
            SpokeResult::Ok(receipt)
        }));
        reply(receipt)
    }
}
