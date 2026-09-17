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
//! [`RemoteServePorts::as_extract`]; it is a service, not a port method.
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

use std::sync::Arc;

use async_trait::async_trait;
use spoke_operations::{
    BaselinePorts, ComputablePort, FindingPort, ForkTimelineQueryPort, HostManifestPort,
    KnowledgeEntryPort, RelationPort, RuleQueryPort, ScopeQueryPort, SpokeResult,
};
use spoke_schemas::{
    ExtractRequest, ExtractResponse, Finding, HostCapabilityManifest, KnowledgeEntry, Relation,
    Rule, Scope, TimelineEvent,
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
        }
    }

    /// Attach the optional `extract` service face (consuming builder).
    #[must_use]
    pub fn with_extract(mut self, extract: Arc<dyn RemoteExtractService>) -> Self {
        self.extract = Some(extract);
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
}
