//! Injection orchestration entrypoints — compose pure helpers with port I/O.

use crate::adapter::ports::{
    BaselinePorts, ComputablePorts, ForkPorts, KnowledgeEntryPort,
};
use crate::assemble::{build_assemble_packet, BuildAssemblePacketInput, KnowledgeEntryForAssemble};
use crate::computable::{validate_compute_request, validate_project_request};
use crate::extensions::ExtensionMap;
use crate::knowledge_entry::{
    assert_unique_active_knowledge_entry, is_valid_knowledge_entry_status_transition,
    AssertUniqueActiveKnowledgeEntryInput,
};
use crate::promote::{apply_promote_acceptance, validate_promote_request};
use crate::relate::validate_relate_request;
use crate::result::{spoke_ok, spoke_ok_unit, spoke_reject, SpokeRejectCode, SpokeResult};
use crate::scope::{filter_knowledge_entries_by_scope, filter_timeline_events_by_scope};
use crate::upsert::{validate_upsert_knowledge_entry, ValidateUpsertKnowledgeEntryContext};
use serde_json::{json, Map, Value};
use spoke_schemas::{
    AssembleRequest, AssembleResponse, CheckRequest, CheckResponse, ComputeRequest, ComputeResponse,
    Finding, KnowledgeEntry, ProjectRequest, ProjectResponse, PromoteRequest, PromoteResponse,
    RelateRequest, RelateResponse, Relation, Rule, Scope, TimelineEvent, UpsertRequest,
    UpsertResponse,
};

/// Checker callback input after ports load scoped data and rules.
#[derive(Debug, Clone)]
pub struct CheckRunInput {
    pub request: CheckRequest,
    pub entries: Vec<KnowledgeEntry>,
    pub events: Vec<TimelineEvent>,
    pub rules: Vec<Rule>,
}

fn wire_convert<T, U>(value: &T) -> SpokeResult<U>
where
    T: serde::Serialize,
    U: serde::de::DeserializeOwned,
{
    let wire = match serde_json::to_value(value) {
        Ok(wire) => wire,
        Err(error) => {
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                format!("Failed to serialize wire value: {error}"),
                None,
            );
        }
    };
    match serde_json::from_value(wire) {
        Ok(value) => spoke_ok(value),
        Err(error) => spoke_reject(
            SpokeRejectCode::InvalidInput,
            format!("Failed to deserialize wire value: {error}"),
            None,
        ),
    }
}

fn success_response<R>(body: Value) -> SpokeResult<R>
where
    R: serde::de::DeserializeOwned,
{
    match serde_json::from_value(body) {
        Ok(value) => spoke_ok(value),
        Err(error) => spoke_reject(
            SpokeRejectCode::InvalidInput,
            format!("Failed to build success response: {error}"),
            None,
        ),
    }
}

fn scope_from_check_request(scope: &spoke_schemas::check_request::CheckRequestScope) -> SpokeResult<Scope> {
    wire_convert(scope)
}

fn scope_from_assemble_request(
    scope: &spoke_schemas::assemble_request::AssembleRequestScope,
) -> SpokeResult<Scope> {
    wire_convert(scope)
}

fn data_knowledge_entry_from_upsert(
    entry: &spoke_schemas::upsert_request::KnowledgeEntry,
) -> SpokeResult<KnowledgeEntry> {
    wire_convert(entry)
}

fn data_rule_from_check(rule: &spoke_schemas::check_request::Rule) -> SpokeResult<Rule> {
    wire_convert(rule)
}

fn require_port_method(available: bool, method: &str) -> SpokeResult<()> {
    if available {
        return spoke_ok_unit();
    }

    let mut details = Map::new();
    details.insert("method".into(), Value::String(method.to_owned()));
    spoke_reject(
        SpokeRejectCode::CapabilityPortMissing,
        format!("Optional port method missing: {method}"),
        Some(details),
    )
}

fn load_stored_knowledge_entry(
    ports: &impl KnowledgeEntryPort,
    entry_id: &str,
) -> SpokeResult<Option<KnowledgeEntry>> {
    match ports.get_knowledge_entry(entry_id) {
        SpokeResult::Ok(entry) => spoke_ok(Some(entry)),
        SpokeResult::Reject(reject)
            if reject.code == SpokeRejectCode::KnowledgeEntryNotFound =>
        {
            spoke_ok(None)
        }
        SpokeResult::Reject(reject) => SpokeResult::Reject(reject),
    }
}

fn assert_status_transition_when_applicable(
    candidate: &KnowledgeEntry,
    stored: Option<&KnowledgeEntry>,
) -> SpokeResult<()> {
    let Some(stored) = stored else {
        return spoke_ok_unit();
    };
    if stored.status == candidate.status {
        return spoke_ok_unit();
    }

    if !is_valid_knowledge_entry_status_transition(&stored.status, &candidate.status) {
        let mut details = Map::new();
        details.insert("from".into(), Value::String(stored.status.clone()));
        details.insert("to".into(), Value::String(candidate.status.clone()));
        return spoke_reject(
            SpokeRejectCode::InvalidKnowledgeEntryStatusTransition,
            format!(
                "Disallowed knowledge entry status transition: {} -> {}",
                stored.status, candidate.status
            ),
            Some(details),
        );
    }

    spoke_ok_unit()
}

fn assert_uniqueness_when_applicable(
    candidate: &KnowledgeEntry,
    existing: &[KnowledgeEntry],
) -> SpokeResult<()> {
    assert_unique_active_knowledge_entry(AssertUniqueActiveKnowledgeEntryInput {
        scope_key: "upsert",
        entry_type: &candidate.entry_type,
        canonical_name: candidate.canonical_name.as_str(),
        candidate,
        existing,
    })
}

/// Upsert KnowledgeEntries: load context, validate, optional status/uniqueness, put.
pub fn orchestrate_upsert(
    ports: &impl BaselinePorts,
    request: UpsertRequest,
) -> SpokeResult<UpsertResponse> {
    let mut persisted: Vec<KnowledgeEntry> = Vec::new();
    let mut batch_peers: Vec<KnowledgeEntry> = Vec::new();

    for candidate_wire in &request.knowledge_entries {
        let candidate = match data_knowledge_entry_from_upsert(candidate_wire) {
            SpokeResult::Ok(entry) => entry,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };

        let stored = match load_stored_knowledge_entry(ports, &candidate.entry_id) {
            SpokeResult::Ok(stored) => stored,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };

        let validation = validate_upsert_knowledge_entry(
            &candidate,
            ValidateUpsertKnowledgeEntryContext {
                stored: stored.as_ref(),
                mode: None,
            },
        );
        if let SpokeResult::Reject(reject) = validation {
            return SpokeResult::Reject(reject);
        }

        if let SpokeResult::Reject(reject) =
            assert_status_transition_when_applicable(&candidate, stored.as_ref())
        {
            return SpokeResult::Reject(reject);
        }

        let mut uniqueness_existing = batch_peers.clone();
        if let Some(stored_entry) = stored {
            uniqueness_existing.push(stored_entry);
        }
        if let SpokeResult::Reject(reject) =
            assert_uniqueness_when_applicable(&candidate, &uniqueness_existing)
        {
            return SpokeResult::Reject(reject);
        }

        let put = match ports.put_knowledge_entry(candidate) {
            SpokeResult::Ok(entry) => entry,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };

        persisted.push(put.clone());
        batch_peers.push(put);
    }

    success_response(json!({ "knowledge_entries": persisted }))
}

/// Promote a provisional candidate, then persist via KnowledgeEntryPort.
pub fn orchestrate_promote(
    ports: &impl BaselinePorts,
    request: PromoteRequest,
) -> SpokeResult<PromoteResponse> {
    if let SpokeResult::Reject(reject) = validate_promote_request(&request) {
        return SpokeResult::Reject(reject);
    }

    let accepted = match apply_promote_acceptance(&request) {
        SpokeResult::Ok(entry) => entry,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let put = match ports.put_knowledge_entry(accepted) {
        SpokeResult::Ok(entry) => entry,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let mut body = json!({ "knowledge_entry": put });
    if let Some(target_entry_id) = &request.target_entry_id {
        body["superseded_id"] = json!(target_entry_id);
    }
    success_response(body)
}

/// Validate and persist a Relation.
pub fn orchestrate_relate(
    ports: &impl BaselinePorts,
    request: RelateRequest,
) -> SpokeResult<RelateResponse> {
    let relation: Relation = match wire_convert(&request.relation) {
        SpokeResult::Ok(relation) => relation,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    if let SpokeResult::Reject(reject) = validate_relate_request(&relation) {
        return SpokeResult::Reject(reject);
    }

    let put = match ports.put_relation(relation) {
        SpokeResult::Ok(relation) => relation,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    success_response(json!({ "relation": put }))
}

fn resolve_check_rules(
    ports: &impl BaselinePorts,
    request: &CheckRequest,
) -> SpokeResult<Vec<Rule>> {
    let mut embedded = Vec::with_capacity(request.rules.len());
    for rule in &request.rules {
        match data_rule_from_check(rule) {
            SpokeResult::Ok(rule) => embedded.push(rule),
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        }
    }

    if request.rule_refs.is_empty() {
        return spoke_ok(embedded);
    }

    let resolved = match ports.list_rules(&request.rule_refs) {
        SpokeResult::Ok(rules) => rules,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let mut combined = resolved;
    combined.extend(embedded);
    spoke_ok(combined)
}

/// Load scoped data and rules, invoke product checker callback, persist findings.
pub fn orchestrate_check<F>(
    ports: &impl BaselinePorts,
    request: CheckRequest,
    run_checker: F,
) -> SpokeResult<CheckResponse>
where
    F: FnOnce(CheckRunInput) -> SpokeResult<Vec<Finding>>,
{
    let rules = match resolve_check_rules(ports, &request) {
        SpokeResult::Ok(rules) => rules,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let scope = match scope_from_check_request(&request.scope) {
        SpokeResult::Ok(scope) => scope,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let entries_result = match ports.list_knowledge_entries(&scope) {
        SpokeResult::Ok(entries) => entries,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let events_result = match ports.list_timeline_events(&scope) {
        SpokeResult::Ok(events) => events,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let entries: Vec<KnowledgeEntry> = filter_knowledge_entries_by_scope(&entries_result, &scope)
        .into_iter()
        .cloned()
        .collect();
    let events: Vec<TimelineEvent> = filter_timeline_events_by_scope(&events_result, &scope)
        .into_iter()
        .cloned()
        .collect();

    let check_result = run_checker(CheckRunInput {
        request,
        entries,
        events,
        rules,
    });
    let findings = match check_result {
        SpokeResult::Ok(findings) => findings,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let put = match ports.put_findings(findings) {
        SpokeResult::Ok(findings) => findings,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    success_response(json!({ "findings": put }))
}

fn assemble_extensions(request: &AssembleRequest) -> SpokeResult<Option<ExtensionMap>> {
    if request.extensions.is_empty() {
        return spoke_ok(None);
    }
    match wire_convert::<_, ExtensionMap>(&request.extensions) {
        SpokeResult::Ok(map) => spoke_ok(Some(map)),
        SpokeResult::Reject(reject) => SpokeResult::Reject(reject),
    }
}

fn assemble_packet_response(
    scope: &Scope,
    entries: &[KnowledgeEntry],
    request: &AssembleRequest,
) -> SpokeResult<AssembleResponse> {
    let for_assemble: Vec<KnowledgeEntryForAssemble> = entries
        .iter()
        .cloned()
        .map(KnowledgeEntryForAssemble::from_entry)
        .collect();

    let extensions = match assemble_extensions(request) {
        SpokeResult::Ok(extensions) => extensions,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let packet_id = format!("assemble:{}", scope.scope_id);
    let packet = match build_assemble_packet(BuildAssemblePacketInput {
        packet_id: &packet_id,
        knowledge_entries: &for_assemble,
        extensions: extensions.as_ref(),
        max_entries: request.max_entries.map(|value| value.get() as usize),
    }) {
        SpokeResult::Ok(packet) => packet,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    success_response(json!({ "packet": packet }))
}

/// Query scoped KnowledgeEntries/events, apply scope helpers, build AssemblePacket.
pub fn orchestrate_assemble(
    ports: &impl BaselinePorts,
    request: AssembleRequest,
) -> SpokeResult<AssembleResponse> {
    let scope = match scope_from_assemble_request(&request.scope) {
        SpokeResult::Ok(scope) => scope,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let entries_result = match ports.list_knowledge_entries(&scope) {
        SpokeResult::Ok(entries) => entries,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let events_result = match ports.list_timeline_events(&scope) {
        SpokeResult::Ok(events) => events,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    // Events are loaded/filtered for sequence parity; packet builders use entries only.
    let _events = filter_timeline_events_by_scope(&events_result, &scope);

    let entries: Vec<KnowledgeEntry> = filter_knowledge_entries_by_scope(&entries_result, &scope)
        .into_iter()
        .cloned()
        .collect();

    assemble_packet_response(&scope, &entries, &request)
}

/// Project Session computable view via ComputablePort after request validation.
pub fn orchestrate_project(
    ports: &impl ComputablePorts,
    request: ProjectRequest,
) -> SpokeResult<ProjectResponse> {
    if let SpokeResult::Reject(reject) =
        require_port_method(ports.as_computable().is_some(), "project")
    {
        return SpokeResult::Reject(reject);
    }

    if let SpokeResult::Reject(reject) = validate_project_request(&request) {
        return SpokeResult::Reject(reject);
    }

    let computable = ports
        .as_computable()
        .expect("require_port_method gated availability");
    computable.project(request)
}

/// Compute Session updates via ComputablePort after request validation.
/// Settled-state persistence remains an explicit adapter step.
pub fn orchestrate_compute(
    ports: &impl ComputablePorts,
    request: ComputeRequest,
) -> SpokeResult<ComputeResponse> {
    if let SpokeResult::Reject(reject) =
        require_port_method(ports.as_computable().is_some(), "compute")
    {
        return SpokeResult::Reject(reject);
    }

    if let SpokeResult::Reject(reject) = validate_compute_request(&request) {
        return SpokeResult::Reject(reject);
    }

    let computable = ports
        .as_computable()
        .expect("require_port_method gated availability");
    computable.compute(request)
}

fn require_fork_scope(scope: &Scope) -> SpokeResult<Scope> {
    match &scope.fork_id {
        Some(fork_id) if !fork_id.as_str().trim().is_empty() => spoke_ok(scope.clone()),
        _ => {
            let mut details = Map::new();
            details.insert("field".into(), Value::String("scope.fork_id".into()));
            spoke_reject(
                SpokeRejectCode::MissingRequiredField,
                "Fork orchestration requires scope.fork_id",
                Some(details),
            )
        }
    }
}

/// Fork-aware check: KE via ScopeQueryPort; timeline via ForkTimelineQueryPort.
pub fn orchestrate_fork_check<F>(
    ports: &impl ForkPorts,
    request: CheckRequest,
    run_checker: F,
) -> SpokeResult<CheckResponse>
where
    F: FnOnce(CheckRunInput) -> SpokeResult<Vec<Finding>>,
{
    if let SpokeResult::Reject(reject) =
        require_port_method(ports.as_fork_timeline().is_some(), "listForkTimelineEvents")
    {
        return SpokeResult::Reject(reject);
    }

    let scope = match scope_from_check_request(&request.scope) {
        SpokeResult::Ok(scope) => scope,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let fork_scope = match require_fork_scope(&scope) {
        SpokeResult::Ok(scope) => scope,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let rules = match resolve_check_rules(ports, &request) {
        SpokeResult::Ok(rules) => rules,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let entries_result = match ports.list_knowledge_entries(&scope) {
        SpokeResult::Ok(entries) => entries,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let fork_timeline = ports
        .as_fork_timeline()
        .expect("require_port_method gated availability");
    let events_result = match fork_timeline.list_fork_timeline_events(&fork_scope) {
        SpokeResult::Ok(events) => events,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let entries: Vec<KnowledgeEntry> = filter_knowledge_entries_by_scope(&entries_result, &scope)
        .into_iter()
        .cloned()
        .collect();
    let events: Vec<TimelineEvent> = filter_timeline_events_by_scope(&events_result, &scope)
        .into_iter()
        .cloned()
        .collect();

    let check_result = run_checker(CheckRunInput {
        request,
        entries,
        events,
        rules,
    });
    let findings = match check_result {
        SpokeResult::Ok(findings) => findings,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let put = match ports.put_findings(findings) {
        SpokeResult::Ok(findings) => findings,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    success_response(json!({ "findings": put }))
}

/// Fork-aware assemble: KE via ScopeQueryPort; timeline via ForkTimelineQueryPort.
pub fn orchestrate_fork_assemble(
    ports: &impl ForkPorts,
    request: AssembleRequest,
) -> SpokeResult<AssembleResponse> {
    if let SpokeResult::Reject(reject) =
        require_port_method(ports.as_fork_timeline().is_some(), "listForkTimelineEvents")
    {
        return SpokeResult::Reject(reject);
    }

    let scope = match scope_from_assemble_request(&request.scope) {
        SpokeResult::Ok(scope) => scope,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let fork_scope = match require_fork_scope(&scope) {
        SpokeResult::Ok(scope) => scope,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let entries_result = match ports.list_knowledge_entries(&scope) {
        SpokeResult::Ok(entries) => entries,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let fork_timeline = ports
        .as_fork_timeline()
        .expect("require_port_method gated availability");
    let events_result = match fork_timeline.list_fork_timeline_events(&fork_scope) {
        SpokeResult::Ok(events) => events,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    // Events are loaded/filtered for sequence parity; packet builders use entries only.
    let _events = filter_timeline_events_by_scope(&events_result, &scope);

    let entries: Vec<KnowledgeEntry> = filter_knowledge_entries_by_scope(&entries_result, &scope)
        .into_iter()
        .cloned()
        .collect();

    assemble_packet_response(&scope, &entries, &request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::ports::{
        ComputablePort, FindingPort, ForkTimelineQueryPort, KnowledgeEntryPort, RelationPort,
        RuleQueryPort, ScopeQueryPort,
    };
    use crate::result::SpokeResult;
    use serde_json::json;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryStore {
        entries: HashMap<String, KnowledgeEntry>,
        relations: HashMap<String, Relation>,
        events: Vec<TimelineEvent>,
        rules: HashMap<String, Rule>,
        findings: Vec<Finding>,
    }

    struct MemoryBaselinePorts {
        store: Mutex<MemoryStore>,
    }

    impl MemoryBaselinePorts {
        fn new(seed: MemoryStore) -> Self {
            Self {
                store: Mutex::new(seed),
            }
        }

        fn with_entry(entry: KnowledgeEntry) -> Self {
            let mut store = MemoryStore::default();
            store.entries.insert(entry.entry_id.clone(), entry);
            Self::new(store)
        }
    }

    impl KnowledgeEntryPort for MemoryBaselinePorts {
        fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
            let store = self.store.lock().expect("store lock");
            match store.entries.get(entry_id) {
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

        fn put_knowledge_entry(&self, entry: KnowledgeEntry) -> SpokeResult<KnowledgeEntry> {
            let mut store = self.store.lock().expect("store lock");
            store.entries.insert(entry.entry_id.clone(), entry.clone());
            spoke_ok(entry)
        }
    }

    impl RelationPort for MemoryBaselinePorts {
        fn put_relation(&self, relation: Relation) -> SpokeResult<Relation> {
            let mut store = self.store.lock().expect("store lock");
            store
                .relations
                .insert(relation.relation_id.clone(), relation.clone());
            spoke_ok(relation)
        }
    }

    impl ScopeQueryPort for MemoryBaselinePorts {
        fn list_knowledge_entries(&self, _scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
            let store = self.store.lock().expect("store lock");
            spoke_ok(store.entries.values().cloned().collect())
        }

        fn list_timeline_events(&self, _scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
            let store = self.store.lock().expect("store lock");
            spoke_ok(store.events.clone())
        }
    }

    impl FindingPort for MemoryBaselinePorts {
        fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
            let mut store = self.store.lock().expect("store lock");
            store.findings.extend(findings.iter().cloned());
            spoke_ok(findings)
        }
    }

    impl RuleQueryPort for MemoryBaselinePorts {
        fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
            let store = self.store.lock().expect("store lock");
            let mut resolved = Vec::new();
            for rule_ref in rule_refs {
                match store.rules.get(rule_ref) {
                    Some(rule) => resolved.push(rule.clone()),
                    None => {
                        let mut details = Map::new();
                        details.insert("rule_ref".into(), json!(rule_ref));
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

    struct MemoryComputablePorts {
        baseline: MemoryBaselinePorts,
        projected: RefCell<Vec<ProjectRequest>>,
        computed: RefCell<Vec<ComputeRequest>>,
    }

    impl MemoryComputablePorts {
        fn new(baseline: MemoryBaselinePorts) -> Self {
            Self {
                baseline,
                projected: RefCell::new(Vec::new()),
                computed: RefCell::new(Vec::new()),
            }
        }
    }

    impl KnowledgeEntryPort for MemoryComputablePorts {
        fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
            self.baseline.get_knowledge_entry(entry_id)
        }
        fn put_knowledge_entry(&self, entry: KnowledgeEntry) -> SpokeResult<KnowledgeEntry> {
            self.baseline.put_knowledge_entry(entry)
        }
    }
    impl RelationPort for MemoryComputablePorts {
        fn put_relation(&self, relation: Relation) -> SpokeResult<Relation> {
            self.baseline.put_relation(relation)
        }
    }
    impl ScopeQueryPort for MemoryComputablePorts {
        fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
            self.baseline.list_knowledge_entries(scope)
        }
        fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
            self.baseline.list_timeline_events(scope)
        }
    }
    impl FindingPort for MemoryComputablePorts {
        fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
            self.baseline.put_findings(findings)
        }
    }
    impl RuleQueryPort for MemoryComputablePorts {
        fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
            self.baseline.list_rules(rule_refs)
        }
    }
    impl ComputablePort for MemoryComputablePorts {
        fn project(&self, request: ProjectRequest) -> SpokeResult<ProjectResponse> {
            self.projected.borrow_mut().push(request.clone());
            let mut computable = request.state.clone();
            computable.insert("projected".into(), json!(true));
            success_response(json!({
                "session_id": request.session_id,
                "entry_id": request.entry_id,
                "computable": computable,
            }))
        }

        fn compute(&self, request: ComputeRequest) -> SpokeResult<ComputeResponse> {
            self.computed.borrow_mut().push(request.clone());
            let mut body = json!({
                "session_id": request.session_id,
                "entry_id": request.entry_id,
                "computable": request.computable,
            });
            if request.settle == Some(true) {
                body["state"] = json!(request.computable);
            }
            success_response(body)
        }
    }

    struct MemoryForkPorts {
        baseline: MemoryBaselinePorts,
        fork_list_calls: RefCell<Vec<Scope>>,
    }

    impl MemoryForkPorts {
        fn new(baseline: MemoryBaselinePorts) -> Self {
            Self {
                baseline,
                fork_list_calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl KnowledgeEntryPort for MemoryForkPorts {
        fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
            self.baseline.get_knowledge_entry(entry_id)
        }
        fn put_knowledge_entry(&self, entry: KnowledgeEntry) -> SpokeResult<KnowledgeEntry> {
            self.baseline.put_knowledge_entry(entry)
        }
    }
    impl RelationPort for MemoryForkPorts {
        fn put_relation(&self, relation: Relation) -> SpokeResult<Relation> {
            self.baseline.put_relation(relation)
        }
    }
    impl ScopeQueryPort for MemoryForkPorts {
        fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
            self.baseline.list_knowledge_entries(scope)
        }
        fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
            self.baseline.list_timeline_events(scope)
        }
    }
    impl FindingPort for MemoryForkPorts {
        fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
            self.baseline.put_findings(findings)
        }
    }
    impl RuleQueryPort for MemoryForkPorts {
        fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
            self.baseline.list_rules(rule_refs)
        }
    }
    impl ForkTimelineQueryPort for MemoryForkPorts {
        fn list_fork_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
            self.fork_list_calls.borrow_mut().push(scope.clone());
            let store = self.baseline.store.lock().expect("store lock");
            let fork_id = scope.fork_id.as_ref().map(|value| value.as_str());
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
        }
    }

    /// Baseline-only adapter that claims ComputablePorts dynamically without the port.
    struct MissingComputablePorts {
        baseline: MemoryBaselinePorts,
    }

    impl KnowledgeEntryPort for MissingComputablePorts {
        fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
            self.baseline.get_knowledge_entry(entry_id)
        }
        fn put_knowledge_entry(&self, entry: KnowledgeEntry) -> SpokeResult<KnowledgeEntry> {
            self.baseline.put_knowledge_entry(entry)
        }
    }
    impl RelationPort for MissingComputablePorts {
        fn put_relation(&self, relation: Relation) -> SpokeResult<Relation> {
            self.baseline.put_relation(relation)
        }
    }
    impl ScopeQueryPort for MissingComputablePorts {
        fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
            self.baseline.list_knowledge_entries(scope)
        }
        fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
            self.baseline.list_timeline_events(scope)
        }
    }
    impl FindingPort for MissingComputablePorts {
        fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
            self.baseline.put_findings(findings)
        }
    }
    impl RuleQueryPort for MissingComputablePorts {
        fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
            self.baseline.list_rules(rule_refs)
        }
    }
    impl ComputablePorts for MissingComputablePorts {
        fn as_computable(&self) -> Option<&dyn ComputablePort> {
            None
        }
    }

    struct MissingForkPorts {
        baseline: MemoryBaselinePorts,
    }

    impl KnowledgeEntryPort for MissingForkPorts {
        fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
            self.baseline.get_knowledge_entry(entry_id)
        }
        fn put_knowledge_entry(&self, entry: KnowledgeEntry) -> SpokeResult<KnowledgeEntry> {
            self.baseline.put_knowledge_entry(entry)
        }
    }
    impl RelationPort for MissingForkPorts {
        fn put_relation(&self, relation: Relation) -> SpokeResult<Relation> {
            self.baseline.put_relation(relation)
        }
    }
    impl ScopeQueryPort for MissingForkPorts {
        fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
            self.baseline.list_knowledge_entries(scope)
        }
        fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
            self.baseline.list_timeline_events(scope)
        }
    }
    impl FindingPort for MissingForkPorts {
        fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
            self.baseline.put_findings(findings)
        }
    }
    impl RuleQueryPort for MissingForkPorts {
        fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
            self.baseline.list_rules(rule_refs)
        }
    }
    impl ForkPorts for MissingForkPorts {
        fn as_fork_timeline(&self) -> Option<&dyn ForkTimelineQueryPort> {
            None
        }
    }

    fn ke(wire: Value) -> KnowledgeEntry {
        serde_json::from_value(wire).expect("valid KnowledgeEntry")
    }

    fn relation(wire: Value) -> Relation {
        serde_json::from_value(wire).expect("valid Relation")
    }

    fn finding(wire: Value) -> Finding {
        serde_json::from_value(wire).expect("valid Finding")
    }

    fn rule(wire: Value) -> Rule {
        serde_json::from_value(wire).expect("valid Rule")
    }

    fn timeline_event(wire: Value) -> TimelineEvent {
        serde_json::from_value(wire).expect("valid TimelineEvent")
    }

    fn upsert_request(entries: Vec<Value>) -> UpsertRequest {
        serde_json::from_value(json!({ "knowledge_entries": entries })).expect("UpsertRequest")
    }

    fn promote_request(candidate: Value) -> PromoteRequest {
        serde_json::from_value(json!({ "candidate": candidate })).expect("PromoteRequest")
    }

    fn relate_request(relation: Value) -> RelateRequest {
        serde_json::from_value(json!({ "relation": relation })).expect("RelateRequest")
    }

    fn check_request(wire: Value) -> CheckRequest {
        serde_json::from_value(wire).expect("CheckRequest")
    }

    fn assemble_request(wire: Value) -> AssembleRequest {
        serde_json::from_value(wire).expect("AssembleRequest")
    }

    fn project_request(wire: Value) -> ProjectRequest {
        serde_json::from_value(wire).expect("ProjectRequest")
    }

    fn compute_request(wire: Value) -> ComputeRequest {
        serde_json::from_value(wire).expect("ComputeRequest")
    }

    #[test]
    fn orchestrate_upsert_creates_knowledge_entry_through_ports() {
        let ports = MemoryBaselinePorts::new(MemoryStore::default());
        let candidate = json!({
            "schema_version": 1,
            "entry_id": "kb_new",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "provisional",
            "body": { "summary": "Protagonist" },
            "extensions": {}
        });

        let result = orchestrate_upsert(&ports, upsert_request(vec![candidate.clone()]));
        assert!(result.is_ok());
        if let SpokeResult::Ok(UpsertResponse::Variant0 { knowledge_entries, .. }) = result {
            assert_eq!(knowledge_entries.len(), 1);
            assert_eq!(knowledge_entries[0].entry_id, "kb_new");
        } else {
            panic!("expected upsert success");
        }

        let stored = ports
            .get_knowledge_entry("kb_new")
            .expect_ok_entry("stored after upsert");
        assert_eq!(stored.entry_id, "kb_new");
    }

    #[test]
    fn orchestrate_promote_persists_confirmed_knowledge_entry() {
        let ports = MemoryBaselinePorts::new(MemoryStore::default());
        let candidate = json!({
            "schema_version": 1,
            "entry_id": "kb_promote",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "provisional",
            "revision": 0,
            "body": { "summary": "Protagonist" },
            "extensions": {}
        });

        let result = orchestrate_promote(&ports, promote_request(candidate));
        assert!(result.is_ok());
        if let SpokeResult::Ok(PromoteResponse::Variant0 { knowledge_entry, .. }) = result {
            assert_eq!(knowledge_entry.status, "confirmed");
            assert_eq!(knowledge_entry.revision, Some(1));
        } else {
            panic!("expected promote success");
        }
    }

    #[test]
    fn orchestrate_relate_persists_relation() {
        let ports = MemoryBaselinePorts::new(MemoryStore::default());
        let rel = json!({
            "schema_version": 1,
            "relation_id": "rel_1",
            "relation_type": "related_to",
            "from_id": "kb_a",
            "to_id": "kb_b",
            "extensions": {}
        });

        let result = orchestrate_relate(&ports, relate_request(rel.clone()));
        assert!(result.is_ok());
        if let SpokeResult::Ok(RelateResponse::Variant0 { relation: got, .. }) = result {
            assert_eq!(got.relation_id, "rel_1");
        } else {
            panic!("expected relate success");
        }
        let _ = relation(rel);
    }

    #[test]
    fn orchestrate_check_loads_scope_runs_checker_and_puts_findings() {
        let entry = ke(json!({
            "schema_version": 1,
            "entry_id": "kb_check",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "provisional",
            "body": { "summary": "Protagonist" },
            "extensions": {}
        }));
        let event = timeline_event(json!({
            "schema_version": 1,
            "timeline_event_id": "te_1",
            "canonical_name": "Arrival",
            "extensions": {}
        }));
        let rule = rule(json!({
            "schema_version": 1,
            "rule_id": "rule_1",
            "canonical_name": "No orphans",
            "kind": "rule",
            "extensions": {}
        }));
        let mut store = MemoryStore::default();
        store.entries.insert(entry.entry_id.clone(), entry.clone());
        store.events.push(event.clone());
        store.rules.insert(rule.rule_id.clone(), rule.clone());
        let ports = MemoryBaselinePorts::new(store);

        let request = check_request(json!({
            "scope": { "scope_id": "world_1", "entry_ids": ["kb_check"] },
            "rule_refs": ["rule_1"]
        }));
        let finding = finding(json!({
            "schema_version": 1,
            "finding_id": "f_1",
            "severity": "warning",
            "status": "open",
            "title": "Issue",
            "description": "Detected by mock checker",
            "target_entry_id": "kb_check",
            "extensions": {}
        }));

        let result = orchestrate_check(&ports, request.clone(), |input| {
            assert_eq!(input.entries.len(), 1);
            assert_eq!(input.entries[0].entry_id, "kb_check");
            assert_eq!(input.events.len(), 1);
            assert_eq!(input.rules.len(), 1);
            assert_eq!(input.rules[0].rule_id, "rule_1");
            spoke_ok(vec![finding.clone()])
        });

        assert!(result.is_ok());
        if let SpokeResult::Ok(CheckResponse::Variant0 { findings, .. }) = result {
            assert_eq!(findings.len(), 1);
            assert_eq!(findings[0].finding_id, "f_1");
        } else {
            panic!("expected check success");
        }
        let _ = (entry, event, rule, request);
    }

    #[test]
    fn orchestrate_assemble_builds_packet_from_scoped_knowledge_entries() {
        let entry = ke(json!({
            "schema_version": 1,
            "entry_id": "kb_assemble",
            "entry_type": "character",
            "canonical_name": "Assemble Hero",
            "status": "provisional",
            "body": { "summary": "Context snippet" },
            "extensions": {}
        }));
        let ports = MemoryBaselinePorts::with_entry(entry);
        let request = assemble_request(json!({
            "scope": { "scope_id": "world_1", "entry_ids": ["kb_assemble"] },
            "max_entries": 10
        }));

        let result = orchestrate_assemble(&ports, request);
        assert!(result.is_ok());
        if let SpokeResult::Ok(AssembleResponse::Variant0 { packet, .. }) = result {
            assert_eq!(packet.packet_id, "assemble:world_1");
            assert_eq!(packet.entries.len(), 1);
            assert_eq!(packet.entries[0].canonical_name.as_str(), "Assemble Hero");
            assert_eq!(packet.entries[0].snippet.as_deref(), Some("Context snippet"));
        } else {
            panic!("expected assemble success");
        }
    }

    #[test]
    fn orchestrate_project_validates_then_calls_computable_port() {
        let ports = MemoryComputablePorts::new(MemoryBaselinePorts::new(MemoryStore::default()));
        let request = project_request(json!({
            "session_id": "sess_1",
            "entry_id": "kb_1",
            "state": { "tide_level": 2.4 }
        }));

        let result = orchestrate_project(&ports, request);
        assert!(result.is_ok());
        if let SpokeResult::Ok(ProjectResponse::Variant0 {
            session_id,
            entry_id,
            computable,
            ..
        }) = result
        {
            assert_eq!(session_id, "sess_1");
            assert_eq!(entry_id, "kb_1");
            assert_eq!(computable.get("tide_level"), Some(&json!(2.4)));
            assert_eq!(computable.get("projected"), Some(&json!(true)));
        } else {
            panic!("expected project success");
        }
        assert_eq!(ports.projected.borrow().len(), 1);
    }

    #[test]
    fn orchestrate_compute_validates_then_calls_computable_port() {
        let ports = MemoryComputablePorts::new(MemoryBaselinePorts::new(MemoryStore::default()));
        let request = compute_request(json!({
            "session_id": "sess_1",
            "entry_id": "kb_1",
            "computable": { "tide_level": 3.1 },
            "settle": true
        }));

        let result = orchestrate_compute(&ports, request);
        assert!(result.is_ok());
        if let SpokeResult::Ok(ComputeResponse::Variant0 {
            session_id,
            entry_id,
            computable,
            state,
            ..
        }) = result
        {
            assert_eq!(session_id, "sess_1");
            assert_eq!(entry_id, "kb_1");
            assert_eq!(computable.get("tide_level"), Some(&json!(3.1)));
            assert_eq!(state.get("tide_level"), Some(&json!(3.1)));
        } else {
            panic!("expected compute success");
        }
        assert_eq!(ports.computed.borrow().len(), 1);
    }

    #[test]
    fn orchestrate_fork_check_uses_fork_timeline_port() {
        let entry = ke(json!({
            "schema_version": 1,
            "entry_id": "kb_fork",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "provisional",
            "body": { "summary": "Protagonist" },
            "extensions": {}
        }));
        let on_fork = timeline_event(json!({
            "schema_version": 1,
            "timeline_event_id": "te_fork",
            "canonical_name": "On branch",
            "fork_id": "fork_a",
            "extensions": {}
        }));
        let other_fork = timeline_event(json!({
            "schema_version": 1,
            "timeline_event_id": "te_other",
            "canonical_name": "Other branch",
            "fork_id": "fork_b",
            "extensions": {}
        }));
        let mut store = MemoryStore::default();
        store.entries.insert(entry.entry_id.clone(), entry);
        store.events.push(on_fork.clone());
        store.events.push(other_fork);
        let ports = MemoryForkPorts::new(MemoryBaselinePorts::new(store));

        let request = check_request(json!({
            "scope": {
                "scope_id": "world_1",
                "entry_ids": ["kb_fork"],
                "fork_id": "fork_a"
            }
        }));
        let finding = finding(json!({
            "schema_version": 1,
            "finding_id": "f_fork",
            "severity": "warning",
            "status": "open",
            "title": "Issue",
            "description": "Detected by mock checker",
            "extensions": {}
        }));

        let result = orchestrate_fork_check(&ports, request, |input| {
            assert_eq!(input.entries.len(), 1);
            assert_eq!(input.events.len(), 1);
            assert_eq!(input.events[0].timeline_event_id, "te_fork");
            spoke_ok(vec![finding.clone()])
        });

        assert!(result.is_ok());
        assert_eq!(ports.fork_list_calls.borrow().len(), 1);
        let _ = on_fork;
    }

    #[test]
    fn returns_capability_port_missing_when_project_absent() {
        let ports = MissingComputablePorts {
            baseline: MemoryBaselinePorts::new(MemoryStore::default()),
        };
        let request = project_request(json!({
            "session_id": "sess_1",
            "entry_id": "kb_1",
            "state": {}
        }));

        let result = orchestrate_project(&ports, request);
        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
        }
    }

    #[test]
    fn returns_capability_port_missing_when_fork_timeline_absent() {
        let ports = MissingForkPorts {
            baseline: MemoryBaselinePorts::new(MemoryStore::default()),
        };
        let request = check_request(json!({
            "scope": { "scope_id": "world_1", "fork_id": "fork_a" }
        }));

        let result = orchestrate_fork_check(&ports, request, |_| spoke_ok(Vec::new()));
        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
        }
    }

    trait ExpectOkEntry {
        fn expect_ok_entry(self, context: &str) -> KnowledgeEntry;
    }

    impl ExpectOkEntry for SpokeResult<KnowledgeEntry> {
        fn expect_ok_entry(self, context: &str) -> KnowledgeEntry {
            match self {
                SpokeResult::Ok(entry) => entry,
                SpokeResult::Reject(reject) => panic!("{context}: {reject:?}"),
            }
        }
    }
}
