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
use std::sync::{Arc, LazyLock, Mutex, Weak};

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::core::jcs::{
    respell_endpoint, respell_paths, EXACT_INTEGER_BOUND as ADOPT_EXACT_INTEGER_BOUND,
};
use spoke_operations::{
    jcs_sha256_hex, spoke_ok, spoke_reject, AdoptionPort, BaselinePorts, ComputablePort,
    ForkTimelineQueryPort, HistoryPort, HistoryPruneResult, MemoryPort, ScopedKnowledgePort,
    SpokeReject, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::data::revision_snapshot::RevisionSnapshotSnapshot;
use spoke_schemas::integers::SCHEMA_VERSION_MAX;
use spoke_schemas::{
    ExtractRequest, ExtractResponse, HistoryRevisionsGetRequest, HistoryRevisionsPruneRequest,
    KnowledgeAdoptRequest, KnowledgeAdoptResponse, Principal, RevisionSnapshot, Scope,
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
/// One callable serves every `memory.*` op instead of a second 34-method
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

/// [`RemoteMemoryService`] over a provider [`MemoryPort`]: the 34 `memory.*`
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
            "memory.spaces.create" => {
                serve(payload, |request| port.create_space(&principal, request)).await
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
            "memory.grants.grant" => {
                serve(payload, |request| port.grant(&principal, request)).await
            }
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
                serve(payload, |request| {
                    port.dismiss_candidate(&principal, request)
                })
                .await
            }
            "memory.candidates.review" => {
                serve(payload, |request| {
                    port.review_candidate(&principal, request)
                })
                .await
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
                serve(payload, |request| {
                    port.get_reflection_state(&principal, request)
                })
                .await
            }
            "memory.reflections.list" => {
                serve(payload, |request| {
                    port.list_reflections(&principal, request)
                })
                .await
            }
            "memory.reflections.generate" => {
                serve(payload, |request| {
                    port.generate_reflection(&principal, request)
                })
                .await
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
pub type HistoryAdmission = Arc<dyn Fn(&str) -> SpokeResult<(Principal, Scope)> + Send + Sync>;

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
/// `(op, effective_scope, requester_principal_id, idempotency_key)` —
/// the RFC 8785 canonical JSON serialization of that four-member tuple
/// is the store key itself, so member boundaries stay structural and a
/// `|` inside a member can never collide two distinct retry identities —
/// and recording `{payload_digest, outcome}`. Host-injectable for durable
/// persistence: entries MUST outlive a Connect session and the
/// principal-bound namespace MUST stay stable across reconnects, which
/// the in-memory default cannot promise — durable `HistoryDedupStore`
/// implementations own that guarantee.
///
/// The store also owns same-key single-flight (spoke-knowledge-adoption
/// contract §Repeat and idempotency): [`HistoryDedupStore::claim_key`]
/// returns the per-key execution lock the service holds across
/// lookup → port call → record, so one store shared by multiple
/// services still yields at most one provider invocation per retry
/// identity while unrelated keys proceed concurrently. Durable stores
/// implement the same contract with atomic probe-and-record or keyed
/// single-flight of their own.
#[async_trait]
pub trait HistoryDedupStore: Send + Sync {
    /// Resolve one dedup key (see [`HistoryDedupLookup`]).
    async fn lookup(&self, key: &str) -> HistoryDedupLookup;

    /// Acquire the per-key execution lock covering the
    /// lookup → provider call → `record` sequence: same-key callers
    /// serialize on the returned guard, unrelated keys never block.
    /// Cross-process atomicity is a store-implementation obligation.
    ///
    /// The default is a process-wide keyed single-flight on the
    /// four-member retry identity: a store that predates the
    /// single-flight rule keeps compiling and still cannot
    /// double-invoke within the process — a `lookup`/`record`-only
    /// store serves one dedup space per store, and the shared default
    /// serializes the same key across every such store (F-015). A
    /// durable store MUST override with its own atomic
    /// probe-and-record or keyed single-flight (contract §Repeat and
    /// idempotency).
    async fn claim_key(&self, key: &str) -> tokio::sync::OwnedMutexGuard<()> {
        static IN_FLIGHT: LazyLock<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>> =
            LazyLock::new(|| Mutex::new(HashMap::new()));
        let lock = {
            let mut map = IN_FLIGHT.lock().expect("dedup claim lock");
            map.retain(|_, weak| weak.strong_count() > 0);
            match map.get(key).and_then(Weak::upgrade) {
                Some(lock) => lock,
                None => {
                    let lock = Arc::new(tokio::sync::Mutex::new(()));
                    map.insert(key.to_owned(), Arc::downgrade(&lock));
                    lock
                }
            }
        };
        lock.lock_owned().await
    }

    /// Persist the digest and settled outcome for one executed key.
    async fn record(&self, key: &str, record: HistoryDedupRecord);
}

/// Bounded in-memory dedup store — the default when the host injects
/// nothing (spoke-history contract §9.1: "the default in-memory store is
/// bounded by an eviction policy the host MAY configure").
/// Eviction keeps a tombstone so a replayed evicted key answers
/// `RECEIPT_EXPIRED` instead of re-executing. Tombstones are NOT subject
/// to capacity eviction (spoke-history contract §9.1): they are retained
/// for the lifetime of the store, so a replayed evicted key answers
/// `RECEIPT_EXPIRED` forever — never `Unseen`.
///
/// shortcut: tombstones grow unboundedly with distinct evicted keys; a
/// durable host store owns the bounded-retention guarantee.
pub struct InMemoryHistoryDedupStore {
    max_entries: usize,
    state: Mutex<InMemoryDedupState>,
}

#[derive(Default)]
struct InMemoryDedupState {
    /// Live records, FIFO by insertion order.
    records: HashMap<String, HistoryDedupRecord>,
    order: VecDeque<String>,
    /// Per-key single-flight locks — the store-interface ownership
    /// point: every service sharing this store serializes on the same
    /// lock for a key, while unrelated keys hold independent locks.
    /// Weak entries drop once no caller holds the lock.
    in_flight: HashMap<String, Weak<tokio::sync::Mutex<()>>>,
    /// Evicted keys — permanent for the lifetime of the store (contract
    /// §9.1): never evicted, so a replayed evicted key never returns
    /// `Unseen`.
    tombstones: HashSet<String>,
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

    async fn claim_key(&self, key: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut state = self.state.lock().expect("dedup lock");
            state.in_flight.retain(|_, weak| weak.strong_count() > 0);
            match state.in_flight.get(key).and_then(Weak::upgrade) {
                Some(lock) => lock,
                None => {
                    let lock = Arc::new(tokio::sync::Mutex::new(()));
                    state
                        .in_flight
                        .insert(key.to_owned(), Arc::downgrade(&lock));
                    lock
                }
            }
        };
        lock.lock_owned().await
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
            state.tombstones.insert(oldest);
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

/// `INVALID_INPUT` details carried by every `history.*` boundary reject:
/// the op under dispatch, so callers branch without parsing `message`
/// (TypeScript `historyInputReject` parity).
fn history_input_details(op: &str) -> Option<Map<String, Value>> {
    let mut details = Map::new();
    details.insert("op".into(), Value::from(op));
    Some(details)
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
            history_input_details(op),
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
fn decode_history_call(op: &str, payload: Value, scope: Option<Value>) -> SpokeResult<HistoryCall> {
    let scope = match decode_history_scope(op, scope) {
        SpokeResult::Ok(scope) => scope,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };
    match op {
        HISTORY_OP_GET => match serde_json::from_value::<HistoryRevisionsGetRequest>(payload) {
            Ok(request) => spoke_ok(HistoryCall::Get {
                scope,
                entity_kind: request.entity_kind.into(),
                entity_id: request.entity_id.into(),
                revision: request.revision,
            }),
            Err(error) => spoke_reject(
                SpokeRejectCode::InvalidInput,
                format!("invalid {op} payload: {error}"),
                history_input_details(op),
            ),
        },
        HISTORY_OP_PRUNE => match serde_json::from_value::<HistoryRevisionsPruneRequest>(payload) {
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
                history_input_details(op),
            ),
        },
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

/// One more than the largest u64 — the TypeScript predicate's upper
/// bound. A number at or beyond it is a float on both stacks and passes;
/// an integral value strictly inside the u64 range but beyond the
/// exact-integer endpoint is refused identically.
const U64_RANGE_BOUND: f64 = 18_446_744_073_709_551_616.0; // 2^64

/// Top-level request members whose contract domain is inclusive of the
/// `2^53` endpoint (plan Clarify decision 3: `revision`, `deleted`,
/// `floor_revision`, `keep`, `expected_base_revision`,
/// `source_revision`). No other member — in particular no product-owned
/// `extensions` value — may use the widened endpoint.
const REVISION_MEMBERS: [&str; 6] = [
    "revision",
    "deleted",
    "floor_revision",
    "keep",
    "expected_base_revision",
    "source_revision",
];

/// Every integer member of `value` is inside the exact-integer range
/// (`[−2^53, 2^53]` for a revision member, `[−2^53 + 1, 2^53 − 1]` for
/// everything else). The adoption dedup digest is SHA-256 over the RFC
/// 8785 encoding of the whole request payload; an integer beyond the
/// member's range would collapse to the same canonical number as a
/// distinct neighbor, so a different request could replay a recorded
/// outcome under its own key. The serving boundary refuses such
/// payloads with `INVALID_INPUT` instead of rounding (spoke-history
/// contract §`HistoryPort`: the runtime never rounds an unrepresentable
/// integer). The history prune digest uses it with `keep` as its only
/// revision member, so its opaque-domain float refusal (magnitude up to
/// `2^64`) is unchanged by the envelope normalizer's narrower negative
/// range.
///
/// `2^53` endpoint is admitted only for the caller's top-level revision
/// members ([`REVISION_MEMBERS`] for adoption, `keep` alone for prune):
/// it is an exact IEEE-754 integer the TypeScript `canonicalize`
/// reference emits as `9007199254740992` (contract C5), so the digest
/// hashes it via [`respell_paths`]. Every other numeric member keeps the
/// pre-widening treatment, so a product-owned `Scope.extensions` value
/// at `2^53` is still refused.
///
/// Integral floats follow the VALUE-based TypeScript rule, not the Rust
/// token's representation (F-013): `1e16` decodes as an integral f64
/// here and as an unsafe integral `number` there — the same wire
/// payload must take the same refusal. Non-integral floats and
/// out-of-u64 integral floats carry no integer identity to lose.
fn payload_integers_jcs_exact(value: &Value, revision_members: &[&str]) -> bool {
    match value {
        Value::Object(members) => members.iter().all(|(key, member)| {
            integers_within_exact_range(member, revision_members.contains(&key.as_str()))
        }),
        other => integers_within_exact_range(other, false),
    }
}

/// Recursive form of [`payload_integers_jcs_exact`]: nested members
/// never take the widened endpoint, and neither do arrays.
fn integers_within_exact_range(value: &Value, revision_member: bool) -> bool {
    let max = if revision_member {
        ADOPT_EXACT_INTEGER_BOUND
    } else {
        ADOPT_EXACT_INTEGER_BOUND - 1
    };
    match value {
        Value::Number(number) => {
            if number.is_u64() || number.is_i64() {
                let magnitude = number
                    .as_u64()
                    .or_else(|| number.as_i64().map(i64::unsigned_abs))
                    .expect("integer number");
                magnitude <= max
            } else {
                let Some(float) = number.as_f64() else {
                    return true;
                };
                if float.fract() != 0.0 {
                    return true;
                }
                let magnitude = float.abs();
                magnitude <= max as f64 || magnitude >= U64_RANGE_BOUND
            }
        }
        Value::Array(items) => items
            .iter()
            .all(|item| integers_within_exact_range(item, false)),
        Value::Object(members) => members
            .values()
            .all(|member| integers_within_exact_range(member, false)),
        _ => true,
    }
}

/// Adoption's digest normalization: `source_revision` is the sole
/// revision-typed member of the adoption request, so it alone takes the
/// `±2^53` endpoint re-spelling. Every other member keeps the raw JCS
/// treatment — `serde_jcs` stops at `2^53 − 1`, so a product-owned
/// `extensions` integer at `2^53` still refuses before the provider.
fn adopt_hashed_payload(payload: &Value) -> Value {
    let Value::Object(mut members) = payload.clone() else {
        return payload.clone();
    };
    if let Some(revision) = members.get("source_revision").and_then(respell_endpoint) {
        members.insert("source_revision".to_owned(), revision);
    }
    Value::Object(members)
}

/// The typed `knowledge.adopt` request decode. `source_revision` is
/// compared through its exact integer representation, not only the
/// generated `u64` binding (qc3-05): an integral-float spelling (`1.0`)
/// names the same wire number and decodes to the same revision, an exact
/// integer beyond the endpoint (`9007199254740993`) refuses instead of
/// rounding back into the domain, and a non-integer or negative value
/// refuses the same way. Every other member decodes through the
/// generated binding (its newtypes enforce the non-empty strings, the
/// 1–128 code-point `idempotency_key`, the `ExtensionMap` key pattern
/// and object values, and unknown members deny).
fn adopt_request_from_payload(op: &str, payload: &Value) -> SpokeResult<KnowledgeAdoptRequest> {
    let invalid = |detail: String| SpokeReject {
        code: SpokeRejectCode::InvalidInput,
        message: format!("invalid {op} payload: {detail}"),
        details: None,
    };
    let Some(revision) = payload.get("source_revision") else {
        return SpokeResult::Reject(invalid("missing source_revision".into()));
    };
    if exact_revision_u64(revision).is_none() {
        return SpokeResult::Reject(invalid(
            "source_revision must be a non-negative integer within [0, 2^53]".into(),
        ));
    }
    match serde_json::from_value(payload.clone()) {
        Ok(request) => SpokeResult::Ok(request),
        Err(error) => SpokeResult::Reject(invalid(error.to_string())),
    }
}

/// Transport-decode reject for a malformed provider success value —
/// `INTERNAL_ERROR` with `details.kind = "transport"`, never `Ok`
/// (spoke-history contract §Connect operations).
fn history_transport_reject<T>(detail: impl Into<String>) -> SpokeResult<T> {
    let mut details = Map::new();
    details.insert("kind".into(), Value::String("transport".into()));
    spoke_reject(SpokeRejectCode::InternalError, detail.into(), Some(details))
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
    if snapshot.schema_version.get() > SCHEMA_VERSION_MAX {
        return history_transport_reject(
            "provider returned a RevisionSnapshot schema_version above 2^53-1",
        );
    }
    if snapshot.revision > ADOPT_EXACT_INTEGER_BOUND {
        return history_transport_reject(
            "provider returned a RevisionSnapshot revision above 2^53",
        );
    }
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
    let (nested_id, nested_revision, nested_schema_version, nested_anchor_versions) =
        match &snapshot.snapshot {
            RevisionSnapshotSnapshot::KnowledgeEntry {
                entry_id,
                revision,
                schema_version,
                source_anchor,
                adoption_anchor,
                ..
            } => (
                entry_id.as_str(),
                *revision,
                schema_version.0.get(),
                [
                    source_anchor
                        .as_ref()
                        .map(|anchor| anchor.schema_version.0.get()),
                    adoption_anchor
                        .as_ref()
                        .map(|anchor| anchor.schema_version.0.get()),
                ],
            ),
            RevisionSnapshotSnapshot::Relation {
                relation_id,
                revision,
                schema_version,
                ..
            } => (
                relation_id.as_str(),
                *revision,
                schema_version.0.get(),
                [None, None],
            ),
        };
    if nested_schema_version > SCHEMA_VERSION_MAX {
        return history_transport_reject(
            "provider returned a malformed RevisionSnapshot: nested schema_version above 2^53-1",
        );
    }
    // The provenance anchors carry their own `schema_version` members
    // (`SourceAnchor` / `AdoptionAnchor`); the generated carriers are
    // unbounded `NonZeroU64`, so the SchemaVersion ceiling applies here
    // too.
    if nested_anchor_versions
        .iter()
        .flatten()
        .any(|version| *version > SCHEMA_VERSION_MAX)
    {
        return history_transport_reject(
            "provider returned a malformed RevisionSnapshot: nested anchor schema_version above 2^53-1",
        );
    }
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
        Some(nested_revision)
            if nested_revision <= ADOPT_EXACT_INTEGER_BOUND && nested_revision == revision =>
        {
            spoke_ok(())
        }
        _ => history_transport_reject(
            "provider returned a malformed RevisionSnapshot: nested entity revision must be present, inside [0, 2^53] and equal the envelope revision",
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
/// `RemoteAdapter`): `deleted` is a count and `floor_revision` a revision,
/// both compared through their exact integer representation
/// ([`exact_revision_u64`]): an out-of-domain exact integer refuses
/// instead of being f64-rounded back into the domain. `floor_revision` is
/// a present nullable member. A value outside the domain is an
/// out-of-domain provider answer and refuses like the adoption guards.
pub(crate) fn validate_history_prune_success(value: &Value) -> SpokeResult<()> {
    let Some(object) = value.as_object() else {
        return history_transport_reject(
            "malformed history.revisions.prune success value: payload is not an object",
        );
    };
    if object.get("deleted").and_then(exact_revision_u64).is_none() {
        return history_transport_reject(
            "malformed history.revisions.prune success value: deleted must be a non-negative integer within the exact-integer range",
        );
    }
    match object.get("floor_revision") {
        Some(floor) if floor.is_null() || exact_revision_u64(floor).is_some() => spoke_ok(()),
        Some(_) => history_transport_reject(
            "malformed history.revisions.prune success value: floor_revision must be an integer within the exact-integer range or null",
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
    /// Owns same-key single-flight via [`HistoryDedupStore::claim_key`]:
    /// two concurrent same-key prunes cannot both observe `Unseen` and
    /// double-execute, while unrelated keys proceed concurrently.
    dedup: Arc<dyn HistoryDedupStore>,
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
            // The digest covers the WHOLE request object under RFC 8785.
            // Refuse any member outside the prune's digest domain before
            // hashing (see `payload_integers_jcs_exact`): `keep` alone may
            // carry the `2^53` endpoint, while an opaque integral float
            // whose magnitude lies in `[2^53, 2^64)` keeps wave E's
            // refusal rather than the envelope normalizer's narrower
            // negative range.
            if !payload_integers_jcs_exact(&payload, &["keep"]) {
                return spoke_reject(
                    SpokeRejectCode::InvalidInput,
                    format!("invalid {op} payload: integer member outside the exact-integer range"),
                    history_input_details(op),
                );
            }
            // Normalization owns only this op's revision member (`keep`):
            // an endpoint elsewhere in the object — including the extra
            // top-level `revision` spelling the shared six-member predicate
            // widens — is not this digest's to re-spell, so the walk refuses
            // it as a structured `INVALID_INPUT` instead of unwinding.
            let normalized = match respell_paths(payload.clone(), &[&["keep"]]) {
                Ok(normalized) => normalized,
                Err(error) => {
                    return spoke_reject(
                        SpokeRejectCode::InvalidInput,
                        format!("invalid {op} payload: {error}"),
                        history_input_details(op),
                    )
                }
            };
            match jcs_sha256_hex(&normalized) {
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
                let payload_digest = payload_digest.expect("prune computes the payload digest");
                let scope_value =
                    serde_json::to_value(&effective_scope).unwrap_or_else(|_| json!({}));
                // The dedup key IS the canonical serialization of the
                // ordered retry-identity tuple — never a delimiter-joined
                // string (contract §Repeat and idempotency): schema-opaque
                // principal ids and arbitrary idempotency keys keep
                // structural member boundaries, so two distinct retry
                // identities can never derive one key.
                let dedup_key = match serde_jcs::to_string(&json!([
                    op,
                    scope_value,
                    principal.principal_id.as_str(),
                    idempotency_key.as_str()
                ])) {
                    Ok(key) => key,
                    Err(error) => {
                        return spoke_reject(
                            SpokeRejectCode::InternalError,
                            format!("history prune dedup key canonicalization failed: {error}"),
                            None,
                        )
                    }
                };
                // The store-interface keyed lock covers
                // lookup → port → record: concurrent same-key prunes
                // cannot both see `Unseen` — including across services
                // sharing this store — while unrelated keys proceed
                // concurrently.
                let _dedup_guard = self.dedup.claim_key(&dedup_key).await;
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

// ── `spoke-knowledge-adoption` serving face (spoke-knowledge-adoption
//    contract §Connect operation) ──────────────────────────────────────
//
// `RemoteAdoptionService` is the invoke seam the responder calls for the
// `knowledge.*` reserved-product ops; `AdoptionPortService` adapts a
// provider `AdoptionPort` to it, mirroring `RemoteHistoryService` /
// `HistoryPortService`. The service owns the concerns the contract keeps
// OUT of `AdoptionPort`: peer admission (`AdoptionAdmission` maps the
// authenticated `peer_id` to the `Principal` and admitted `Scope`), and
// the connect idempotency envelope (the recorded-outcome model keyed
// `(effective_scope, requester_principal_id, op, idempotency_key)`, digest
// over RFC 8785 canonical JSON of the complete request payload).

/// `knowledge.adopt` wire op.
pub const KNOWLEDGE_OP_ADOPT: &str = "knowledge.adopt";

/// Admission context for the adoption face: maps the authenticated
/// connect peer to the `Principal` and the `Scope` the provider admits —
/// the same `(Principal, Scope)` shape as [`HistoryAdmission`]. Neither
/// comes from the payload (MEM-R003; contract §Admission and
/// owner-private absence).
pub type AdoptionAdmission = HistoryAdmission;

/// Connect-owned service face for the `spoke-knowledge-adoption` family
/// (`knowledge.*`, spoke-knowledge-adoption contract §Connect operation):
/// the serving host owns the provider — the
/// [`spoke_operations::AdoptionPort`] implementation — the admission
/// context that turns the authenticated peer into a `Principal` +
/// admitted `Scope` (never payload-asserted), the mapping of the op onto
/// its typed port call, and the idempotency dedup store.
///
/// One callable serves the family (same no-wrapper rule as `memory.*` /
/// `history.*`): `payload` is the op's request object and the returned
/// value is its response object — `{outcome, entry}` for adopt.
#[async_trait]
pub trait RemoteAdoptionService: Send + Sync {
    /// Serve one `knowledge.*` op for the authenticated `peer_id`.
    async fn invoke(
        &self,
        peer_id: &str,
        op: &str,
        payload: serde_json::Value,
    ) -> SpokeResult<serde_json::Value>;
}

/// `RemoteAdoptionService` over a provider [`AdoptionPort`]: the
/// `knowledge.adopt` op maps onto its typed port call, so a host attaches
/// an existing provider with
/// `RemoteServePortsComposite::new(..).with_adoption(Arc::new(AdoptionPortService::new(port, admission)))`
/// — mirroring [`HistoryPortService`]. The dedup key is the RFC 8785
/// canonical JSON serialization of the ordered retry-identity tuple
/// `(op, effective_scope, requester_principal_id, idempotency_key)` —
/// member boundaries stay structural, never delimiter-joined (contract
/// §Repeat and idempotency: the contract key names that tuple followed
/// by `payload_digest`; the digest is recorded on the dedup record and
/// compared on replay, so a same-identity different-digest replay still
/// answers `IDEMPOTENCY_CONFLICT`). A same-identity same-digest replay
/// returns the recorded outcome with `outcome` normalized to
/// `already_present` — the existing copy, no second write.
///
/// The dedup store reuses the `HistoryDedupStore` seam — the
/// recorded-outcome shape and bounded in-memory default are identical;
/// host-injectable for durable persistence. The store also owns same-key
/// single-flight through [`HistoryDedupStore::claim_key`].
pub struct AdoptionPortService {
    port: Arc<dyn AdoptionPort + Send + Sync>,
    admission: AdoptionAdmission,
    dedup: Arc<dyn HistoryDedupStore>,
}

impl AdoptionPortService {
    /// Attach a provider port + admission with the bounded in-memory
    /// dedup default.
    #[must_use]
    pub fn new(port: Arc<dyn AdoptionPort + Send + Sync>, admission: AdoptionAdmission) -> Self {
        Self {
            port,
            admission,
            dedup: Arc::new(InMemoryHistoryDedupStore::new()),
        }
    }

    /// Inject a durable dedup store (entries MUST outlive a Connect
    /// session and the principal-bound namespace MUST stay stable across
    /// reconnects — the in-memory default cannot promise that).
    #[must_use]
    pub fn with_dedup_store(mut self, dedup: Arc<dyn HistoryDedupStore>) -> Self {
        self.dedup = dedup;
        self
    }
}

#[async_trait]
impl RemoteAdoptionService for AdoptionPortService {
    async fn invoke(
        &self,
        peer_id: &str,
        op: &str,
        payload: serde_json::Value,
    ) -> SpokeResult<serde_json::Value> {
        if op != KNOWLEDGE_OP_ADOPT {
            return SpokeResult::Reject(SpokeReject {
                code: SpokeRejectCode::CapabilityPortMissing,
                message: format!("unimplemented adoption op {op}"),
                details: None,
            });
        }
        // The digest covers the WHOLE request object under RFC 8785 —
        // refuse any member outside the exact-integer rule before hashing
        // (see `payload_integers_jcs_exact`: only a revision-typed member
        // may carry the `2^53` endpoint), then hash the copy whose
        // `source_revision` endpoint is the equal f64 (`serde_jcs` stops
        // one below it, while the endpoint is a valid revision spelling).
        // An unrelated `2^53` — a product-owned `extensions` value
        // included — keeps the raw JCS treatment and refuses here.
        if !payload_integers_jcs_exact(&payload, &REVISION_MEMBERS) {
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                format!("invalid {op} payload: integer member outside the exact-integer range"),
                None,
            );
        }
        let payload_digest = match jcs_sha256_hex(&adopt_hashed_payload(&payload)) {
            SpokeResult::Ok(digest) => digest,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        // Decode at the boundary first: a malformed payload keeps its
        // `INVALID_INPUT` without paying an admission lookup (contract:
        // validation and decode complete before admission). An explicit
        // `destination_entry_id: null` is malformed — the selector is a
        // string or absent — but the generated `Option` field decodes
        // null as absent, so the null check runs on the raw payload.
        let explicit_null_selector = payload
            .get("destination_entry_id")
            .is_some_and(Value::is_null);
        if explicit_null_selector {
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                format!("invalid {op} payload: destination_entry_id must be a string or absent, not null"),
                None,
            );
        }
        let request = match adopt_request_from_payload(op, &payload) {
            SpokeResult::Ok(request) => request,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        let (principal, effective_scope) = match (self.admission)(peer_id) {
            SpokeResult::Ok(admitted) => admitted,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        // Destination admission (contract Wire-wide rules): the
        // destination `scope_id` is authorized against the admitted
        // context BEFORE any source-chain work — an out-of-scope
        // destination refuses `NOT_AUTHORIZED` without touching the
        // source (qc1 F-016 / qc2 QC2-08 / qc3 F-017).
        if request.destination_scope_id.as_str() != effective_scope.scope_id.as_str() {
            return spoke_reject(
                SpokeRejectCode::NotAuthorized,
                "adoption destination scope is outside the admitted context",
                None,
            );
        }
        let scope_value = serde_json::to_value(&effective_scope).unwrap_or_else(|_| json!({}));
        // The dedup key IS the canonical serialization — never a
        // delimiter-joined string (contract §Repeat and idempotency).
        let dedup_key = match serde_jcs::to_string(&json!([
            op,
            scope_value,
            principal.principal_id.as_str(),
            request.idempotency_key.as_str()
        ])) {
            Ok(key) => key,
            Err(error) => {
                return spoke_reject(
                    SpokeRejectCode::InternalError,
                    format!("adoption dedup key canonicalization failed: {error}"),
                    None,
                )
            }
        };
        // The store-interface keyed lock covers lookup → port → record:
        // concurrent same-identity adoptions cannot both see `Unseen` —
        // including across services sharing this store.
        let _dedup_guard = self.dedup.claim_key(&dedup_key).await;
        match self.dedup.lookup(&dedup_key).await {
            HistoryDedupLookup::Evicted => {
                return spoke_reject(
                    SpokeRejectCode::ReceiptExpired,
                    "adoption idempotency record was evicted; the recorded outcome is no longer available and the adoption is not re-executed",
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
                // Same-key same-digest replay: the recorded outcome with
                // `outcome` normalized to `already_present` — the
                // existing copy, no second write, no re-evaluated
                // preconditions (contract §Repeat and idempotency).
                return adoption_replay(record.outcome);
            }
            HistoryDedupLookup::Unseen => {}
        }
        let outcome = match self
            .port
            .adopt_knowledge_entry(&principal, &effective_scope, &request)
            .await
        {
            SpokeResult::Ok(response) => valid_served_adopt_result(
                response,
                request.destination_entry_id.as_deref().map(String::as_str),
            ),
            SpokeResult::Reject(mut reject) => {
                // qc1 F-031 — a provider Reject receipt's revision-typed
                // detail gets the same domain guard as the served
                // branches: an out-of-domain `details.adopted_revision`
                // settles as a provider fault instead of reaching the
                // wire.
                if let Some(fault) = normalize_adopt_reject_details(&mut reject) {
                    reject = fault;
                }
                SpokeResult::Reject(reject)
            }
        };
        // The settled outcome is recorded — success or reject — so a
        // same-key replay never re-executes the adoption.
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

/// Same-key same-digest replay for `knowledge.adopt` (contract §Repeat and
/// idempotency): returns the existing copy with `outcome:
/// "already_present"` — the recorded success value carries the
/// destination entry; only its outcome word changes. A recorded reject or
/// a non-object success replays verbatim.
fn adoption_replay(outcome: SpokeResult<Value>) -> SpokeResult<Value> {
    match outcome {
        SpokeResult::Ok(Value::Object(mut response)) => {
            if response.contains_key("outcome") {
                response.insert("outcome".into(), Value::String("already_present".into()));
            }
            spoke_ok(Value::Object(response))
        }
        other => other,
    }
}

/// The exact integer a revision member names, without an f64 round-trip:
/// an integer token keeps its own value (an exact `9007199254740993`
/// refuses instead of rounding back into the domain), an integral float
/// spelling is admitted by its parsed value (`9007199254740992.0` ≡
/// `2^53`). One rule shared by the prune success gate and the relation
/// put fence (qc3-05).
pub(crate) fn exact_revision_u64(value: &Value) -> Option<u64> {
    if let Some(revision) = value.as_u64() {
        return (revision <= ADOPT_EXACT_INTEGER_BOUND).then_some(revision);
    }
    let float = value.as_f64()?;
    (float.fract() == 0.0 && float >= 0.0 && float <= ADOPT_EXACT_INTEGER_BOUND as f64)
        .then_some(float as u64)
}

/// The f64-observed identity of a revision member — the number JavaScript
/// reads for the same token (`9007199254740993` ≡ `9007199254740992.0`).
/// Used only where the emitted wire value must carry the parsed identity
/// (adoption normalization); admission gates use [`exact_revision_u64`].
pub(crate) fn f64_identity_revision_u64(value: &Value) -> Option<u64> {
    let float = value.as_f64()?;
    (float.fract() == 0.0 && float >= 0.0 && float <= ADOPT_EXACT_INTEGER_BOUND as f64)
        .then_some(float as u64)
}

/// Every structural `schema_version` member inside a decoded payload must
/// be in the SchemaVersion domain. The generated carrier is `NonZeroU64`
/// with no ceiling, so this raw-JSON scan re-applies the bound on every
/// payload before its typed decode. Product-owned opaque bags
/// (`extensions`, `modules`, `state`, `computable`), the explicitly
/// open `metadata` object, and `Finding.text_position` are skipped: their
/// keys are not protocol members, so a product value that happens to be
/// named `schema_version` must reach the provider untouched (qc2
/// Important 6 / qc3-07).
pub(crate) fn schema_versions_in_domain(value: &Value) -> bool {
    match value {
        Value::Object(members) => members.iter().all(|(key, member)| {
            if key == "schema_version" {
                schema_version_in_domain(member)
            } else if matches!(
                key.as_str(),
                "extensions" | "modules" | "state" | "computable" | "metadata" | "text_position"
            ) {
                true
            } else {
                schema_versions_in_domain(member)
            }
        }),
        Value::Array(items) => items.iter().all(schema_versions_in_domain),
        _ => true,
    }
}

/// One wire number against the SchemaVersion domain, read by its parsed
/// value: an integral float spelling names the same wire number JavaScript
/// reads (`1.0` ≡ `1`, and `9007199254740992.0` ≡ `2^53` is out of domain)
/// while an exact integer token keeps its own value.
fn schema_version_in_domain(value: &Value) -> bool {
    let Some(number) = value.as_number() else {
        return false;
    };
    if let Some(version) = number.as_u64() {
        return (1..=SCHEMA_VERSION_MAX).contains(&version);
    }
    number.as_f64().is_some_and(|version| {
        version.fract() == 0.0 && version >= 1.0 && version <= SCHEMA_VERSION_MAX as f64
    })
}

/// Provider-fault guard for a Reject receipt's revision-typed detail
/// (qc1 F-031 / qc3-05): `details.adopted_revision` obeys the `[0, 2^53]`
/// domain compared on the exact integer representation — an exact
/// `9007199254740993` is out of domain and settles as a provider-fault
/// `INTERNAL_ERROR` (transport kind) instead of being rounded back into the
/// domain, while an integral-float spelling is admitted by its parsed value
/// and normalizes to that integer. `Some(fault)` means the receipt is
/// malformed; `None` means it was conformant (normalized in place when a
/// rewrite applied).
pub(crate) fn normalize_adopt_reject_details(reject: &mut SpokeReject) -> Option<SpokeReject> {
    let details = reject.details.as_mut()?;
    let adopted_revision = details.get_mut("adopted_revision")?;
    let Some(revision) = exact_revision_u64(adopted_revision) else {
        let mut fault_details = Map::new();
        fault_details.insert("kind".into(), Value::String("transport".into()));
        return Some(SpokeReject {
            code: SpokeRejectCode::InternalError,
            message:
                "adoption provider returned an error details.adopted_revision outside [0, 2^53]"
                    .into(),
            details: Some(fault_details),
        });
    };
    *adopted_revision = json!(revision);
    None
}

/// Revision-domain rule for the adoption served bounds: the bound is
/// evaluated on the f64 VALUE — the same number JavaScript observes —
/// not the wire token, so an integer spelling between `u64` and its f64
/// rounding (`9007199254740993` ≡ `9007199254740992.0`) admits or
/// refuses identically on both stacks (contract E1 / qc3 F-006). Admission
/// gates that must not round use [`exact_revision_u64`].
fn revision_in_domain(value: &Value) -> bool {
    f64_identity_revision_u64(value).is_some()
}

/// Served-side adopt-response validation (spoke-knowledge-adoption
/// contract §`KnowledgeAdoptResponse`): two branches travel as a served
/// `Ok`.
/// - Success (`OneOf0`): `{outcome, entry, extensions?}` — `outcome` is
///   `"adopted"` or `"already_present"` and `entry` is the destination
///   `KnowledgeEntry` with every required wire member, not merely a
///   well-formed anchor. `destination_entry_id` binds the served
///   `entry.entry_id` when the request supplied one; an absent selector
///   defaults to the entry's own id (`entry.entry_id` supplies it), so no
///   equality check applies then.
/// - Failure (`OneOf1`, contract E3): `{error, extensions?}` — a
///   provider error envelope emitted verbatim, never converted into a
///   transport reject.
///
/// Any shape outside the two closed branches is a transport decode
/// reject, never `Ok` (parity with TS `validServedAdoptResult` — the
/// port's expected refusals travel as `SpokeResult`, not the envelope's
/// error variant). The typed `KnowledgeAdoptResponse` decode already
/// enforces the closed top-level member sets and the `ErrorEnvelope`
/// shape (`deny_unknown_fields`); the checks here are the
/// defense-in-depth layer for the revision domain and entry completeness.
fn valid_served_adopt_result(
    response: KnowledgeAdoptResponse,
    destination_entry_id: Option<&str>,
) -> SpokeResult<Value> {
    let value = match serde_json::to_value(response) {
        Ok(value) => value,
        Err(error) => {
            return spoke_reject(
                SpokeRejectCode::InternalError,
                format!("adoption response serialize failed: {error}"),
                None,
            )
        }
    };
    let Some(object) = value.as_object() else {
        return history_transport_reject("adoption provider returned a non-object response");
    };
    // Failure branch (`OneOf1 {error, extensions}` - contract E3): a
    // provider error envelope with an optional `extensions` member is
    // emitted verbatim. Supplied `extensions` members are
    // presence-faithful (E2), and their `ExtensionMap` well-formedness
    // is already enforced by the typed `Shared2` decode - a provider
    // cannot construct a key outside `^[a-z][a-z0-9_-]*$`.
    if let Some(error) = object.get("error") {
        let error_ok = error.is_object()
            && serde_json::from_value::<spoke_schemas::common::ErrorEnvelope>(error.clone())
                .is_ok();
        if !error_ok {
            return history_transport_reject(
                "adoption provider returned a malformed error envelope",
            );
        }
        // qc3 F-021 / qc3-05 - a revision-typed member inside the
        // failure `details` (`details.adopted_revision`, contract
        // §Refusal vocabulary) obeys the `[0, 2^53]` domain on its exact
        // integer representation: an exact out-of-domain integer refuses
        // instead of rounding, an integral-float spelling normalizes to
        // its parsed integer.
        // (`details.retirement_chain` is an array of source-entry-id
        // strings - it carries no revision member.)
        let mut emitted = value.clone();
        if let Some(adopted_revision) = emitted
            .get_mut("error")
            .and_then(Value::as_object_mut)
            .and_then(|error| error.get_mut("details"))
            .and_then(Value::as_object_mut)
            .and_then(|details| details.get_mut("adopted_revision"))
        {
            let Some(revision) = exact_revision_u64(adopted_revision) else {
                return history_transport_reject(
                    "adoption provider returned an error details.adopted_revision outside [0, 2^53]",
                );
            };
            *adopted_revision = json!(revision);
        }
        return spoke_ok(emitted);
    }
    let outcome = object
        .get("outcome")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let outcome_ok = outcome
        .as_deref()
        .is_some_and(|outcome| outcome == "adopted" || outcome == "already_present");
    if !outcome_ok {
        return history_transport_reject(
            "adoption provider returned a missing or invalid outcome member",
        );
    }
    let Some(entry) = object.get("entry").filter(|entry| entry.is_object()) else {
        return history_transport_reject(
            "adoption provider returned a missing or malformed entry member",
        );
    };
    // A success entry always carries `adoption_anchor`, and it must
    // satisfy the `AdoptionAnchor` schema — presence alone is malformed
    // (contract §`KnowledgeAdoptResponse`): a provider result whose entry
    // lacks a well-formed anchor is a serving refusal, never `Ok`.
    let anchor_ok = entry.get("adoption_anchor").is_some_and(|anchor| {
        serde_json::from_value::<spoke_schemas::data::AdoptionAnchor>(anchor.clone()).is_ok()
    });
    if !anchor_ok {
        return history_transport_reject(
            "adoption provider returned an entry without a well-formed adoption_anchor member",
        );
    }
    // A valid anchor does not make an invalid entry a valid adoption
    // success (QC2-03): the emitted entry decodes as a full
    // `KnowledgeEntry` — every required wire member must be present and
    // well-formed.
    let Ok(decoded_entry) =
        serde_json::from_value::<spoke_schemas::data::KnowledgeEntry>(entry.clone())
    else {
        return history_transport_reject(
            "adoption provider returned an entry that is not a valid KnowledgeEntry",
        );
    };
    // SchemaVersion is `[1, 2^53-1]`; scan the entry and its nested
    // anchors on the raw provider value (the typed carriers are unbounded
    // `NonZeroU64`).
    if !schema_versions_in_domain(entry) {
        return history_transport_reject(
            "adoption provider returned an entry with a schema_version outside [1, 2^53-1]",
        );
    }
    // Contract E1 — revision-typed members live in the interoperable
    // integer domain `[0, 2^53]` on the wire; the bound runs on the RAW
    // provider value (f64-observed) BEFORE any normalization to `u64`,
    // so an out-of-domain revision refuses instead of being rounded into
    // the domain (qc1 F-013).
    let entry_revision_ok = entry.get("revision").is_none_or(revision_in_domain);
    if !entry_revision_ok {
        return history_transport_reject(
            "adoption provider returned an entry with revision outside [0, 2^53]",
        );
    }
    let anchor_revision_ok = entry
        .get("adoption_anchor")
        .and_then(Value::as_object)
        .and_then(|anchor| anchor.get("source_revision"))
        .is_some_and(revision_in_domain);
    if !anchor_revision_ok {
        return history_transport_reject(
            "adoption provider returned an anchor with source_revision outside [0, 2^53]",
        );
    }
    if let Some(destination_entry_id) = destination_entry_id {
        if decoded_entry.entry_id != destination_entry_id {
            return history_transport_reject(
                "adoption provider returned an entry whose entry_id does not match the requested destination_entry_id",
            );
        }
    }
    // qc3 F-006 residual — JavaScript parses every number to one f64,
    // so the emitted revision members normalize to the SAME integer
    // identity a JS reader observes (`9007199254740993` serves as the
    // exact `9007199254740992.0`, float spellings decode like integer
    // ones): rewrite the in-domain members to the f64-rounded `u64`
    // before emit, mirroring `adopt_request_from_payload`'s
    // `float as u64` rewrite.
    let mut entry = entry.clone();
    {
        let normalize = |member: Option<&mut Value>| {
            let Some(member) = member else { return };
            if let Some(rounded) = f64_identity_revision_u64(member) {
                *member = json!(rounded);
            }
        };
        normalize(entry.get_mut("revision"));
        normalize(
            entry
                .get_mut("adoption_anchor")
                .and_then(Value::as_object_mut)
                .and_then(|anchor| anchor.get_mut("source_revision")),
        );
    }
    // The emitted success carries `{outcome, entry}` plus a
    // producer-supplied `extensions` member verbatim (contract E2:
    // absent ≡ present-empty, but a supplied member is never dropped).
    let mut response = serde_json::Map::new();
    response.insert("outcome".into(), Value::String(outcome.expect("validated above")));
    response.insert("entry".into(), entry);
    if let Some(extensions) = object.get("extensions") {
        response.insert("extensions".into(), extensions.clone());
    }
    spoke_ok(Value::Object(response))
}

/// Decode the op's request object, call the provider once, re-encode its
/// response object. The decode is the trust boundary: an invalid payload is
/// `INVALID_INPUT` before the provider runs (same rule as `extract`).
async fn serve<Req, Resp, Fut>(
    payload: serde_json::Value,
    call: impl FnOnce(Req) -> Fut,
) -> SpokeResult<Value>
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

    /// Optional `spoke-knowledge-adoption` service face (adopt a shared
    /// `KnowledgeEntry` into a destination scope), when provided. Absent
    /// means the host declared `spoke-knowledge-adoption` without
    /// providing the service — the responder answers the dispatch-deny
    /// branch.
    ///
    /// Owned (not borrowed) like [`RemoteServePorts::as_memory`]: the
    /// adopt op mutates, so the responder detaches the admitted invoke
    /// from its response deadline and the spawned task must own the face.
    fn as_adoption(&self) -> Option<Arc<dyn RemoteAdoptionService>> {
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
/// [`RemoteServePortsComposite::knowledge_only`]; extraction, memory,
/// history and adoption are added with the consuming builders.
pub struct RemoteServePortsComposite {
    baseline: Option<Arc<dyn BaselinePorts + Send + Sync>>,
    knowledge: Option<Arc<RemoteKnowledgeFace>>,
    computable: Option<Arc<dyn ComputablePort + Send + Sync>>,
    fork: Option<Arc<dyn ForkTimelineQueryPort + Send + Sync>>,
    extract: Option<Arc<dyn RemoteExtractService>>,
    memory: Option<Arc<dyn RemoteMemoryService>>,
    history: Option<Arc<dyn RemoteHistoryService>>,
    adoption: Option<Arc<dyn RemoteAdoptionService>>,
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
            adoption: None,
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

    /// Attach the optional `spoke-knowledge-adoption` service face
    /// (consuming builder).
    #[must_use]
    pub fn with_adoption(mut self, adoption: Arc<dyn RemoteAdoptionService>) -> Self {
        self.adoption = Some(adoption);
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

    fn as_adoption(&self) -> Option<Arc<dyn RemoteAdoptionService>> {
        self.adoption.clone()
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
            serde_json::from_value(snapshot_json(entity_id, revision)).map_or_else(
                |e| spoke_reject(SpokeRejectCode::InternalError, e.to_string(), None),
                spoke_ok,
            )
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
                floor_revision: (keep < ADOPT_EXACT_INTEGER_BOUND).then_some(keep + 1),
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
        let service =
            HistoryPortService::new(port.clone(), admission(scope("s1"), principal("peer-p")));
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
            port.last_principal
                .lock()
                .expect("lock")
                .as_ref()
                .unwrap()
                .principal_id
                .as_str(),
            "peer-p"
        );
        assert_eq!(
            port.last_scope
                .lock()
                .expect("lock")
                .as_ref()
                .unwrap()
                .scope_id,
            "s1"
        );
    }

    #[tokio::test]
    async fn scope_mismatch_denies_without_port_call() {
        let port = Arc::new(StubHistoryPort::new());
        let service =
            HistoryPortService::new(port.clone(), admission(scope("s1"), principal("peer-p")));
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
            SpokeResult::Reject(SpokeReject {
                code: SpokeRejectCode::NotFound,
                ..
            })
        ));
        let prune = service
            .invoke(
                "peer-a",
                HISTORY_OP_PRUNE,
                prune_payload(
                    "k1",
                    2,
                    Some(&json!({"scope_id": "other", "extensions": {}})),
                ),
            )
            .await;
        assert!(matches!(
            prune,
            SpokeResult::Reject(SpokeReject {
                code: SpokeRejectCode::NotAuthorized,
                ..
            })
        ));
        assert_eq!(port.get_calls.load(AtomicOrdering::SeqCst), 0);
        assert_eq!(port.prune_calls.load(AtomicOrdering::SeqCst), 0);
    }

    #[tokio::test]
    async fn prune_replays_same_key_without_reexecuting() {
        let port = Arc::new(StubHistoryPort::new());
        let service =
            HistoryPortService::new(port.clone(), admission(scope("s1"), principal("peer-p")));
        let payload = prune_payload("replay-key", 2, None);
        let first = service
            .invoke("peer-a", HISTORY_OP_PRUNE, payload.clone())
            .await;
        let second = service.invoke("peer-a", HISTORY_OP_PRUNE, payload).await;
        assert!(matches!(first, SpokeResult::Ok(_)));
        // The recorded outcome replays verbatim — no second port call.
        assert_eq!(first, second);
        assert_eq!(port.prune_calls.load(AtomicOrdering::SeqCst), 1);
    }

    #[tokio::test]
    async fn prune_same_key_different_digest_conflicts() {
        let port = Arc::new(StubHistoryPort::new());
        let service =
            HistoryPortService::new(port.clone(), admission(scope("s1"), principal("peer-p")));
        let first = service
            .invoke("peer-a", HISTORY_OP_PRUNE, prune_payload("k1", 2, None))
            .await;
        assert!(matches!(first, SpokeResult::Ok(_)));
        let conflict = service
            .invoke("peer-a", HISTORY_OP_PRUNE, prune_payload("k1", 5, None))
            .await;
        assert!(matches!(
            conflict,
            SpokeResult::Reject(SpokeReject {
                code: SpokeRejectCode::IdempotencyConflict,
                ..
            })
        ));
        assert_eq!(port.prune_calls.load(AtomicOrdering::SeqCst), 1);
    }

    /// PR #210 pin: a `|` inside the schema-opaque `principal_id` or an
    /// arbitrary `idempotency_key` must never merge two distinct retry
    /// identities into one dedup key. The joined key was
    /// `{scope_digest}|{principal}|{op}|{key}`, so
    /// `(principal "p", key "a|history.revisions.prune|b")` and
    /// `(principal "p|history.revisions.prune|a", key "b")` produced the
    /// SAME key — the second call collided with the first's recorded
    /// outcome (an `IDEMPOTENCY_CONFLICT` on the differing payload
    /// digest, never its own execution). The canonical-array key keeps
    /// member boundaries structural: the two identities record two
    /// independent outcomes and an exact repeat still replays one.
    #[tokio::test]
    async fn prune_dedup_key_is_structural_never_delimiter_colliding() {
        let port = Arc::new(StubHistoryPort::new());
        let by_peer: HistoryAdmission = Arc::new(|peer_id| {
            if peer_id == "peer-1" {
                spoke_ok((principal("p"), scope("s1")))
            } else {
                spoke_ok((
                    principal("p|history.revisions.prune|a"),
                    scope("s1"),
                ))
            }
        });
        let service = HistoryPortService::new(port.clone(), by_peer);
        let first = service
            .invoke(
                "peer-1",
                HISTORY_OP_PRUNE,
                prune_payload("a|history.revisions.prune|b", 2, None),
            )
            .await;
        assert!(matches!(first, SpokeResult::Ok(_)), "{first:?}");
        // The delimiter-shifted identity: a distinct retry that must
        // execute its own prune — not collide with the first record.
        let second = service
            .invoke(
                "peer-2",
                HISTORY_OP_PRUNE,
                prune_payload("b", 2, None),
            )
            .await;
        assert!(matches!(second, SpokeResult::Ok(_)), "{second:?}");
        // An exact repeat of the second identity still replays its
        // recorded outcome without re-executing.
        let replay = service
            .invoke(
                "peer-2",
                HISTORY_OP_PRUNE,
                prune_payload("b", 2, None),
            )
            .await;
        assert_eq!(second, replay);
        assert_eq!(port.prune_calls.load(AtomicOrdering::SeqCst), 2);
    }

    #[tokio::test]
    async fn evicted_receipt_expires_without_reexecuting() {
        let port = Arc::new(StubHistoryPort::new());
        // Capacity 1: the second prune's record evicts the first's; its
        // replay must answer RECEIPT_EXPIRED rather than re-execute.
        let service =
            HistoryPortService::new(port.clone(), admission(scope("s1"), principal("peer-p")))
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
            SpokeResult::Reject(SpokeReject {
                code: SpokeRejectCode::ReceiptExpired,
                ..
            })
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
        let service =
            HistoryPortService::new(port.clone(), admission(scope("s1"), principal("peer-p")));
        let mut payload = prune_payload("k1", 2, None);
        payload
            .as_object_mut()
            .expect("object")
            .remove("idempotency_key");
        let result = service.invoke("peer-a", HISTORY_OP_PRUNE, payload).await;
        let SpokeResult::Reject(reject) = result else {
            panic!("expected INVALID_INPUT, got {result:?}");
        };
        assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        assert_eq!(
            reject.details.as_ref().and_then(|details| details.get("op")),
            Some(&Value::from(HISTORY_OP_PRUNE))
        );
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
            SpokeResult::Reject(SpokeReject {
                code: SpokeRejectCode::InternalError,
                ..
            })
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
        // Provider answers outside the interoperable `[0, 2^53]` domain
        // refuse like the adoption guards — both members are revisions.
        assert!(matches!(
            validate_history_prune_success(&json!({"deleted": u64::MAX, "floor_revision": null})),
            SpokeResult::Reject(_)
        ));
        assert!(matches!(
            validate_history_prune_success(&json!({"deleted": 1, "floor_revision": u64::MAX})),
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
        doc["snapshot"]
            .as_object_mut()
            .expect("entity")
            .remove("revision");
        let snapshot: RevisionSnapshot =
            serde_json::from_value(doc).expect("absent nested revision still decodes");
        let result = valid_served_snapshot(snapshot, "knowledge_entry", "e1", 7);
        assert!(matches!(
            result,
            SpokeResult::Reject(SpokeReject {
                code: SpokeRejectCode::InternalError,
                ..
            })
        ));
    }

    /// SchemaVersion is `[1, 2^53-1]`; the generated carriers are
    /// unbounded `NonZeroU64`, so the raw-JSON scan re-applies the bound
    /// to every member, nested anchors included.
    #[test]
    fn schema_version_scan_bounds_every_member() {
        assert!(schema_versions_in_domain(&json!({"schema_version": 1})));
        assert!(schema_versions_in_domain(
            &json!({"schema_version": 9_007_199_254_740_991u64})
        ));
        assert!(!schema_versions_in_domain(&json!({"schema_version": 0})));
        assert!(!schema_versions_in_domain(
            &json!({"schema_version": 9_007_199_254_740_992u64})
        ));
        assert!(!schema_versions_in_domain(&json!({
            "entry": {"adoption_anchor": {"schema_version": 9_007_199_254_740_992u64}}
        })));
        // Integral-float spellings name the same wire number JavaScript
        // reads: `1.0` is the parsed `1`, and `2^53` in either spelling is
        // outside the SchemaVersion domain (C-03).
        assert!(schema_versions_in_domain(&json!({"schema_version": 1.0})));
        assert!(!schema_versions_in_domain(
            &json!({"schema_version": 9_007_199_254_740_992.0})
        ));
        assert!(!schema_versions_in_domain(&json!({"schema_version": 1.5})));
        assert!(schema_versions_in_domain(
            &json!({"findings": [{"schema_version": 2}]})
        ));
        // The open `metadata` object is product data: a member that happens
        // to be named `schema_version` is never a protocol version.
        assert!(schema_versions_in_domain(
            &json!({"metadata": {"schema_version": "product-v2"}})
        ));
    }

    /// A request revision outside `[0, 2^53]` refuses before the port is
    /// called: the generated `revision` carrier decodes through
    /// `deserialize_revision`, which owns the bound.
    #[tokio::test]
    async fn history_get_revision_above_domain_is_invalid_input() {
        let port = Arc::new(StubHistoryPort::new());
        let service =
            HistoryPortService::new(port.clone(), admission(scope("s1"), principal("peer-p")));
        let result = service
            .invoke(
                "peer-a",
                HISTORY_OP_GET,
                json!({
                    "entity_kind": "knowledge_entry",
                    "entity_id": "e1",
                    "revision": 9_007_199_254_740_993u64,
                }),
            )
            .await;
        assert!(matches!(
            result,
            SpokeResult::Reject(SpokeReject {
                code: SpokeRejectCode::InvalidInput,
                ..
            })
        ));
        assert_eq!(port.get_calls.load(AtomicOrdering::SeqCst), 0);
    }

    /// A provider snapshot outside the published SchemaVersion / revision
    /// domains never decodes into the typed carrier — envelope, nested
    /// snapshot, provenance anchor or revision (the generated bounds).
    #[tokio::test]
    async fn snapshot_domains_are_enforced() {
        let mut envelope = snapshot_json("e1", 3);
        envelope["schema_version"] = json!(9_007_199_254_740_992u64);
        let mut nested = snapshot_json("e1", 3);
        nested["snapshot"]["schema_version"] = json!(9_007_199_254_740_992u64);
        let mut source_anchor = snapshot_json("e1", 3);
        source_anchor["snapshot"]["source_anchor"] = json!({
            "schema_version": 9_007_199_254_740_992u64,
            "source_id": "src",
            "extensions": {},
        });
        let mut adoption_anchor = snapshot_json("e1", 3);
        adoption_anchor["snapshot"]["adoption_anchor"] = json!({
            "schema_version": 9_007_199_254_740_992u64,
            "source_entry_id": "src",
            "source_revision": 1,
            "source_scope_id": "s",
            "extensions": {},
        });
        let revision = snapshot_json("e1", 9_007_199_254_740_993u64);
        for (name, doc) in [
            ("envelope schema_version", envelope),
            ("nested schema_version", nested),
            ("source_anchor schema_version", source_anchor),
            ("adoption_anchor schema_version", adoption_anchor),
            ("revision", revision),
        ] {
            assert!(
                serde_json::from_value::<RevisionSnapshot>(doc).is_err(),
                "{name} outside the domain must not decode"
            );
        }

        // The SchemaVersion endpoint itself (2^53-1) is accepted.
        let mut doc = snapshot_json("e1", 3);
        doc["snapshot"]["source_anchor"] = json!({
            "schema_version": SCHEMA_VERSION_MAX,
            "source_id": "src",
            "extensions": {},
        });
        doc["snapshot"]["adoption_anchor"] = json!({
            "schema_version": SCHEMA_VERSION_MAX,
            "source_entry_id": "src",
            "source_revision": 1,
            "source_scope_id": "s",
            "extensions": {},
        });
        let snapshot: RevisionSnapshot = serde_json::from_value(doc).expect("decodes");
        assert!(valid_served_snapshot(snapshot, "knowledge_entry", "e1", 3).is_ok());
    }

    /// The prune dedup digest is SHA-256 over RFC 8785 JSON, which
    /// encodes integers through f64 — a payload integer beyond
    /// 2^53 − 1 would merge distinct requests under one digest, so the
    /// serving boundary answers `INVALID_INPUT` before any port call (the
    /// generated `keep` carrier enforces the domain at decode).
    #[tokio::test]
    async fn prune_payload_out_of_range_integer_is_invalid_input() {
        let port = Arc::new(StubHistoryPort::new());
        let service =
            HistoryPortService::new(port.clone(), admission(scope("s1"), principal("peer-p")));
        let result = service
            .invoke(
                "peer-a",
                HISTORY_OP_PRUNE,
                prune_payload("k-unsafe", 9_007_199_254_740_993, None),
            )
            .await;
        assert!(matches!(
            result,
            SpokeResult::Reject(SpokeReject {
                code: SpokeRejectCode::InvalidInput,
                ..
            })
        ));
        assert_eq!(port.prune_calls.load(AtomicOrdering::SeqCst), 0);
    }

    /// F-QC2-01 — the prune digest keeps its previous opaque numeric
    /// domain: an integral float `-1e19` in `extensions` lies below the
    /// envelope normalizer's `-2^63` floor but inside the digest's old
    /// magnitude-below-`2^64` refusal, so it is `INVALID_INPUT` before any
    /// port call. The `keep` endpoint at `2^53` stays admitted.
    #[tokio::test]
    async fn prune_opaque_float_outside_previous_domain_is_invalid_input() {
        let port = Arc::new(StubHistoryPort::new());
        let service =
            HistoryPortService::new(port.clone(), admission(scope("s1"), principal("peer-p")));
        let mut payload = prune_payload("k-opaque-float", 2, None);
        payload["extensions"] = json!({ "ns": { "n": -1e19 } });
        let result = service
            .invoke("peer-a", HISTORY_OP_PRUNE, payload)
            .await;
        assert!(
            matches!(
                result,
                SpokeResult::Reject(SpokeReject {
                    code: SpokeRejectCode::InvalidInput,
                    ..
                })
            ),
            "an opaque float in the previous refusal range must reject: {result:?}"
        );
        assert_eq!(port.prune_calls.load(AtomicOrdering::SeqCst), 0);

        let keep = service
            .invoke(
                "peer-a",
                HISTORY_OP_PRUNE,
                prune_payload("k-opaque-float-keep", 9_007_199_254_740_992, None),
            )
            .await;
        assert!(matches!(keep, SpokeResult::Ok(_)), "{keep:?}");
        assert_eq!(port.prune_calls.load(AtomicOrdering::SeqCst), 1);
    }

    /// E-01 / E-QC1-01 — the prune digest normalization follows the op's
    /// `keep`-only ownership. An extra top-level `revision` with the float
    /// endpoint spelling passes the shared six-member predicate, but it is
    /// not this digest's to re-spell, so the service answers a structured
    /// `INVALID_INPUT` instead of unwinding on a failed normalization.
    #[tokio::test]
    async fn prune_extra_endpoint_member_is_invalid_input_not_a_panic() {
        let port = Arc::new(StubHistoryPort::new());
        let service =
            HistoryPortService::new(port.clone(), admission(scope("s1"), principal("peer-p")));
        let mut payload = prune_payload("k-extra-endpoint", 2, None);
        payload["revision"] = json!(9_007_199_254_740_992.0);
        let result = service
            .invoke("peer-a", HISTORY_OP_PRUNE, payload)
            .await;
        assert!(
            matches!(
                result,
                SpokeResult::Reject(SpokeReject {
                    code: SpokeRejectCode::InvalidInput,
                    ..
                })
            ),
            "the wrong-slot endpoint must reject: {result:?}"
        );
        assert_eq!(port.prune_calls.load(AtomicOrdering::SeqCst), 0);
    }

    /// qc2 Important 3 / qc3-05 — the typed history request decode accepts
    /// an integral-float revision spelling on both request branches
    /// (`revision`, `keep`); the port receives the parsed endpoint value.
    #[tokio::test]
    async fn history_request_decodes_integral_float_revisions() {
        let port = Arc::new(StubHistoryPort::new());
        let service =
            HistoryPortService::new(port.clone(), admission(scope("s1"), principal("peer-p")));
        let get = service
            .invoke(
                "peer-a",
                HISTORY_OP_GET,
                json!({
                    "entity_kind": "knowledge_entry",
                    "entity_id": "e1",
                    "revision": 9_007_199_254_740_992.0,
                }),
            )
            .await;
        assert!(matches!(get, SpokeResult::Ok(_)), "{get:?}");
        assert_eq!(port.get_calls.load(AtomicOrdering::SeqCst), 1);

        let mut prune = prune_payload("k-float", 0, None);
        prune["keep"] = json!(2.0);
        let prune = service.invoke("peer-a", HISTORY_OP_PRUNE, prune).await;
        assert!(matches!(prune, SpokeResult::Ok(_)), "{prune:?}");
        assert_eq!(port.prune_calls.load(AtomicOrdering::SeqCst), 1);
    }

    /// The contract's revision domain is inclusive of `2^53` (C5): the
    /// `9007199254740992` endpoint is an exact f64, canonicalizes without
    /// an exponent like the TypeScript `canonicalize` reference, and the
    /// prune reaches the port. Only `2^53 + 1` is refused.
    #[tokio::test]
    async fn prune_keep_at_exact_integer_endpoint_reaches_port() {
        assert_eq!(
            serde_jcs::to_string(&respell_paths(
                json!({ "keep": 9_007_199_254_740_992u64 }),
                &[&["keep"]],
            )
            .expect("the endpoint re-spells"))
            .expect("canonical string"),
            "{\"keep\":9007199254740992}"
        );
        let port = Arc::new(StubHistoryPort::new());
        let service =
            HistoryPortService::new(port.clone(), admission(scope("s1"), principal("peer-p")));
        let result = service
            .invoke(
                "peer-a",
                HISTORY_OP_PRUNE,
                prune_payload("k-endpoint", 9_007_199_254_740_992, None),
            )
            .await;
        assert!(matches!(result, SpokeResult::Ok(_)), "{result:?}");
        assert_eq!(port.prune_calls.load(AtomicOrdering::SeqCst), 1);
    }

    /// qc3-05: the prune success gate compares each revision member's exact
    /// integer representation — an exact `2^53 + 1` from a provider refuses
    /// as a transport fault instead of being f64-rounded back into the
    /// domain, while the `2^53` endpoint serves in either spelling.
    #[test]
    fn prune_success_gate_compares_revision_members_exactly() {
        assert!(validate_history_prune_success(&json!({
            "deleted": 9_007_199_254_740_992_u64,
            "floor_revision": Value::Null,
        }))
        .is_ok());
        assert!(validate_history_prune_success(&json!({
            "deleted": 9_007_199_254_740_992.0,
            "floor_revision": 9_007_199_254_740_992.0,
        }))
        .is_ok());
        for refused in [9_007_199_254_740_993_u64, 9_007_199_254_740_994_u64] {
            for payload in [
                json!({ "deleted": refused, "floor_revision": Value::Null }),
                json!({ "deleted": 1, "floor_revision": refused }),
            ] {
                assert!(
                    validate_history_prune_success(&payload).is_reject(),
                    "{payload} carries an out-of-domain exact integer"
                );
            }
        }
    }

    /// qc3-05 end to end through the service: a provider's exact
    /// `9007199254740993` `deleted` answer is a fault, never a rounded
    /// success; the `2^53` endpoint reaches the caller unchanged.
    #[tokio::test]
    async fn prune_provider_exact_integer_beyond_domain_is_a_fault() {
        struct FixedPrunePort(u64);
        #[async_trait]
        impl HistoryPort for FixedPrunePort {
            async fn get_history_snapshot(
                &self,
                _principal: &Principal,
                _scope: &Scope,
                _entity_kind: &str,
                _entity_id: &str,
                _revision: u64,
            ) -> SpokeResult<RevisionSnapshot> {
                unreachable!("prune-only fixture")
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
                    deleted: self.0,
                    floor_revision: None,
                })
            }
        }
        let serve = |deleted: u64| async move {
            let port: Arc<dyn HistoryPort + Send + Sync> = Arc::new(FixedPrunePort(deleted));
            HistoryPortService::new(port, admission(scope("s1"), principal("peer-p")))
                .invoke("peer-a", HISTORY_OP_PRUNE, prune_payload("k-exact", 1, None))
                .await
        };
        let served = serve(9_007_199_254_740_992).await;
        let SpokeResult::Ok(value) = served else {
            panic!("the 2^53 endpoint serves: {served:?}")
        };
        assert_eq!(value.get("deleted"), Some(&json!(9_007_199_254_740_992_u64)));
        for refused in [9_007_199_254_740_993_u64, 9_007_199_254_740_994_u64] {
            match serve(refused).await {
                SpokeResult::Reject(reject) => {
                    assert_eq!(reject.code, SpokeRejectCode::InternalError, "{refused}");
                }
                SpokeResult::Ok(value) => panic!("{refused} must refuse, served {value}"),
            }
        }
    }

    /// The widened `2^53` admission is revision-typed only: `keep`
    /// reaches the port at the endpoint, while a product-owned
    /// `Scope.extensions` value at the same magnitude keeps the
    /// pre-widening refusal (`INVALID_INPUT`, no port call). The scope
    /// here matches the admission so the digest guard — not scope
    /// equality — is what refuses.
    #[tokio::test]
    async fn prune_guard_admits_the_endpoint_only_for_revision_members() {
        let port = Arc::new(StubHistoryPort::new());
        let service =
            HistoryPortService::new(port.clone(), admission(scope("s1"), principal("peer-p")));
        let endpoint_scope = json!({
            "scope_id": "s1",
            "extensions": {
                "ns": { "value": 9_007_199_254_740_992.0 },
            },
        });
        let extensions_at_endpoint = service
            .invoke(
                "peer-a",
                HISTORY_OP_PRUNE,
                prune_payload("k-ext", 2, Some(&endpoint_scope)),
            )
            .await;
        assert!(
            matches!(
                extensions_at_endpoint,
                SpokeResult::Reject(SpokeReject {
                    code: SpokeRejectCode::InvalidInput,
                    ..
                })
            ),
            "a non-revision member at 2^53 must refuse: {extensions_at_endpoint:?}"
        );
        assert_eq!(port.prune_calls.load(AtomicOrdering::SeqCst), 0);

        let keep_at_endpoint = service
            .invoke(
                "peer-a",
                HISTORY_OP_PRUNE,
                prune_payload("k-keep", 9_007_199_254_740_992, None),
            )
            .await;
        assert!(matches!(keep_at_endpoint, SpokeResult::Ok(_)), "{keep_at_endpoint:?}");
        assert_eq!(port.prune_calls.load(AtomicOrdering::SeqCst), 1);
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
        let service = HistoryPortService::new(port, admission(scope("s1"), principal("peer-p")));
        let payload = prune_payload("reject-key", 2, None);
        let first = service
            .invoke("peer-a", HISTORY_OP_PRUNE, payload.clone())
            .await;
        let second = service.invoke("peer-a", HISTORY_OP_PRUNE, payload).await;
        assert!(matches!(
            first,
            SpokeResult::Reject(SpokeReject {
                code: SpokeRejectCode::NotFound,
                ..
            })
        ));
        // The recorded reject replays verbatim.
        assert_eq!(first, second);
    }

    // ── `spoke-knowledge-adoption` serving-face tests ──────────────────

    /// Provider stub for request-validation tests: valid `invoke` calls
    /// never reach the port (validation answers first), so the port
    /// panics if it does.
    struct PanicAdoptionPort;

    #[async_trait]
    impl AdoptionPort for PanicAdoptionPort {
        async fn adopt_knowledge_entry(
            &self,
            _principal: &Principal,
            _scope: &Scope,
            _request: &KnowledgeAdoptRequest,
        ) -> SpokeResult<KnowledgeAdoptResponse> {
            panic!("a malformed request must refuse before the port runs")
        }
    }

    /// A dedup store implementing only `lookup`/`record` — the
    /// pre-single-flight surface — keeps compiling unchanged now that
    /// `claim_key` carries a default no-claim implementation.
    struct LookupOnlyDedupStore;

    #[async_trait]
    impl HistoryDedupStore for LookupOnlyDedupStore {
        async fn lookup(&self, _key: &str) -> HistoryDedupLookup {
            HistoryDedupLookup::Unseen
        }

        async fn record(&self, _key: &str, _record: HistoryDedupRecord) {}
    }

    fn adopt_payload(extensions: Option<Value>) -> Value {
        let mut payload = json!({
            "source_scope_id": "src-scope",
            "source_entry_id": "src-entry",
            "source_revision": 3,
            "destination_scope_id": "dst-scope",
            "idempotency_key": "key-1",
        });
        if let Some(extensions) = extensions {
            payload["extensions"] = extensions;
        }
        payload
    }

    /// F-013 numeric parity: the predicate is value-based — `1e16`
    /// (integral f64 inside the u64 range) refuses like the integer
    /// literal `9007199254740993`, while `1e20`/`1e21` (integral floats
    /// beyond the u64 range) and non-integral floats pass, matching the
    /// TypeScript `payloadIntegersJcsExact`.
    #[test]
    fn integral_floats_follow_the_shared_exact_range() {
        for refused in [
            json!(9_007_199_254_740_993u64),
            json!(1e16),
            json!(9.1e15),
            json!(-1e16),
        ] {
            assert!(
                !payload_integers_jcs_exact(&refused, &REVISION_MEMBERS),
                "{refused} must be refused"
            );
        }
        for accepted in [
            json!(9_007_199_254_740_991u64),
            json!(1e20),
            json!(1e21),
            json!(1.5),
            json!(0.0),
        ] {
            assert!(
                payload_integers_jcs_exact(&accepted, &REVISION_MEMBERS),
                "{accepted} must be accepted"
            );
        }
    }

    /// F-013 canonical spelling parity: integral floats the predicate
    /// admits canonicalize without an exponent — the same bytes the
    /// TypeScript `canonicalize` (JSON.stringify) emits.
    #[test]
    fn integral_floats_canonicalize_without_exponent() {
        for (value, expected) in [
            (json!(1e16), "10000000000000000"),
            (json!(9.1e15), "9100000000000000"),
            (json!(1e20), "100000000000000000000"),
        ] {
            assert_eq!(
                serde_jcs::to_string(&value).expect("canonical string"),
                expected
            );
        }
    }

    /// The shared `ExtensionMap` constraints bind at request validation:
    /// a key outside `^[a-z][a-z0-9_-]*$` or a non-object value answers
    /// `INVALID_INPUT` before admission or any provider call.
    #[tokio::test]
    async fn adopt_extensions_enforces_extension_map() {
        let service = AdoptionPortService::new(
            Arc::new(PanicAdoptionPort),
            Arc::new(|_peer| spoke_reject(SpokeRejectCode::InternalError, "unused", None)),
        )
        .with_dedup_store(Arc::new(LookupOnlyDedupStore));
        for extensions in [
            json!({"Bad-Key": {}}),
            json!({"ns": 5}),
            json!({"ns": "scalar"}),
            json!("scalar"),
            json!([1]),
            json!(5),
        ] {
            let result = service
                .invoke(
                    "peer-a",
                    KNOWLEDGE_OP_ADOPT,
                    adopt_payload(Some(extensions.clone())),
                )
                .await;
            assert!(
                matches!(
                    result,
                    SpokeResult::Reject(SpokeReject {
                        code: SpokeRejectCode::InvalidInput,
                        ..
                    })
                ),
                "extensions {extensions} must refuse INVALID_INPUT: {result:?}"
            );
        }
    }

    /// The typed request decode sees the raw wire number for
    /// `source_revision`: an integral-float spelling names the same wire
    /// number as its integer spelling, while an exact integer beyond the
    /// endpoint refuses `INVALID_INPUT` instead of rounding (qc3-05).
    fn decode_revision(revision: Value) -> SpokeResult<KnowledgeAdoptRequest> {
        let payload = adopt_payload(None);
        let mut payload = serde_json::to_value(&payload).expect("payload object");
        payload["source_revision"] = revision;
        adopt_request_from_payload("knowledge.adopt", &payload)
    }

    #[test]
    fn adopt_request_decode_accepts_integral_float_revisions() {
        let decoded_revision = |revision: Value| match decode_revision(revision) {
            SpokeResult::Ok(request) => Some(request.source_revision),
            SpokeResult::Reject(_) => None,
        };
        assert_eq!(decoded_revision(json!(1.0)), Some(1));
        assert_eq!(decoded_revision(json!(3)), Some(3));
        assert_eq!(decoded_revision(json!(9007199254740992.0)), Some(1 << 53));
        for refused in [
            json!(9007199254740993u64), // the first exact integer past the endpoint
            json!(9007199254740995u64), // f64 2^53+2
            json!(1.5),
            json!(-1.0),
            json!(-3),
        ] {
            assert!(
                matches!(
                    decode_revision(refused.clone()),
                    SpokeResult::Reject(SpokeReject {
                        code: SpokeRejectCode::InvalidInput,
                        ..
                    })
                ),
                "{refused} must refuse INVALID_INPUT"
            );
        }
    }

    /// QC2-06 / B5 member presence (qc3 F-006): the typed serializer is
    /// presence-sensitive — a present `extensions` member emits
    /// byte-identically (an explicitly empty `{}` stays a present `{}`),
    /// and an absent member stays absent, so the provider-visible
    /// request matches the TypeScript `KnowledgeAdoptRequest` shape.
    #[test]
    fn typed_request_serialization_preserves_extension_presence() {
        let SpokeResult::Ok(absent) = decode_revision(json!(3)) else {
            panic!("a valid payload decodes");
        };
        assert!(
            absent.extensions.is_none(),
            "an absent extensions member decodes as absent, not an empty map"
        );
        let emitted = serde_json::to_value(&absent).expect("request serializes");
        assert!(
            emitted.get("extensions").is_none(),
            "an absent member stays absent on the wire — never emitted as an empty map"
        );
        // The explicit `extensions: {}` member stays a present member
        // through decode AND serialization (contract §Request
        // `extensions` emission).
        for (member, expected) in [(json!({}), json!({})), (json!({"ns": {}}), json!({"ns": {}}))] {
            let payload = adopt_payload(Some(member));
            let SpokeResult::Ok(request) =
                adopt_request_from_payload("knowledge.adopt", &payload)
            else {
                panic!("a valid payload decodes");
            };
            assert_eq!(
                request
                    .extensions
                    .as_ref()
                    .map(serde_json::to_value)
                    .transpose()
                    .expect("map serializes"),
                Some(expected.clone()),
                "a present member keeps its value through the typed decode"
            );
            let emitted = serde_json::to_value(&request).expect("request serializes");
            assert_eq!(emitted.get("extensions"), Some(&expected));
        }
    }

    /// Destination admission (contract Wire-wide rules): an out-of-scope
    /// destination refuses `NOT_AUTHORIZED` before the provider runs —
    /// even when the source is also unadmitted (or retired), so no
    /// source-dependent refusal can escape first (qc1 F-016 / qc3 F-017).
    #[tokio::test]
    async fn destination_admission_refuses_not_authorized_before_source_access() {
        struct NoCallPort;
        #[async_trait]
        impl AdoptionPort for NoCallPort {
            async fn adopt_knowledge_entry(
                &self,
                _principal: &Principal,
                _scope: &Scope,
                _request: &KnowledgeAdoptRequest,
            ) -> SpokeResult<KnowledgeAdoptResponse> {
                panic!("an out-of-scope destination must refuse before the provider")
            }
        }
        // Admission admits only `spc_local` — the fixture destination.
        let service = AdoptionPortService::new(
            Arc::new(NoCallPort),
            admission(scope("spc_local"), principal("prn_peer-a")),
        );
        for source_scope in ["spc_shared", "spc_other", "spc_local"] {
            let mut payload = adopt_payload(None);
            payload["source_scope_id"] = json!(source_scope);
            payload["destination_scope_id"] = json!("spc_elsewhere");
            let result = service.invoke("peer-a", KNOWLEDGE_OP_ADOPT, payload).await;
            assert!(
                matches!(
                    result,
                    SpokeResult::Reject(SpokeReject {
                        code: SpokeRejectCode::NotAuthorized,
                        ..
                    })
                ),
                "destination outside admission must refuse NOT_AUTHORIZED (source {source_scope}): {result:?}"
            );
        }
    }

    /// The numeric decode is wired into the serving path: an
    /// integral-float `source_revision` spelling reaches the provider as
    /// the same revision its integer spelling carries (qc3 F-006).
    #[tokio::test]
    async fn integral_float_source_revision_serves_end_to_end() {
        struct RevisionCapturePort(Mutex<Vec<u64>>);
        #[async_trait]
        impl AdoptionPort for RevisionCapturePort {
            async fn adopt_knowledge_entry(
                &self,
                _principal: &Principal,
                _scope: &Scope,
                request: &KnowledgeAdoptRequest,
            ) -> SpokeResult<KnowledgeAdoptResponse> {
                self.0
                    .lock()
                    .expect("capture lock")
                    .push(request.source_revision);
                let response: KnowledgeAdoptResponse = serde_json::from_value(json!({
                    "outcome": "adopted",
                    "entry": {
                        "schema_version": 1,
                        "entry_id": "ke_dst",
                        "entry_type": "note",
                        "canonical_name": "copy",
                        "status": "confirmed",
                        "revision": 1,
                        "body": {},
                        "adoption_anchor": {
                            "schema_version": 1,
                            "source_scope_id": "spc_local",
                            "source_entry_id": "ke_src",
                            "source_revision": request.source_revision,
                            "extensions": {},
                        },
                        "extensions": {},
                    },
                }))
                .expect("response decodes");
                spoke_ok(response)
            }
        }
        let port = Arc::new(RevisionCapturePort(Mutex::new(Vec::new())));
        let service = AdoptionPortService::new(
            Arc::clone(&port) as Arc<dyn AdoptionPort + Send + Sync>,
            admission(scope("spc_local"), principal("prn_peer-a")),
        );
        let mut payload = adopt_payload(None);
        payload["source_scope_id"] = json!("spc_local");
        payload["destination_scope_id"] = json!("spc_local");
        payload["source_revision"] = json!(7.0);
        let result = service.invoke("peer-a", KNOWLEDGE_OP_ADOPT, payload).await;
        assert!(
            matches!(result, SpokeResult::Ok(_)),
            "an integral-float revision serves: {result:?}"
        );
        assert_eq!(*port.0.lock().expect("capture lock"), vec![7]);
    }

    /// The adoption dedup digest hashes the WHOLE request under RFC
    /// 8785, and `source_revision` is contract-inclusive of `2^53` (C5).
    /// `serde_jcs` stops one below the endpoint, so the serving face
    /// re-spells it as the equal f64 — canonical bytes
    /// `9007199254740992`, the same bytes the TypeScript canonicalizer
    /// emits — and the adoption reaches the provider. Replay keeps
    /// whole-request identity; a different payload under the same key
    /// conflicts; the literal `2^53 + 1` still refuses.
    #[tokio::test]
    async fn adopt_source_revision_endpoint_serves_replays_and_conflicts() {
        struct EndpointPort {
            calls: AtomicU64,
        }
        #[async_trait]
        impl AdoptionPort for EndpointPort {
            async fn adopt_knowledge_entry(
                &self,
                _principal: &Principal,
                _scope: &Scope,
                request: &KnowledgeAdoptRequest,
            ) -> SpokeResult<KnowledgeAdoptResponse> {
                self.calls.fetch_add(1, AtomicOrdering::SeqCst);
                assert_eq!(request.source_revision, 1 << 53);
                spoke_ok(
                    serde_json::from_value(json!({
                        "outcome": "adopted",
                        "entry": {
                            "schema_version": 1,
                            "entry_id": "ke_dst",
                            "entry_type": "note",
                            "canonical_name": "copy",
                            "status": "confirmed",
                            "revision": 1,
                            "body": {},
                            "adoption_anchor": {
                                "schema_version": 1,
                                "source_scope_id": "spc_local",
                                "source_entry_id": "src-entry",
                                "source_revision": 9_007_199_254_740_992u64,
                                "extensions": {},
                            },
                            "extensions": {},
                        },
                    }))
                    .expect("response decodes"),
                )
            }
        }
        let port = Arc::new(EndpointPort {
            calls: AtomicU64::new(0),
        });
        let service = AdoptionPortService::new(
            Arc::clone(&port) as Arc<dyn AdoptionPort + Send + Sync>,
            admission(scope("spc_local"), principal("prn_peer-a")),
        );
        let payload = || {
            let mut payload = adopt_payload(None);
            payload["source_scope_id"] = json!("spc_local");
            payload["destination_scope_id"] = json!("spc_local");
            payload["source_revision"] = json!(9_007_199_254_740_992u64);
            payload
        };

        let first = service
            .invoke("peer-a", KNOWLEDGE_OP_ADOPT, payload())
            .await;
        let SpokeResult::Ok(first) = first else {
            panic!("a source_revision at 2^53 must serve: {first:?}")
        };
        assert_eq!(first["outcome"], "adopted");

        let replay = service
            .invoke("peer-a", KNOWLEDGE_OP_ADOPT, payload())
            .await;
        let SpokeResult::Ok(replay) = replay else {
            panic!("same-key same-digest must replay: {replay:?}")
        };
        assert_eq!(replay["outcome"], "already_present");
        assert_eq!(port.calls.load(AtomicOrdering::SeqCst), 1);

        let mut conflicting = payload();
        conflicting["source_entry_id"] = json!("src-entry-2");
        let conflict = service
            .invoke("peer-a", KNOWLEDGE_OP_ADOPT, conflicting)
            .await;
        assert!(
            matches!(
                conflict,
                SpokeResult::Reject(SpokeReject {
                    code: SpokeRejectCode::IdempotencyConflict,
                    ..
                })
            ),
            "same key with a different digest must conflict: {conflict:?}"
        );
        assert_eq!(port.calls.load(AtomicOrdering::SeqCst), 1);

        let mut over_bound = payload();
        over_bound["idempotency_key"] = json!("key-over");
        over_bound["source_revision"] = json!(9_007_199_254_740_993u64);
        let refused = service
            .invoke("peer-a", KNOWLEDGE_OP_ADOPT, over_bound)
            .await;
        assert!(
            matches!(
                refused,
                SpokeResult::Reject(SpokeReject {
                    code: SpokeRejectCode::InvalidInput,
                    ..
                })
            ),
            "the literal 2^53 + 1 must refuse INVALID_INPUT: {refused:?}"
        );
        assert_eq!(port.calls.load(AtomicOrdering::SeqCst), 1);
    }

    /// Important 1 (fix 3): the adoption digest's `±2^53` endpoint is
    /// revision-typed only. `source_revision = 2^53` reaches the provider,
    /// while the same integer in a product-owned `extensions` value keeps
    /// the pre-widening JCS treatment and refuses before any provider
    /// call.
    #[tokio::test]
    async fn adopt_endpoint_is_admitted_only_for_source_revision() {
        struct CountingPort(AtomicU64);
        #[async_trait]
        impl AdoptionPort for CountingPort {
            async fn adopt_knowledge_entry(
                &self,
                _principal: &Principal,
                _scope: &Scope,
                request: &KnowledgeAdoptRequest,
            ) -> SpokeResult<KnowledgeAdoptResponse> {
                self.0.fetch_add(1, AtomicOrdering::SeqCst);
                assert_eq!(request.source_revision, 1 << 53);
                spoke_ok(
                    serde_json::from_value(json!({
                        "outcome": "adopted",
                        "entry": served_entry(1 << 53, 1),
                    }))
                    .expect("response decodes"),
                )
            }
        }
        let port = Arc::new(CountingPort(AtomicU64::new(0)));
        let service = AdoptionPortService::new(
            Arc::clone(&port) as Arc<dyn AdoptionPort + Send + Sync>,
            admission(scope("spc_local"), principal("prn_peer-a")),
        );
        let payload = |extensions: Option<Value>| {
            let mut payload = adopt_payload(extensions);
            payload["source_scope_id"] = json!("spc_local");
            payload["destination_scope_id"] = json!("spc_local");
            payload
        };

        for (index, spelling) in [
            json!(9_007_199_254_740_992u64),
            json!(9_007_199_254_740_992.0),
        ]
        .into_iter()
        .enumerate()
        {
            let mut extensions_at_endpoint = payload(Some(json!({
                "ns": { "value": spelling },
            })));
            extensions_at_endpoint["idempotency_key"] = json!(format!("key-ext-{index}"));
            let refused = service
                .invoke("peer-a", KNOWLEDGE_OP_ADOPT, extensions_at_endpoint)
                .await;
            assert!(
                matches!(
                    refused,
                    SpokeResult::Reject(SpokeReject {
                        code: SpokeRejectCode::InvalidInput,
                        ..
                    })
                ),
                "an extensions {spelling} at 2^53 must refuse: {refused:?}"
            );
            assert_eq!(port.0.load(AtomicOrdering::SeqCst), 0);
        }

        let mut revision_at_endpoint = payload(None);
        revision_at_endpoint["source_revision"] = json!(9_007_199_254_740_992u64);
        let served = service
            .invoke("peer-a", KNOWLEDGE_OP_ADOPT, revision_at_endpoint)
            .await;
        assert!(matches!(served, SpokeResult::Ok(_)), "{served:?}");
        assert_eq!(port.0.load(AtomicOrdering::SeqCst), 1);
    }

    /// E1/E2 fixtures: a provider result factory with an overridable
    /// anchor `source_revision`, `entry.revision` and response
    /// `extensions`.
    fn served_entry(anchor_revision: u64, entry_revision: u64) -> Value {
        json!({
            "schema_version": 1,
            "entry_id": "ke_dst",
            "entry_type": "note",
            "canonical_name": "copy",
            "status": "confirmed",
            "revision": entry_revision,
            "body": {},
            "adoption_anchor": {
                "schema_version": 1,
                "source_scope_id": "spc_local",
                "source_entry_id": "ke_src",
                "source_revision": anchor_revision,
                "extensions": {},
            },
            "extensions": {},
        })
    }

    /// Contract E1 — revision-typed payload members live in the
    /// interoperable integer domain `[0, 2^53]` on the wire: the served
    /// adopt result refuses an anchor `source_revision` or
    /// `entry.revision` beyond it (TS parity: the deep-check bound).
    /// Contract E2 — a producer-supplied response `extensions` member
    /// survives verbatim.
    #[tokio::test]
    async fn served_result_enforces_e1_revision_domain_and_preserves_extensions() {
        struct ServedPort(Value, bool);
        #[async_trait]
        impl AdoptionPort for ServedPort {
            async fn adopt_knowledge_entry(
                &self,
                _principal: &Principal,
                _scope: &Scope,
                _request: &KnowledgeAdoptRequest,
            ) -> SpokeResult<KnowledgeAdoptResponse> {
                let mut response = json!({
                    "outcome": "adopted",
                    "entry": self.0.clone(),
                });
                if self.1 {
                    response["extensions"] = json!({ "product": { "v": 1 } });
                }
                spoke_ok(
                    serde_json::from_value(response).expect("response decodes"),
                )
            }
        }
        let admission = || admission(scope("spc_local"), principal("prn_peer-a"));
        let payload = || {
            let mut payload = adopt_payload(None);
            payload["source_scope_id"] = json!("spc_local");
            payload["destination_scope_id"] = json!("spc_local");
            payload
        };

        // The inclusive endpoint admits on the served path. An
        // out-of-domain provider revision never decodes into the typed
        // carrier, so it cannot reach the port's caller at all.
        for (anchor_revision, entry_revision, served_ok) in [
            (9007199254740992_u64, 1_u64, true),
            (9007199254740994_u64, 1_u64, false),
            (1_u64, 9007199254740992_u64, true),
            (1_u64, 9007199254740994_u64, false),
        ] {
            if !served_ok {
                assert!(
                    serde_json::from_value::<KnowledgeAdoptResponse>(json!({
                        "outcome": "adopted",
                        "entry": served_entry(anchor_revision, entry_revision),
                    }))
                    .is_err(),
                    "anchor_revision={anchor_revision} entry_revision={entry_revision}: out-of-domain must not decode"
                );
                continue;
            }
            let service = AdoptionPortService::new(
                Arc::new(ServedPort(served_entry(anchor_revision, entry_revision), false)),
                admission(),
            );
            let result = service.invoke("peer-a", KNOWLEDGE_OP_ADOPT, payload()).await;
            assert!(matches!(result, SpokeResult::Ok(_)), "{result:?}");
        }

        // E2: a producer-supplied `extensions` member rides the served
        // response verbatim — never dropped (absent ≡ present-empty, but
        // present stays present).
        for with_extensions in [true, false] {
            let service = AdoptionPortService::new(
                Arc::new(ServedPort(served_entry(3, 1), with_extensions)),
                admission(),
            );
            let result = service.invoke("peer-a", KNOWLEDGE_OP_ADOPT, payload()).await;
            let SpokeResult::Ok(value) = result else {
                panic!("a valid provider result must serve: {result:?}")
            };
            if with_extensions {
                assert_eq!(
                    value["extensions"],
                    json!({ "product": { "v": 1 } }),
                    "supplied extensions must survive verbatim: {value}"
                );
            } else {
                assert!(
                    value.get("extensions").is_none()
                        || value["extensions"] == json!({}),
                    "absent ≡ present-empty is conformant: {value}"
                );
            }
        }
    }

    /// qc1 F-004 / qc3 F-021/F-023 — a supplied response `extensions`
    /// member must satisfy the shared `ExtensionMap` rule on BOTH
    /// outcome branches, and a producer refusal envelope
    /// (`OneOf1 {error, extensions}`) serves verbatim.
    #[tokio::test]
    async fn served_result_validates_extensions_and_serves_the_failure_branch() {
        struct FixedPort(Value);
        #[async_trait]
        impl AdoptionPort for FixedPort {
            async fn adopt_knowledge_entry(
                &self,
                _principal: &Principal,
                _scope: &Scope,
                _request: &KnowledgeAdoptRequest,
            ) -> SpokeResult<KnowledgeAdoptResponse> {
                spoke_ok(
                    serde_json::from_value(self.0.clone()).expect("response decodes"),
                )
            }
        }
        let admission = || admission(scope("spc_local"), principal("prn_peer-a"));
        let payload = || {
            let mut payload = adopt_payload(None);
            payload["source_scope_id"] = json!("spc_local");
            payload["destination_scope_id"] = json!("spc_local");
            payload
        };
        let serve = |response: Value| async move {
            AdoptionPortService::new(Arc::new(FixedPort(response)), admission())
                .invoke("peer-a", KNOWLEDGE_OP_ADOPT, payload())
                .await
        };

        // A failure branch with a supplied `extensions` member serves
        // verbatim (E3 + E2 parity with the success branch).
        let refusal = serve(json!({
            "error": {
                "code": "REVISION_CONFLICT",
                "message": "conflict",
                "details": {},
                "extensions": {},
            },
            "extensions": { "product": { "v": 1 } },
        }))
        .await;
        let SpokeResult::Ok(value) = refusal else {
            panic!("a valid failure branch must serve: {refusal:?}")
        };
        assert_eq!(value["extensions"], json!({ "product": { "v": 1 } }));
        assert!(value.get("error").is_some());

        // qc3 F-021 / qc3-05 — a revision-typed member inside failure
        // `details` obeys the `[0, 2^53]` domain on its exact integer
        // representation: an exact out-of-domain integer refuses
        // `INTERNAL_ERROR`, an integral-float spelling normalizes to its
        // parsed integer, and an ordinary detail rides verbatim.
        let verbatim_details = serve(json!({
            "error": {
                "code": "REVISION_CONFLICT",
                "message": "conflict",
                "details": { "adopted_revision": 7 },
                "extensions": { "product": { "v": 1 } },
            },
        }))
        .await;
        let SpokeResult::Ok(value) = verbatim_details else {
            panic!("valid details must serve: {verbatim_details:?}")
        };
        assert_eq!(value["error"]["details"], json!({ "adopted_revision": 7 }));
        assert_eq!(
            value["error"]["extensions"],
            json!({ "product": { "v": 1 } }),
            "error-envelope extensions ride verbatim"
        );

        let normalized_details = serve(json!({
            "error": {
                "code": "REVISION_CONFLICT",
                "message": "conflict",
                "details": { "adopted_revision": 9_007_199_254_740_992.0 },
                "extensions": {},
            },
        }))
        .await;
        let SpokeResult::Ok(value) = normalized_details else {
            panic!("an in-domain details spelling must serve: {normalized_details:?}")
        };
        assert_eq!(
            value["error"]["details"]["adopted_revision"],
            json!(9007199254740992_u64),
            "an integral-float adopted_revision normalizes to its parsed integer"
        );

        let out_of_domain_details = serve(json!({
            "error": {
                "code": "REVISION_CONFLICT",
                "message": "conflict",
                "details": { "adopted_revision": 9007199254740993_u64 },
                "extensions": {},
            },
        }))
        .await;
        let SpokeResult::Reject(reject) = out_of_domain_details else {
            panic!("out-of-domain adopted_revision must refuse: {out_of_domain_details:?}")
        };
        assert_eq!(reject.code, SpokeRejectCode::InternalError);
        assert!(reject.message.contains("adopted_revision outside [0, 2^53]"));

        // An out-of-domain exact revision never decodes into the typed
        // carrier (`9007199254740993` is beyond the `[0, 2^53]` domain),
        // so the serving guard never sees one from a JSON provider
        // answer; the `details.adopted_revision` receipt above is opaque
        // JSON and keeps the f64-observed normalization.
        assert!(
            serde_json::from_value::<KnowledgeAdoptResponse>(json!({
                "outcome": "adopted",
                "entry": served_entry(9007199254740993, 9007199254740993),
            }))
            .is_err(),
            "an out-of-domain entry revision must not decode"
        );
    }

    /// qc1 F-019/F-020/F-029, qc2 QC2-19, qc3 F-024 — parity pin for the
    /// TS `validServedAdoptResult` envelope rules: the typed
    /// `KnowledgeAdoptResponse` decode is the Rust trust boundary, so a
    /// mixed envelope (`entry`/`outcome` alongside `error`), an unknown
    /// TOP-LEVEL member on either branch, a non-object `details`, an
    /// unknown member inside `error`, and a missing required `extensions`
    /// all refuse before the served validator runs. TS mirrors each rule
    /// on the raw provider object (a JS provider can hand it any shape).
    #[test]
    fn adopt_response_typed_decode_refuses_malformed_envelopes() {
        let entry = served_entry(2, 1);
        for bad in [
            json!({
                "outcome": "adopted",
                "entry": entry,
                "error": { "code": "REVISION_CONFLICT", "message": "x", "extensions": {} },
            }),
            // unknown TOP-LEVEL member on the success branch
            json!({ "outcome": "adopted", "entry": entry, "surprise": true }),
            // `outcome` outside the closed vocabulary
            json!({ "outcome": "unexpected", "entry": entry }),
            // success `extensions` must be an ExtensionMap
            json!({ "outcome": "adopted", "entry": entry, "extensions": [] }),
            // unknown TOP-LEVEL member on the failure branch
            json!({
                "error": { "code": "REVISION_CONFLICT", "message": "x", "extensions": {} },
                "surprise": true,
            }),
            json!({
                "error": {
                    "code": "REVISION_CONFLICT",
                    "message": "x",
                    "details": 5,
                    "extensions": {},
                },
            }),
            json!({
                "error": {
                    "code": "REVISION_CONFLICT",
                    "message": "x",
                    "extensions": {},
                    "surprise": true,
                },
            }),
            json!({ "error": { "code": "REVISION_CONFLICT", "message": "x" } }),
        ] {
            assert!(
                serde_json::from_value::<KnowledgeAdoptResponse>(bad.clone()).is_err(),
                "typed decode must refuse {bad}"
            );
        }
    }

    /// Dispute pin (qc1 F-013 residual) — an out-of-domain provider
    /// revision refuses `INTERNAL_ERROR` in EVERY revision slot, and the
    /// normalizing rewrite never runs before the bound: `2^53 + 2`
    /// (f64-exact at this magnitude) is never rounded down into the
    /// domain. The third slot rides a provider refusal receipt
    /// (`details.adopted_revision`), proving the ordinary
    /// refusal/failure path validates its revision member too.
    #[tokio::test]
    async fn out_of_domain_provider_revision_refuses_in_every_slot() {
        struct FixedPort(Value);
        #[async_trait]
        impl AdoptionPort for FixedPort {
            async fn adopt_knowledge_entry(
                &self,
                _principal: &Principal,
                _scope: &Scope,
                _request: &KnowledgeAdoptRequest,
            ) -> SpokeResult<KnowledgeAdoptResponse> {
                spoke_ok(serde_json::from_value(self.0.clone()).expect("response decodes"))
            }
        }
        let admission = || admission(scope("spc_local"), principal("prn_peer-a"));
        let payload = || {
            let mut payload = adopt_payload(None);
            payload["source_scope_id"] = json!("spc_local");
            payload["destination_scope_id"] = json!("spc_local");
            payload
        };
        let serve = |response: Value| async move {
            AdoptionPortService::new(Arc::new(FixedPort(response)), admission())
                .invoke("peer-a", KNOWLEDGE_OP_ADOPT, payload())
                .await
        };
        let assert_refused = |result: SpokeResult<Value>, slot: &str| {
            let SpokeResult::Reject(reject) = result else {
                panic!("{slot} outside [0, 2^53] must refuse: {result:?}")
            };
            assert_eq!(reject.code, SpokeRejectCode::InternalError, "{slot}");
            assert!(
                reject.message.contains("outside [0, 2^53]"),
                "{slot}: {reject:?}"
            );
        };

        // 2^53 + 2 — f64-exact, one step beyond the inclusive endpoint.
        const OUT: u64 = 9007199254740994;

        // An out-of-domain entry revision never decodes into the typed
        // carrier, so the typed decode is the refusal for both entry
        // slots.
        for entry in [served_entry(1, OUT), served_entry(OUT, 1)] {
            assert!(
                serde_json::from_value::<KnowledgeAdoptResponse>(json!({
                    "outcome": "adopted",
                    "entry": entry,
                }))
                .is_err(),
                "an out-of-domain provider entry must not decode"
            );
        }
        // The refusal receipt is opaque `details` JSON, so the serving
        // guard is what refuses it.
        assert_refused(
            serve(json!({
                "error": {
                    "code": "REVISION_CONFLICT",
                    "message": "conflict",
                    "details": { "adopted_revision": OUT },
                    "extensions": {},
                },
            }))
            .await,
            "error.details.adopted_revision",
        );
    }

    /// qc2 QC2-21 (F-030) — the client decode path (`invoke_mapped`'s
    /// success payload branch) accepts a served failure envelope as
    /// `Variant1`: a wire-valid `{error, extensions?}` payload is a
    /// successful port outcome, so the client returns it verbatim (TS
    /// parity: `isAdoptFailurePayload` in the adapter's payload gate).
    #[test]
    fn served_failure_payload_decodes_on_the_client_path() {
        let decoded = serde_json::from_value::<KnowledgeAdoptResponse>(json!({
            "error": {
                "code": "REVISION_CONFLICT",
                "message": "conflict",
                "details": { "adopted_revision": 7 },
                "extensions": { "product": { "v": 1 } },
            },
            "extensions": { "transport": { "trace": "t-1" } },
        }))
        .expect("a wire-valid failure envelope decodes as Variant1");
        assert!(matches!(decoded, KnowledgeAdoptResponse::Variant1 { .. }));
    }

    /// qc1 F-033 — parity pin for the TS client gate (`isValidAdoptSuccessPayload`):
    /// the typed `KnowledgeAdoptResponse` decode refuses a success entry
    /// without `adoption_anchor`, so the shallow `{outcome, entry}` shape
    /// alone is never a valid success payload.
    #[test]
    fn adopt_success_entry_requires_adoption_anchor() {
        let mut unanchored = served_entry(2, 1);
        unanchored
            .as_object_mut()
            .expect("entry is an object")
            .remove("adoption_anchor");
        assert!(
            serde_json::from_value::<KnowledgeAdoptResponse>(json!({
                "outcome": "adopted",
                "entry": unanchored,
            }))
            .is_err(),
            "a success entry without adoption_anchor must not decode"
        );
        assert!(
            serde_json::from_value::<KnowledgeAdoptResponse>(json!({
                "outcome": "adopted",
                "entry": served_entry(2, 1),
            }))
            .is_ok(),
            "a complete anchored entry decodes"
        );
    }

    /// qc1 F-031 / qc3-05 — a provider Reject receipt's
    /// `details.adopted_revision` obeys the `[0, 2^53]` domain compared on
    /// its exact integer representation: an exact out-of-domain integer
    /// settles as a provider-fault `INTERNAL_ERROR`, an in-domain value
    /// (integer or integral float) serves normalized to its integer.
    #[tokio::test]
    async fn served_reject_receipt_enforces_the_details_revision_domain() {
        struct RejectPort(SpokeReject);
        #[async_trait]
        impl AdoptionPort for RejectPort {
            async fn adopt_knowledge_entry(
                &self,
                _principal: &Principal,
                _scope: &Scope,
                _request: &KnowledgeAdoptRequest,
            ) -> SpokeResult<KnowledgeAdoptResponse> {
                SpokeResult::Reject(self.0.clone())
            }
        }
        let admission = || admission(scope("spc_local"), principal("prn_peer-a"));
        let payload = || {
            let mut payload = adopt_payload(None);
            payload["source_scope_id"] = json!("spc_local");
            payload["destination_scope_id"] = json!("spc_local");
            payload
        };
        let serve = |adopted_revision: Value| async move {
            let mut details = Map::new();
            details.insert("adopted_revision".into(), adopted_revision);
            AdoptionPortService::new(
                Arc::new(RejectPort(SpokeReject {
                    code: SpokeRejectCode::RevisionConflict,
                    message: "conflict".into(),
                    details: Some(details),
                })),
                admission(),
            )
            .invoke("peer-a", KNOWLEDGE_OP_ADOPT, payload())
            .await
        };

        for (spelling, served_as, expected) in [
            (json!(7), SpokeRejectCode::RevisionConflict, json!(7)),
            (
                json!(9007199254740992_u64),
                SpokeRejectCode::RevisionConflict,
                json!(9007199254740992_u64),
            ),
            (
                json!(9_007_199_254_740_992.0),
                SpokeRejectCode::RevisionConflict,
                json!(9007199254740992_u64),
            ),
            (
                json!(9007199254740993_u64),
                SpokeRejectCode::InternalError,
                // An exact integer past the endpoint is never rounded in.
                json!(null),
            ),
            (
                json!(9007199254740994_u64),
                SpokeRejectCode::InternalError,
                // The provider fault names the domain; no receipt travels.
                json!(null),
            ),
        ] {
            let result = serve(spelling.clone()).await;
            let SpokeResult::Reject(reject) = result else {
                panic!("adopted_revision={spelling}: receipt must reject: {result:?}")
            };
            assert_eq!(reject.code, served_as, "adopted_revision={spelling}");
            let details = reject.details.expect("details survive");
            if served_as == SpokeRejectCode::InternalError {
                assert_eq!(details.get("kind"), Some(&json!("transport")));
                assert!(
                    !details.contains_key("adopted_revision"),
                    "a provider fault carries no receipt: {details:?}"
                );
            } else {
                assert_eq!(
                    details.get("adopted_revision"),
                    Some(&expected),
                    "adopted_revision={spelling}"
                );
            }
        }
    }

    /// F-007 fixture: a lookup/record-only dedup store whose every
    /// lookup reports `Unseen` — the pre-single-flight surface that
    /// inherits the default `claim_key`. `lookups` records when the
    /// dedup probe ran relative to provider invocations.
    struct RecordingDedupStore {
        lookups: Arc<AtomicU64>,
    }

    #[async_trait]
    impl HistoryDedupStore for RecordingDedupStore {
        async fn lookup(&self, _key: &str) -> HistoryDedupLookup {
            self.lookups.fetch_add(1, AtomicOrdering::SeqCst);
            HistoryDedupLookup::Unseen
        }

        async fn record(&self, _key: &str, _record: HistoryDedupRecord) {}
    }

    /// A valid `knowledge.adopt` provider success for the F-007
    /// single-flight tests — a well-formed anchored entry per B6.
    fn adopted_success() -> KnowledgeAdoptResponse {
        serde_json::from_value(json!({
            "outcome": "adopted",
            "entry": {
                "schema_version": 1,
                "entry_id": "ke_dst",
                "entry_type": "note",
                "canonical_name": "copy",
                "status": "confirmed",
                "revision": 1,
                "body": {},
                "adoption_anchor": {
                    "schema_version": 1,
                    "source_scope_id": "dst-scope",
                    "source_entry_id": "src-entry",
                    "source_revision": 3,
                    "extensions": {},
                },
                "extensions": {},
            },
        }))
        .expect("response decodes")
    }

    /// F-007 default-claim exclusion: two services sharing ONE
    /// lookup/record-only store serialize the same retry identity
    /// across lookup → provider → record (the second call's dedup
    /// probe cannot run while the first is in flight), while an
    /// unrelated key proceeds concurrently (qc1 F-007).
    #[tokio::test]
    async fn default_claim_serializes_same_key_across_services() {
        struct GatedPort {
            calls: AtomicU64,
            in_flight: AtomicU64,
            max_in_flight: AtomicU64,
            gate: tokio::sync::Notify,
        }
        #[async_trait]
        impl AdoptionPort for GatedPort {
            async fn adopt_knowledge_entry(
                &self,
                _principal: &Principal,
                _scope: &Scope,
                _request: &KnowledgeAdoptRequest,
            ) -> SpokeResult<KnowledgeAdoptResponse> {
                self.calls.fetch_add(1, AtomicOrdering::SeqCst);
                let depth = self.in_flight.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                self.max_in_flight.fetch_max(depth, AtomicOrdering::SeqCst);
                self.gate.notified().await;
                self.in_flight.fetch_sub(1, AtomicOrdering::SeqCst);
                spoke_ok(adopted_success())
            }
        }
        let lookups = Arc::new(AtomicU64::new(0));
        let dedup: Arc<dyn HistoryDedupStore> = Arc::new(RecordingDedupStore {
            lookups: Arc::clone(&lookups),
        });
        let port_a = Arc::new(GatedPort {
            calls: AtomicU64::new(0),
            in_flight: AtomicU64::new(0),
            max_in_flight: AtomicU64::new(0),
            gate: tokio::sync::Notify::new(),
        });
        let port_b = Arc::new(GatedPort {
            calls: AtomicU64::new(0),
            in_flight: AtomicU64::new(0),
            max_in_flight: AtomicU64::new(0),
            gate: tokio::sync::Notify::new(),
        });
        let service_a = Arc::new(
            AdoptionPortService::new(
                Arc::clone(&port_a) as Arc<dyn AdoptionPort + Send + Sync>,
                admission(scope("dst-scope"), principal("prn_peer-a")),
            )
            .with_dedup_store(Arc::clone(&dedup)),
        );
        let service_b = Arc::new(
            AdoptionPortService::new(
                Arc::clone(&port_b) as Arc<dyn AdoptionPort + Send + Sync>,
                admission(scope("dst-scope"), principal("prn_peer-a")),
            )
            .with_dedup_store(dedup),
        );
        // Service A serves the same retry identity twice; service B
        // shares the store and must wait on A's claim. An unrelated key
        // on B proceeds concurrently.
        let first = tokio::spawn({
            let service = Arc::clone(&service_a);
            async move {
                service
                    .invoke("peer-a", KNOWLEDGE_OP_ADOPT, adopt_payload(None))
                    .await
            }
        });
        let second = tokio::spawn({
            let service = Arc::clone(&service_b);
            async move {
                service
                    .invoke("peer-a", KNOWLEDGE_OP_ADOPT, adopt_payload(None))
                    .await
            }
        });
        let mut payload_k2 = adopt_payload(None);
        payload_k2["idempotency_key"] = json!("key-2");
        let third = tokio::spawn({
            let service = Arc::clone(&service_a);
            async move {
                service
                    .invoke("peer-a", KNOWLEDGE_OP_ADOPT, payload_k2)
                    .await
            }
        });
        // Wait until the first same-key call and the unrelated call are
        // inside their provider gates: the unrelated key reaches its
        // port while the same-key replay is still claimed.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while port_a.calls.load(AtomicOrdering::SeqCst) < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the first same-key call and the unrelated key reach the provider");
        // The second same-key call waits on the default claim — its
        // dedup lookup has not run (only the first and unrelated calls
        // probed) and its provider never invoked.
        assert_eq!(
            lookups.load(AtomicOrdering::SeqCst),
            2,
            "the second same-key lookup is held behind the default claim"
        );
        assert_eq!(port_b.calls.load(AtomicOrdering::SeqCst), 0);
        // Release A's held call; the unrelated call releases too; the
        // claimed second call then serializes behind the released claim.
        port_a.gate.notify_waiters();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while port_b.calls.load(AtomicOrdering::SeqCst) == 0 {
                tokio::task::yield_now().await;
                port_b.gate.notify_one();
            }
        })
        .await
        .expect("the claimed second call reaches its provider after release");
        port_b.gate.notify_waiters();
        let (first, second, third) = tokio::join!(first, second, third);
        for (name, result) in [
            ("first", first),
            ("second", second),
            ("third", third),
        ] {
            let result = result.expect("task completes");
            assert!(
                matches!(result, SpokeResult::Ok(_)),
                "{name} must succeed: {result:?}"
            );
        }
        // The same-key second call ran its provider AFTER the first
        // released the claim — serialized, never overlapped.
        assert_eq!(port_a.calls.load(AtomicOrdering::SeqCst), 2);
        assert_eq!(port_b.calls.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(
            port_a.max_in_flight.load(AtomicOrdering::SeqCst)
                .max(port_b.max_in_flight.load(AtomicOrdering::SeqCst)),
            2,
            "only the unrelated key overlapped — same-key calls never share a window"
        );
    }

    /// F-007 unsettled retry: a caller whose invoke is cancelled while
    /// its provider call is in flight releases the default claim — the
    /// next same-key call invokes the provider again (B3: an unsettled
    /// invocation frees the retry identity).
    #[tokio::test]
    async fn default_claim_releases_on_an_unsettled_invocation() {
        struct BlockingPort {
            calls: AtomicU64,
            gate: tokio::sync::Notify,
        }
        #[async_trait]
        impl AdoptionPort for BlockingPort {
            async fn adopt_knowledge_entry(
                &self,
                _principal: &Principal,
                _scope: &Scope,
                _request: &KnowledgeAdoptRequest,
            ) -> SpokeResult<KnowledgeAdoptResponse> {
                self.calls.fetch_add(1, AtomicOrdering::SeqCst);
                self.gate.notified().await;
                spoke_ok(adopted_success())
            }
        }
        let port = Arc::new(BlockingPort {
            calls: AtomicU64::new(0),
            gate: tokio::sync::Notify::new(),
        });
        let service = Arc::new(
            AdoptionPortService::new(
                Arc::clone(&port) as Arc<dyn AdoptionPort + Send + Sync>,
                admission(scope("dst-scope"), principal("prn_peer-a")),
            )
            .with_dedup_store(Arc::new(RecordingDedupStore {
                lookups: Arc::new(AtomicU64::new(0)),
            })),
        );
        // A cancelled caller: the spawned invoke is aborted mid-provider
        // — the unsettled outcome drops the claim guard.
        let handle = tokio::spawn({
            let service = Arc::clone(&service);
            async move {
                service
                    .invoke("peer-a", KNOWLEDGE_OP_ADOPT, adopt_payload(None))
                    .await
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while port.calls.load(AtomicOrdering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the first provider call starts");
        handle.abort();
        let _ = handle.await;
        // The retry identity is free again: a fresh same-key invoke
        // claims it and reaches the provider — no stale-claim deadlock.
        let retry = tokio::spawn({
            let service = Arc::clone(&service);
            async move {
                service
                    .invoke("peer-a", KNOWLEDGE_OP_ADOPT, adopt_payload(None))
                    .await
            }
        });
        // Release each provider call as it arrives; the retry must
        // reach the port for this loop to exit.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while port.calls.load(AtomicOrdering::SeqCst) < 2 {
                if tokio::time::timeout(
                    std::time::Duration::from_millis(50),
                    port.gate.notified(),
                )
                .await
                .is_err()
                {
                    port.gate.notify_one();
                }
            }
        })
        .await
        .expect("the released claim must not deadlock the retry");
        port.gate.notify_waiters();
        let result = retry.await.expect("task completes");
        assert!(matches!(result, SpokeResult::Ok(_)), "{result:?}");
        assert_eq!(port.calls.load(AtomicOrdering::SeqCst), 2);
    }
}
