//! Capability-sliced adapter port contracts for injection orchestration.
//! Async `SpokeResult` surface — port methods return awaitable results; the
//! library stays I/O-free and only awaits injected ports.
//!
//! Port traits use `#[async_trait]` (Send futures) so the dyn probes
//! ([`ComputablePorts::as_computable`], [`ForkPorts::as_fork_timeline`])
//! stay object-safe. No runtime is required by this crate itself.

use async_trait::async_trait;
use crate::result::SpokeResult;
use spoke_schemas::{
    ComputeRequest, ComputeResponse, Finding, HostCapabilityManifest, KnowledgeEntry,
    ProjectRequest, ProjectResponse, Relation, Rule, Scope, TimelineEvent,
};

/// Knowledge entry persistence — get / put by entry id.
#[async_trait]
pub trait KnowledgeEntryPort {
    async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry>;
    /// Persist a KnowledgeEntry with optimistic concurrency control.
    ///
    /// Adapters MUST treat `expected_base_revision` as the store’s required current
    /// revision before accepting the write (conditional put / OCC / CAS).
    /// `None` means the entry must be absent (create). A non-null value means the
    /// store’s current revision for `entry.entry_id` MUST equal
    /// `expected_base_revision`; otherwise reject with `STORED_REVISION_STALE` or
    /// `REVISION_CONFLICT`. True concurrent safety requires atomic compare-and-put
    /// in the adapter; the library stays I/O-free.
    async fn put_knowledge_entry(
        &self,
        entry: KnowledgeEntry,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry>;
}

/// Relation persistence — get / put by relation id.
#[async_trait]
pub trait RelationPort {
    async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation>;
    /// Persist a Relation with optimistic concurrency control.
    ///
    /// Adapters MUST treat `expected_base_revision` as the store’s required current
    /// revision before accepting the write (conditional put / OCC / CAS).
    /// `None` means the relation must be absent (create). A non-null value means the
    /// store’s current revision for `relation.relation_id` MUST equal
    /// `expected_base_revision`; otherwise reject with `STORED_REVISION_STALE` or
    /// `REVISION_CONFLICT`. True concurrent safety requires atomic compare-and-put
    /// in the adapter; the library stays I/O-free.
    ///
    /// Revision assignment is adapter-owned: on create
    /// (`expected_base_revision` `None`) the adapter MUST seed `revision = 1`;
    /// on an accepted update it MUST persist `revision = stored + 1`. The
    /// returned Relation carries the assigned revision — callers must not set
    /// it themselves.
    async fn put_relation(
        &self,
        relation: Relation,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<Relation>;
}

/// Scope query for check / assemble — knowledge entries and timeline events.
#[async_trait]
pub trait ScopeQueryPort {
    async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>>;
    async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>>;
}

/// Finding persistence.
#[async_trait]
pub trait FindingPort {
    async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>>;
}

/// Rule query by reference list.
#[async_trait]
pub trait RuleQueryPort {
    async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>>;
}

/// Host collaboration metadata — self manifest and product-known peer manifests.
///
/// Integrators call explicitly; orchestrators do not auto-fetch manifests.
#[async_trait]
pub trait HostManifestPort {
    async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest>;
    async fn list_peer_host_capability_manifests(&self) -> SpokeResult<Vec<HostCapabilityManifest>>;
}

/// Optional l2-computable session — project / compute.
#[async_trait]
pub trait ComputablePort {
    async fn project(&self, request: ProjectRequest) -> SpokeResult<ProjectResponse>;
    async fn compute(&self, request: ComputeRequest) -> SpokeResult<ComputeResponse>;
}

/// Optional l5-fork timeline query.
///
/// Capability-specific refinement of [`ScopeQueryPort`]; one type MAY satisfy both.
#[async_trait]
pub trait ForkTimelineQueryPort {
    async fn list_fork_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>>;
}

/// Ports required for spoke-baseline orchestration.
pub trait BaselinePorts:
    KnowledgeEntryPort
    + RelationPort
    + ScopeQueryPort
    + FindingPort
    + RuleQueryPort
    + HostManifestPort
{
}

impl<T> BaselinePorts for T where
    T: KnowledgeEntryPort
        + RelationPort
        + ScopeQueryPort
        + FindingPort
        + RuleQueryPort
        + HostManifestPort
{
}

/// Baseline plus optional computable capability.
///
/// Types that implement [`ComputablePort`] get a blanket availability probe.
/// Baseline-only types MAY implement this trait returning `None` for dynamically
/// assembled boundaries (parity with TS `requirePortMethod`).
pub trait ComputablePorts: BaselinePorts {
    fn as_computable(&self) -> Option<&dyn ComputablePort>;
}

impl<T> ComputablePorts for T
where
    T: BaselinePorts + ComputablePort,
{
    fn as_computable(&self) -> Option<&dyn ComputablePort> {
        Some(self)
    }
}

/// Baseline plus optional fork timeline capability.
pub trait ForkPorts: BaselinePorts {
    fn as_fork_timeline(&self) -> Option<&dyn ForkTimelineQueryPort>;
}

impl<T> ForkPorts for T
where
    T: BaselinePorts + ForkTimelineQueryPort,
{
    fn as_fork_timeline(&self) -> Option<&dyn ForkTimelineQueryPort> {
        Some(self)
    }
}

/// Full composition of baseline, computable, and fork ports.
pub trait FullPorts: ComputablePorts + ForkPorts {}

impl<T> FullPorts for T where T: ComputablePorts + ForkPorts {}

/// Ergonomic marker for baseline adapter composition.
pub trait BaselineAdapter: BaselinePorts {}

impl<T: BaselinePorts> BaselineAdapter for T {}

/// Ergonomic marker for baseline plus computable adapter composition.
pub trait ComputableAdapter: ComputablePorts {}

impl<T: ComputablePorts> ComputableAdapter for T {}

/// Ergonomic marker for baseline plus fork adapter composition.
pub trait ForkAdapter: ForkPorts {}

impl<T: ForkPorts> ForkAdapter for T {}

/// Ergonomic marker for full adapter composition.
pub trait FullAdapter: FullPorts {}

impl<T: FullPorts> FullAdapter for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{spoke_ok, SpokeRejectCode};
    use serde_json::json;
    use std::collections::HashMap;

    fn make_manifest(
        host_id: &str,
        namespaces: &[&str],
        roles: &[&str],
    ) -> HostCapabilityManifest {
        serde_json::from_value(json!({
            "schema_version": 1,
            "host_id": host_id,
            "roles": roles,
            "capabilities": ["spoke-baseline"],
            "namespaces": namespaces,
            "extensions": {}
        }))
        .expect("valid HostCapabilityManifest")
    }

    fn normalize_peer_manifests(
        self_host_id: &str,
        peers: &[HostCapabilityManifest],
    ) -> Vec<HostCapabilityManifest> {
        let mut by_host_id: HashMap<String, HostCapabilityManifest> = HashMap::new();
        for peer in peers {
            if peer.host_id.as_str() == self_host_id {
                continue;
            }
            by_host_id.insert(peer.host_id.as_str().to_string(), peer.clone());
        }
        let mut normalized: Vec<HostCapabilityManifest> = by_host_id.into_values().collect();
        normalized.sort_by(|left, right| left.host_id.as_str().cmp(right.host_id.as_str()));
        normalized
    }

    struct HostManifestPortMock {
        self_manifest: HostCapabilityManifest,
        raw_peers: Vec<HostCapabilityManifest>,
    }

    impl HostManifestPortMock {
        fn new(
            self_manifest: HostCapabilityManifest,
            raw_peers: Vec<HostCapabilityManifest>,
        ) -> Self {
            Self {
                self_manifest,
                raw_peers,
            }
        }
    }

    #[async_trait]
    impl HostManifestPort for HostManifestPortMock {
        async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest> {
            spoke_ok(self.self_manifest.clone())
        }

        async fn list_peer_host_capability_manifests(&self) -> SpokeResult<Vec<HostCapabilityManifest>> {
            spoke_ok(normalize_peer_manifests(
                self.self_manifest.host_id.as_str(),
                &self.raw_peers,
            ))
        }
    }

    struct BaselinePortStub {
        host_manifest: HostManifestPortMock,
    }

    impl BaselinePortStub {
        fn new(host_manifest: HostManifestPortMock) -> Self {
            Self { host_manifest }
        }
    }

    #[async_trait]
    impl KnowledgeEntryPort for BaselinePortStub {
        async fn get_knowledge_entry(&self, _entry_id: &str) -> SpokeResult<KnowledgeEntry> {
            unreachable!("baseline port stub")
        }

        async fn put_knowledge_entry(
            &self,
            entry: KnowledgeEntry,
            _expected_base_revision: Option<u64>,
        ) -> SpokeResult<KnowledgeEntry> {
            spoke_ok(entry)
        }
    }

    #[async_trait]
    impl RelationPort for BaselinePortStub {
        async fn get_relation(&self, _relation_id: &str) -> SpokeResult<Relation> {
            unreachable!("baseline port stub")
        }

        async fn put_relation(
            &self,
            relation: Relation,
            _expected_base_revision: Option<u64>,
        ) -> SpokeResult<Relation> {
            spoke_ok(relation)
        }
    }

    #[async_trait]
    impl ScopeQueryPort for BaselinePortStub {
        async fn list_knowledge_entries(&self, _scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
            spoke_ok(Vec::new())
        }

        async fn list_timeline_events(&self, _scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
            spoke_ok(Vec::new())
        }
    }

    #[async_trait]
    impl FindingPort for BaselinePortStub {
        async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
            spoke_ok(findings)
        }
    }

    #[async_trait]
    impl RuleQueryPort for BaselinePortStub {
        async fn list_rules(&self, _rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
            spoke_ok(Vec::new())
        }
    }

    #[async_trait]
    impl HostManifestPort for BaselinePortStub {
        async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest> {
            self.host_manifest.get_host_capability_manifest().await
        }

        async fn list_peer_host_capability_manifests(&self) -> SpokeResult<Vec<HostCapabilityManifest>> {
            self.host_manifest
                .list_peer_host_capability_manifests()
                .await
        }
    }

    #[test]
    fn capability_port_missing_is_twentieth_spoke_reject_code() {
        assert_eq!(
            SpokeRejectCode::CapabilityPortMissing.as_str(),
            "CAPABILITY_PORT_MISSING"
        );
        assert_eq!(
            SpokeRejectCode::try_from_str("CAPABILITY_PORT_MISSING"),
            Some(SpokeRejectCode::CapabilityPortMissing)
        );
    }

    #[test]
    fn baseline_ports_accepts_all_six_baseline_families() {
        let ports = BaselinePortStub::new(HostManifestPortMock::new(
            make_manifest("self-host", &["self-ns"], &["data-store"]),
            Vec::new(),
        ));

        let _: &dyn BaselinePorts = &ports;
        let _: &dyn KnowledgeEntryPort = &ports;
        let _: &dyn RelationPort = &ports;
        let _: &dyn ScopeQueryPort = &ports;
        let _: &dyn FindingPort = &ports;
        let _: &dyn RuleQueryPort = &ports;
        let _: &dyn HostManifestPort = &ports;
    }

    #[test]
    fn host_manifest_port_returns_self_manifest() {
        let self_manifest = make_manifest(
            "adapter-self",
            &["alpha"],
            &["data-store", "checker"],
        );
        let ports = HostManifestPortMock::new(self_manifest.clone(), Vec::new());

        let result = pollster::block_on(ports.get_host_capability_manifest());

        match result {
            SpokeResult::Ok(value) => assert_eq!(value.host_id.as_str(), "adapter-self"),
            SpokeResult::Reject(_) => panic!("expected ok"),
        }
    }

    #[test]
    fn host_manifest_port_accepts_empty_peer_list() {
        let ports = HostManifestPortMock::new(
            make_manifest("self-host", &["default"], &["data-store"]),
            Vec::new(),
        );

        let result = pollster::block_on(ports.list_peer_host_capability_manifests());

        match result {
            SpokeResult::Ok(value) => assert!(value.is_empty()),
            SpokeResult::Reject(_) => panic!("expected ok"),
        }
    }

    #[test]
    fn host_manifest_port_returns_seeded_peers_with_disjoint_namespaces() {
        let self_manifest = make_manifest("self-host", &["self-ns"], &["data-store"]);
        let peer_a = make_manifest("peer-a", &["peer-a-ns"], &["checker"]);
        let peer_b = make_manifest("peer-b", &["peer-b-ns"], &["assembler"]);
        let ports = HostManifestPortMock::new(self_manifest, vec![peer_b.clone(), peer_a.clone()]);

        let result = pollster::block_on(ports.list_peer_host_capability_manifests());

        match result {
            SpokeResult::Ok(value) => {
                assert_eq!(value.len(), 2);
                assert_eq!(value[0].host_id.as_str(), "peer-a");
                assert_eq!(value[1].host_id.as_str(), "peer-b");
                let namespaces: Vec<&str> = value
                    .iter()
                    .flat_map(|manifest| manifest.namespaces.iter().map(|ns| ns.as_str()))
                    .collect();
                let unique: std::collections::HashSet<_> = namespaces.iter().copied().collect();
                assert_eq!(namespaces.len(), unique.len());
                assert!(!namespaces.contains(&"self-ns"));
            }
            SpokeResult::Reject(_) => panic!("expected ok"),
        }
    }

    #[test]
    fn host_manifest_port_excludes_self_dedupes_and_sorts_peers() {
        let self_manifest = make_manifest("self-host", &["self-ns"], &["data-store"]);
        let peer_z = make_manifest("peer-z", &["z-ns"], &["data-store"]);
        let peer_a_dupe = make_manifest("peer-a", &["a-ns-dup"], &["checker"]);
        let peer_a = make_manifest("peer-a", &["a-ns"], &["assembler"]);
        let ports = HostManifestPortMock::new(
            self_manifest.clone(),
            vec![peer_z.clone(), self_manifest, peer_a_dupe, peer_a.clone()],
        );

        let result = pollster::block_on(ports.list_peer_host_capability_manifests());

        match result {
            SpokeResult::Ok(value) => {
                let host_ids: Vec<&str> = value.iter().map(|m| m.host_id.as_str()).collect();
                assert_eq!(host_ids, vec!["peer-a", "peer-z"]);
                assert_eq!(value[0].roles, peer_a.roles);
            }
            SpokeResult::Reject(_) => panic!("expected ok"),
        }
    }
}
