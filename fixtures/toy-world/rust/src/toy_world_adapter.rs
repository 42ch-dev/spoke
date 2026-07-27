//! Toy-world reference adapter — FullAdapter over an in-memory OCC store.

use std::sync::Mutex;

use serde_json::json;
use spoke_operations::{
    spoke_ok, spoke_reject, ComputablePort, ComputablePorts, FindingPort, ForkPorts,
    ForkTimelineQueryPort, HostManifestPort, KnowledgeEntryPort, RelationPort, RuleQueryPort,
    ScopeQueryPort, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::{
    ComputeRequest, ComputeResponse, Finding, HostCapabilityManifest, KnowledgeEntry,
    ProjectRequest, ProjectResponse, Relation, Rule, Scope, TimelineEvent,
};

use crate::memory_store::{load_op_fixture, MemoryStore, MemoryStoreSeed};

/// Toy-world reference adapter — implements FullAdapter over an in-memory store.
#[derive(Debug)]
pub struct ToyWorldAdapter {
    store: Mutex<MemoryStore>,
}

impl ToyWorldAdapter {
    pub fn new(seed: Option<MemoryStoreSeed>) -> Self {
        Self {
            store: Mutex::new(MemoryStore::new(seed)),
        }
    }

    pub fn from_store(store: MemoryStore) -> Self {
        Self {
            store: Mutex::new(store),
        }
    }

    /// Construct with committed kb / rel / evt / rule / fnd fixtures loaded.
    pub fn with_committed_fixtures() -> Self {
        Self::from_store(MemoryStore::from_committed_fixtures())
    }

    /// Borrow the in-memory store under the adapter lock.
    pub fn with_store<R>(&self, f: impl FnOnce(&MemoryStore) -> R) -> R {
        let store = self.store.lock().expect("toy-world store lock");
        f(&store)
    }

    /// Mutably borrow the in-memory store under the adapter lock.
    pub fn with_store_mut<R>(&self, f: impl FnOnce(&mut MemoryStore) -> R) -> R {
        let mut store = self.store.lock().expect("toy-world store lock");
        f(&mut store)
    }
}

impl Default for ToyWorldAdapter {
    fn default() -> Self {
        Self::new(None)
    }
}

impl KnowledgeEntryPort for ToyWorldAdapter {
    fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
        self.with_store(|store| store.get_knowledge_entry(entry_id))
    }

    fn put_knowledge_entry(
        &self,
        entry: KnowledgeEntry,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry> {
        self.with_store_mut(|store| store.put_knowledge_entry(entry, expected_base_revision))
    }
}

impl RelationPort for ToyWorldAdapter {
    fn put_relation(&self, relation: Relation) -> SpokeResult<Relation> {
        self.with_store_mut(|store| store.put_relation(relation))
    }
}

impl ScopeQueryPort for ToyWorldAdapter {
    fn list_knowledge_entries(&self, _scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
        self.with_store(|store| store.list_knowledge_entries())
    }

    fn list_timeline_events(&self, _scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
        self.with_store(|store| store.list_timeline_events())
    }
}

impl FindingPort for ToyWorldAdapter {
    fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
        self.with_store_mut(|store| store.put_findings(findings))
    }
}

impl RuleQueryPort for ToyWorldAdapter {
    fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
        self.with_store(|store| store.list_rules(rule_refs))
    }
}

fn toy_world_host_manifest() -> HostCapabilityManifest {
    serde_json::from_value(json!({
        "schema_version": 1,
        "host_id": "host_tw_adapter",
        "roles": ["data-store", "checker", "assembler", "input-source"],
        "capabilities": ["spoke-baseline"],
        "namespaces": ["toy_world"],
        "extensions": {}
    }))
    .expect("valid HostCapabilityManifest")
}

impl HostManifestPort for ToyWorldAdapter {
    fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest> {
        spoke_ok(toy_world_host_manifest())
    }

    fn list_peer_host_capability_manifests(&self) -> SpokeResult<Vec<HostCapabilityManifest>> {
        spoke_ok(Vec::new())
    }
}

impl ComputablePort for ToyWorldAdapter {
    /// Minimal wire-valid ProjectResponse from committed `op_tw_project_response.json`.
    fn project(&self, request: ProjectRequest) -> SpokeResult<ProjectResponse> {
        let fixture: ProjectResponse = load_op_fixture("op_tw_project_response.json");
        let computable = match fixture {
            ProjectResponse::Variant0 { computable, .. } => computable,
            ProjectResponse::Variant1 { error, .. } => {
                return spoke_reject(
                    SpokeRejectCode::InvalidInput,
                    format!("fixture project response is an error envelope: {}", error.message),
                    None,
                );
            }
        };
        match serde_json::from_value(json!({
            "session_id": request.session_id,
            "entry_id": request.entry_id,
            "computable": computable,
        })) {
            Ok(response) => spoke_ok(response),
            Err(error) => spoke_reject(
                SpokeRejectCode::InvalidInput,
                format!("failed to build ProjectResponse: {error}"),
                None,
            ),
        }
    }

    /// Minimal wire-valid ComputeResponse from committed settle response fixture.
    fn compute(&self, request: ComputeRequest) -> SpokeResult<ComputeResponse> {
        let fixture: ComputeResponse = load_op_fixture("op_tw_compute_settle_response.json");
        let fixture_state = match fixture {
            ComputeResponse::Variant0 { state, .. } => state,
            ComputeResponse::Variant1 { error, .. } => {
                return spoke_reject(
                    SpokeRejectCode::InvalidInput,
                    format!("fixture compute response is an error envelope: {}", error.message),
                    None,
                );
            }
        };

        // Always echo request.computable — never backfill from the fixture.
        let mut body = json!({
            "session_id": request.session_id,
            "entry_id": request.entry_id,
            "computable": request.computable.clone(),
        });
        if request.settle == Some(true) {
            let state = if fixture_state.is_empty() {
                request.computable.clone()
            } else {
                fixture_state
            };
            body["state"] = json!(state);
        }

        match serde_json::from_value(body) {
            Ok(response) => spoke_ok(response),
            Err(error) => spoke_reject(
                SpokeRejectCode::InvalidInput,
                format!("failed to build ComputeResponse: {error}"),
                None,
            ),
        }
    }
}

impl ForkTimelineQueryPort for ToyWorldAdapter {
    /// Fork timeline query — seeded events filtered by `scope.fork_id`.
    fn list_fork_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
        let fork_id = scope.fork_id.as_ref().map(|value| value.as_str());
        self.with_store(|store| {
            let events = store
                .events
                .iter()
                .filter(|event| {
                    event
                        .fork_id
                        .as_ref()
                        .map(|value| value.as_str())
                        == fork_id
                })
                .cloned()
                .collect();
            spoke_ok(events)
        })
    }
}

/// Baseline-only view of a [`ToyWorldAdapter`] — omits optional Full methods so
/// dynamic orchestrators surface `CAPABILITY_PORT_MISSING`.
#[derive(Debug)]
pub struct BaselineOnlyAdapter {
    inner: ToyWorldAdapter,
}

/// Wrap a full adapter as baseline-only (no computable / fork port methods).
pub fn as_baseline_only(adapter: ToyWorldAdapter) -> BaselineOnlyAdapter {
    BaselineOnlyAdapter { inner: adapter }
}

impl KnowledgeEntryPort for BaselineOnlyAdapter {
    fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
        self.inner.get_knowledge_entry(entry_id)
    }

    fn put_knowledge_entry(
        &self,
        entry: KnowledgeEntry,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry> {
        self.inner
            .put_knowledge_entry(entry, expected_base_revision)
    }
}

impl RelationPort for BaselineOnlyAdapter {
    fn put_relation(&self, relation: Relation) -> SpokeResult<Relation> {
        self.inner.put_relation(relation)
    }
}

impl ScopeQueryPort for BaselineOnlyAdapter {
    fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
        self.inner.list_knowledge_entries(scope)
    }

    fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
        self.inner.list_timeline_events(scope)
    }
}

impl FindingPort for BaselineOnlyAdapter {
    fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
        self.inner.put_findings(findings)
    }
}

impl RuleQueryPort for BaselineOnlyAdapter {
    fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
        self.inner.list_rules(rule_refs)
    }
}

impl HostManifestPort for BaselineOnlyAdapter {
    fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest> {
        self.inner.get_host_capability_manifest()
    }

    fn list_peer_host_capability_manifests(&self) -> SpokeResult<Vec<HostCapabilityManifest>> {
        self.inner.list_peer_host_capability_manifests()
    }
}

impl ComputablePorts for BaselineOnlyAdapter {
    fn as_computable(&self) -> Option<&dyn ComputablePort> {
        None
    }
}

impl ForkPorts for BaselineOnlyAdapter {
    fn as_fork_timeline(&self) -> Option<&dyn ForkTimelineQueryPort> {
        None
    }
}
