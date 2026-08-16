//! Toy-world reference adapter — FullAdapter over an in-memory OCC store.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use serde_json::{json, Value};
use spoke_operations::{
    find_tool, list_tools, parse_tool_capability_id, spoke_ok, spoke_reject,
    validate_tool_arguments, ComputablePort, ComputablePorts, FindingPort, ForkPorts,
    ForkTimelineQueryPort, HostManifestPort, KnowledgeEntryPort, RelationPort, RuleQueryPort,
    ScopeQueryPort, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::host_capability_manifest::ToolDescriptor;
use spoke_schemas::{
    ComputeRequest, ComputeResponse, Finding, HostCapabilityManifest, KnowledgeEntry,
    ProjectRequest, ProjectResponse, Relation, Rule, Scope, TimelineEvent,
};

use crate::memory_store::{load_op_fixture, MemoryStore, MemoryStoreSeed};
use crate::toy_world_tools::{default_tool_handlers, ToolHandler};

/// Toy-world reference adapter — implements FullAdapter over an in-memory store.
pub struct ToyWorldAdapter {
    store: Arc<Mutex<MemoryStore>>,
    /// Tool-handler registry (plan-3 serving surface): `register_tool_handler`
    /// fills it; `invoke_tool` dispatches by exact capability id. The registry
    /// MUST NOT mutate the manifest — descriptor truth for discovery stays in
    /// the manifest's `tools[]`; a registry/manifest mismatch is a provider
    /// bug caught by `validate_manifest_tools`.
    tool_handlers: Mutex<HashMap<String, ToolHandler>>,
}

impl fmt::Debug for ToyWorldAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let handler_count = self
            .tool_handlers
            .lock()
            .map(|handlers| handlers.len())
            .unwrap_or(0);
        f.debug_struct("ToyWorldAdapter")
            .field("store", &self.store)
            .field("tool_handler_count", &handler_count)
            .finish()
    }
}

impl ToyWorldAdapter {
    pub fn new(seed: Option<MemoryStoreSeed>) -> Self {
        let store = Arc::new(Mutex::new(MemoryStore::new(seed)));
        Self {
            store: store.clone(),
            tool_handlers: Mutex::new(default_tool_handlers(store)),
        }
    }

    pub fn from_store(store: MemoryStore) -> Self {
        let store = Arc::new(Mutex::new(store));
        Self {
            store: store.clone(),
            tool_handlers: Mutex::new(default_tool_handlers(store)),
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

#[async_trait]
impl KnowledgeEntryPort for ToyWorldAdapter {
    async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
        self.with_store(|store| store.get_knowledge_entry(entry_id))
    }

    async fn put_knowledge_entry(
        &self,
        entry: KnowledgeEntry,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry> {
        self.with_store_mut(|store| store.put_knowledge_entry(entry, expected_base_revision))
    }
}

#[async_trait]
impl RelationPort for ToyWorldAdapter {
    async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
        self.with_store(|store| store.get_relation(relation_id))
    }

    async fn put_relation(
        &self,
        relation: Relation,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<Relation> {
        self.with_store_mut(|store| store.put_relation(relation, expected_base_revision))
    }
}

#[async_trait]
impl ScopeQueryPort for ToyWorldAdapter {
    async fn list_knowledge_entries(&self, _scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
        self.with_store(|store| store.list_knowledge_entries())
    }

    async fn list_timeline_events(&self, _scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
        self.with_store(|store| store.list_timeline_events())
    }
}

#[async_trait]
impl FindingPort for ToyWorldAdapter {
    async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
        self.with_store_mut(|store| store.put_findings(findings))
    }
}

#[async_trait]
impl RuleQueryPort for ToyWorldAdapter {
    async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
        self.with_store(|store| store.list_rules(rule_refs))
    }
}

fn toy_world_self_manifest() -> HostCapabilityManifest {
    load_op_fixture("host_tw_primary.json")
}

fn toy_world_peer_manifests() -> Vec<HostCapabilityManifest> {
    vec![load_op_fixture("host_tw_peer.json")]
}

fn normalize_peer_manifests(
    self_host_id: &str,
    peers: &[HostCapabilityManifest],
) -> Vec<HostCapabilityManifest> {
    let mut by_host_id = std::collections::HashMap::new();
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

#[async_trait]
impl HostManifestPort for ToyWorldAdapter {
    async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest> {
        spoke_ok(toy_world_self_manifest())
    }

    async fn list_peer_host_capability_manifests(&self) -> SpokeResult<Vec<HostCapabilityManifest>> {
        let self_manifest = toy_world_self_manifest();
        spoke_ok(normalize_peer_manifests(
            self_manifest.host_id.as_str(),
            &toy_world_peer_manifests(),
        ))
    }
}

#[async_trait]
impl ComputablePort for ToyWorldAdapter {
    /// Minimal wire-valid ProjectResponse from committed `op_tw_project_response.json`.
    async fn project(&self, request: ProjectRequest) -> SpokeResult<ProjectResponse> {
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
    async fn compute(&self, request: ComputeRequest) -> SpokeResult<ComputeResponse> {
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

#[async_trait]
impl ForkTimelineQueryPort for ToyWorldAdapter {
    /// Fork timeline query — seeded events filtered by `scope.fork_id`.
    async fn list_fork_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
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

// ── Tool serving (reference provider surface) ─────────────────────────────

impl ToyWorldAdapter {
    /// List the tools this provider declares, in manifest order (owned
    /// clone — parity with `list_tools`).
    pub fn tool_descriptors(&self) -> Vec<ToolDescriptor> {
        list_tools(&toy_world_self_manifest())
    }

    /// Register a handler for a `tools.<ns>.<tool_id>` capability (plan-3
    /// surface parity). Grammar-asserted: a non-`tools.` id panics (provider
    /// misuse, mirroring `RemoteAdapter::register_tool_handler`). Duplicate
    /// registration OVERWRITES the previous handler (last-wins, documented).
    /// The registry does NOT mutate the manifest.
    pub fn register_tool_handler(&self, capability_id: &str, handler: ToolHandler) {
        match parse_tool_capability_id(capability_id) {
            SpokeResult::Ok(_) => {}
            SpokeResult::Reject(reject) => {
                panic!("{}", reject.message);
            }
        }
        self.tool_handlers
            .lock()
            .expect("tool handlers lock")
            .insert(capability_id.to_owned(), handler);
    }

    /// Invoke a declared tool by capability id. Grammar gate, then descriptor
    /// lookup in the self manifest, then the structural argument gate
    /// (`validate_tool_arguments`), then dispatch to the registered handler.
    /// A tool that is not listed in the manifest rejects with
    /// `CAPABILITY_PORT_MISSING` (no silent success); a declared tool without
    /// a registered handler is a provider bug and rejects the same way.
    pub async fn invoke_tool(&self, capability_id: &str, args: Value) -> SpokeResult<Value> {
        match parse_tool_capability_id(capability_id) {
            SpokeResult::Ok(_) => {}
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        }
        let manifest = toy_world_self_manifest();
        let Some(descriptor) = find_tool(&manifest, capability_id) else {
            return spoke_reject(
                SpokeRejectCode::CapabilityPortMissing,
                format!("Tool \"{capability_id}\" is not declared in the toy-world manifest (tools[])"),
                None,
            );
        };
        if let SpokeResult::Reject(reject) = validate_tool_arguments(&descriptor, &args) {
            return SpokeResult::Reject(reject);
        }
        let handler = self
            .tool_handlers
            .lock()
            .expect("tool handlers lock")
            .get(capability_id)
            .cloned();
        let Some(handler) = handler else {
            return spoke_reject(
                SpokeRejectCode::CapabilityPortMissing,
                format!("Tool \"{capability_id}\" is declared but has no registered handler"),
                None,
            );
        };
        handler(args).await
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

#[async_trait]
impl KnowledgeEntryPort for BaselineOnlyAdapter {
    async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
        self.inner.get_knowledge_entry(entry_id).await
    }

    async fn put_knowledge_entry(
        &self,
        entry: KnowledgeEntry,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry> {
        self.inner
            .put_knowledge_entry(entry, expected_base_revision)
            .await
    }
}

#[async_trait]
impl RelationPort for BaselineOnlyAdapter {
    async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
        self.inner.get_relation(relation_id).await
    }

    async fn put_relation(
        &self,
        relation: Relation,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<Relation> {
        self.inner.put_relation(relation, expected_base_revision).await
    }
}

#[async_trait]
impl ScopeQueryPort for BaselineOnlyAdapter {
    async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
        self.inner.list_knowledge_entries(scope).await
    }

    async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
        self.inner.list_timeline_events(scope).await
    }
}

#[async_trait]
impl FindingPort for BaselineOnlyAdapter {
    async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
        self.inner.put_findings(findings).await
    }
}

#[async_trait]
impl RuleQueryPort for BaselineOnlyAdapter {
    async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
        self.inner.list_rules(rule_refs).await
    }
}

#[async_trait]
impl HostManifestPort for BaselineOnlyAdapter {
    async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest> {
        self.inner.get_host_capability_manifest().await
    }

    async fn list_peer_host_capability_manifests(&self) -> SpokeResult<Vec<HostCapabilityManifest>> {
        self.inner.list_peer_host_capability_manifests().await
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

#[cfg(test)]
mod normalize_peer_manifests_tests {
    use super::normalize_peer_manifests;
    use serde_json::json;
    use spoke_schemas::HostCapabilityManifest;

    fn load_fixture<T: serde::de::DeserializeOwned>(filename: &str) -> T {
        crate::memory_store::load_op_fixture(filename)
    }

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

    #[test]
    fn accepts_empty_peer_list() {
        let primary = load_fixture::<HostCapabilityManifest>("host_tw_primary.json");

        let result = normalize_peer_manifests(primary.host_id.as_str(), &[]);

        assert!(result.is_empty());
    }

    #[test]
    fn excludes_self_dedupes_last_wins_and_sorts_ascending_by_host_id() {
        let primary = load_fixture::<HostCapabilityManifest>("host_tw_primary.json");
        let peer = load_fixture::<HostCapabilityManifest>("host_tw_peer.json");
        let peer_zulu = make_manifest("host_tw_zulu", &["zulu-ns"], &["checker"]);
        let peer_alpha_dupe = make_manifest(
            "host_tw_alpha",
            &["alpha-ns-dup"],
            &["checker"],
        );
        let peer_alpha = make_manifest("host_tw_alpha", &["alpha-ns"], &["assembler"]);
        let raw = vec![
            peer_zulu.clone(),
            primary.clone(),
            peer.clone(),
            peer_alpha_dupe,
            peer_alpha.clone(),
        ];

        let result = normalize_peer_manifests(primary.host_id.as_str(), &raw);

        let host_ids: Vec<_> = result.iter().map(|m| m.host_id.as_str()).collect();
        assert_eq!(
            host_ids,
            vec!["host_tw_alpha", "host_tw_peer", "host_tw_zulu"]
        );
        assert_eq!(result[0].namespaces, peer_alpha.namespaces);
        assert_eq!(result[1].host_id.as_str(), peer.host_id.as_str());
        assert_eq!(result[2].host_id.as_str(), peer_zulu.host_id.as_str());
    }
}
