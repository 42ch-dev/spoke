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
}
