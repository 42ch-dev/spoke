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
//! `spoke-history` family probes the injected [`HistoryPort`] directly
//! through [`RemoteServePorts::as_history`].
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

use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use spoke_operations::{
    spoke_reject, BaselinePorts, ComputablePort, ForkTimelineQueryPort, HistoryPort, MemoryPort,
    ScopedKnowledgePort, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::{ExtractRequest, ExtractResponse, Principal, Scope};

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

    /// Optional `spoke-history` face (revision replay + explicit pruning),
    /// when provided. Absent means the host declared `spoke-history`
    /// without providing the port — the responder answers the dispatch-deny
    /// branch.
    ///
    /// Owned (not borrowed) like [`RemoteServePorts::as_memory`]: the prune
    /// op mutates, so the responder may detach the admitted invoke from its
    /// response deadline and the spawned task must own the port.
    fn as_history(&self) -> Option<Arc<dyn HistoryPort + Send + Sync>> {
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
    history: Option<Arc<dyn HistoryPort + Send + Sync>>,
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

    /// Attach the optional `spoke-history` port face (consuming builder).
    #[must_use]
    pub fn with_history(mut self, history: Arc<dyn HistoryPort + Send + Sync>) -> Self {
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

    fn as_history(&self) -> Option<Arc<dyn HistoryPort + Send + Sync>> {
        self.history.clone()
    }
}
