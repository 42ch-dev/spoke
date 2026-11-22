//! Responder ports face — the D4 catalogue seam `connect_responder` serves
//! `port.*` ops through.
//!
//! [`RemoteServePorts`] probes each family it can serve instead of extending
//! one provider trait: the distributed baseline six through
//! [`RemoteServePorts::as_baseline`], the three knowledge ops through
//! [`RemoteServePorts::as_knowledge`] (a [`RemoteKnowledgeFace`]: scoped
//! store + per-peer admission), the optional l2-computable / l5-fork faces
//! through [`RemoteServePorts::as_computable`] /
//! [`RemoteServePorts::as_fork_timeline`] — the operations-crate
//! dyn-availability probe convention (`ComputablePorts` / `ForkPorts`). The
//! core `extract` op is served through the separate optional
//! [`RemoteExtractService`] face, probed with
//! [`RemoteServePorts::as_extract`]; it is a service, not a port method. The
//! memory family is served the same way through the optional
//! [`RemoteMemoryService`] face ([`RemoteServePorts::as_memory`]); the
//! `spoke-history` family is served the same way through the optional
//! [`RemoteHistoryService`] face ([`RemoteServePorts::as_history`]).
//! Serving order is gate → probe → serve/deny: after the capability gate
//! passes, the responder probes the injected face; a `None` probe means the
//! host declared a family it does not provide (host misconfiguration) and
//! answers the existing `op_unsupported` dispatch-deny branch.
//!
//! Composition: full providers (`BaselinePorts + ComputablePort +
//! ForkTimelineQueryPort`) are covered by the blanket impl; baseline-only
//! and mixed hosts compose via [`RemoteServePortsComposite::new`] without
//! manual trait impls, opting into extraction with
//! [`RemoteServePortsComposite::with_extract`]; a host that serves only the
//! knowledge ops composes with [`RemoteServePortsComposite::knowledge_only`].

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Map, Value};
use spoke_operations::{
    jcs_sha256_hex, spoke_ok, spoke_reject, BaselinePorts, ComputablePort,
    ForkTimelineQueryPort, HistoryPort, HistoryPruneResult, MemoryPort, ScopedKnowledgePort,
    SpokeReject, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::data::revision_snapshot::RevisionSnapshotSnapshot;
use spoke_schemas::{
    ExtractRequest, ExtractResponse, HistoryRevisionsGetRequest,
    HistoryRevisionsPruneRequest, Principal, RevisionSnapshot, Scope,
};

/// Connect-owned service face for the core `extract` op: the serving host
/// runs its own extraction — local loader, local extractor, local
/// provisional-candidate validation — and returns the wire
/// [`ExtractResponse`]. The loaded input value is host-local and never a
/// transport parameter.
///
/// This is deliberately **not** `spoke_operations::ExtractionPort`: the port
/// is an in-process loader boundary, while this trait is the connect
/// responder's service seam. A host implements it by calling
/// `orchestrate_extract(local_extraction_port, request, run_extractor)`.
#[async_trait]
pub trait RemoteExtractService: Send + Sync {
    /// Serve one extract request. A rejection (or an `ExtractResponse` error
    /// branch) travels the existing application reject mapping.
    async fn extract(&self, request: ExtractRequest) -> SpokeResult<ExtractResponse>;
}

/// Connect-owned service face for the memory family (`memory.*`, protocol
/// §13): the serving host owns the provider — the `spoke_operations::MemoryPort`
/// implementation — the admission context that turns the authenticated peer
/// into a `Principal` (MEM-R003: never payload-asserted), and the mapping of
/// each op onto its typed port call.
///
/// One callable serves every `memory.*` op instead of a second 33-method
/// trait: `MemoryPort` already owns the typed per-op contract. `payload` is
/// the op's request object and the returned value is its response object
/// (same no-wrapper rule as `extract`).
#[async_trait]
pub trait RemoteMemoryService: Send + Sync {
    /// Serve one `memory.*` op for the authenticated `peer_id`.
    async fn invoke(
        &self,
        peer_id: &str,
        op: &str,
        payload: serde_json::Value,
    ) -> SpokeResult<serde_json::Value>;
}

/// Admission context: maps the authenticated connect peer to the memory
/// `Principal` the provider admits. The principal never comes from the
/// payload (MEM-R003).
pub type MemoryAdmission = Arc<dyn Fn(&str) -> SpokeResult<Principal> + Send + Sync>;

/// Admission context: maps the authenticated connect peer to the `Scope` it
/// may read and write. The scope never comes from the payload.
///
/// The responder calls it inside the local serve-wait budget, on the blocking
/// pool: a host resolver MUST return within that budget on its own — the
/// blocking thread cannot be cancelled, so the responder's bound (not a
/// forced stop) is what the caller is guaranteed.
pub type KnowledgeAdmission = Arc<dyn Fn(&str) -> SpokeResult<Scope> + Send + Sync>;

/// Knowledge serving face: a scoped store plus per-peer admission. The three
/// knowledge ops are served only inside the admitted scope; `get`/`put`
/// carry no wire scope, and the list op's wire scope must match the
/// admission exactly.
pub struct RemoteKnowledgeFace {
    pub store: Arc<dyn ScopedKnowledgePort + Send + Sync>,
    pub admission: KnowledgeAdmission,
}

/// [`RemoteMemoryService`] over a provider [`MemoryPort`]: the 33 `memory.*`
/// op names map onto their typed port methods, so a host attaches an existing
/// provider with
/// `RemoteServePortsComposite::new(..).with_memory(Arc::new(MemoryPortService::new(port, admission)))`
/// instead of hand-writing the wire mapping per host.
pub struct MemoryPortService {
    port: Arc<dyn MemoryPort + Send + Sync>,
    admission: MemoryAdmission,
}

impl MemoryPortService {
    #[must_use]
    pub fn new(port: Arc<dyn MemoryPort + Send + Sync>, admission: MemoryAdmission) -> Self {
        Self { port, admission }
    }
}

#[async_trait]
impl RemoteMemoryService for MemoryPortService {
    async fn invoke(
        &self,
        peer_id: &str,
        op: &str,
        payload: serde_json::Value,
    ) -> SpokeResult<serde_json::Value> {
        let principal = match (self.admission)(peer_id) {
            SpokeResult::Ok(principal) => principal,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        let port = self.port.as_ref();
        match op {
            "memory.describe" => serve(payload, |request| port.describe(&principal, request)).await,
            "memory.spaces.list" => {
                serve(payload, |request| port.list_spaces(&principal, request)).await
            }
            "memory.spaces.freeze" => {
                serve(payload, |request| port.freeze_space(&principal, request)).await
            }
            "memory.spaces.unfreeze" => {
                serve(payload, |request| port.unfreeze_space(&principal, request)).await
            }
            "memory.subjects.ensure" => {
                serve(payload, |request| port.ensure_subject(&principal, request)).await
            }
            "memory.subjects.get" => {
                serve(payload, |request| port.get_subject(&principal, request)).await
            }
            "memory.subjects.fence" => {
                serve(payload, |request| port.fence_subject(&principal, request)).await
            }
            "memory.grants.list" => {
                serve(payload, |request| port.list_grants(&principal, request)).await
            }
            "memory.grants.grant" => serve(payload, |request| port.grant(&principal, request)).await,
            "memory.grants.revoke" => {
                serve(payload, |request| port.revoke_grant(&principal, request)).await
            }
            "memory.capture" => serve(payload, |request| port.capture(&principal, request)).await,
            "memory.candidates.list" => {
                serve(payload, |request| port.list_candidates(&principal, request)).await
            }
            "memory.candidates.get" => {
                serve(payload, |request| port.get_candidate(&principal, request)).await
            }
            "memory.candidates.dismiss" => {
                serve(payload, |request| port.dismiss_candidate(&principal, request)).await
            }
            "memory.candidates.review" => {
                serve(payload, |request| port.review_candidate(&principal, request)).await
            }
            "memory.records.create" => {
                serve(payload, |request| port.create_record(&principal, request)).await
            }
            "memory.records.get" => {
                serve(payload, |request| port.get_record(&principal, request)).await
            }
            "memory.records.update" => {
                serve(payload, |request| port.update_record(&principal, request)).await
            }
            "memory.records.delete" => {
                serve(payload, |request| port.delete_record(&principal, request)).await
            }
            "memory.records.list" => {
                serve(payload, |request| port.list_records(&principal, request)).await
            }
            "memory.records.promote" => {
                serve(payload, |request| port.promote_record(&principal, request)).await
            }
            "memory.recall" => serve(payload, |request| port.recall(&principal, request)).await,
            "memory.profile.get" => {
                serve(payload, |request| port.get_profile(&principal, request)).await
            }
            "memory.profile.update" => {
                serve(payload, |request| port.update_profile(&principal, request)).await
            }
            "memory.reflections.state" => {
                serve(payload, |request| port.get_reflection_state(&principal, request)).await
            }
            "memory.reflections.list" => {
                serve(payload, |request| port.list_reflections(&principal, request)).await
            }
            "memory.reflections.generate" => {
                serve(payload, |request| port.generate_reflection(&principal, request)).await
            }
            "memory.receipts.get" => {
                serve(payload, |request| port.get_receipt(&principal, request)).await
            }
            "memory.exports.create" => {
                serve(payload, |request| port.create_export(&principal, request)).await
            }
            "memory.exports.get" => {
                serve(payload, |request| port.get_export(&principal, request)).await
            }
            "memory.imports.stage" => {
                serve(payload, |request| port.stage_import(&principal, request)).await
            }
            "memory.imports.activate" => {
                serve(payload, |request| port.activate_import(&principal, request)).await
            }
            "memory.imports.discard" => {
                serve(payload, |request| port.discard_import(&principal, request)).await
            }
            other => spoke_reject(
                SpokeRejectCode::InternalError,
                format!("{other} is not a memory op"),
                None,
            ),
        }
    }
}

// ── `spoke-history` serving face (spoke-history contract §Connect
//    operations / §9.1) ─────────────────────────────────────────────────
//
// `RemoteHistoryService` is the invoke seam the responder calls for both
// `history.*` ops; `HistoryPortService` adapts a provider `HistoryPort` to
// it, mirroring `RemoteMemoryService` / `MemoryPortService`. The service
// owns the three concerns the contract keeps OUT of `HistoryPort`: peer
// admission (`HistoryAdmission` maps the authenticated `peer_id` to the
// `Principal` and admitted `Scope`), effective-scope resolution (a present
// request `scope` MUST equal the admission; absent uses the admission),
// and prune deduplication (the recorded-outcome model keyed
// `(effective_scope, requester_principal_id, op, idempotency_key)`).

/// Admission context: maps the authenticated connect peer to the history
/// `Principal` and the `Scope` the provider admits. Neither comes from the
/// payload (MEM-R003; spoke-history contract §Authorization). The responder
/// calls it inside the local serve-wait budget: a host resolver MUST return
/// within that budget on its own (same bound as [`KnowledgeAdmission`]).
pub type HistoryAdmission =
    Arc<dyn Fn(&str) -> SpokeResult<(Principal, Scope)> + Send + Sync>;

/// `history.revisions.get` wire op.
pub const HISTORY_OP_GET: &str = "history.revisions.get";
/// `history.revisions.prune` wire op.
pub const HISTORY_OP_PRUNE: &str = "history.revisions.prune";

/// Connect-owned service face for the `spoke-history` family (`history.*`,
/// spoke-history contract §Connect operations): the serving host owns the
/// provider — the `spoke_operations::HistoryPort` implementation — the
/// admission context that turns the authenticated peer into a `Principal`
/// + admitted `Scope` (never payload-asserted), the mapping of each op
/// onto its typed port call, and the prune dedup store.
///
/// One callable serves both ops (same no-wrapper rule as `extract` /
/// `memory.*`): `payload` is the op's request object and the returned
/// value is its response object — `{"snapshot": ...}` for get,
/// `{deleted, floor_revision}` for prune.
#[async_trait]
pub trait RemoteHistoryService: Send + Sync {
    /// Serve one `history.*` op for the authenticated `peer_id`.
    async fn invoke(
        &self,
        peer_id: &str,
        op: &str,
        payload: serde_json::Value,
    ) -> SpokeResult<serde_json::Value>;
}

/// One dedup lookup: the recorded `{payload_digest, outcome}` for the key.
/// The `SpokeResult` is stored, not re-wrapped, so a replayed outcome
/// returns exactly what the original call settled to — a recorded reject
/// replays as that reject, not as a success carrying its fields.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryDedupRecord {
    /// SHA-256 hex over RFC 8785 canonical JSON of the request payload
    /// object (the whole request, idempotency key included).
    pub payload_digest: String,
    /// The outcome the admitted call settled to (response object or
    /// reject), replayed verbatim on same-digest replays.
    pub outcome: SpokeResult<Value>,
}

/// Result of a dedup [`HistoryDedupStore::lookup`]: `Unseen` executes,
/// `Evicted` answers `RECEIPT_EXPIRED` (a discarded recorded outcome must
/// NEVER silently re-execute), `Recorded` compares digests.
#[derive(Debug, Clone, PartialEq)]
pub enum HistoryDedupLookup {
    /// No entry — first sighting of the key.
    Unseen,
    /// The key's recorded outcome was evicted; never re-execute.
    Evicted,
    /// A live recorded outcome exists.
    Recorded(HistoryDedupRecord),
}

/// The prune dedup store (spoke-history contract §9.1), keyed
/// `(effective_scope, requester_principal_id, op, idempotency_key)` —
/// serialized `{scopeDigest}|{principalId}|{op}|{key}` — and recording
/// `{payload_digest, outcome}`. Host-injectable for durable persistence:
/// entries MUST outlive a Connect session and the principal-bound
/// namespace MUST stay stable across reconnects, which the in-memory
/// default cannot promise — durable `HistoryDedupStore` implementations
/// own that guarantee.
#[async_trait]
pub trait HistoryDedupStore: Send + Sync {
    /// Resolve one dedup key (see [`HistoryDedupLookup`]).
    async fn lookup(&self, key: &str) -> HistoryDedupLookup;

    /// Persist the digest and settled outcome for one executed key.
    async fn record(&self, key: &str, record: HistoryDedupRecord);
}

/// Bounded in-memory dedup store — the default when the host injects
/// nothing (spoke-history contract §9.1: "the default in-memory store is
/// bounded by an eviction policy the host MAY configure").
///
/// Eviction keeps a tombstone so a replayed evicted key answers
/// `RECEIPT_EXPIRED` instead of re-executing; tombstones are bounded by
/// the same capacity.
///
/// shortcut: a tombstone older than `max_entries` later keys is itself
/// forgotten and the key then replays as `Unseen`; a durable host store
/// never ages tombstones.
pub struct InMemoryHistoryDedupStore {
    max_entries: usize,
    state: Mutex<InMemoryDedupState>,
}

#[derive(Default)]
struct InMemoryDedupState {
    /// Live records, FIFO by insertion order.
    records: HashMap<String, HistoryDedupRecord>,
    order: VecDeque<String>,
    /// Evicted keys, bounded FIFO by eviction order.
    tombstones: HashSet<String>,
    tombstone_order: VecDeque<String>,
}

impl InMemoryHistoryDedupStore {
    /// Default bound: 1024 recorded outcomes.
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(1024)
    }

    /// Bounded store retaining at most `max_entries` records;
    /// capacity 0 evicts every record immediately — every replay answers
    /// `RECEIPT_EXPIRED`.
    #[must_use]
    pub fn with_capacity(max_entries: usize) -> Self {
        Self {
            max_entries,
            state: Mutex::new(InMemoryDedupState::default()),
        }
    }
}

impl Default for InMemoryHistoryDedupStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HistoryDedupStore for InMemoryHistoryDedupStore {
    async fn lookup(&self, key: &str) -> HistoryDedupLookup {
        let state = self.state.lock().expect("dedup lock");
        if let Some(record) = state.records.get(key) {
            return HistoryDedupLookup::Recorded(record.clone());
        }
        if state.tombstones.contains(key) {
            return HistoryDedupLookup::Evicted;
        }
        HistoryDedupLookup::Unseen
    }

    async fn record(&self, key: &str, record: HistoryDedupRecord) {
        let mut state = self.state.lock().expect("dedup lock");
        state.tombstones.remove(key);
        if !state.records.contains_key(key) {
            state.order.push_back(key.to_owned());
        }
        state.records.insert(key.to_owned(), record);
        while state.records.len() > self.max_entries {
            let Some(oldest) = state.order.pop_front() else {
                break;
            };
            state.records.remove(&oldest);
            if state.tombstones.insert(oldest.clone()) {
                state.tombstone_order.push_back(oldest);
            }
        }
        while state.tombstones.len() > self.max_entries {
            let Some(oldest) = state.tombstone_order.pop_front() else {
                break;
            };
            state.tombstones.remove(&oldest);
        }
    }
}

/// One decoded `history.*` call: the validated arguments produced at the
/// service boundary — a malformed payload answers `INVALID_INPUT` before
/// admission, dedup, and any port call. The prune op keeps its
/// `idempotency_key` (a connect-layer dedup concern that never reaches
/// the port).
enum HistoryCall {
    /// `history.revisions.get` — `(scope, entity_kind, entity_id, revision)`.
    Get {
        scope: Option<Scope>,
        entity_kind: String,
        entity_id: String,
        revision: u64,
    },
    /// `history.revisions.prune` — `(scope, entity_kind, entity_id, keep,
    /// idempotency_key)`.
    Prune {
        scope: Option<Scope>,
        entity_kind: String,
        entity_id: String,
        keep: u64,
        idempotency_key: String,
    },
}

/// Decode the request's optional `scope` member as the shared `Scope`:
/// absent is `None` (the admission supplies the effective scope), a
/// present malformed member is `INVALID_INPUT`. Takes the owned member
/// value — the caller removes it from the payload before the typed
/// request decode, so the subtree moves instead of cloning.
fn decode_history_scope(op: &str, scope: Option<Value>) -> SpokeResult<Option<Scope>> {
    let Some(scope) = scope else {
        return spoke_ok(None);
    };
    match serde_json::from_value(scope) {
        Ok(scope) => spoke_ok(Some(scope)),
        Err(error) => spoke_reject(
            SpokeRejectCode::InvalidInput,
            format!("invalid {op} payload: {error}"),
            None,
        ),
    }
}

/// Decode and validate one `history.*` op payload. The typed request
/// decode is the trust boundary — a malformed or unknown-member payload
/// answers `INVALID_INPUT` before admission, dedup, and any port call
/// (same rule as `extract` and the memory service boundary). `revision`
/// and `keep` decode as `u64` — the receiving runtime never rounds them
/// (spoke-history contract §`HistoryPort`); an over-range or non-integer
/// value fails the decode with `INVALID_INPUT`.
///
/// Consumes the payload: the optional `scope` member moves out first
/// (the request types carry a scope-shaped but distinct member, so the
/// shared `Scope` decodes once from the owned subtree), then the typed
/// request decodes from the remaining object and its selector strings
/// move into the call — no full-payload or selector clones.
fn decode_history_call(
    op: &str,
    payload: Value,
    scope: Option<Value>,
) -> SpokeResult<HistoryCall> {
    let scope = match decode_history_scope(op, scope) {
        SpokeResult::Ok(scope) => scope,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };
    match op {
        HISTORY_OP_GET => {
            match serde_json::from_value::<HistoryRevisionsGetRequest>(payload) {
                Ok(request) => spoke_ok(HistoryCall::Get {
                    scope,
                    entity_kind: request.entity_kind.into(),
                    entity_id: request.entity_id.into(),
                    revision: request.revision,
                }),
                Err(error) => spoke_reject(
                    SpokeRejectCode::InvalidInput,
                    format!("invalid {op} payload: {error}"),
                    None,
                ),
            }
        }
        HISTORY_OP_PRUNE => {
            match serde_json::from_value::<HistoryRevisionsPruneRequest>(payload) {
                Ok(request) => spoke_ok(HistoryCall::Prune {
                    scope,
                    entity_kind: request.entity_kind.into(),
                    entity_id: request.entity_id.into(),
                    keep: request.keep,
                    idempotency_key: request.idempotency_key.into(),
                }),
                Err(error) => spoke_reject(
                    SpokeRejectCode::InvalidInput,
                    format!("invalid {op} payload: {error}"),
                    None,
                ),
            }
        }
        _ => SpokeResult::Reject(SpokeReject {
            code: SpokeRejectCode::CapabilityPortMissing,
            message: format!("unimplemented history op {op}"),
            details: None,
        }),
    }
}

/// Canonical-byte scope equality: a present request `scope` MUST equal
/// the admitted scope exactly (spoke-history contract §Connect
/// operations). Equality is membership-independent — canonical JSON —
/// because the contract admits no subset semantics for history.
fn scope_wire_equal(requested: &Scope, admitted: &Scope) -> bool {
    match (serde_jcs::to_vec(requested), serde_jcs::to_vec(admitted)) {
        (Ok(left), Ok(right)) => left == right,
        // A scope that cannot canonicalize cannot prove equality: refuse.
        _ => false,
    }
}

/// Largest integer that survives `serde_jcs`'s RFC 8785 number
/// formatting: the crate encodes integers through `f64`, so IEEE-754
/// exact integers (|n| ≤ 2^53 − 1) are the only members whose digest
/// identity is preserved.
const JCS_EXACT_INTEGER_MAX: u64 = 9_007_199_254_740_991;

/// Every integer member of `value` is inside the JCS exact-integer
/// range. The prune dedup digest is SHA-256 over the RFC 8785 encoding
/// of the whole request payload; an integer beyond 2^53 − 1 would
/// collapse to the same canonical number as a distinct neighbor, so a
/// different request could replay a recorded outcome under its own key.
/// The serving boundary refuses such payloads with `INVALID_INPUT`
/// instead of rounding (spoke-history contract §`HistoryPort`: the
/// runtime never rounds an unrepresentable integer).
fn payload_integers_jcs_exact(value: &Value) -> bool {
    match value {
        Value::Number(number) => {
            if number.is_u64() || number.is_i64() {
                let magnitude = number
                    .as_u64()
                    .or_else(|| number.as_i64().map(i64::unsigned_abs))
                    .expect("integer number");
                magnitude <= JCS_EXACT_INTEGER_MAX
            } else {
                // A non-integral f64 member carries no integer identity
                // to lose in canonicalization.
                true
            }
        }
        Value::Array(items) => items.iter().all(payload_integers_jcs_exact),
        Value::Object(members) => members.values().all(payload_integers_jcs_exact),
        _ => true,
    }
}

/// Transport-decode reject for a malformed provider success value —
/// `INTERNAL_ERROR` with `details.kind = "transport"`, never `Ok`
/// (spoke-history contract §Connect operations).
fn history_transport_reject<T>(detail: impl Into<String>) -> SpokeResult<T> {
    let mut details = Map::new();
    details.insert("kind".into(), Value::String("transport".into()));
    spoke_reject(
        SpokeRejectCode::InternalError,
        detail.into(),
        Some(details),
    )
}

/// Shared `history.revisions.get` snapshot validation (contract §Connect
/// operations — one rule for the serving face and each `RemoteAdapter`):
/// the `RevisionSnapshot` must target the request selectors — envelope
/// `entity_kind` / `entity_id` / `revision` equal — and satisfy the
/// §`RevisionSnapshot` equality rules: the nested entity's own id and
/// revision agree with the envelope, and the `snapshot` variant agrees
/// with `entity_kind` for the core kinds. A malformed or wrong-target
/// success is a transport decode error, never `Ok`.
pub(crate) fn validate_history_snapshot(
    snapshot: &RevisionSnapshot,
    entity_kind: &str,
    entity_id: &str,
    revision: u64,
) -> SpokeResult<()> {
    if *snapshot.entity_kind != entity_kind
        || *snapshot.entity_id != entity_id
        || snapshot.revision != revision
    {
        return history_transport_reject(
            "provider returned a wrong-target RevisionSnapshot: envelope selectors do not match the request",
        );
    }
    // Core kinds bind the snapshot variant; an additional product kind
    // declares its core shape and encodes under it consistently, so only
    // the two core kinds get the variant check.
    let variant_ok = match (entity_kind, &snapshot.snapshot) {
        ("knowledge_entry", RevisionSnapshotSnapshot::KnowledgeEntry { .. })
        | ("relation", RevisionSnapshotSnapshot::Relation { .. }) => true,
        ("knowledge_entry" | "relation", _) => false,
        _ => true,
    };
    if !variant_ok {
        return history_transport_reject(
            "provider returned a malformed RevisionSnapshot: snapshot variant does not agree with entity_kind",
        );
    }
    let (nested_id, nested_revision) = match &snapshot.snapshot {
        RevisionSnapshotSnapshot::KnowledgeEntry {
            entry_id,
            revision,
            ..
        } => (entry_id.as_str(), *revision),
        RevisionSnapshotSnapshot::Relation {
            relation_id,
            revision,
            ..
        } => (relation_id.as_str(), *revision),
    };
    if nested_id != entity_id {
        return history_transport_reject(
            "provider returned a malformed RevisionSnapshot: nested entity id does not equal the envelope entity_id",
        );
    }
    // The nested entity's own revision is REQUIRED and MUST equal the
    // envelope revision (contract §`RevisionSnapshot`: "The selected
    // entity snapshot's own `revision` member is required and MUST equal
    // this envelope value") — an absent member is malformed, never
    // skipped.
    match nested_revision {
        Some(nested_revision) if nested_revision == revision => spoke_ok(()),
        _ => history_transport_reject(
            "provider returned a malformed RevisionSnapshot: nested entity revision must be present and equal the envelope revision",
        ),
    }
}

/// Serving-side get validation: `validate_history_snapshot` on the
/// provider's typed snapshot, then serialize it exactly once into the
/// `{ snapshot }` get-response member — no second Value → typed → Value
/// round-trip.
fn valid_served_snapshot(
    snapshot: RevisionSnapshot,
    entity_kind: &str,
    entity_id: &str,
    revision: u64,
) -> SpokeResult<Value> {
    if let SpokeResult::Reject(reject) =
        validate_history_snapshot(&snapshot, entity_kind, entity_id, revision)
    {
        return SpokeResult::Reject(reject);
    }
    match serde_json::to_value(snapshot) {
        Ok(snapshot) => {
            // Insert the serialized snapshot into the response member
            // directly — `json!({ "snapshot": snapshot })` would
            // re-serialize the `Value` tree it already holds.
            let mut response = Map::new();
            response.insert("snapshot".into(), snapshot);
            spoke_ok(Value::Object(response))
        }
        Err(error) => spoke_reject(
            SpokeRejectCode::InternalError,
            format!("history response serialize failed: {error}"),
            None,
        ),
    }
}
/// Shared `history.revisions.prune` success-shape validation (contract
/// §Connect operations — one shape for the serving face and each
/// `RemoteAdapter`): `deleted` is a `u64`-range integer and
/// `floor_revision` is a present nullable member.
pub(crate) fn validate_history_prune_success(value: &Value) -> SpokeResult<()> {
    let Some(object) = value.as_object() else {
        return history_transport_reject(
            "malformed history.revisions.prune success value: payload is not an object",
        );
    };
    if !object.get("deleted").is_some_and(Value::is_u64) {
        return history_transport_reject(
            "malformed history.revisions.prune success value: deleted must be a non-negative integer",
        );
    }
    match object.get("floor_revision") {
        Some(floor) if floor.is_null() || floor.is_u64() => spoke_ok(()),
        Some(_) => history_transport_reject(
            "malformed history.revisions.prune success value: floor_revision must be an integer or null",
        ),
        None => history_transport_reject(
            "malformed history.revisions.prune success value: floor_revision must be present (nullable)",
        ),
    }
}

/// Serving-side prune validation: `HistoryPruneResult` is structurally
/// typed, so the check is `validate_history_prune_success` on the wire
/// object — `floor_revision` serializes present even when `None` (a
/// required member of the plain-object success shape).
fn valid_served_prune_result(result: HistoryPruneResult) -> SpokeResult<Value> {
    match serde_json::to_value(result) {
        Ok(value) => match validate_history_prune_success(&value) {
            SpokeResult::Ok(()) => spoke_ok(value),
            SpokeResult::Reject(reject) => SpokeResult::Reject(reject),
        },
        Err(error) => spoke_reject(
            SpokeRejectCode::InternalError,
            format!("history response serialize failed: {error}"),
            None,
        ),
    }
}

/// `RemoteHistoryService` over a provider [`HistoryPort`]: the two
/// `history.*` ops map onto their typed port calls, so a host attaches an
/// existing provider with
/// `RemoteServePortsComposite::new(..).with_history(Arc::new(HistoryPortService::new(port, admission)))`
/// — mirroring [`MemoryPortService`]. Admission maps the authenticated
/// peer to its `Principal` + admitted `Scope`; a present request `scope`
/// MUST equal the admission (get answers `NOT_FOUND`, prune the entity's
/// existing write-authorization denial, both without invoking the port);
/// the admitted scope is the effective scope for every port call. Prune
/// runs the §9.1 recorded-outcome dedup before the admitted call reaches
/// the port, and the service validates provider success values before
/// emitting them.
pub struct HistoryPortService {
    port: Arc<dyn HistoryPort + Send + Sync>,
    admission: HistoryAdmission,
    dedup: Arc<dyn HistoryDedupStore>,
    /// Serializes the dedup lookup → port call → record sequence so two
    /// concurrent same-key prunes cannot both observe `Unseen` and
    /// double-execute.
    dedup_lock: tokio::sync::Mutex<()>,
}

impl HistoryPortService {
    /// Attach a provider port + admission with the bounded in-memory
    /// dedup default.
    #[must_use]
    pub fn new(port: Arc<dyn HistoryPort + Send + Sync>, admission: HistoryAdmission) -> Self {
        Self {
            port,
            admission,
            dedup: Arc::new(InMemoryHistoryDedupStore::new()),
            dedup_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Inject a durable dedup store (spoke-history contract §9.1: the
    /// store is host-injectable; entries MUST outlive a Connect session).
    #[must_use]
    pub fn with_dedup_store(mut self, dedup: Arc<dyn HistoryDedupStore>) -> Self {
        self.dedup = dedup;
        self
    }
}

#[async_trait]
impl RemoteHistoryService for HistoryPortService {
    async fn invoke(
        &self,
        peer_id: &str,
        op: &str,
        payload: serde_json::Value,
    ) -> SpokeResult<serde_json::Value> {
        // §9.1: the prune payload digest covers the WHOLE request object
        // (idempotency key included) — compute it before the scope member
        // moves out for decoding.
        let payload_digest = if op == HISTORY_OP_PRUNE {
            // The digest covers the WHOLE request object under RFC 8785
            // — refuse any member outside the JCS exact-integer range
            // before hashing (see `payload_integers_jcs_exact`).
            if !payload_integers_jcs_exact(&payload) {
                return spoke_reject(
                    SpokeRejectCode::InvalidInput,
                    format!("invalid {op} payload: integer member outside the exact-integer range"),
                    None,
                );
            }
            match jcs_sha256_hex(&payload) {
                SpokeResult::Ok(digest) => Some(digest),
                SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
            }
        } else {
            None
        };
        // The optional `scope` member moves out once for the shared
        // `Scope` decode (see `decode_history_call`); a non-object payload
        // extracts nothing and fails the typed request decode.
        let mut payload = payload;
        let scope = payload
            .as_object_mut()
            .and_then(|object| object.remove("scope"));
        // Decode at the boundary first: a malformed payload keeps its
        // `INVALID_INPUT` without paying an admission lookup.
        let call = match decode_history_call(op, payload, scope) {
            SpokeResult::Ok(call) => call,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        let (principal, admitted_scope) = match (self.admission)(peer_id) {
            SpokeResult::Ok(admitted) => admitted,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        let (requested_scope, is_get) = match &call {
            HistoryCall::Get { scope, .. } => (scope, true),
            HistoryCall::Prune { scope, .. } => (scope, false),
        };
        // A present request scope MUST equal the admitted scope; absent,
        // the admitted scope is the effective scope (contract §Connect
        // operations). The refusal runs before any port call.
        if let Some(requested_scope) = requested_scope {
            if !scope_wire_equal(requested_scope, &admitted_scope) {
                return if is_get {
                    spoke_reject(
                        SpokeRejectCode::NotFound,
                        "history entity not found in the admitted scope",
                        None,
                    )
                } else {
                    spoke_reject(
                        SpokeRejectCode::NotAuthorized,
                        "history prune denied: scope outside admission",
                        None,
                    )
                };
            }
        }
        let effective_scope = admitted_scope;
        match call {
            HistoryCall::Get {
                entity_kind,
                entity_id,
                revision,
                ..
            } => {
                let result = self
                    .port
                    .get_history_snapshot(
                        &principal,
                        &effective_scope,
                        &entity_kind,
                        &entity_id,
                        revision,
                    )
                    .await;
                match result {
                    SpokeResult::Ok(snapshot) => {
                        valid_served_snapshot(snapshot, &entity_kind, &entity_id, revision)
                    }
                    SpokeResult::Reject(reject) => SpokeResult::Reject(reject),
                }
            }
            HistoryCall::Prune {
                entity_kind,
                entity_id,
                keep,
                idempotency_key,
                ..
            } => {
                // §9.1: the digest over the whole request payload was
                // computed before the `scope` member moved out.
                let payload_digest =
                    payload_digest.expect("prune computes the payload digest");
                let scope_digest = match jcs_sha256_hex(
                    &serde_json::to_value(&effective_scope).unwrap_or_else(|_| json!({})),
                ) {
                    SpokeResult::Ok(digest) => digest,
                    SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
                };
                let dedup_key = format!(
                    "{scope_digest}|{}|{op}|{idempotency_key}",
                    principal.principal_id.as_str()
                );
                // One lock covers lookup → port → record: concurrent
                // same-key prunes cannot both see `Unseen`.
                let _dedup_guard = self.dedup_lock.lock().await;
                match self.dedup.lookup(&dedup_key).await {
                    HistoryDedupLookup::Evicted => {
                        return spoke_reject(
                            SpokeRejectCode::ReceiptExpired,
                            "prune idempotency record was evicted; the recorded outcome is no longer available and the prune is not re-executed",
                            None,
                        );
                    }
                    HistoryDedupLookup::Recorded(record) => {
                        if record.payload_digest != payload_digest {
                            return spoke_reject(
                                SpokeRejectCode::IdempotencyConflict,
                                "idempotency_key replayed with a different payload digest",
                                None,
                            );
                        }
                        // Same-key same-digest replay: the recorded
                        // outcome, no port call, no re-evaluated
                        // preconditions (contract §9.1).
                        return record.outcome;
                    }
                    HistoryDedupLookup::Unseen => {}
                }
                let outcome = match self
                    .port
                    .prune_history(&principal, &effective_scope, &entity_kind, &entity_id, keep)
                    .await
                {
                    SpokeResult::Ok(result) => valid_served_prune_result(result),
                    SpokeResult::Reject(reject) => SpokeResult::Reject(reject),
                };
                // The settled outcome is recorded — success or reject —
                // so a same-key replay never re-executes the prune
                // (contract §9.1: a malformed provider success settles as
                // the transport decode error; the recorded outcome
                // replays that error verbatim without a second port
                // call).
                self.dedup
                    .record(
                        &dedup_key,
                        HistoryDedupRecord {
                            payload_digest,
                            outcome: outcome.clone(),
                        },
                    )
                    .await;
                outcome
            }
        }
    }
}

/// Decode the op's request object, call the provider once, re-encode its
/// response object. The decode is the trust boundary: an invalid payload is
/// `INVALID_INPUT` before the provider runs (same rule as `extract`).
async fn serve<Req, Resp, Fut>(payload: serde_json::Value, call: impl FnOnce(Req) -> Fut) -> SpokeResult<Value>
where
    Req: DeserializeOwned,
    Resp: Serialize,
    Fut: Future<Output = SpokeResult<Resp>>,
{
    let request: Req = match serde_json::from_value(payload) {
        Ok(request) => request,
        Err(error) => {
            // §14.2: a memory-boundary validation failure is `terminal`.
            let mut details = serde_json::Map::new();
            details.insert("retry_class".into(), Value::String("terminal".into()));
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                format!("invalid memory payload: {error}"),
                Some(details),
            );
        }
    };
    match call(request).await {
        SpokeResult::Ok(response) => match serde_json::to_value(response) {
            Ok(value) => SpokeResult::Ok(value),
            Err(error) => spoke_reject(
                SpokeRejectCode::InternalError,
                format!("memory response serialize failed: {error}"),
                None,
            ),
        },
        SpokeResult::Reject(reject) => SpokeResult::Reject(reject),
    }
}

/// Responder ports face (D4 catalogue): per-family probes, capability-gated.
/// `Option` probes mirror the operations-crate `ComputablePorts` /
/// `ForkPorts` availability convention; every probe defaults to `None`, so a
/// host opts into exactly the families it serves. The knowledge ops are
/// served through [`RemoteServePorts::as_knowledge`] instead of the
/// distributed baseline six.
pub trait RemoteServePorts: Send + Sync {
    /// Optional distributed baseline face (the six port families), when
    /// provided.
    fn as_baseline(&self) -> Option<&(dyn BaselinePorts + Send + Sync)> {
        None
    }

    /// Optional knowledge serving face (scoped store + admission), when
    /// provided.
    fn as_knowledge(&self) -> Option<Arc<RemoteKnowledgeFace>> {
        None
    }

    /// Optional l2-computable face (project / compute), when provided.
    fn as_computable(&self) -> Option<&dyn ComputablePort> {
        None
    }

    /// Optional l5-fork timeline face, when provided.
    fn as_fork_timeline(&self) -> Option<&dyn ForkTimelineQueryPort> {
        None
    }

    /// Optional `extract` service face, when provided. `None` is genuine
    /// optional absence (a host that does not serve extraction), not a stub
    /// service — the responder answers the dispatch-deny branch.
    fn as_extract(&self) -> Option<&dyn RemoteExtractService> {
        None
    }

    /// Optional memory service face, when provided. Absent means the host
    /// declared `spoke-memory` without providing the service — the responder
    /// answers the dispatch-deny branch.
    ///
    /// Owned (not borrowed) because the responder detaches the admitted
    /// invoke from its response deadline (§14.3): the spawned task must own
    /// the face.
    fn as_memory(&self) -> Option<Arc<dyn RemoteMemoryService>> {
        None
    }

    /// Optional `spoke-history` service face (revision replay + explicit
    /// pruning), when provided. Absent means the host declared
    /// `spoke-history` without providing the service — the responder
    /// answers the dispatch-deny branch.
    ///
    /// Owned (not borrowed) like [`RemoteServePorts::as_memory`]: the
    /// prune op mutates, so the responder detaches the admitted invoke
    /// from its response deadline and the spawned task must own the face.
    fn as_history(&self) -> Option<Arc<dyn RemoteHistoryService>> {
        None
    }
}

/// Blanket impl: a provider carrying the baseline six + both optional faces
/// serves those families through the responder (the probes return the
/// faces). The full-optional surface does **not** imply extraction or a
/// knowledge face: a provider opts into those explicitly through
/// [`RemoteServePortsComposite`].
impl<T> RemoteServePorts for T
where
    T: BaselinePorts + ComputablePort + ForkTimelineQueryPort + Send + Sync,
{
    fn as_baseline(&self) -> Option<&(dyn BaselinePorts + Send + Sync)> {
        Some(self)
    }

    fn as_computable(&self) -> Option<&dyn ComputablePort> {
        Some(self)
    }

    fn as_fork_timeline(&self) -> Option<&dyn ForkTimelineQueryPort> {
        Some(self)
    }
}

/// Composite responder ports face: each family is optional and independent.
/// Baseline-only and mixed hosts compose via [`RemoteServePortsComposite::new`]
/// without manual trait impls; a knowledge-only host composes with
/// [`RemoteServePortsComposite::knowledge_only`]; extraction, memory and
/// knowledge are added with the consuming builders.
pub struct RemoteServePortsComposite {
    baseline: Option<Arc<dyn BaselinePorts + Send + Sync>>,
    knowledge: Option<Arc<RemoteKnowledgeFace>>,
    computable: Option<Arc<dyn ComputablePort + Send + Sync>>,
    fork: Option<Arc<dyn ForkTimelineQueryPort + Send + Sync>>,
    extract: Option<Arc<dyn RemoteExtractService>>,
    memory: Option<Arc<dyn RemoteMemoryService>>,
    history: Option<Arc<dyn RemoteHistoryService>>,
}

impl RemoteServePortsComposite {
    /// Compose a responder ports face from a baseline provider plus
    /// optional computable / fork faces. Baseline-only hosts pass `None`
    /// for both optional faces (the composite still serves the baseline
    /// six); mixed hosts pass the faces they provide. Extraction is absent
    /// — opt in with [`RemoteServePortsComposite::with_extract`].
    pub fn new(
        baseline: Arc<dyn BaselinePorts + Send + Sync>,
        computable: Option<Arc<dyn ComputablePort + Send + Sync>>,
        fork: Option<Arc<dyn ForkTimelineQueryPort + Send + Sync>>,
    ) -> Self {
        Self::faces(Some(baseline), computable, fork)
    }

    /// Compose from per-family options: `None` is genuine face absence (the
    /// responder answers the dispatch-deny branch), so a host that serves only
    /// an optional family needs no baseline stub.
    pub(crate) fn faces(
        baseline: Option<Arc<dyn BaselinePorts + Send + Sync>>,
        computable: Option<Arc<dyn ComputablePort + Send + Sync>>,
        fork: Option<Arc<dyn ForkTimelineQueryPort + Send + Sync>>,
    ) -> Self {
        Self {
            baseline,
            knowledge: None,
            computable,
            fork,
            extract: None,
            memory: None,
            history: None,
        }
    }

    /// Compose a knowledge-only face: no baseline provider, so `assemble`
    /// and every other baseline op stays denied.
    pub fn knowledge_only(knowledge: Arc<RemoteKnowledgeFace>) -> Self {
        Self {
            knowledge: Some(knowledge),
            ..Self::faces(None, None, None)
        }
    }

    /// Attach the knowledge serving face (consuming builder).
    #[must_use]
    pub fn with_knowledge(mut self, knowledge: Arc<RemoteKnowledgeFace>) -> Self {
        self.knowledge = Some(knowledge);
        self
    }

    /// Attach the optional `extract` service face (consuming builder).
    #[must_use]
    pub fn with_extract(mut self, extract: Arc<dyn RemoteExtractService>) -> Self {
        self.extract = Some(extract);
        self
    }

    /// Attach the optional memory service face (consuming builder).
    #[must_use]
    pub fn with_memory(mut self, memory: Arc<dyn RemoteMemoryService>) -> Self {
        self.memory = Some(memory);
        self
    }

    /// Attach the optional `spoke-history` service face (consuming
    /// builder).
    #[must_use]
    pub fn with_history(mut self, history: Arc<dyn RemoteHistoryService>) -> Self {
        self.history = Some(history);
        self
    }
}

impl RemoteServePorts for RemoteServePortsComposite {
    fn as_baseline(&self) -> Option<&(dyn BaselinePorts + Send + Sync)> {
        self.baseline
            .as_ref()
            .map(|baseline| baseline.as_ref() as &(dyn BaselinePorts + Send + Sync))
    }

    fn as_knowledge(&self) -> Option<Arc<RemoteKnowledgeFace>> {
        self.knowledge.clone()
    }

    fn as_computable(&self) -> Option<&dyn ComputablePort> {
        self.computable
            .as_ref()
            .map(|computable| computable.as_ref() as &dyn ComputablePort)
    }

    fn as_fork_timeline(&self) -> Option<&dyn ForkTimelineQueryPort> {
        self.fork
            .as_ref()
            .map(|fork| fork.as_ref() as &dyn ForkTimelineQueryPort)
    }

    fn as_extract(&self) -> Option<&dyn RemoteExtractService> {
        self.extract
            .as_ref()
            .map(|extract| extract.as_ref() as &dyn RemoteExtractService)
    }

    fn as_memory(&self) -> Option<Arc<dyn RemoteMemoryService>> {
        self.memory.clone()
    }

    fn as_history(&self) -> Option<Arc<dyn RemoteHistoryService>> {
        self.history.clone()
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use serde_json::json;
    use spoke_operations::{spoke_ok_unit, HistoryPruneResult};
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    fn scope(id: &str) -> Scope {
        serde_json::from_value(json!({
            "scope_id": id,
            "extensions": {}
        }))
        .expect("valid Scope")
    }

    fn principal(id: &str) -> Principal {
        serde_json::from_value(json!({
            "principal_id": id,
            "extensions": {}
        }))
        .expect("valid Principal")
    }

    fn admission(scope: Scope, principal: Principal) -> HistoryAdmission {
        Arc::new(move |_peer| spoke_ok((principal.clone(), scope.clone())))
    }

    fn snapshot_json(entity_id: &str, revision: u64) -> serde_json::Value {
        json!({
            "schema_version": 1,
            "entity_kind": "knowledge_entry",
            "entity_id": entity_id,
            "revision": revision,
            "snapshot": {
                "schema_version": 1,
                "entry_id": entity_id,
                "canonical_name": "fixture",
                "entry_type": "note",
                "status": "active",
                "body": {},
                "extensions": {},
                "revision": revision,
            },
            "extensions": {}
        })
    }

    /// Provider stub: records call count + the principal/scope it was
    /// invoked with, returns fixed results.
    struct StubHistoryPort {
        prune_calls: AtomicU64,
        get_calls: AtomicU64,
        last_principal: Mutex<Option<Principal>>,
        last_scope: Mutex<Option<Scope>>,
    }

    impl StubHistoryPort {
        fn new() -> Self {
            Self {
                prune_calls: AtomicU64::new(0),
                get_calls: AtomicU64::new(0),
                last_principal: Mutex::new(None),
                last_scope: Mutex::new(None),
            }
        }
    }

    #[async_trait]
    impl HistoryPort for StubHistoryPort {
        async fn get_history_snapshot(
            &self,
            principal: &Principal,
            scope: &Scope,
            _entity_kind: &str,
            entity_id: &str,
            revision: u64,
        ) -> SpokeResult<RevisionSnapshot> {
            self.get_calls.fetch_add(1, AtomicOrdering::SeqCst);
            *self.last_principal.lock().expect("lock") = Some(principal.clone());
            *self.last_scope.lock().expect("lock") = Some(scope.clone());
            serde_json::from_value(snapshot_json(entity_id, revision))
                .map_or_else(|e| spoke_reject(SpokeRejectCode::InternalError, e.to_string(), None), spoke_ok)
        }

        async fn prune_history(
            &self,
            principal: &Principal,
            scope: &Scope,
            _entity_kind: &str,
            _entity_id: &str,
            keep: u64,
        ) -> SpokeResult<HistoryPruneResult> {
            self.prune_calls.fetch_add(1, AtomicOrdering::SeqCst);
            *self.last_principal.lock().expect("lock") = Some(principal.clone());
            *self.last_scope.lock().expect("lock") = Some(scope.clone());
            spoke_ok(HistoryPruneResult {
                deleted: 1,
                floor_revision: Some(keep + 1),
            })
        }
    }

    fn prune_payload(key: &str, keep: u64, scope: Option<&Value>) -> Value {
        let mut payload = json!({
            "entity_kind": "knowledge_entry",
            "entity_id": "e1",
            "keep": keep,
            "idempotency_key": key,
        });
        if let Some(scope) = scope {
            payload["scope"] = scope.clone();
        }
        payload
    }

    #[tokio::test]
    async fn get_passes_principal_and_admitted_scope_to_port() {
        let port = Arc::new(StubHistoryPort::new());
        let service = HistoryPortService::new(
            port.clone(),
            admission(scope("s1"), principal("peer-p")),
        );
        let result = service
            .invoke(
                "peer-a",
                HISTORY_OP_GET,
                json!({
                    "entity_kind": "knowledge_entry",
                    "entity_id": "e1",
                    "revision": 7,
                    // The request scope equals the admission — allowed.
                    "scope": {"scope_id": "s1", "extensions": {}}
                }),
            )
            .await;
        let SpokeResult::Ok(value) = result else {
            panic!("expected Ok, got {result:?}")
        };
        assert_eq!(value["snapshot"]["entity_id"], "e1");
        assert_eq!(value["snapshot"]["revision"], 7);
        assert_eq!(
            port.last_principal.lock().expect("lock").as_ref().unwrap().principal_id.as_str(),
            "peer-p"
        );
        assert_eq!(
            port.last_scope.lock().expect("lock").as_ref().unwrap().scope_id,
            "s1"
        );
    }

    #[tokio::test]
    async fn scope_mismatch_denies_without_port_call() {
        let port = Arc::new(StubHistoryPort::new());
        let service = HistoryPortService::new(
            port.clone(),
            admission(scope("s1"), principal("peer-p")),
        );
        let get = service
            .invoke(
                "peer-a",
                HISTORY_OP_GET,
                json!({
                    "entity_kind": "knowledge_entry",
                    "entity_id": "e1",
                    "revision": 7,
                    "scope": {"scope_id": "other", "extensions": {}}
                }),
            )
            .await;
        assert!(matches!(
            get,
            SpokeResult::Reject(SpokeReject { code: SpokeRejectCode::NotFound, .. })
        ));
        let prune = service
            .invoke(
                "peer-a",
                HISTORY_OP_PRUNE,
                prune_payload("k1", 2, Some(&json!({"scope_id": "other", "extensions": {}}))),
            )
            .await;
        assert!(matches!(
            prune,
            SpokeResult::Reject(SpokeReject { code: SpokeRejectCode::NotAuthorized, .. })
        ));
        assert_eq!(port.get_calls.load(AtomicOrdering::SeqCst), 0);
        assert_eq!(port.prune_calls.load(AtomicOrdering::SeqCst), 0);
    }

    #[tokio::test]
    async fn prune_replays_same_key_without_reexecuting() {
        let port = Arc::new(StubHistoryPort::new());
        let service = HistoryPortService::new(
            port.clone(),
            admission(scope("s1"), principal("peer-p")),
        );
        let payload = prune_payload("replay-key", 2, None);
        let first = service.invoke("peer-a", HISTORY_OP_PRUNE, payload.clone()).await;
        let second = service.invoke("peer-a", HISTORY_OP_PRUNE, payload).await;
        assert!(matches!(first, SpokeResult::Ok(_)));
        // The recorded outcome replays verbatim — no second port call.
        assert_eq!(first, second);
        assert_eq!(port.prune_calls.load(AtomicOrdering::SeqCst), 1);
    }

    #[tokio::test]
    async fn prune_same_key_different_digest_conflicts() {
        let port = Arc::new(StubHistoryPort::new());
        let service = HistoryPortService::new(
            port.clone(),
            admission(scope("s1"), principal("peer-p")),
        );
        let first = service
            .invoke("peer-a", HISTORY_OP_PRUNE, prune_payload("k1", 2, None))
            .await;
        assert!(matches!(first, SpokeResult::Ok(_)));
        let conflict = service
            .invoke("peer-a", HISTORY_OP_PRUNE, prune_payload("k1", 5, None))
            .await;
        assert!(matches!(
            conflict,
            SpokeResult::Reject(SpokeReject { code: SpokeRejectCode::IdempotencyConflict, .. })
        ));
        assert_eq!(port.prune_calls.load(AtomicOrdering::SeqCst), 1);
    }

    #[tokio::test]
    async fn evicted_receipt_expires_without_reexecuting() {
        let port = Arc::new(StubHistoryPort::new());
        // Capacity 1: the second prune's record evicts the first's; its
        // replay must answer RECEIPT_EXPIRED rather than re-execute.
        let service = HistoryPortService::new(
            port.clone(),
            admission(scope("s1"), principal("peer-p")),
        )
        .with_dedup_store(Arc::new(InMemoryHistoryDedupStore::with_capacity(1)));
        let first = service
            .invoke("peer-a", HISTORY_OP_PRUNE, prune_payload("k1", 2, None))
            .await;
        assert!(matches!(first, SpokeResult::Ok(_)));
        let second = service
            .invoke("peer-a", HISTORY_OP_PRUNE, prune_payload("k2", 2, None))
            .await;
        assert!(matches!(second, SpokeResult::Ok(_)));
        let replay = service
            .invoke("peer-a", HISTORY_OP_PRUNE, prune_payload("k1", 2, None))
            .await;
        assert!(matches!(
            replay,
            SpokeResult::Reject(SpokeReject { code: SpokeRejectCode::ReceiptExpired, .. })
        ));
        // The evicted outcome never re-executes; the live key replays.
        let live = service
            .invoke("peer-a", HISTORY_OP_PRUNE, prune_payload("k2", 2, None))
            .await;
        assert_eq!(live, second);
        assert_eq!(port.prune_calls.load(AtomicOrdering::SeqCst), 2);
    }

    #[tokio::test]
    async fn missing_idempotency_key_is_invalid_input() {
        let port = Arc::new(StubHistoryPort::new());
        let service = HistoryPortService::new(
            port.clone(),
            admission(scope("s1"), principal("peer-p")),
        );
        let mut payload = prune_payload("k1", 2, None);
        payload.as_object_mut().expect("object").remove("idempotency_key");
        let result = service.invoke("peer-a", HISTORY_OP_PRUNE, payload).await;
        assert!(matches!(
            result,
            SpokeResult::Reject(SpokeReject { code: SpokeRejectCode::InvalidInput, .. })
        ));
        assert_eq!(port.prune_calls.load(AtomicOrdering::SeqCst), 0);
    }

    #[tokio::test]
    async fn wrong_target_snapshot_is_transport_decode_error() {
        struct WrongTarget;
        #[async_trait]
        impl HistoryPort for WrongTarget {
            async fn get_history_snapshot(
                &self,
                _principal: &Principal,
                _scope: &Scope,
                _entity_kind: &str,
                _entity_id: &str,
                _revision: u64,
            ) -> SpokeResult<RevisionSnapshot> {
                serde_json::from_value(snapshot_json("OTHER", 99)).map_or_else(
                    |e| spoke_reject(SpokeRejectCode::InternalError, e.to_string(), None),
                    spoke_ok,
                )
            }
            async fn prune_history(
                &self,
                _principal: &Principal,
                _scope: &Scope,
                _entity_kind: &str,
                _entity_id: &str,
                _keep: u64,
            ) -> SpokeResult<HistoryPruneResult> {
                spoke_ok(HistoryPruneResult {
                    deleted: 0,
                    floor_revision: None,
                })
            }
        }
        let port: Arc<dyn HistoryPort + Send + Sync> = Arc::new(WrongTarget);
        let service = HistoryPortService::new(port, admission(scope("s1"), principal("p")));
        let result = service
            .invoke(
                "peer-a",
                HISTORY_OP_GET,
                json!({
                    "entity_kind": "knowledge_entry",
                    "entity_id": "e1",
                    "revision": 7,
                }),
            )
            .await;
        assert!(matches!(
            result,
            SpokeResult::Reject(SpokeReject { code: SpokeRejectCode::InternalError, .. })
        ));
    }

    /// Wire-presence rule (contract §`RevisionSnapshot`): an
    /// explicitly-present empty member of the served response payload
    /// survives the serving path byte-identically. The raw face boundary
    /// carries the response `Value` through unprojected, so a service
    /// that emits `"modules": {}` (or `body.tags: []`) preserves it.
    struct PresenceHistoryService;
    #[async_trait]
    impl RemoteHistoryService for PresenceHistoryService {
        async fn invoke(
            &self,
            _peer_id: &str,
            _op: &str,
            _payload: serde_json::Value,
        ) -> SpokeResult<serde_json::Value> {
            spoke_ok(json!({
                "snapshot": {
                    "schema_version": 1,
                    "entity_kind": "knowledge_entry",
                    "entity_id": "e1",
                    "revision": 3,
                    "snapshot": {
                        "schema_version": 1,
                        "entry_id": "e1",
                        "canonical_name": "fixture",
                        "entry_type": "note",
                        "status": "active",
                        "body": {"tags": [], "state": {}, "attributes": []},
                        "modules": {},
                        "extensions": {},
                        "revision": 3,
                    },
                    "extensions": {}
                }
            }))
        }
    }

    #[tokio::test]
    async fn served_response_preserves_explicit_empty_members() {
        let service = PresenceHistoryService;
        let result = service
            .invoke(
                "peer",
                HISTORY_OP_GET,
                json!({"entity_kind": "knowledge_entry", "entity_id": "e1", "revision": 3}),
            )
            .await;
        let SpokeResult::Ok(value) = result else {
            panic!("expected Ok, got {result:?}")
        };
        let entity = &value["snapshot"]["snapshot"];
        assert_eq!(entity["modules"], json!({}));
        assert_eq!(entity["body"]["tags"], json!([]));
        assert_eq!(entity["body"]["state"], json!({}));
        assert_eq!(entity["body"]["attributes"], json!([]));
        // The serving path emits the response payload verbatim — compare
        // the whole member byte-identically.
        assert_eq!(
            entity,
            &json!({
                "schema_version": 1,
                "entry_id": "e1",
                "canonical_name": "fixture",
                "entry_type": "note",
                "status": "active",
                "body": {"tags": [], "state": {}, "attributes": []},
                "modules": {},
                "extensions": {},
                "revision": 3,
            })
        );
    }

    #[tokio::test]
    async fn prune_result_shape_validation() {
        assert!(matches!(
            validate_history_prune_success(&json!({"deleted": 0, "floor_revision": null})),
            SpokeResult::Ok(())
        ));
        assert!(matches!(
            validate_history_prune_success(&json!({"deleted": 1, "floor_revision": 4})),
            SpokeResult::Ok(())
        ));
        // Present member required: absent floor is malformed.
        assert!(matches!(
            validate_history_prune_success(&json!({"deleted": 1})),
            SpokeResult::Reject(_)
        ));
        assert!(matches!(
            validate_history_prune_success(&json!({"deleted": -1, "floor_revision": null})),
            SpokeResult::Reject(_)
        ));
        assert!(matches!(
            validate_history_prune_success(&json!({"deleted": 1, "floor_revision": "x"})),
            SpokeResult::Reject(_)
        ));
    }

    /// Contract §`RevisionSnapshot`: the nested entity revision member is
    /// REQUIRED and must equal the envelope — a provider snapshot that
    /// omits it is a transport decode error, never `Ok`.
    #[tokio::test]
    async fn absent_nested_revision_is_transport_decode_error() {
        // Decode the stub's snapshot shape, drop the nested revision
        // member, and re-encode: the generated `Option<u64>` field
        // tolerates absence, so only the validator can refuse it.
        let mut doc = snapshot_json("e1", 7);
        doc["snapshot"].as_object_mut().expect("entity").remove("revision");
        let snapshot: RevisionSnapshot =
            serde_json::from_value(doc).expect("absent nested revision still decodes");
        let result = valid_served_snapshot(snapshot, "knowledge_entry", "e1", 7);
        assert!(matches!(
            result,
            SpokeResult::Reject(SpokeReject { code: SpokeRejectCode::InternalError, .. })
        ));
    }

    /// The prune dedup digest is SHA-256 over RFC 8785 JSON, which
    /// encodes integers through f64 — a payload integer beyond
    /// 2^53 − 1 would merge distinct requests under one digest, so the
    /// serving boundary answers `INVALID_INPUT` before any port call.
    #[tokio::test]
    async fn prune_payload_out_of_range_integer_is_invalid_input() {
        let port = Arc::new(StubHistoryPort::new());
        let service = HistoryPortService::new(
            port.clone(),
            admission(scope("s1"), principal("peer-p")),
        );
        let result = service
            .invoke(
                "peer-a",
                HISTORY_OP_PRUNE,
                prune_payload("k-unsafe", 9_007_199_254_740_993, None),
            )
            .await;
        assert!(matches!(
            result,
            SpokeResult::Reject(SpokeReject { code: SpokeRejectCode::InvalidInput, .. })
        ));
        assert_eq!(port.prune_calls.load(AtomicOrdering::SeqCst), 0);
    }

    /// §9.1 recorded-outcome model: a provider reject is a settled
    /// outcome too — the same-key replay returns it verbatim without
    /// re-executing the port.
    #[tokio::test]
    async fn recorded_reject_replays_without_reexecuting() {
        struct RejectingPort;
        #[async_trait]
        impl HistoryPort for RejectingPort {
            async fn get_history_snapshot(
                &self,
                _principal: &Principal,
                _scope: &Scope,
                _kind: &str,
                _id: &str,
                _revision: u64,
            ) -> SpokeResult<RevisionSnapshot> {
                spoke_reject(SpokeRejectCode::NotFound, "no such entity", None)
            }
            async fn prune_history(
                &self,
                _principal: &Principal,
                _scope: &Scope,
                _kind: &str,
                _id: &str,
                _keep: u64,
            ) -> SpokeResult<HistoryPruneResult> {
                spoke_reject(SpokeRejectCode::NotFound, "no such entity", None)
            }
        }
        let port: Arc<dyn HistoryPort + Send + Sync> = Arc::new(RejectingPort);
        let service = HistoryPortService::new(
            port,
            admission(scope("s1"), principal("peer-p")),
        );
        let payload = prune_payload("reject-key", 2, None);
        let first = service
            .invoke("peer-a", HISTORY_OP_PRUNE, payload.clone())
            .await;
        let second = service.invoke("peer-a", HISTORY_OP_PRUNE, payload).await;
        assert!(matches!(
            first,
            SpokeResult::Reject(SpokeReject { code: SpokeRejectCode::NotFound, .. })
        ));
        // The recorded reject replays verbatim.
        assert_eq!(first, second);
    }
}
