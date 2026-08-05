//! Injection orchestration entrypoints — compose pure helpers with port I/O.

use crate::adapter::ports::{
    BaselinePorts, ComputablePorts, ForkPorts, KnowledgeEntryPort, RelationPort,
};
use crate::assemble::{build_assemble_packet, BuildAssemblePacketInput, KnowledgeEntryForAssemble};
use crate::computable::{validate_compute_request, validate_project_request};
use crate::extensions::ExtensionMap;
use crate::knowledge_entry::{
    assert_unique_active_knowledge_entry, is_valid_knowledge_entry_status_transition,
    AssertUniqueActiveKnowledgeEntryInput,
};
use crate::occ::assert_revision_match;
use crate::promote::{apply_promote_acceptance, validate_promote_request};
use crate::relate::{validate_relate_request, RelateMode, ValidateRelateRequestContext};
use crate::result::{spoke_ok, spoke_ok_unit, spoke_reject, SpokeRejectCode, SpokeResult};
use crate::scope::{filter_knowledge_entries_by_scope, filter_timeline_events_by_scope};
use crate::upsert::{validate_upsert_knowledge_entry, ValidateUpsertKnowledgeEntryContext};

const TERMINAL_KNOWLEDGE_ENTRY_STATUSES: &[&str] = &["merged", "deleted"];
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

async fn load_stored_knowledge_entry(
    ports: &impl KnowledgeEntryPort,
    entry_id: &str,
) -> SpokeResult<Option<KnowledgeEntry>> {
    match ports.get_knowledge_entry(entry_id).await {
        SpokeResult::Ok(entry) => spoke_ok(Some(entry)),
        SpokeResult::Reject(reject)
            if reject.code == SpokeRejectCode::KnowledgeEntryNotFound =>
        {
            spoke_ok(None)
        }
        SpokeResult::Reject(reject) => SpokeResult::Reject(reject),
    }
}

async fn load_stored_relation(
    ports: &impl RelationPort,
    relation_id: &str,
) -> SpokeResult<Option<Relation>> {
    match ports.get_relation(relation_id).await {
        SpokeResult::Ok(relation) => spoke_ok(Some(relation)),
        SpokeResult::Reject(reject) if reject.code == SpokeRejectCode::RelationNotFound => {
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
pub async fn orchestrate_upsert(
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

        let stored = match load_stored_knowledge_entry(ports, &candidate.entry_id).await {
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

        let expected_base_revision = stored
            .as_ref()
            .map(|stored| stored.revision.unwrap_or(0));

        let mut uniqueness_existing = batch_peers.clone();
        if let Some(stored_entry) = stored {
            uniqueness_existing.push(stored_entry);
        }
        if let SpokeResult::Reject(reject) =
            assert_uniqueness_when_applicable(&candidate, &uniqueness_existing)
        {
            return SpokeResult::Reject(reject);
        }

        let put = match ports.put_knowledge_entry(candidate, expected_base_revision).await {
            SpokeResult::Ok(entry) => entry,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };

        persisted.push(put.clone());
        batch_peers.push(put);
    }

    success_response(json!({ "knowledge_entries": persisted }))
}

/// Promote a provisional candidate, then persist via KnowledgeEntryPort.
pub async fn orchestrate_promote(
    ports: &impl BaselinePorts,
    request: PromoteRequest,
) -> SpokeResult<PromoteResponse> {
    let stored = match load_stored_knowledge_entry(ports, &request.candidate.entry_id).await {
        SpokeResult::Ok(stored) => stored,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    if let Some(stored) = stored.as_ref() {
        if TERMINAL_KNOWLEDGE_ENTRY_STATUSES.contains(&stored.status.as_str()) {
            let mut details = Map::new();
            details.insert("status".into(), Value::String(stored.status.clone()));
            return spoke_reject(
                SpokeRejectCode::KnowledgeEntryTerminalStatus,
                format!("Stored KnowledgeEntry has terminal status: {}", stored.status),
                Some(details),
            );
        }

        if let SpokeResult::Reject(reject) = assert_revision_match(
            request.candidate.revision.unwrap_or(0),
            stored.revision.unwrap_or(0),
        ) {
            return SpokeResult::Reject(reject);
        }
    }

    if let SpokeResult::Reject(reject) = validate_promote_request(&request) {
        return SpokeResult::Reject(reject);
    }

    let mut accepted = match apply_promote_acceptance(&request) {
        SpokeResult::Ok(entry) => entry,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    // When stored exists, base the persisted revision on stored — do not trust
    // candidate-only bump if it could diverge from the loaded OCC base.
    if let Some(stored) = stored.as_ref() {
        accepted.revision = Some(stored.revision.unwrap_or(0) + 1);
    }

    let expected_base_revision = stored.as_ref().map(|stored| stored.revision.unwrap_or(0));

    let put = match ports.put_knowledge_entry(accepted, expected_base_revision).await {
        SpokeResult::Ok(entry) => entry,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let mut body = json!({ "knowledge_entry": put });
    if let Some(target_entry_id) = &request.target_entry_id {
        body["superseded_id"] = json!(target_entry_id);
    }
    success_response(body)
}

/// Validate and persist a Relation: load stored, validate (create vs update),
/// run OCC-aware put. Mirrors orchestrate_upsert.
pub async fn orchestrate_relate(
    ports: &impl BaselinePorts,
    request: RelateRequest,
) -> SpokeResult<RelateResponse> {
    let relation: Relation = match wire_convert(&request.relation) {
        SpokeResult::Ok(relation) => relation,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let stored = match load_stored_relation(ports, &relation.relation_id).await {
        SpokeResult::Ok(stored) => stored,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let validation = validate_relate_request(
        &relation,
        ValidateRelateRequestContext {
            stored: stored.as_ref(),
            mode: match stored.as_ref() {
                None => Some(RelateMode::Create),
                Some(_) => Some(RelateMode::Update),
            },
        },
    );
    if let SpokeResult::Reject(reject) = validation {
        return SpokeResult::Reject(reject);
    }

    let expected_base_revision = stored.as_ref().map(|stored| stored.revision.unwrap_or(0));

    let put = match ports.put_relation(relation, expected_base_revision).await {
        SpokeResult::Ok(relation) => relation,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    success_response(json!({ "relation": put }))
}

async fn resolve_check_rules(
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

    let resolved = match ports.list_rules(&request.rule_refs).await {
        SpokeResult::Ok(rules) => rules,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    // Start from resolved refs; embedded rules win by rule_id (replace or append).
    let mut merged = resolved;
    for rule in embedded {
        if let Some(index) = merged.iter().position(|item| item.rule_id == rule.rule_id) {
            merged[index] = rule;
        } else {
            merged.push(rule);
        }
    }
    spoke_ok(merged)
}

/// Load scoped data and rules, invoke product checker callback, persist findings.
pub async fn orchestrate_check<F>(
    ports: &impl BaselinePorts,
    request: CheckRequest,
    run_checker: F,
) -> SpokeResult<CheckResponse>
where
    F: FnOnce(CheckRunInput) -> SpokeResult<Vec<Finding>>,
{
    let rules = match resolve_check_rules(ports, &request).await {
        SpokeResult::Ok(rules) => rules,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let scope = match scope_from_check_request(&request.scope) {
        SpokeResult::Ok(scope) => scope,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let entries_result = match ports.list_knowledge_entries(&scope).await {
        SpokeResult::Ok(entries) => entries,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let events_result = match ports.list_timeline_events(&scope).await {
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

    let put = match ports.put_findings(findings).await {
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
pub async fn orchestrate_assemble(
    ports: &impl BaselinePorts,
    request: AssembleRequest,
) -> SpokeResult<AssembleResponse> {
    let scope = match scope_from_assemble_request(&request.scope) {
        SpokeResult::Ok(scope) => scope,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let entries_result = match ports.list_knowledge_entries(&scope).await {
        SpokeResult::Ok(entries) => entries,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let events_result = match ports.list_timeline_events(&scope).await {
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
pub async fn orchestrate_project(
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
    computable.project(request).await
}

/// Compute Session updates via ComputablePort after request validation.
/// Settled-state persistence remains an explicit adapter step.
pub async fn orchestrate_compute(
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
    computable.compute(request).await
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
pub async fn orchestrate_fork_check<F>(
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

    let rules = match resolve_check_rules(ports, &request).await {
        SpokeResult::Ok(rules) => rules,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let entries_result = match ports.list_knowledge_entries(&scope).await {
        SpokeResult::Ok(entries) => entries,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let fork_timeline = ports
        .as_fork_timeline()
        .expect("require_port_method gated availability");
    let events_result = match fork_timeline.list_fork_timeline_events(&fork_scope).await {
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

    let put = match ports.put_findings(findings).await {
        SpokeResult::Ok(findings) => findings,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    success_response(json!({ "findings": put }))
}

/// Fork-aware assemble: KE via ScopeQueryPort; timeline via ForkTimelineQueryPort.
pub async fn orchestrate_fork_assemble(
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

    let entries_result = match ports.list_knowledge_entries(&scope).await {
        SpokeResult::Ok(entries) => entries,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let fork_timeline = ports
        .as_fork_timeline()
        .expect("require_port_method gated availability");
    let events_result = match fork_timeline.list_fork_timeline_events(&fork_scope).await {
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
    use async_trait::async_trait;
    use crate::adapter::ports::{
        ComputablePort, FindingPort, ForkTimelineQueryPort, HostManifestPort, KnowledgeEntryPort,
        RelationPort, RuleQueryPort, ScopeQueryPort,
    };
    use crate::result::SpokeResult;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Mutex;

    fn put_knowledge_entry_with_occ(
        entries: &mut HashMap<String, KnowledgeEntry>,
        entry: KnowledgeEntry,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<KnowledgeEntry> {
        let existing = entries.get(&entry.entry_id);
        if expected_base_revision.is_none() {
            if existing.is_some() {
                let mut details = Map::new();
                details.insert("entry_id".into(), json!(entry.entry_id));
                return spoke_reject(
                    SpokeRejectCode::RevisionConflict,
                    format!("Entry already exists: {}", entry.entry_id),
                    Some(details),
                );
            }
        } else {
            let expected = expected_base_revision.unwrap_or(0);
            match existing {
                None => {
                    let mut details = Map::new();
                    details.insert("entry_id".into(), json!(entry.entry_id));
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
            }
        }
        entries.insert(entry.entry_id.clone(), entry.clone());
        spoke_ok(entry)
    }

    fn put_relation_with_occ(
        relations: &mut HashMap<String, Relation>,
        mut relation: Relation,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<Relation> {
        let existing = relations.get(&relation.relation_id);
        match expected_base_revision {
            None => {
                if existing.is_some() {
                    let mut details = Map::new();
                    details.insert("relation_id".into(), json!(relation.relation_id));
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
                    details.insert("relation_id".into(), json!(relation.relation_id));
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
        relations.insert(relation.relation_id.clone(), relation.clone());
        spoke_ok(relation)
    }

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

    #[async_trait]
    impl KnowledgeEntryPort for MemoryBaselinePorts {
        async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
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

        async fn put_knowledge_entry(
            &self,
            entry: KnowledgeEntry,
            expected_base_revision: Option<u64>,
        ) -> SpokeResult<KnowledgeEntry> {
            let mut store = self.store.lock().expect("store lock");
            put_knowledge_entry_with_occ(&mut store.entries, entry, expected_base_revision)
        }
    }

    #[async_trait]
    impl RelationPort for MemoryBaselinePorts {
        async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
            let store = self.store.lock().expect("store lock");
            match store.relations.get(relation_id) {
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

        async fn put_relation(
            &self,
            relation: Relation,
            expected_base_revision: Option<u64>,
        ) -> SpokeResult<Relation> {
            let mut store = self.store.lock().expect("store lock");
            put_relation_with_occ(&mut store.relations, relation, expected_base_revision)
        }
    }

    #[async_trait]
    impl ScopeQueryPort for MemoryBaselinePorts {
        async fn list_knowledge_entries(&self, _scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
            let store = self.store.lock().expect("store lock");
            spoke_ok(store.entries.values().cloned().collect())
        }

        async fn list_timeline_events(&self, _scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
            let store = self.store.lock().expect("store lock");
            spoke_ok(store.events.clone())
        }
    }

    #[async_trait]
    impl FindingPort for MemoryBaselinePorts {
        async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
            let mut store = self.store.lock().expect("store lock");
            store.findings.extend(findings.iter().cloned());
            spoke_ok(findings)
        }
    }

    #[async_trait]
    impl RuleQueryPort for MemoryBaselinePorts {
        async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
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

    fn memory_baseline_host_manifest() -> spoke_schemas::HostCapabilityManifest {
        serde_json::from_value(json!({
            "schema_version": 1,
            "host_id": "memory-baseline",
            "roles": ["data-store"],
            "capabilities": ["spoke-baseline"],
            "namespaces": ["default"],
            "extensions": {}
        }))
        .expect("valid HostCapabilityManifest")
    }

    #[async_trait]
    impl HostManifestPort for MemoryBaselinePorts {
        async fn get_host_capability_manifest(&self) -> SpokeResult<spoke_schemas::HostCapabilityManifest> {
            spoke_ok(memory_baseline_host_manifest())
        }

        async fn list_peer_host_capability_manifests(
            &self,
        ) -> SpokeResult<Vec<spoke_schemas::HostCapabilityManifest>> {
            spoke_ok(Vec::new())
        }
    }

    struct MemoryComputablePorts {
        baseline: MemoryBaselinePorts,
        projected: Mutex<Vec<ProjectRequest>>,
        computed: Mutex<Vec<ComputeRequest>>,
    }

    impl MemoryComputablePorts {
        fn new(baseline: MemoryBaselinePorts) -> Self {
            Self {
                baseline,
                projected: Mutex::new(Vec::new()),
                computed: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl KnowledgeEntryPort for MemoryComputablePorts {
        async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
            self.baseline.get_knowledge_entry(entry_id).await
        }
        async fn put_knowledge_entry(
            &self,
            entry: KnowledgeEntry,
            expected_base_revision: Option<u64>,
        ) -> SpokeResult<KnowledgeEntry> {
            self.baseline
                .put_knowledge_entry(entry, expected_base_revision).await
        }
    }
    #[async_trait]
    impl RelationPort for MemoryComputablePorts {
        async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
            self.baseline.get_relation(relation_id).await
        }
        async fn put_relation(
            &self,
            relation: Relation,
            expected_base_revision: Option<u64>,
        ) -> SpokeResult<Relation> {
            self.baseline.put_relation(relation, expected_base_revision).await
        }
    }
    #[async_trait]
    impl ScopeQueryPort for MemoryComputablePorts {
        async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
            self.baseline.list_knowledge_entries(scope).await
        }
        async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
            self.baseline.list_timeline_events(scope).await
        }
    }
    #[async_trait]
    impl FindingPort for MemoryComputablePorts {
        async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
            self.baseline.put_findings(findings).await
        }
    }
    #[async_trait]
    impl RuleQueryPort for MemoryComputablePorts {
        async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
            self.baseline.list_rules(rule_refs).await
        }
    }
    #[async_trait]
    impl HostManifestPort for MemoryComputablePorts {
        async fn get_host_capability_manifest(&self) -> SpokeResult<spoke_schemas::HostCapabilityManifest> {
            self.baseline.get_host_capability_manifest().await
        }

        async fn list_peer_host_capability_manifests(
            &self,
        ) -> SpokeResult<Vec<spoke_schemas::HostCapabilityManifest>> {
            self.baseline.list_peer_host_capability_manifests().await
        }
    }
    #[async_trait]
    impl ComputablePort for MemoryComputablePorts {
        async fn project(&self, request: ProjectRequest) -> SpokeResult<ProjectResponse> {
            self.projected.lock().expect("projected lock").push(request.clone());
            let mut computable = request.state.clone();
            computable.insert("projected".into(), json!(true));
            success_response(json!({
                "session_id": request.session_id,
                "entry_id": request.entry_id,
                "computable": computable,
            }))
        }

        async fn compute(&self, request: ComputeRequest) -> SpokeResult<ComputeResponse> {
            self.computed.lock().expect("computed lock").push(request.clone());
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
        fork_list_calls: Mutex<Vec<Scope>>,
    }

    impl MemoryForkPorts {
        fn new(baseline: MemoryBaselinePorts) -> Self {
            Self {
                baseline,
                fork_list_calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl KnowledgeEntryPort for MemoryForkPorts {
        async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
            self.baseline.get_knowledge_entry(entry_id).await
        }
        async fn put_knowledge_entry(
            &self,
            entry: KnowledgeEntry,
            expected_base_revision: Option<u64>,
        ) -> SpokeResult<KnowledgeEntry> {
            self.baseline
                .put_knowledge_entry(entry, expected_base_revision).await
        }
    }
    #[async_trait]
    impl RelationPort for MemoryForkPorts {
        async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
            self.baseline.get_relation(relation_id).await
        }
        async fn put_relation(
            &self,
            relation: Relation,
            expected_base_revision: Option<u64>,
        ) -> SpokeResult<Relation> {
            self.baseline.put_relation(relation, expected_base_revision).await
        }
    }
    #[async_trait]
    impl ScopeQueryPort for MemoryForkPorts {
        async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
            self.baseline.list_knowledge_entries(scope).await
        }
        async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
            self.baseline.list_timeline_events(scope).await
        }
    }
    #[async_trait]
    impl FindingPort for MemoryForkPorts {
        async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
            self.baseline.put_findings(findings).await
        }
    }
    #[async_trait]
    impl RuleQueryPort for MemoryForkPorts {
        async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
            self.baseline.list_rules(rule_refs).await
        }
    }
    #[async_trait]
    impl HostManifestPort for MemoryForkPorts {
        async fn get_host_capability_manifest(&self) -> SpokeResult<spoke_schemas::HostCapabilityManifest> {
            self.baseline.get_host_capability_manifest().await
        }

        async fn list_peer_host_capability_manifests(
            &self,
        ) -> SpokeResult<Vec<spoke_schemas::HostCapabilityManifest>> {
            self.baseline.list_peer_host_capability_manifests().await
        }
    }
    #[async_trait]
    impl ForkTimelineQueryPort for MemoryForkPorts {
        async fn list_fork_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
            self.fork_list_calls.lock().expect("fork list lock").push(scope.clone());
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

    #[async_trait]
    impl KnowledgeEntryPort for MissingComputablePorts {
        async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
            self.baseline.get_knowledge_entry(entry_id).await
        }
        async fn put_knowledge_entry(
            &self,
            entry: KnowledgeEntry,
            expected_base_revision: Option<u64>,
        ) -> SpokeResult<KnowledgeEntry> {
            self.baseline
                .put_knowledge_entry(entry, expected_base_revision).await
        }
    }
    #[async_trait]
    impl RelationPort for MissingComputablePorts {
        async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
            self.baseline.get_relation(relation_id).await
        }
        async fn put_relation(
            &self,
            relation: Relation,
            expected_base_revision: Option<u64>,
        ) -> SpokeResult<Relation> {
            self.baseline.put_relation(relation, expected_base_revision).await
        }
    }
    #[async_trait]
    impl ScopeQueryPort for MissingComputablePorts {
        async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
            self.baseline.list_knowledge_entries(scope).await
        }
        async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
            self.baseline.list_timeline_events(scope).await
        }
    }
    #[async_trait]
    impl FindingPort for MissingComputablePorts {
        async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
            self.baseline.put_findings(findings).await
        }
    }
    #[async_trait]
    impl RuleQueryPort for MissingComputablePorts {
        async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
            self.baseline.list_rules(rule_refs).await
        }
    }
    #[async_trait]
    impl HostManifestPort for MissingComputablePorts {
        async fn get_host_capability_manifest(&self) -> SpokeResult<spoke_schemas::HostCapabilityManifest> {
            self.baseline.get_host_capability_manifest().await
        }

        async fn list_peer_host_capability_manifests(
            &self,
        ) -> SpokeResult<Vec<spoke_schemas::HostCapabilityManifest>> {
            self.baseline.list_peer_host_capability_manifests().await
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

    #[async_trait]
    impl KnowledgeEntryPort for MissingForkPorts {
        async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
            self.baseline.get_knowledge_entry(entry_id).await
        }
        async fn put_knowledge_entry(
            &self,
            entry: KnowledgeEntry,
            expected_base_revision: Option<u64>,
        ) -> SpokeResult<KnowledgeEntry> {
            self.baseline
                .put_knowledge_entry(entry, expected_base_revision).await
        }
    }
    #[async_trait]
    impl RelationPort for MissingForkPorts {
        async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
            self.baseline.get_relation(relation_id).await
        }
        async fn put_relation(
            &self,
            relation: Relation,
            expected_base_revision: Option<u64>,
        ) -> SpokeResult<Relation> {
            self.baseline.put_relation(relation, expected_base_revision).await
        }
    }
    #[async_trait]
    impl ScopeQueryPort for MissingForkPorts {
        async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
            self.baseline.list_knowledge_entries(scope).await
        }
        async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
            self.baseline.list_timeline_events(scope).await
        }
    }
    #[async_trait]
    impl FindingPort for MissingForkPorts {
        async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
            self.baseline.put_findings(findings).await
        }
    }
    #[async_trait]
    impl RuleQueryPort for MissingForkPorts {
        async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
            self.baseline.list_rules(rule_refs).await
        }
    }
    #[async_trait]
    impl HostManifestPort for MissingForkPorts {
        async fn get_host_capability_manifest(&self) -> SpokeResult<spoke_schemas::HostCapabilityManifest> {
            self.baseline.get_host_capability_manifest().await
        }

        async fn list_peer_host_capability_manifests(
            &self,
        ) -> SpokeResult<Vec<spoke_schemas::HostCapabilityManifest>> {
            self.baseline.list_peer_host_capability_manifests().await
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

        let result = pollster::block_on(orchestrate_upsert(&ports, upsert_request(vec![candidate.clone()])));
        assert!(result.is_ok());
        if let SpokeResult::Ok(UpsertResponse::Variant0 { knowledge_entries, .. }) = result {
            assert_eq!(knowledge_entries.len(), 1);
            assert_eq!(knowledge_entries[0].entry_id, "kb_new");
        } else {
            panic!("expected upsert success");
        }

        let stored = pollster::block_on(ports
            .get_knowledge_entry("kb_new"))
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

        let result = pollster::block_on(orchestrate_promote(&ports, promote_request(candidate)));
        assert!(result.is_ok());
        if let SpokeResult::Ok(PromoteResponse::Variant0 { knowledge_entry, .. }) = result {
            assert_eq!(knowledge_entry.status, "confirmed");
            assert_eq!(knowledge_entry.revision, Some(1));
        } else {
            panic!("expected promote success");
        }
    }

    #[test]
    fn orchestrate_promote_rejects_when_stored_status_is_terminal() {
        let stored = ke(json!({
            "schema_version": 1,
            "entry_id": "kb_promote_terminal",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "merged",
            "revision": 1,
            "body": { "summary": "Protagonist" },
            "extensions": {}
        }));
        let mut store = MemoryStore::default();
        store.entries.insert(stored.entry_id.clone(), stored);
        let ports = MemoryBaselinePorts::new(store);

        let candidate = json!({
            "schema_version": 1,
            "entry_id": "kb_promote_terminal",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "provisional",
            "revision": 1,
            "body": { "summary": "Protagonist" },
            "extensions": {}
        });

        let result = pollster::block_on(orchestrate_promote(&ports, promote_request(candidate)));
        assert!(!result.is_ok());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(
                reject.code,
                SpokeRejectCode::KnowledgeEntryTerminalStatus
            );
        } else {
            panic!("expected terminal status reject");
        }
    }

    #[test]
    fn orchestrate_promote_rejects_on_stored_revision_mismatch() {
        let stored = ke(json!({
            "schema_version": 1,
            "entry_id": "kb_promote_rev",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "provisional",
            "revision": 3,
            "body": { "summary": "Protagonist" },
            "extensions": {}
        }));
        let mut store = MemoryStore::default();
        store.entries.insert(stored.entry_id.clone(), stored);
        let ports = MemoryBaselinePorts::new(store);

        let candidate = json!({
            "schema_version": 1,
            "entry_id": "kb_promote_rev",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "provisional",
            "revision": 1,
            "body": { "summary": "Protagonist" },
            "extensions": {}
        });

        let result = pollster::block_on(orchestrate_promote(&ports, promote_request(candidate)));
        assert!(!result.is_ok());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::StoredRevisionStale);
        } else {
            panic!("expected revision mismatch reject");
        }
    }

    #[test]
    fn orchestrate_promote_succeeds_when_stored_provisional_matches_revision() {
        let stored = ke(json!({
            "schema_version": 1,
            "entry_id": "kb_promote_match",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "provisional",
            "revision": 2,
            "body": { "summary": "Protagonist" },
            "extensions": {}
        }));
        let mut store = MemoryStore::default();
        store.entries.insert(stored.entry_id.clone(), stored);
        let ports = MemoryBaselinePorts::new(store);

        let candidate = json!({
            "schema_version": 1,
            "entry_id": "kb_promote_match",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "provisional",
            "revision": 2,
            "body": { "summary": "Protagonist" },
            "extensions": {}
        });

        let result = pollster::block_on(orchestrate_promote(&ports, promote_request(candidate)));
        assert!(result.is_ok());
        if let SpokeResult::Ok(PromoteResponse::Variant0 { knowledge_entry, .. }) = result {
            assert_eq!(knowledge_entry.status, "confirmed");
            assert_eq!(knowledge_entry.revision, Some(3));
        } else {
            panic!("expected promote success");
        }
    }

    /// OCC-aware adapter: put rejects when store revision does not match `expected_base_revision`.
    /// `advance_on_get` simulates a concurrent writer racing between get and put.
    struct OccBaselinePorts {
        baseline: MemoryBaselinePorts,
        store_revision: Mutex<u64>,
        advance_on_get: Mutex<bool>,
        puts: Mutex<Vec<(KnowledgeEntry, Option<u64>)>>,
    }

    impl OccBaselinePorts {
        fn new(entry: KnowledgeEntry, advance_on_get: bool) -> Self {
            let store_revision = entry.revision.unwrap_or(0);
            Self {
                baseline: MemoryBaselinePorts::with_entry(entry),
                store_revision: Mutex::new(store_revision),
                advance_on_get: Mutex::new(advance_on_get),
                puts: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl KnowledgeEntryPort for OccBaselinePorts {
        async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
            let result = self.baseline.get_knowledge_entry(entry_id).await;
            if result.is_ok() {
                let mut advance = self.advance_on_get.lock().expect("advance lock");
                if *advance {
                    *self.store_revision.lock().expect("rev lock") += 1;
                    *advance = false;
                }
            }
            result
        }

        async fn put_knowledge_entry(
            &self,
            entry: KnowledgeEntry,
            expected_base_revision: Option<u64>,
        ) -> SpokeResult<KnowledgeEntry> {
            let store_revision = *self.store_revision.lock().expect("rev lock");
            if let Some(expected_base) = expected_base_revision {
                if store_revision != expected_base {
                    let mut details = Map::new();
                    details.insert("expectedBaseRevision".into(), json!(expected_base));
                    details.insert("storeRevision".into(), json!(store_revision));
                    return spoke_reject(
                        SpokeRejectCode::StoredRevisionStale,
                        format!(
                            "Store revision {store_revision} is ahead of expected base {expected_base}"
                        ),
                        Some(details),
                    );
                }
            }
            *self.store_revision.lock().expect("rev lock") = entry.revision.unwrap_or(0);
            self.puts
                .lock()
                .expect("puts lock")
                .push((entry.clone(), expected_base_revision));
            self.baseline
                .put_knowledge_entry(entry, expected_base_revision).await
        }
    }

    #[async_trait]
    impl RelationPort for OccBaselinePorts {
        async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
            self.baseline.get_relation(relation_id).await
        }
        async fn put_relation(
            &self,
            relation: Relation,
            expected_base_revision: Option<u64>,
        ) -> SpokeResult<Relation> {
            self.baseline.put_relation(relation, expected_base_revision).await
        }
    }
    #[async_trait]
    impl ScopeQueryPort for OccBaselinePorts {
        async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
            self.baseline.list_knowledge_entries(scope).await
        }
        async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
            self.baseline.list_timeline_events(scope).await
        }
    }
    #[async_trait]
    impl FindingPort for OccBaselinePorts {
        async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
            self.baseline.put_findings(findings).await
        }
    }
    #[async_trait]
    impl RuleQueryPort for OccBaselinePorts {
        async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
            self.baseline.list_rules(rule_refs).await
        }
    }
    #[async_trait]
    impl HostManifestPort for OccBaselinePorts {
        async fn get_host_capability_manifest(&self) -> SpokeResult<spoke_schemas::HostCapabilityManifest> {
            self.baseline.get_host_capability_manifest().await
        }

        async fn list_peer_host_capability_manifests(
            &self,
        ) -> SpokeResult<Vec<spoke_schemas::HostCapabilityManifest>> {
            self.baseline.list_peer_host_capability_manifests().await
        }
    }

    #[test]
    fn orchestrate_promote_forces_persisted_revision_to_stored_plus_one() {
        let stored = ke(json!({
            "schema_version": 1,
            "entry_id": "kb_promote_force_rev",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "provisional",
            "revision": 7,
            "body": { "summary": "Protagonist" },
            "extensions": {}
        }));
        let ports = OccBaselinePorts::new(stored, false);

        let candidate = json!({
            "schema_version": 1,
            "entry_id": "kb_promote_force_rev",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "provisional",
            "revision": 7,
            "body": { "summary": "Protagonist" },
            "extensions": {}
        });

        let result = pollster::block_on(orchestrate_promote(&ports, promote_request(candidate)));
        assert!(result.is_ok());
        let puts = ports.puts.lock().expect("puts lock");
        assert_eq!(puts.len(), 1);
        assert_eq!(puts[0].1, Some(7));
        assert_eq!(puts[0].0.revision, Some(8));
        if let SpokeResult::Ok(PromoteResponse::Variant0 { knowledge_entry, .. }) = result {
            assert_eq!(knowledge_entry.revision, Some(8));
        } else {
            panic!("expected promote success");
        }
    }

    #[test]
    fn orchestrate_promote_propagates_adapter_occ_reject_on_concurrent_advance() {
        let stored = ke(json!({
            "schema_version": 1,
            "entry_id": "kb_promote_occ",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "provisional",
            "revision": 2,
            "body": { "summary": "Protagonist" },
            "extensions": {}
        }));
        let ports = OccBaselinePorts::new(stored, true);

        let candidate = json!({
            "schema_version": 1,
            "entry_id": "kb_promote_occ",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "provisional",
            "revision": 2,
            "body": { "summary": "Protagonist" },
            "extensions": {}
        });

        let result = pollster::block_on(orchestrate_promote(&ports, promote_request(candidate)));
        assert!(!result.is_ok());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::StoredRevisionStale);
        } else {
            panic!("expected OCC reject");
        }
    }

    #[test]
    fn orchestrate_upsert_passes_none_expected_base_revision_on_create() {
        let ports = MemoryBaselinePorts::new(MemoryStore::default());
        let candidate = json!({
            "schema_version": 1,
            "entry_id": "kb_create_occ",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "provisional",
            "body": { "summary": "Protagonist" },
            "extensions": {}
        });

        let result = pollster::block_on(orchestrate_upsert(&ports, upsert_request(vec![candidate])));
        assert!(result.is_ok());
    }

    #[test]
    fn orchestrate_upsert_rejects_concurrent_update_when_expected_base_is_stale() {
        let stored = ke(json!({
            "schema_version": 1,
            "entry_id": "kb_upsert_occ",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "provisional",
            "revision": 1,
            "body": { "summary": "Protagonist" },
            "extensions": {}
        }));
        let baseline = MemoryBaselinePorts::with_entry(stored);
        let store_revision = Mutex::new(1_u64);
        let advance_on_get = Mutex::new(true);

        struct UpsertOccPorts {
            baseline: MemoryBaselinePorts,
            store_revision: Mutex<u64>,
            advance_on_get: Mutex<bool>,
        }

        #[async_trait]
        impl KnowledgeEntryPort for UpsertOccPorts {
            async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
                let result = self.baseline.get_knowledge_entry(entry_id).await;
                if result.is_ok() {
                    let mut advance = self.advance_on_get.lock().expect("advance lock");
                    if *advance {
                        *self.store_revision.lock().expect("rev lock") = 2;
                        *advance = false;
                    }
                }
                result
            }

            async fn put_knowledge_entry(
                &self,
                entry: KnowledgeEntry,
                expected_base_revision: Option<u64>,
            ) -> SpokeResult<KnowledgeEntry> {
                let store_revision = *self.store_revision.lock().expect("rev lock");
                if let Some(expected) = expected_base_revision {
                    if store_revision != expected {
                        return spoke_reject(
                            SpokeRejectCode::StoredRevisionStale,
                            format!(
                                "Store revision {store_revision} does not match expected base {expected}"
                            ),
                            None,
                        );
                    }
                }
                self.baseline
                    .put_knowledge_entry(entry, expected_base_revision).await
            }
        }

        #[async_trait]
        impl RelationPort for UpsertOccPorts {
            async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
                self.baseline.get_relation(relation_id).await
            }
            async fn put_relation(
                &self,
                relation: Relation,
                expected_base_revision: Option<u64>,
            ) -> SpokeResult<Relation> {
                self.baseline.put_relation(relation, expected_base_revision).await
            }
        }
        #[async_trait]
        impl ScopeQueryPort for UpsertOccPorts {
            async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
                self.baseline.list_knowledge_entries(scope).await
            }
            async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
                self.baseline.list_timeline_events(scope).await
            }
        }
        #[async_trait]
        impl FindingPort for UpsertOccPorts {
            async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
                self.baseline.put_findings(findings).await
            }
        }
        #[async_trait]
        impl RuleQueryPort for UpsertOccPorts {
            async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
                self.baseline.list_rules(rule_refs).await
            }
        }
        #[async_trait]
        impl HostManifestPort for UpsertOccPorts {
            async fn get_host_capability_manifest(
                &self,
            ) -> SpokeResult<spoke_schemas::HostCapabilityManifest> {
                self.baseline.get_host_capability_manifest().await
            }

            async fn list_peer_host_capability_manifests(
                &self,
            ) -> SpokeResult<Vec<spoke_schemas::HostCapabilityManifest>> {
                self.baseline.list_peer_host_capability_manifests().await
            }
        }

        let ports = UpsertOccPorts {
            baseline,
            store_revision,
            advance_on_get,
        };

        let candidate = json!({
            "schema_version": 1,
            "entry_id": "kb_upsert_occ",
            "entry_type": "character",
            "canonical_name": "Raced",
            "status": "provisional",
            "revision": 1,
            "body": { "summary": "Protagonist" },
            "extensions": {}
        });

        let result = pollster::block_on(orchestrate_upsert(&ports, upsert_request(vec![candidate])));
        assert!(!result.is_ok());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::StoredRevisionStale);
        } else {
            panic!("expected OCC reject");
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

        let result = pollster::block_on(orchestrate_relate(&ports, relate_request(rel.clone())));
        assert!(result.is_ok());
        if let SpokeResult::Ok(RelateResponse::Variant0 { relation: got, .. }) = result {
            assert_eq!(got.relation_id, "rel_1");
            // Create path seeds revision 1 (adapter owns revision assignment).
            assert_eq!(got.revision, Some(1));
        } else {
            panic!("expected relate success");
        }
        let _ = relation(rel);
    }

    #[test]
    fn orchestrate_relate_passes_none_expected_base_revision_on_create() {
        let ports = MemoryBaselinePorts::new(MemoryStore::default());
        let rel = json!({
            "schema_version": 1,
            "relation_id": "rel_create_occ",
            "relation_type": "related_to",
            "from_id": "kb_a",
            "to_id": "kb_b",
            "extensions": {}
        });

        let result = pollster::block_on(orchestrate_relate(&ports, relate_request(rel)));
        assert!(result.is_ok());
    }

    #[test]
    fn orchestrate_relate_passes_stored_revision_as_expected_base_on_update() {
        let stored = relation(json!({
            "schema_version": 1,
            "relation_id": "rel_update_occ",
            "relation_type": "related_to",
            "from_id": "kb_a",
            "to_id": "kb_b",
            "revision": 4,
            "extensions": {}
        }));
        let mut store = MemoryStore::default();
        store.relations.insert(stored.relation_id.clone(), stored);
        let ports = MemoryBaselinePorts::new(store);

        let candidate = json!({
            "schema_version": 1,
            "relation_id": "rel_update_occ",
            "relation_type": "related_to",
            "from_id": "kb_a",
            "to_id": "kb_b",
            "revision": 4,
            "extensions": {}
        });

        let result = pollster::block_on(orchestrate_relate(&ports, relate_request(candidate)));
        assert!(result.is_ok());
        if let SpokeResult::Ok(RelateResponse::Variant0 { relation: got, .. }) = result {
            // Update bumps stored revision 4 -> 5.
            assert_eq!(got.revision, Some(5));
        } else {
            panic!("expected relate success");
        }
    }

    #[test]
    fn orchestrate_relate_rejects_update_when_candidate_revision_conflicts() {
        let stored = relation(json!({
            "schema_version": 1,
            "relation_id": "rel_conflict",
            "relation_type": "related_to",
            "from_id": "kb_a",
            "to_id": "kb_b",
            "revision": 2,
            "extensions": {}
        }));
        let mut store = MemoryStore::default();
        store.relations.insert(stored.relation_id.clone(), stored);
        let ports = MemoryBaselinePorts::new(store);

        // Candidate claims revision 7 while store holds 2 -> validator returns
        // REVISION_CONFLICT (candidate ahead of stored) before the put.
        let candidate = json!({
            "schema_version": 1,
            "relation_id": "rel_conflict",
            "relation_type": "related_to",
            "from_id": "kb_a",
            "to_id": "kb_b",
            "revision": 7,
            "extensions": {}
        });

        let result = pollster::block_on(orchestrate_relate(&ports, relate_request(candidate)));
        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::RevisionConflict);
        }
    }

    #[test]
    fn orchestrate_relate_rejects_concurrent_update_when_store_revision_advances() {
        let stored = relation(json!({
            "schema_version": 1,
            "relation_id": "rel_stale",
            "relation_type": "related_to",
            "from_id": "kb_a",
            "to_id": "kb_b",
            "revision": 5,
            "extensions": {}
        }));
        let baseline = MemoryBaselinePorts::with_entry(serde_json::from_value(json!({
            "schema_version": 1,
            "entry_id": "kb_placeholder",
            "entry_type": "character",
            "canonical_name": "Placeholder",
            "status": "provisional",
            "body": { "summary": "placeholder" },
            "extensions": {}
        })).expect("placeholder KE"));
        // Seed the relation into the baseline store directly.
        {
            let mut store = baseline.store.lock().expect("store lock");
            store.relations.insert(stored.relation_id.clone(), stored);
        }
        let store_revision = Mutex::new(5_u64);
        let advance_on_get = Mutex::new(true);

        struct RelateOccPorts {
            baseline: MemoryBaselinePorts,
            store_revision: Mutex<u64>,
            advance_on_get: Mutex<bool>,
        }

        #[async_trait]
        impl KnowledgeEntryPort for RelateOccPorts {
            async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
                self.baseline.get_knowledge_entry(entry_id).await
            }
            async fn put_knowledge_entry(
                &self,
                entry: KnowledgeEntry,
                expected_base_revision: Option<u64>,
            ) -> SpokeResult<KnowledgeEntry> {
                self.baseline
                    .put_knowledge_entry(entry, expected_base_revision).await
            }
        }
        #[async_trait]
        impl RelationPort for RelateOccPorts {
            async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
                let result = self.baseline.get_relation(relation_id).await;
                if result.is_ok() {
                    let mut advance = self.advance_on_get.lock().expect("advance lock");
                    if *advance {
                        *self.store_revision.lock().expect("rev lock") = 6;
                        *advance = false;
                    }
                }
                result
            }
            async fn put_relation(
                &self,
                relation: Relation,
                expected_base_revision: Option<u64>,
            ) -> SpokeResult<Relation> {
                let store_revision = *self.store_revision.lock().expect("rev lock");
                if let Some(expected) = expected_base_revision {
                    if store_revision != expected {
                        return spoke_reject(
                            SpokeRejectCode::StoredRevisionStale,
                            format!(
                                "Store revision {store_revision} does not match expected base {expected}"
                            ),
                            None,
                        );
                    }
                }
                self.baseline.put_relation(relation, expected_base_revision).await
            }
        }
        #[async_trait]
        impl ScopeQueryPort for RelateOccPorts {
            async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
                self.baseline.list_knowledge_entries(scope).await
            }
            async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
                self.baseline.list_timeline_events(scope).await
            }
        }
        #[async_trait]
        impl FindingPort for RelateOccPorts {
            async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
                self.baseline.put_findings(findings).await
            }
        }
        #[async_trait]
        impl RuleQueryPort for RelateOccPorts {
            async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
                self.baseline.list_rules(rule_refs).await
            }
        }
        #[async_trait]
        impl HostManifestPort for RelateOccPorts {
            async fn get_host_capability_manifest(
                &self,
            ) -> SpokeResult<spoke_schemas::HostCapabilityManifest> {
                self.baseline.get_host_capability_manifest().await
            }
            async fn list_peer_host_capability_manifests(
                &self,
            ) -> SpokeResult<Vec<spoke_schemas::HostCapabilityManifest>> {
                self.baseline.list_peer_host_capability_manifests().await
            }
        }

        let ports = RelateOccPorts {
            baseline,
            store_revision,
            advance_on_get,
        };

        let candidate = json!({
            "schema_version": 1,
            "relation_id": "rel_stale",
            "relation_type": "related_to",
            "from_id": "kb_a",
            "to_id": "kb_b",
            "revision": 5,
            "extensions": {}
        });

        let result = pollster::block_on(orchestrate_relate(&ports, relate_request(candidate)));
        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::StoredRevisionStale);
        }
    }

    #[test]
    fn orchestrate_relate_propagates_relation_already_exists_when_create_races_existing_id() {
        // The create-path RelationAlreadyExists can only surface through a
        // read-then-put CAS race: the orchestrator's get_relation snapshot
        // missed a concurrently-inserted row, so validate routes to create and
        // the adapter rejects put_relation(existing, None). The orchestrator
        // must propagate it.
        let stored = relation(json!({
            "schema_version": 1,
            "relation_id": "rel_race",
            "relation_type": "related_to",
            "from_id": "kb_a",
            "to_id": "kb_b",
            "revision": 1,
            "extensions": {}
        }));
        let baseline = MemoryBaselinePorts::with_entry(serde_json::from_value(json!({
            "schema_version": 1,
            "entry_id": "kb_placeholder",
            "entry_type": "character",
            "canonical_name": "Placeholder",
            "status": "provisional",
            "body": { "summary": "placeholder" },
            "extensions": {}
        })).expect("placeholder KE"));
        {
            let mut store = baseline.store.lock().expect("store lock");
            store.relations.insert(stored.relation_id.clone(), stored);
        }

        struct RelateCreateRacePorts {
            baseline: MemoryBaselinePorts,
        }
        #[async_trait]
        impl KnowledgeEntryPort for RelateCreateRacePorts {
            async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry> {
                self.baseline.get_knowledge_entry(entry_id).await
            }
            async fn put_knowledge_entry(
                &self,
                entry: KnowledgeEntry,
                expected_base_revision: Option<u64>,
            ) -> SpokeResult<KnowledgeEntry> {
                self.baseline
                    .put_knowledge_entry(entry, expected_base_revision).await
            }
        }
        #[async_trait]
        impl RelationPort for RelateCreateRacePorts {
            async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation> {
                // Stale snapshot: pretend the row is absent so validate routes to
                // create and the orchestrator passes expected_base_revision None.
                let mut details = Map::new();
                details.insert("relation_id".into(), json!(relation_id));
                spoke_reject(
                    SpokeRejectCode::RelationNotFound,
                    format!("Stale snapshot missed: {relation_id}"),
                    Some(details),
                )
            }
            async fn put_relation(
                &self,
                relation: Relation,
                expected_base_revision: Option<u64>,
            ) -> SpokeResult<Relation> {
                // Delegate to the real baseline OCC store, which still holds the
                // seeded relation and rejects create-when-exists.
                self.baseline.put_relation(relation, expected_base_revision).await
            }
        }
        #[async_trait]
        impl ScopeQueryPort for RelateCreateRacePorts {
            async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>> {
                self.baseline.list_knowledge_entries(scope).await
            }
            async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>> {
                self.baseline.list_timeline_events(scope).await
            }
        }
        #[async_trait]
        impl FindingPort for RelateCreateRacePorts {
            async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>> {
                self.baseline.put_findings(findings).await
            }
        }
        #[async_trait]
        impl RuleQueryPort for RelateCreateRacePorts {
            async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>> {
                self.baseline.list_rules(rule_refs).await
            }
        }
        #[async_trait]
        impl HostManifestPort for RelateCreateRacePorts {
            async fn get_host_capability_manifest(
                &self,
            ) -> SpokeResult<spoke_schemas::HostCapabilityManifest> {
                self.baseline.get_host_capability_manifest().await
            }
            async fn list_peer_host_capability_manifests(
                &self,
            ) -> SpokeResult<Vec<spoke_schemas::HostCapabilityManifest>> {
                self.baseline.list_peer_host_capability_manifests().await
            }
        }

        let ports = RelateCreateRacePorts { baseline };

        // Create candidate carries no revision so validate chooses create.
        let candidate = json!({
            "schema_version": 1,
            "relation_id": "rel_race",
            "relation_type": "related_to",
            "from_id": "kb_a",
            "to_id": "kb_b",
            "extensions": {}
        });

        let result = pollster::block_on(orchestrate_relate(&ports, relate_request(candidate)));
        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::RelationAlreadyExists);
        }
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

        let result = pollster::block_on(orchestrate_check(&ports, request.clone(), |input| {
            assert_eq!(input.entries.len(), 1);
            assert_eq!(input.entries[0].entry_id, "kb_check");
            assert_eq!(input.events.len(), 1);
            assert_eq!(input.rules.len(), 1);
            assert_eq!(input.rules[0].rule_id, "rule_1");
            spoke_ok(vec![finding.clone()])
        }));

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
    fn orchestrate_check_lets_embedded_rules_win_by_rule_id_over_refs() {
        let entry = ke(json!({
            "schema_version": 1,
            "entry_id": "kb_check_merge",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "provisional",
            "body": { "summary": "Protagonist" },
            "extensions": {}
        }));
        let stored_rule = rule(json!({
            "schema_version": 1,
            "rule_id": "rule_shared",
            "canonical_name": "Stored rule",
            "kind": "rule",
            "statement": "from-store",
            "extensions": {}
        }));
        let other_stored = rule(json!({
            "schema_version": 1,
            "rule_id": "rule_other",
            "canonical_name": "Other stored",
            "kind": "rule",
            "statement": "keep-me",
            "extensions": {}
        }));
        let mut store = MemoryStore::default();
        store.entries.insert(entry.entry_id.clone(), entry);
        store.rules.insert(stored_rule.rule_id.clone(), stored_rule);
        store
            .rules
            .insert(other_stored.rule_id.clone(), other_stored.clone());
        let ports = MemoryBaselinePorts::new(store);

        let embedded_override = json!({
            "schema_version": 1,
            "rule_id": "rule_shared",
            "canonical_name": "Embedded wins",
            "kind": "rule",
            "statement": "from-embed",
            "extensions": {}
        });
        let embedded_new = json!({
            "schema_version": 1,
            "rule_id": "rule_new",
            "canonical_name": "New embed",
            "kind": "rule",
            "statement": "append-me",
            "extensions": {}
        });

        let request = check_request(json!({
            "scope": { "scope_id": "world_1", "entry_ids": ["kb_check_merge"] },
            "rule_refs": ["rule_shared", "rule_other"],
            "rules": [embedded_override, embedded_new]
        }));

        let result = pollster::block_on(orchestrate_check(&ports, request, |input| {
            assert_eq!(input.rules.len(), 3);
            assert_eq!(input.rules[0].rule_id, "rule_shared");
            assert_eq!(input.rules[0].canonical_name.as_str(), "Embedded wins");
            assert_eq!(
                input.rules[0].statement.as_deref(),
                Some("from-embed")
            );
            assert_eq!(input.rules[1].rule_id, "rule_other");
            assert_eq!(input.rules[1].statement.as_deref(), Some("keep-me"));
            assert_eq!(input.rules[2].rule_id, "rule_new");
            assert_eq!(input.rules[2].statement.as_deref(), Some("append-me"));
            spoke_ok(vec![])
        }));

        assert!(result.is_ok());
        let _ = other_stored;
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

        let result = pollster::block_on(orchestrate_assemble(&ports, request));
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

        let result = pollster::block_on(orchestrate_project(&ports, request));
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
        assert_eq!(ports.projected.lock().expect("projected lock").len(), 1);
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

        let result = pollster::block_on(orchestrate_compute(&ports, request));
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
        assert_eq!(ports.computed.lock().expect("computed lock").len(), 1);
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

        let result = pollster::block_on(orchestrate_fork_check(&ports, request, |input| {
            assert_eq!(input.entries.len(), 1);
            assert_eq!(input.events.len(), 1);
            assert_eq!(input.events[0].timeline_event_id, "te_fork");
            spoke_ok(vec![finding.clone()])
        }));

        assert!(result.is_ok());
        assert_eq!(ports.fork_list_calls.lock().expect("fork list lock").len(), 1);
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

        let result = pollster::block_on(orchestrate_project(&ports, request));
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

        let result = pollster::block_on(orchestrate_fork_check(&ports, request, |_| spoke_ok(Vec::new())));
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
