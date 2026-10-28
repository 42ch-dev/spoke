//! Responder ports face — the D4 catalogue seam `connect_responder` serves
//! `port.*` ops through.
//!
//! [`RemoteServePorts`] carries the baseline six families (via the
//! `BaselinePorts` supertrait) plus the optional l2-computable / l5-fork
//! faces, probed with [`RemoteServePorts::as_computable`] /
//! [`RemoteServePorts::as_fork_timeline`] — the operations-crate
//! dyn-availability probe convention (`ComputablePorts` / `ForkPorts`). The
//! core `extract` op is served through the separate optional
//! [`RemoteExtractService`] face, probed with
//! [`RemoteServePorts::as_extract`]; it is a service, not a port method. The
//! memory family is served the same way through the optional
//! [`RemoteMemoryService`] face ([`RemoteServePorts::as_memory`]).
//! Serving order is gate → probe → serve/deny: after the capability gate
//! passes, the responder probes the injected face; a `None` probe means the
//! host declared a family it does not provide (host misconfiguration) and
//! answers the existing `op_unsupported` dispatch-deny branch.
//!
//! Composition: full providers (`BaselinePorts + ComputablePort +
//! ForkTimelineQueryPort`) are covered by the blanket impl; baseline-only
//! and mixed hosts compose via [`RemoteServePortsComposite::new`] without
//! manual trait impls, opting into extraction with
//! [`RemoteServePortsComposite::with_extract`].

use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use spoke_operations::{
    spoke_reject, BaselinePorts, ComputablePort, FindingPort, ForkTimelineQueryPort,
    HostManifestPort, KnowledgeEntryPort, MemoryPort, RelationPort, RuleQueryPort, ScopeQueryPort,
    SpokeRejectCode, SpokeResult,
};
use spoke_schemas::{
    ExtractRequest, ExtractResponse, Finding, HostCapabilityManifest, KnowledgeEntry, Principal,
    Relation, Rule, Scope, TimelineEvent,
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
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                format!("invalid memory payload: {error}"),
                None,
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

/// Responder ports face (D4 catalogue): baseline six + optional families,
/// capability-gated. `Option` probes mirror the operations-crate
/// `ComputablePorts` / `ForkPorts` availability convention.
pub trait RemoteServePorts: BaselinePorts {
    /// Optional l2-computable face (project / compute), when provided.
    fn as_computable(&self) -> Option<&dyn ComputablePort>;

    /// Optional l5-fork timeline face, when provided.
    fn as_fork_timeline(&self) -> Option<&dyn ForkTimelineQueryPort>;

    /// Optional `extract` service face, when provided. `None` is genuine
    /// optional absence (a host that does not serve extraction), not a stub
    /// service — the responder answers the dispatch-deny branch.
    fn as_extract(&self) -> Option<&dyn RemoteExtractService>;

    /// Optional memory service face, when provided. Absent means the host
    /// declared `spoke-memory` without providing the service — the responder
    /// answers the dispatch-deny branch.
    ///
    /// Defaults to `None`, so adding this optional family stays additive:
    /// every existing `RemoteServePorts` implementation compiles unchanged
    /// and hosts opt in by overriding it (as
    /// [`RemoteServePortsComposite::with_memory`] does).
    fn as_memory(&self) -> Option<&dyn RemoteMemoryService> {
        None
    }
}

/// Blanket impl: a provider carrying the baseline six + both optional faces
/// serves every family through the responder (the probes return the faces).
/// The full-optional surface does **not** imply extraction: a provider opts
/// into the `extract` service explicitly through
/// [`RemoteServePortsComposite::with_extract`].
impl<T> RemoteServePorts for T
where
    T: BaselinePorts + ComputablePort + ForkTimelineQueryPort,
{
    fn as_computable(&self) -> Option<&dyn ComputablePort> {
        Some(self)
    }

    fn as_fork_timeline(&self) -> Option<&dyn ForkTimelineQueryPort> {
        Some(self)
    }

    fn as_extract(&self) -> Option<&dyn RemoteExtractService> {
        None
    }
}

/// Composite responder ports face: a baseline provider plus optional
/// computable / fork / extract faces (each may be absent). Baseline-only and
/// mixed hosts compose via [`RemoteServePortsComposite::new`] without manual
/// trait impls; extraction is added with the consuming
/// [`RemoteServePortsComposite::with_extract`].
pub struct RemoteServePortsComposite {
    baseline: Arc<dyn BaselinePorts + Send + Sync>,
    computable: Option<Arc<dyn ComputablePort + Send + Sync>>,
    fork: Option<Arc<dyn ForkTimelineQueryPort + Send + Sync>>,
    extract: Option<Arc<dyn RemoteExtractService>>,
    memory: Option<Arc<dyn RemoteMemoryService>>,
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
        Self {
            baseline,
            computable,
            fork,
            extract: None,
            memory: None,
        }
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
}

#[async_trait]
impl KnowledgeEntryPort for RemoteServePortsComposite {
    async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
        self.baseline.get_knowledge_entry(entry_id).await
    }

    async fn put_knowledge_entry(
        &self,
        entry: KnowledgeEntry,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry> {
        self.baseline
            .put_knowledge_entry(entry, expected_base_revision)
            .await
    }
}

#[async_trait]
impl RelationPort for RemoteServePortsComposite {
    async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
        self.baseline.get_relation(relation_id).await
    }

    async fn put_relation(
        &self,
        relation: Relation,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<Relation> {
        self.baseline
            .put_relation(relation, expected_base_revision)
            .await
    }
}

#[async_trait]
impl ScopeQueryPort for RemoteServePortsComposite {
    async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
        self.baseline.list_knowledge_entries(scope).await
    }

    async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
        self.baseline.list_timeline_events(scope).await
    }
}

#[async_trait]
impl FindingPort for RemoteServePortsComposite {
    async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
        self.baseline.put_findings(findings).await
    }
}

#[async_trait]
impl RuleQueryPort for RemoteServePortsComposite {
    async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
        self.baseline.list_rules(rule_refs).await
    }
}

#[async_trait]
impl HostManifestPort for RemoteServePortsComposite {
    async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest> {
        self.baseline.get_host_capability_manifest().await
    }

    async fn list_peer_host_capability_manifests(
        &self,
    ) -> SpokeResult<Vec<HostCapabilityManifest>> {
        self.baseline.list_peer_host_capability_manifests().await
    }
}

impl RemoteServePorts for RemoteServePortsComposite {
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

    fn as_memory(&self) -> Option<&dyn RemoteMemoryService> {
        self.memory
            .as_ref()
            .map(|memory| memory.as_ref() as &dyn RemoteMemoryService)
    }
}
