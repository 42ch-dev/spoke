//! Minimal in-crate baseline adapter for binding smoke hosts.
//!
//! Implements [`BaselinePorts`] over an empty in-memory store so
//! `ffi-smoke-host` builds compile without the unpublished toy-world fixture
//! crate. Semantics mirror [`ToyWorldAdapter::default()`]: knowledge put/get
//! round trips work; other families return empty lists or not-found rejects.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use spoke_operations::{
    spoke_ok, spoke_reject, ComputablePort, FindingPort, ForkTimelineQueryPort,
    HostManifestPort, KnowledgeEntryPort, RelationPort, RuleQueryPort, ScopeQueryPort,
    SpokeRejectCode, SpokeResult,
};
use spoke_schemas::{
    ComputeRequest, ComputeResponse, Finding, HostCapabilityManifest, KnowledgeEntry,
    ProjectRequest, ProjectResponse, Relation, Rule, Scope, TimelineEvent,
};

#[derive(Debug, Default)]
struct SmokeStore {
    entries: HashMap<String, KnowledgeEntry>,
    relations: HashMap<String, Relation>,
    events: Vec<TimelineEvent>,
    rules: HashMap<String, Rule>,
    findings: Vec<Finding>,
}

impl SmokeStore {
    fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
        match self.entries.get(entry_id) {
            Some(entry) => spoke_ok(entry.clone()),
            None => {
                let mut details = Map::new();
                details.insert("entry_id".into(), json!(entry_id));
                spoke_reject(
                    SpokeRejectCode::KnowledgeEntryNotFound,
                    format!("KnowledgeEntry not found: {entry_id}"),
                    Some(details),
                )
            }
        }
    }

    fn put_knowledge_entry(
        &mut self,
        entry: KnowledgeEntry,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry> {
        let existing = self.entries.get(&entry.entry_id);
        match expected_base_revision {
            None => {
                if existing.is_some() {
                    let mut details = Map::new();
                    details.insert("entry_id".into(), Value::String(entry.entry_id.clone()));
                    return spoke_reject(
                        SpokeRejectCode::RevisionConflict,
                        format!("Entry already exists: {}", entry.entry_id),
                        Some(details),
                    );
                }
            }
            Some(expected) => match existing {
                None => {
                    let mut details = Map::new();
                    details.insert("entry_id".into(), Value::String(entry.entry_id.clone()));
                    details.insert("expectedBaseRevision".into(), json!(expected));
                    return spoke_reject(
                        SpokeRejectCode::StoredRevisionStale,
                        format!("KnowledgeEntry not found for update: {}", entry.entry_id),
                        Some(details),
                    );
                }
                Some(stored) => {
                    let current = stored.revision.unwrap_or(0);
                    if current != expected {
                        let mut details = Map::new();
                        details.insert("expectedBaseRevision".into(), json!(expected));
                        details.insert("storeRevision".into(), json!(current));
                        return spoke_reject(
                            SpokeRejectCode::StoredRevisionStale,
                            format!(
                                "Store revision {current} does not match expected base {expected}"
                            ),
                            Some(details),
                        );
                    }
                }
            },
        }
        self.entries.insert(entry.entry_id.clone(), entry.clone());
        spoke_ok(entry)
    }

    fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
        match self.relations.get(relation_id) {
            Some(relation) => spoke_ok(relation.clone()),
            None => {
                let mut details = Map::new();
                details.insert("relation_id".into(), json!(relation_id));
                spoke_reject(
                    SpokeRejectCode::RelationNotFound,
                    format!("Relation not found: {relation_id}"),
                    Some(details),
                )
            }
        }
    }

    fn put_relation(
        &mut self,
        mut relation: Relation,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<Relation> {
        let existing = self.relations.get(&relation.relation_id);
        match expected_base_revision {
            None => {
                if existing.is_some() {
                    let mut details = Map::new();
                    details.insert(
                        "relation_id".into(),
                        Value::String(relation.relation_id.clone()),
                    );
                    return spoke_reject(
                        SpokeRejectCode::RelationAlreadyExists,
                        format!("Relation already exists: {}", relation.relation_id),
                        Some(details),
                    );
                }
                relation.revision = Some(1);
            }
            Some(expected) => match existing {
                None => {
                    let mut details = Map::new();
                    details.insert(
                        "relation_id".into(),
                        Value::String(relation.relation_id.clone()),
                    );
                    details.insert("expectedBaseRevision".into(), json!(expected));
                    return spoke_reject(
                        SpokeRejectCode::StoredRevisionStale,
                        format!("Relation not found for update: {}", relation.relation_id),
                        Some(details),
                    );
                }
                Some(stored) => {
                    let current = stored.revision.unwrap_or(0);
                    if current != expected {
                        let mut details = Map::new();
                        details.insert("expectedBaseRevision".into(), json!(expected));
                        details.insert("storeRevision".into(), json!(current));
                        return spoke_reject(
                            SpokeRejectCode::StoredRevisionStale,
                            format!(
                                "Store revision {current} does not match expected base {expected}"
                            ),
                            Some(details),
                        );
                    }
                    relation.revision = Some(current + 1);
                }
            },
        }
        self.relations
            .insert(relation.relation_id.clone(), relation.clone());
        spoke_ok(relation)
    }

    fn list_knowledge_entries(&self) -> SpokeResult<Vec<KnowledgeEntry>> {
        spoke_ok(self.entries.values().cloned().collect())
    }

    fn list_timeline_events(&self) -> SpokeResult<Vec<TimelineEvent>> {
        spoke_ok(self.events.clone())
    }

    fn put_findings(&mut self, next: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
        self.findings.extend(next.iter().cloned());
        spoke_ok(next)
    }

    fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
        let mut resolved = Vec::with_capacity(rule_refs.len());
        for rule_ref in rule_refs {
            match self.rules.get(rule_ref) {
                Some(rule) => resolved.push(rule.clone()),
                None => {
                    let mut details = Map::new();
                    details.insert("rule_ref".into(), Value::String(rule_ref.clone()));
                    return spoke_reject(
                        SpokeRejectCode::InvalidInput,
                        format!("Rule not found: {rule_ref}"),
                        Some(details),
                    );
                }
            }
        }
        spoke_ok(resolved)
    }
}

fn smoke_host_manifest() -> HostCapabilityManifest {
    serde_json::from_value(json!({
        "schema_version": 1,
        "host_id": "smoke-host",
        "roles": ["data-store"],
        "capabilities": ["spoke-baseline"],
        "namespaces": ["smoke"],
        "extensions": {},
    }))
    .expect("valid smoke HostCapabilityManifest")
}

/// Empty-store baseline adapter for binding smoke hosts (no fixture crate).
#[derive(Debug, Default)]
pub struct SmokeBaselineAdapter {
    store: Mutex<SmokeStore>,
}

#[async_trait]
impl KnowledgeEntryPort for SmokeBaselineAdapter {
    async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
        let store = self.store.lock().expect("smoke store lock");
        store.get_knowledge_entry(entry_id)
    }

    async fn put_knowledge_entry(
        &self,
        entry: KnowledgeEntry,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry> {
        let mut store = self.store.lock().expect("smoke store lock");
        store.put_knowledge_entry(entry, expected_base_revision)
    }
}

#[async_trait]
impl RelationPort for SmokeBaselineAdapter {
    async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
        let store = self.store.lock().expect("smoke store lock");
        store.get_relation(relation_id)
    }

    async fn put_relation(
        &self,
        relation: Relation,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<Relation> {
        let mut store = self.store.lock().expect("smoke store lock");
        store.put_relation(relation, expected_base_revision)
    }
}

#[async_trait]
impl ScopeQueryPort for SmokeBaselineAdapter {
    async fn list_knowledge_entries(&self, _scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
        let store = self.store.lock().expect("smoke store lock");
        store.list_knowledge_entries()
    }

    async fn list_timeline_events(&self, _scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
        let store = self.store.lock().expect("smoke store lock");
        store.list_timeline_events()
    }
}

#[async_trait]
impl FindingPort for SmokeBaselineAdapter {
    async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
        let mut store = self.store.lock().expect("smoke store lock");
        store.put_findings(findings)
    }
}

#[async_trait]
impl RuleQueryPort for SmokeBaselineAdapter {
    async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
        let store = self.store.lock().expect("smoke store lock");
        store.list_rules(rule_refs)
    }
}

#[async_trait]
impl HostManifestPort for SmokeBaselineAdapter {
    async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest> {
        spoke_ok(smoke_host_manifest())
    }

    async fn list_peer_host_capability_manifests(&self) -> SpokeResult<Vec<HostCapabilityManifest>> {
        spoke_ok(Vec::new())
    }
}

// Optional families (D4 catalogue): the smoke host adapter carries the
// full ports face so `LoopbackHostOptions.adapter` (widened to `FullPorts`
// by plan 1) accepts it. Empty-store semantics mirror the baseline rows:
// project / compute echo the request's computable state, and the fork
// timeline answers an empty list (the store's baseline events are not
// fork-branch events).
#[async_trait]
impl ComputablePort for SmokeBaselineAdapter {
    async fn project(&self, request: ProjectRequest) -> SpokeResult<ProjectResponse> {
        spoke_ok(ProjectResponse::Variant0 {
            session_id: request.session_id,
            entry_id: request.entry_id,
            computable: request.state,
            extensions: Default::default(),
        })
    }

    async fn compute(&self, request: ComputeRequest) -> SpokeResult<ComputeResponse> {
        let state = request.computable.clone();
        spoke_ok(ComputeResponse::Variant0 {
            session_id: request.session_id,
            entry_id: request.entry_id,
            computable: request.computable,
            state,
            extensions: Default::default(),
        })
    }
}

#[async_trait]
impl ForkTimelineQueryPort for SmokeBaselineAdapter {
    async fn list_fork_timeline_events(&self, _scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
        spoke_ok(Vec::new())
    }
}

