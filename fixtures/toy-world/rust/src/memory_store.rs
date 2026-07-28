//! In-memory OCC store for the toy-world reference adapter.
//! Optional seed from committed fixture JSON under `fixtures/toy-world/`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};
use spoke_operations::{spoke_ok, spoke_reject, SpokeRejectCode, SpokeResult};
use spoke_schemas::{Finding, KnowledgeEntry, Relation, Rule, TimelineEvent};

/// Absolute path to committed toy-world JSON fixtures (sibling of this crate).
///
/// Parity name for the TypeScript `TOY_WORLD_FIXTURES_ROOT` export.
pub fn toy_world_fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join(".."))
}

fn load_json<T: DeserializeOwned>(filename: &str) -> T {
    let path = toy_world_fixtures_root().join(filename);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read fixture {}: {error}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("failed to parse fixture {}: {error}", path.display()))
}

/// Seed payload for constructing a [`MemoryStore`].
#[derive(Debug, Clone, Default)]
pub struct MemoryStoreSeed {
    pub entries: Vec<KnowledgeEntry>,
    pub relations: Vec<Relation>,
    pub events: Vec<TimelineEvent>,
    pub rules: Vec<Rule>,
    pub findings: Vec<Finding>,
}

/// Mutable in-memory maps with KnowledgeEntry OCC on put.
#[derive(Debug, Clone, Default)]
pub struct MemoryStore {
    pub entries: HashMap<String, KnowledgeEntry>,
    pub relations: HashMap<String, Relation>,
    pub events: Vec<TimelineEvent>,
    pub rules: HashMap<String, Rule>,
    pub findings: Vec<Finding>,
}

impl MemoryStore {
    pub fn new(seed: Option<MemoryStoreSeed>) -> Self {
        let seed = seed.unwrap_or_default();
        Self {
            entries: seed
                .entries
                .into_iter()
                .map(|entry| (entry.entry_id.clone(), entry))
                .collect(),
            relations: seed
                .relations
                .into_iter()
                .map(|relation| (relation.relation_id.clone(), relation))
                .collect(),
            events: seed.events,
            rules: seed
                .rules
                .into_iter()
                .map(|rule| (rule.rule_id.clone(), rule))
                .collect(),
            findings: seed.findings,
        }
    }

    /// Seed from committed toy-world JSON (kb / rel / evt / rule / fnd).
    pub fn from_committed_fixtures() -> Self {
        Self::new(Some(MemoryStoreSeed {
            entries: vec![
                load_json("kb_tw_mira.json"),
                load_json("kb_tw_harbor.json"),
                load_json("kb_tw_harbor_dawn_event.json"),
                load_json("kb_tw_harbor_market_square_event.json"),
                load_json("kb_tw_harbor_customs_gate_beat.json"),
                load_json("kb_tw_harbor_berth_confirm_event.json"),
            ],
            relations: vec![
                load_json("rel_tw_mira_harbor.json"),
                load_json("rel_tw_harbor_precedes_dawn_to_market.json"),
                load_json("rel_tw_harbor_precedes_market_to_customs.json"),
                load_json("rel_tw_harbor_precedes_customs_to_berth.json"),
            ],
            events: vec![
                load_json("evt_tw_harbor_dawn.json"),
                load_json("evt_tw_harbor_market_square.json"),
                load_json("evt_tw_harbor_customs_gate.json"),
                load_json("evt_tw_harbor_berth_confirm.json"),
                load_json("evt_tw_harbor_storm_delay.json"),
            ],
            rules: vec![load_json("rule_tw_consistency.json")],
            findings: vec![load_json("fnd_tw_open.json")],
        }))
    }

    pub fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
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

    /// Conditional put — `None` expected_base_revision means create (must be absent).
    pub fn put_knowledge_entry(
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

    pub fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
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

    /// Conditional put — `None` expected_base_revision means create (must be absent).
    /// Mirrors put_knowledge_entry OCC, with the store owning revision assignment
    /// (seed 1 on create, bump +1 on accepted update).
    pub fn put_relation(
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

    pub fn list_knowledge_entries(&self) -> SpokeResult<Vec<KnowledgeEntry>> {
        spoke_ok(self.entries.values().cloned().collect())
    }

    pub fn list_timeline_events(&self) -> SpokeResult<Vec<TimelineEvent>> {
        spoke_ok(self.events.clone())
    }

    pub fn put_findings(&mut self, next: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
        self.findings.extend(next.iter().cloned());
        spoke_ok(next)
    }

    pub fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
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

/// Load an ops fixture JSON file from the committed toy-world directory.
pub(crate) fn load_op_fixture<T: DeserializeOwned>(filename: &str) -> T {
    load_json(filename)
}
