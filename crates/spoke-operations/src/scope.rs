//! Scope matching and filtering helpers.

use crate::cognition::{
    cognition_record_matches_filter, validate_cognition_module, validate_cognition_selection,
};
use crate::knowledge_entry::knowledge_entry_visible_to_viewpoint;
use crate::result::{spoke_ok, spoke_ok_unit, SpokeReject, SpokeRejectCode, SpokeResult};
use serde_json::{Map, Value};
use spoke_schemas::data::cognition::{
    CognitionFilter as CognitionFilterWire, CognitionModule, CognitionModuleRecordsItem,
    CognitionRecord, CognitiveView,
};
use spoke_schemas::knowledge_entry::{KnowledgeEntry, KnowledgeEntryModulesValue};
use spoke_schemas::{Scope, TimelineEvent};

/// Scope refinements match by presence: `None` is absent (wildcard) while a
/// present array — including an explicit `[]` — is an AND refinement that
/// matches nothing when empty. The generated `Scope` arrays keep that
/// distinction through typed JSON decode (`deserialize_presence_sensitive_array`),
/// so the served path cannot widen a declared `[]` to absent.
///
/// Internal authorization stage: KnowledgeEntry passes optional Scope refinements
/// (AND when present), including the core disclosure predicate applied to
/// `scope.viewpoint`.
///
/// Cognition is a derivative selection: this boolean predicate deliberately does
/// not evaluate `scope.cognition` and is therefore crate-internal. Served Scope
/// list consumers use [`filter_knowledge_entries_by_scope`], which applies
/// cognition after this stage so an unsupported requirement rejects instead of
/// silently matching every entry. Assembly builders that need the pre-cognition
/// authorized source set use [`authorize_knowledge_entries_by_scope`].
#[must_use]
pub(crate) fn knowledge_entry_passes_scope_authorization(
    knowledge_entry: &KnowledgeEntry,
    scope: &Scope,
) -> bool {
    if scope
        .entry_ids
        .as_ref()
        .is_some_and(|entry_ids| !entry_ids.contains(&knowledge_entry.entry_id))
    {
        return false;
    }

    if scope
        .entry_types
        .as_ref()
        .is_some_and(|entry_types| !entry_types.contains(&knowledge_entry.entry_type))
    {
        return false;
    }

    if let Some(source_id) = &scope.source_id {
        if knowledge_entry
            .source_anchor
            .as_ref()
            .is_none_or(|anchor| anchor.source_id.as_str() != source_id)
        {
            return false;
        }
    }

    if !knowledge_entry_visible_to_viewpoint(
        knowledge_entry,
        scope
            .viewpoint
            .as_ref()
            .map(|viewpoint| viewpoint.as_str()),
    ) {
        return false;
    }

    true
}

/// Explicit authorization stage: KnowledgeEntries passing the optional Scope
/// refinements and the disclosure predicate, before any cognition selection.
pub(crate) fn authorize_knowledge_entries_by_scope<'a>(
    knowledge_entries: &'a [KnowledgeEntry],
    scope: &Scope,
) -> Vec<&'a KnowledgeEntry> {
    knowledge_entries
        .iter()
        .filter(|knowledge_entry| knowledge_entry_passes_scope_authorization(knowledge_entry, scope))
        .collect()
}

/// Served Scope KnowledgeEntry list filter.
///
/// Authorization/disclosure runs first, then the validated cognition selection:
/// an unsupported or unavailable requirement rejects the whole call instead of
/// silently ignoring it or matching nothing, and the absence of
/// `scope.cognition` returns the baseline authorized set unchanged.
pub fn filter_knowledge_entries_by_scope<'a>(
    knowledge_entries: &'a [KnowledgeEntry],
    scope: &Scope,
) -> SpokeResult<Vec<&'a KnowledgeEntry>> {
    let authorized_entries = authorize_knowledge_entries_by_scope(knowledge_entries, scope);
    match scope.cognition.as_ref() {
        Some(selection) => filter_knowledge_entries_by_scope_cognition(
            &authorized_entries,
            selection,
            "scope.cognition",
        ),
        None => spoke_ok(authorized_entries),
    }
}

fn invalid_cognition_module(entry_id: &str) -> SpokeReject {
    let mut details = Map::new();
    details.insert(
        "reason".into(),
        Value::String("invalid_cognition_record".into()),
    );
    details.insert("entry_id".into(), Value::String(entry_id.to_owned()));
    details.insert("field".into(), Value::String("modules.cognition".into()));
    SpokeReject {
        code: SpokeRejectCode::InvalidInput,
        message: format!("Invalid cognition module on authorized entry: {entry_id}"),
        details: Some(details),
    }
}

fn cognition_module_of_entry(entry: &KnowledgeEntry) -> SpokeResult<Option<CognitionModule>> {
    let Some(module_value) = entry
        .modules
        .iter()
        .find(|(key, _)| key.as_str() == "cognition")
        .map(|(_, value)| value)
    else {
        return spoke_ok(None);
    };

    let wire = match module_value {
        KnowledgeEntryModulesValue::Object(map) => Value::Object(map.clone()),
        KnowledgeEntryModulesValue::Array(_) => {
            return SpokeResult::Reject(invalid_cognition_module(&entry.entry_id));
        }
    };

    if let SpokeResult::Reject(_) = validate_cognition_module(&wire) {
        return SpokeResult::Reject(invalid_cognition_module(&entry.entry_id));
    }

    match serde_json::from_value::<CognitionModule>(wire) {
        Ok(module) => spoke_ok(Some(module)),
        Err(_) => SpokeResult::Reject(invalid_cognition_module(&entry.entry_id)),
    }
}

fn cognition_record_from_module_item(item: CognitionModuleRecordsItem) -> CognitionRecord {
    match item {
        CognitionModuleRecordsItem::BeliefRecord(record) => CognitionRecord::BeliefRecord(record),
        CognitionModuleRecordsItem::EpistemicAssessment(record) => {
            CognitionRecord::EpistemicAssessment(record)
        }
        CognitionModuleRecordsItem::PerceptionRecord(record) => {
            CognitionRecord::PerceptionRecord(record)
        }
    }
}

/// Cognition records carried by one already-authorized entry.
///
/// Authorization/disclosure runs first: callers pass only entries that already
/// passed [`knowledge_entry_passes_scope_authorization`] (including the viewpoint disclosure
/// predicate), so an unauthorized holder's cognition never reaches filtering
/// or projection. Records come only from the closed `modules.cognition`
/// carrier; unknown module namespaces stay opaque. A malformed carrier rejects
/// the whole request as `invalid_cognition_record` instead of silently
/// dropping records and claiming no cognition was recorded.
pub fn cognition_records_of_authorized_entry(
    entry: &KnowledgeEntry,
) -> SpokeResult<Vec<CognitionRecord>> {
    let module = match cognition_module_of_entry(entry) {
        SpokeResult::Ok(module) => module,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };
    let Some(module) = module else {
        return spoke_ok(Vec::new());
    };

    spoke_ok(
        module
            .records
            .into_iter()
            .map(cognition_record_from_module_item)
            .collect(),
    )
}

/// Cognition records carried by already-authorized entries.
pub fn cognition_records_of_authorized_entries(
    authorized_entries: &[&KnowledgeEntry],
) -> SpokeResult<Vec<CognitionRecord>> {
    let mut records = Vec::new();

    for entry in authorized_entries {
        let entry_records = match cognition_records_of_authorized_entry(entry) {
            SpokeResult::Ok(records) => records,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        records.extend(entry_records);
    }

    spoke_ok(records)
}

/// Reject value for a request that cannot serve a cognition selection.
///
/// Intentionally distinct from `cognition::unsupported_requirement`, and not a
/// consolidation candidate: this helper returns the `SpokeReject` value itself
/// because the scope call sites wrap it in `SpokeResult::Reject(..)`, keeps the
/// message fixed, and always carries the request JSON path as `details.field`
/// so callers can point at the offending selection.
///
/// Both helpers are load-bearing for the same wire contract: the identical
/// `reason: "unsupported_cognition_requirement"` string, asserted at
/// `adapter/orchestrate.rs:3452`, `assemble.rs:688` and `scope.rs:1191`.
fn unsupported_cognition_requirement(field: &str) -> SpokeReject {
    let mut details = Map::new();
    details.insert(
        "reason".into(),
        Value::String("unsupported_cognition_requirement".into()),
    );
    details.insert("field".into(), Value::String(field.to_owned()));
    SpokeReject {
        code: SpokeRejectCode::InvalidInput,
        message: "Unsupported cognition requirement".into(),
        details: Some(details),
    }
}

fn cognition_record_view(record: &CognitionRecord) -> &CognitiveView {
    match record {
        CognitionRecord::BeliefRecord(record) => &record.view,
        CognitionRecord::EpistemicAssessment(record) => &record.view,
        CognitionRecord::PerceptionRecord(record) => &record.view,
    }
}

/// Reject a cognition selection this KnowledgeEntry carrier cannot serve.
///
/// The carrier holds current cognition only. `recorded_at` is a separate Memory
/// source view, so it is unavailable here rather than an ordinary empty
/// result: the request rejects before any source output or checker/finding
/// side effect. `field_path` is the request's JSON path to the selection.
///
/// The existing selection grammar gate runs here too: generated Rust keeps
/// the selection typed, so a decodable but closed-grammar-invalid predicate
/// (`{absent: false}`, a `q` range with no bound) would otherwise match
/// nothing and silently succeed.
pub fn assert_supported_cognition_selection(
    selection: Option<&spoke_schemas::CognitionSelection>,
    field_path: &str,
) -> SpokeResult<()> {
    let Some(selection) = selection else {
        return spoke_ok_unit();
    };

    // Generated Rust re-declares the cognition family per wire module; the
    // established canonical-import rule converts at this boundary.
    let wire = match serde_json::to_value(selection) {
        Ok(wire) => wire,
        Err(_) => return SpokeResult::Reject(unsupported_cognition_requirement(field_path)),
    };
    if let SpokeResult::Reject(reject) = validate_cognition_selection(&wire) {
        let relative = reject
            .details
            .as_ref()
            .and_then(|details| details.get("field"))
            .and_then(Value::as_str);
        let field = match relative {
            // A rendered non-identifier member already carries its brackets.
            Some(relative) if relative.starts_with('[') => format!("{field_path}{relative}"),
            Some(relative) => format!("{field_path}.{relative}"),
            None => field_path.to_owned(),
        };
        return SpokeResult::Reject(unsupported_cognition_requirement(&field));
    }

    match selection {
        spoke_schemas::CognitionSelection::Current { .. } => spoke_ok_unit(),
        spoke_schemas::CognitionSelection::RecordedAt { .. } => SpokeResult::Reject(
            unsupported_cognition_requirement(&format!("{field_path}.view")),
        ),
    }
}

/// Applies a supported `Scope.cognition` selection to already-authorized entries.
///
/// Order is disclosure first, cognition second: callers pass only entries that
/// already passed [`knowledge_entry_passes_scope_authorization`]. An entry is served
/// when at least one of its carried records satisfies the one-record
/// conjunction; a selection this carrier cannot serve rejects the request
/// instead of matching nothing. Entries without carried cognition are excluded
/// by a cognition selection.
pub(crate) fn filter_knowledge_entries_by_scope_cognition<'a>(
    authorized_entries: &[&'a KnowledgeEntry],
    selection: &spoke_schemas::CognitionSelection,
    field_path: &str,
) -> SpokeResult<Vec<&'a KnowledgeEntry>> {
    if let SpokeResult::Reject(reject) = assert_supported_cognition_selection(Some(selection), field_path)
    {
        return SpokeResult::Reject(reject);
    }

    let spoke_schemas::CognitionSelection::Current { filter, .. } = selection else {
        return SpokeResult::Reject(unsupported_cognition_requirement(&format!(
            "{field_path}.view"
        )));
    };

    // Generated Rust re-declares the cognition family per wire module; the
    // established canonical-import rule converts at this boundary.
    let filter: CognitionFilterWire =
        match serde_json::to_value(filter).ok().and_then(|wire| serde_json::from_value(wire).ok()) {
            Some(filter) => filter,
            None => return SpokeResult::Reject(unsupported_cognition_requirement(field_path)),
        };

    let mut kept = Vec::new();
    for entry in authorized_entries {
        let records = match cognition_records_of_authorized_entry(entry) {
            SpokeResult::Ok(records) => records,
            SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
        };
        if records.iter().any(|record| {
            cognition_record_view(record) == &CognitiveView::Current
                && cognition_record_matches_filter(record, &filter)
        }) {
            kept.push(*entry);
        }
    }

    spoke_ok(kept)
}

/// TimelineEvent passes optional Scope refinements (AND when present).
#[must_use]
pub fn timeline_event_matches_scope(timeline_event: &TimelineEvent, scope: &Scope) -> bool {
    if scope
        .timeline_event_ids
        .as_ref()
        .is_some_and(|ids| !ids.contains(&timeline_event.timeline_event_id))
    {
        return false;
    }

    if let Some(timeline_scale) = &scope.timeline_scale {
        if timeline_event.timeline_scale.as_deref() != Some(timeline_scale.as_str()) {
            return false;
        }
    }

    if let Some(fork_id) = &scope.fork_id {
        if timeline_event.fork_id.as_ref().map(|id| id.as_str()) != Some(fork_id.as_str()) {
            return false;
        }
    }

    true
}

/// Filter TimelineEvents by optional Scope refinements.
#[must_use]
pub fn filter_timeline_events_by_scope<'a>(
    timeline_events: &'a [TimelineEvent],
    scope: &Scope,
) -> Vec<&'a TimelineEvent> {
    timeline_events
        .iter()
        .filter(|timeline_event| timeline_event_matches_scope(timeline_event, scope))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use spoke_schemas::knowledge_entry::{
        KnowledgeEntryBody, KnowledgeEntryCanonicalName, KnowledgeEntryDisclosure,
        KnowledgeEntryOwner, SourceAnchor,
    };
    use spoke_schemas::timeline_event::TimelineEventCanonicalName;
    use spoke_schemas::ScopeViewpoint;
    use std::collections::HashMap;
    use std::num::NonZeroU64;

    fn base_scope() -> Scope {
        Scope {
            cognition: None,
            entry_ids: None,
            entry_types: None,
            extensions: None,
            fork_id: None,
            scope_id: "world_1".into(),
            source_id: None,
            timeline_event_ids: None,
            timeline_scale: None,
            viewpoint: None,
        }
    }

    /// Run the public served-list filter and unwrap; an unexpected rejection fails the test.
    fn authorized_scope_entries<'a>(
        knowledge_entries: &'a [KnowledgeEntry],
        scope: &Scope,
    ) -> Vec<&'a KnowledgeEntry> {
        match filter_knowledge_entries_by_scope(knowledge_entries, scope) {
            SpokeResult::Ok(entries) => entries,
            SpokeResult::Reject(reject) => panic!("scope filter rejected: {reject:?}"),
        }
    }

    fn make_knowledge_entry(overrides: impl FnOnce(&mut KnowledgeEntry)) -> KnowledgeEntry {
        let mut entry = KnowledgeEntry {
            body: KnowledgeEntryBody::default(),
            canonical_name: KnowledgeEntryCanonicalName::try_from("Mira Vale".to_owned()).unwrap(),
            created_at: None,
            disclosure: None,
            entry_id: "kb_1".into(),
            entry_type: "character".into(),
            extensions: HashMap::new(),
            modules: HashMap::new(),
            owner: None,
            revision: None,
            schema_version: NonZeroU64::new(1).unwrap(),
            source_anchor: None,
            status: "confirmed".into(),
            updated_at: None,
        };
        overrides(&mut entry);
        entry
    }

    fn make_timeline_event(overrides: impl FnOnce(&mut TimelineEvent)) -> TimelineEvent {
        let mut event = TimelineEvent {
            canonical_name: TimelineEventCanonicalName::try_from("Battle of Harbor".to_owned())
                .unwrap(),
            computable_logs: Vec::new(),
            created_at: None,
            description: None,
            extensions: HashMap::new(),
            fork_id: None,
            modules: HashMap::new(),
            occurred_at: None,
            parent_fork_id: None,
            participant_entry_ids: Vec::new(),
            schema_version: NonZeroU64::new(1).unwrap(),
            sort_key: None,
            source_anchor: None,
            timeline_event_id: "evt_1".into(),
            timeline_scale: None,
            updated_at: None,
        };
        overrides(&mut event);
        event
    }

    /// A typed JSON decode keeps an explicitly present `[]` distinct from an
    /// omitted list, and re-serializing keeps `[]` on the wire. Omitted is a
    /// wildcard; present-empty matches nothing; present-nonempty is the AND
    /// refinement. One round-trip covers all three Scope lists.
    #[test]
    fn typed_scope_decode_keeps_present_empty_refinements() {
        let knowledge_entry = make_knowledge_entry(|_| {});
        let timeline_event = make_timeline_event(|event| {
            event.timeline_scale = Some("narrative".into());
        });

        let entry_ids: Scope = serde_json::from_value(json!({
            "scope_id": "world_1",
            "entry_ids": [],
        }))
        .expect("valid scope");
        assert_eq!(entry_ids.entry_ids, Some(Vec::new()));
        assert_eq!(
            serde_json::to_value(&entry_ids).expect("serialize")["entry_ids"],
            json!([])
        );
        assert!(!knowledge_entry_passes_scope_authorization(&knowledge_entry, &entry_ids));
        assert!(knowledge_entry_passes_scope_authorization(&knowledge_entry, &base_scope()));
        let matching_entry_ids: Scope = serde_json::from_value(json!({
            "scope_id": "world_1",
            "entry_ids": ["kb_1"],
        }))
        .expect("valid scope");
        assert!(knowledge_entry_passes_scope_authorization(
            &knowledge_entry,
            &matching_entry_ids
        ));

        let entry_types: Scope = serde_json::from_value(json!({
            "scope_id": "world_1",
            "entry_types": [],
        }))
        .expect("valid scope");
        assert_eq!(entry_types.entry_types, Some(Vec::new()));
        assert_eq!(
            serde_json::to_value(&entry_types).expect("serialize")["entry_types"],
            json!([])
        );
        assert!(!knowledge_entry_passes_scope_authorization(&knowledge_entry, &entry_types));
        let matching_entry_types: Scope = serde_json::from_value(json!({
            "scope_id": "world_1",
            "entry_types": ["character"],
        }))
        .expect("valid scope");
        assert!(knowledge_entry_passes_scope_authorization(
            &knowledge_entry,
            &matching_entry_types
        ));

        let timeline_event_ids: Scope = serde_json::from_value(json!({
            "scope_id": "world_1",
            "timeline_event_ids": [],
        }))
        .expect("valid scope");
        assert_eq!(timeline_event_ids.timeline_event_ids, Some(Vec::new()));
        assert_eq!(
            serde_json::to_value(&timeline_event_ids).expect("serialize")["timeline_event_ids"],
            json!([])
        );
        assert!(!timeline_event_matches_scope(&timeline_event, &timeline_event_ids));
        assert!(timeline_event_matches_scope(&timeline_event, &base_scope()));
        let matching_timeline_event_ids: Scope = serde_json::from_value(json!({
            "scope_id": "world_1",
            "timeline_event_ids": ["evt_1"],
        }))
        .expect("valid scope");
        assert!(timeline_event_matches_scope(
            &timeline_event,
            &matching_timeline_event_ids
        ));
    }

    #[test]
    fn knowledge_entry_passes_when_only_scope_id_is_set() {
        let knowledge_entry = make_knowledge_entry(|entry| {
            entry.source_anchor = Some(SourceAnchor {
                extensions: spoke_schemas::knowledge_entry::Shared2(HashMap::new()),
                label: None,
                mime_type: None,
                schema_version: spoke_schemas::knowledge_entry::Shared1(NonZeroU64::new(1).unwrap()),
                source_id: "manuscript_1".into(),
                span: None,
            });
        });

        assert!(knowledge_entry_passes_scope_authorization(
            &knowledge_entry,
            &base_scope()
        ));
    }

    #[test]
    fn knowledge_entry_matches_entry_ids_refinement() {
        let knowledge_entry = make_knowledge_entry(|entry| {
            entry.source_anchor = Some(SourceAnchor {
                extensions: spoke_schemas::knowledge_entry::Shared2(HashMap::new()),
                label: None,
                mime_type: None,
                schema_version: spoke_schemas::knowledge_entry::Shared1(NonZeroU64::new(1).unwrap()),
                source_id: "manuscript_1".into(),
                span: None,
            });
        });

        let mut matching_scope = base_scope();
        matching_scope.entry_ids = Some(vec!["kb_1".into(), "kb_2".into()]);
        assert!(knowledge_entry_passes_scope_authorization(
            &knowledge_entry,
            &matching_scope
        ));

        matching_scope.entry_ids = Some(vec!["kb_2".into()]);
        assert!(!knowledge_entry_passes_scope_authorization(
            &knowledge_entry,
            &matching_scope
        ));
    }

    #[test]
    fn knowledge_entry_matches_entry_types_refinement() {
        let knowledge_entry = make_knowledge_entry(|entry| {
            entry.source_anchor = Some(SourceAnchor {
                extensions: spoke_schemas::knowledge_entry::Shared2(HashMap::new()),
                label: None,
                mime_type: None,
                schema_version: spoke_schemas::knowledge_entry::Shared1(NonZeroU64::new(1).unwrap()),
                source_id: "manuscript_1".into(),
                span: None,
            });
        });

        let mut matching_scope = base_scope();
        matching_scope.entry_types = Some(vec!["character".into()]);
        assert!(knowledge_entry_passes_scope_authorization(
            &knowledge_entry,
            &matching_scope
        ));

        matching_scope.entry_types = Some(vec!["location".into()]);
        assert!(!knowledge_entry_passes_scope_authorization(
            &knowledge_entry,
            &matching_scope
        ));
    }

    #[test]
    fn knowledge_entry_matches_source_id_refinement() {
        let knowledge_entry = make_knowledge_entry(|entry| {
            entry.source_anchor = Some(SourceAnchor {
                extensions: spoke_schemas::knowledge_entry::Shared2(HashMap::new()),
                label: None,
                mime_type: None,
                schema_version: spoke_schemas::knowledge_entry::Shared1(NonZeroU64::new(1).unwrap()),
                source_id: "manuscript_1".into(),
                span: None,
            });
        });

        let mut matching_scope = base_scope();
        matching_scope.source_id = Some("manuscript_1".into());
        assert!(knowledge_entry_passes_scope_authorization(
            &knowledge_entry,
            &matching_scope
        ));

        matching_scope.source_id = Some("other".into());
        assert!(!knowledge_entry_passes_scope_authorization(
            &knowledge_entry,
            &matching_scope
        ));
    }

    #[test]
    fn knowledge_entry_ignores_timeline_event_refinements() {
        let knowledge_entry = make_knowledge_entry(|entry| {
            entry.source_anchor = Some(SourceAnchor {
                extensions: spoke_schemas::knowledge_entry::Shared2(HashMap::new()),
                label: None,
                mime_type: None,
                schema_version: spoke_schemas::knowledge_entry::Shared1(NonZeroU64::new(1).unwrap()),
                source_id: "manuscript_1".into(),
                span: None,
            });
        });

        let mut scope = base_scope();
        scope.timeline_event_ids = Some(vec!["evt_missing".into()]);
        scope.timeline_scale = Some(spoke_schemas::TimelineScale("brief".to_owned()));

        assert!(knowledge_entry_passes_scope_authorization(
            &knowledge_entry,
            &scope
        ));
    }

    #[test]
    fn knowledge_entry_requires_all_present_refinements() {
        let knowledge_entry = make_knowledge_entry(|entry| {
            entry.source_anchor = Some(SourceAnchor {
                extensions: spoke_schemas::knowledge_entry::Shared2(HashMap::new()),
                label: None,
                mime_type: None,
                schema_version: spoke_schemas::knowledge_entry::Shared1(NonZeroU64::new(1).unwrap()),
                source_id: "manuscript_1".into(),
                span: None,
            });
        });

        let mut scope = base_scope();
        scope.entry_ids = Some(vec!["kb_1".into()]);
        scope.entry_types = Some(vec!["location".into()]);

        assert!(!knowledge_entry_passes_scope_authorization(
            &knowledge_entry,
            &scope
        ));
    }

    #[test]
    fn filter_knowledge_entries_by_combined_refinements() {
        let knowledge_entries = [
            make_knowledge_entry(|entry| {
                entry.entry_id = "kb_1".into();
                entry.entry_type = "character".into();
            }),
            make_knowledge_entry(|entry| {
                entry.entry_id = "kb_2".into();
                entry.entry_type = "location".into();
            }),
        ];

        let mut scope = base_scope();
        scope.entry_types = Some(vec!["character".into()]);

        let filtered = authorized_scope_entries(&knowledge_entries, &scope);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].entry_id, "kb_1");
    }

    #[test]
    fn timeline_event_passes_when_only_scope_id_is_set() {
        let timeline_event = make_timeline_event(|event| {
            event.timeline_scale = Some("narrative".into());
        });

        assert!(timeline_event_matches_scope(
            &timeline_event,
            &base_scope()
        ));
    }

    #[test]
    fn timeline_event_matches_timeline_event_ids_refinement() {
        let timeline_event = make_timeline_event(|event| {
            event.timeline_scale = Some("narrative".into());
        });

        let mut matching_scope = base_scope();
        matching_scope.timeline_event_ids = Some(vec!["evt_1".into()]);
        assert!(timeline_event_matches_scope(
            &timeline_event,
            &matching_scope
        ));

        matching_scope.timeline_event_ids = Some(vec!["evt_2".into()]);
        assert!(!timeline_event_matches_scope(
            &timeline_event,
            &matching_scope
        ));
    }

    #[test]
    fn timeline_event_matches_timeline_scale_refinement() {
        let timeline_event = make_timeline_event(|event| {
            event.timeline_scale = Some("narrative".into());
        });

        let mut matching_scope = base_scope();
        matching_scope.timeline_scale = Some(spoke_schemas::TimelineScale("narrative".to_owned()));
        assert!(timeline_event_matches_scope(
            &timeline_event,
            &matching_scope
        ));

        matching_scope.timeline_scale = Some(spoke_schemas::TimelineScale("brief".to_owned()));
        assert!(!timeline_event_matches_scope(
            &timeline_event,
            &matching_scope
        ));
    }

    #[test]
    fn timeline_event_ignores_knowledge_entry_refinements() {
        let timeline_event = make_timeline_event(|event| {
            event.timeline_scale = Some("narrative".into());
        });

        let mut scope = base_scope();
        scope.entry_ids = Some(vec!["kb_missing".into()]);
        scope.entry_types = Some(vec!["character".into()]);
        scope.source_id = Some("manuscript_1".into());

        assert!(timeline_event_matches_scope(&timeline_event, &scope));
    }

    #[test]
    fn timeline_event_matches_fork_id_refinement_when_equal() {
        let timeline_event = make_timeline_event(|event| {
            event.timeline_event_id = "evt_fork".into();
            event.fork_id = Some(
                spoke_schemas::timeline_event::TimelineEventForkId::try_from(
                    "fork_mainline_a".to_owned(),
                )
                .unwrap(),
            );
        });

        let mut scope = base_scope();
        scope.fork_id = Some(
            spoke_schemas::ForkId::try_from("fork_mainline_a".to_owned()).unwrap(),
        );

        assert!(timeline_event_matches_scope(&timeline_event, &scope));
    }

    #[test]
    fn timeline_event_misses_fork_id_refinement_when_unequal() {
        let timeline_event = make_timeline_event(|event| {
            event.timeline_event_id = "evt_fork".into();
            event.fork_id = Some(
                spoke_schemas::timeline_event::TimelineEventForkId::try_from(
                    "fork_mainline_a".to_owned(),
                )
                .unwrap(),
            );
        });

        let mut scope = base_scope();
        scope.fork_id = Some(
            spoke_schemas::ForkId::try_from("fork_what_if_b".to_owned()).unwrap(),
        );

        assert!(!timeline_event_matches_scope(&timeline_event, &scope));
    }

    #[test]
    fn timeline_event_misses_fork_id_refinement_when_event_lacks_fork_id() {
        let timeline_event = make_timeline_event(|event| {
            event.timeline_scale = Some("narrative".into());
        });

        let mut scope = base_scope();
        scope.fork_id = Some(
            spoke_schemas::ForkId::try_from("fork_mainline_a".to_owned()).unwrap(),
        );

        assert!(!timeline_event_matches_scope(&timeline_event, &scope));
    }

    #[test]
    fn timeline_event_does_not_filter_on_parent_fork_id() {
        let timeline_event = make_timeline_event(|event| {
            event.timeline_event_id = "evt_fork".into();
            event.parent_fork_id = Some(
                spoke_schemas::timeline_event::Shared2::try_from("fork_mainline_a".to_owned())
                    .expect("valid fork id"),
            );
        });

        let mut scope = base_scope();
        scope.fork_id = Some(
            spoke_schemas::ForkId::try_from("fork_mainline_a".to_owned()).unwrap(),
        );

        assert!(!timeline_event_matches_scope(&timeline_event, &scope));
    }

    #[test]
    fn timeline_event_passes_when_scope_omits_fork_id() {
        let timeline_event = make_timeline_event(|event| {
            event.timeline_event_id = "evt_fork".into();
            event.fork_id = Some(
                spoke_schemas::timeline_event::TimelineEventForkId::try_from(
                    "fork_mainline_a".to_owned(),
                )
                .unwrap(),
            );
        });

        assert!(timeline_event_matches_scope(
            &timeline_event,
            &base_scope()
        ));
    }

    #[test]
    fn filter_timeline_events_by_timeline_scale() {
        let timeline_events = [
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_1".into();
                event.timeline_scale = Some("brief".into());
            }),
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_2".into();
                event.timeline_scale = Some("narrative".into());
            }),
        ];

        let mut scope = base_scope();
        scope.timeline_scale = Some(spoke_schemas::TimelineScale("narrative".to_owned()));

        let filtered = filter_timeline_events_by_scope(&timeline_events, &scope);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].timeline_event_id, "evt_2");
    }

    #[test]
    fn filter_timeline_events_by_fork_id() {
        let timeline_events = [
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_1".into();
                event.fork_id = Some(
                    spoke_schemas::timeline_event::TimelineEventForkId::try_from(
                        "fork_mainline_a".to_owned(),
                    )
                    .unwrap(),
                );
            }),
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_2".into();
                event.fork_id = Some(
                    spoke_schemas::timeline_event::TimelineEventForkId::try_from(
                        "fork_what_if_b".to_owned(),
                    )
                    .unwrap(),
                );
            }),
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_3".into();
            }),
        ];

        let mut scope = base_scope();
        scope.fork_id = Some(
            spoke_schemas::ForkId::try_from("fork_mainline_a".to_owned()).unwrap(),
        );

        let filtered = filter_timeline_events_by_scope(&timeline_events, &scope);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].timeline_event_id, "evt_1");
    }

    #[test]
    fn scope_extensions_are_opaque_to_matchers() {
        // Scope carries product-scoped query metadata under extensions.<namespace>.
        // Protocol matchers match core Scope fields only and must neither consume nor
        // alter extensions. Build via wire JSON to also exercise round-trip parsing.
        let scope: Scope = serde_json::from_value(json!({
            "scope_id": "world_1",
            "entry_types": ["character"],
            "extensions": {
                "toy": { "branch_id": "fork_mainline_a" },
                "nexus": { "text_search": "harbor", "limit": 10 }
            }
        }))
        .expect("scope with extensions");

        let matching_entry = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_1".into();
            entry.entry_type = "character".into();
        });
        let non_matching_entry = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_other".into();
            entry.entry_type = "location".into();
        });

        let mut core_scope = base_scope();
        core_scope.entry_types = Some(vec!["character".into()]);

        // (a) matching uses core fields — identical selection with or without extensions.
        assert_eq!(
            knowledge_entry_passes_scope_authorization(&matching_entry, &scope),
            knowledge_entry_passes_scope_authorization(&matching_entry, &core_scope),
        );
        assert_eq!(
            knowledge_entry_passes_scope_authorization(&non_matching_entry, &scope),
            knowledge_entry_passes_scope_authorization(&non_matching_entry, &core_scope),
        );
        assert!(knowledge_entry_passes_scope_authorization(&matching_entry, &scope));
        assert!(!knowledge_entry_passes_scope_authorization(&non_matching_entry, &scope));

        // Timeline matchers also ignore extensions.
        let mut event = make_timeline_event(|ev| {
            ev.timeline_scale = Some("narrative".into());
        });
        assert!(timeline_event_matches_scope(&event, &scope));

        // (b) extensions preserved verbatim after driving matcher + filter surfaces.
        let snapshot = scope.extensions.clone();
        let entries = [matching_entry, non_matching_entry];
        let _ = authorized_scope_entries(&entries, &scope);
        let _events = [event];
        let _ = filter_timeline_events_by_scope(&_events, &scope);
        assert_eq!(
            serde_json::to_value(&scope.extensions).expect("serialize current extensions"),
            serde_json::to_value(&snapshot).expect("serialize snapshot extensions")
        );

        // Product query metadata survives untouched (round-trip serialize the bag).
        let extensions_wire = serde_json::to_value(&scope.extensions).expect("serialize extensions");
        assert_eq!(
            extensions_wire,
            json!({
                "toy": { "branch_id": "fork_mainline_a" },
                "nexus": { "text_search": "harbor", "limit": 10 }
            }),
        );
    }

    fn owner(id: &str) -> KnowledgeEntryOwner {
        KnowledgeEntryOwner::try_from(id).expect("non-empty owner entry id")
    }

    fn disclosure(value: &str) -> KnowledgeEntryDisclosure {
        KnowledgeEntryDisclosure::try_from(value).expect("non-empty disclosure value")
    }

    fn viewpoint(id: &str) -> Option<ScopeViewpoint> {
        Some(ScopeViewpoint::try_from(id).expect("non-empty viewpoint entry id"))
    }

    fn owner_private_entry(entry_id: &str, holder_id: &str) -> KnowledgeEntry {
        make_knowledge_entry(|entry| {
            entry.entry_id = entry_id.into();
            entry.owner = Some(owner(holder_id));
            entry.disclosure = Some(disclosure("owner-private"));
        })
    }

    #[test]
    fn knowledge_entry_without_disclosure_matches_foreign_scope_viewpoint() {
        let knowledge_entry = make_knowledge_entry(|entry| {
            entry.owner = Some(owner("kb_mira"));
        });

        let mut scope = base_scope();
        scope.viewpoint = viewpoint("kb_other");

        assert!(knowledge_entry_passes_scope_authorization(&knowledge_entry, &scope));
    }

    #[test]
    fn knowledge_entry_owner_private_matches_its_own_scope_viewpoint() {
        let knowledge_entry = owner_private_entry("kb_1", "kb_mira");

        let mut scope = base_scope();
        scope.viewpoint = viewpoint("kb_mira");

        assert!(knowledge_entry_passes_scope_authorization(&knowledge_entry, &scope));
    }

    #[test]
    fn knowledge_entry_owner_private_excluded_from_scope_without_viewpoint() {
        let knowledge_entry = owner_private_entry("kb_1", "kb_mira");

        assert!(!knowledge_entry_passes_scope_authorization(&knowledge_entry, &base_scope()));
    }

    #[test]
    fn knowledge_entry_owner_private_excluded_from_foreign_scope_viewpoint() {
        let knowledge_entry = owner_private_entry("kb_1", "kb_mira");

        let mut scope = base_scope();
        scope.viewpoint = viewpoint("kb_other");

        assert!(!knowledge_entry_passes_scope_authorization(&knowledge_entry, &scope));
    }

    #[test]
    fn knowledge_entry_composes_disclosure_predicate_with_entry_refinements() {
        let knowledge_entry = owner_private_entry("kb_1", "kb_mira");

        let mut scope = base_scope();
        scope.entry_ids = Some(vec!["kb_1".into()]);
        scope.viewpoint = viewpoint("kb_mira");
        assert!(knowledge_entry_passes_scope_authorization(&knowledge_entry, &scope));

        // entry_ids admits the entry; the disclosure predicate still excludes it.
        scope.viewpoint = viewpoint("kb_other");
        assert!(!knowledge_entry_passes_scope_authorization(&knowledge_entry, &scope));

        // viewpoint admits the entry; the entry refinement still excludes it.
        scope.entry_ids = Some(vec!["kb_missing".into()]);
        scope.viewpoint = viewpoint("kb_mira");
        assert!(!knowledge_entry_passes_scope_authorization(&knowledge_entry, &scope));
    }

    #[test]
    fn knowledge_entry_unknown_disclosure_excluded_under_matching_scope_viewpoint() {
        let knowledge_entry = make_knowledge_entry(|entry| {
            entry.owner = Some(owner("kb_mira"));
            entry.disclosure = Some(disclosure("group-private"));
        });

        let mut scope = base_scope();
        scope.viewpoint = viewpoint("kb_mira");

        assert!(!knowledge_entry_passes_scope_authorization(&knowledge_entry, &scope));
    }

    #[test]
    fn knowledge_entry_matches_scope_keeps_viewpoint_and_wire_array_presence() {
        let knowledge_entry = owner_private_entry("kb_1", "kb_mira");

        let scope: Scope = serde_json::from_value(json!({
            "scope_id": "world_1",
            "viewpoint": "kb_mira",
        }))
        .expect("valid scope");
        assert!(knowledge_entry_passes_scope_authorization(&knowledge_entry, &scope));

        // Present-empty entry_ids still matches nothing, even with a matching viewpoint.
        let scope: Scope = serde_json::from_value(json!({
            "scope_id": "world_1",
            "entry_ids": [],
            "viewpoint": "kb_mira",
        }))
        .expect("valid scope");
        assert!(!knowledge_entry_passes_scope_authorization(&knowledge_entry, &scope));
    }

    #[test]
    fn filter_knowledge_entries_keeps_shared_and_own_private_entries() {
        let knowledge_entries = [
            make_knowledge_entry(|entry| {
                entry.entry_id = "kb_shared".into();
            }),
            owner_private_entry("kb_mine", "kb_mira"),
            owner_private_entry("kb_theirs", "kb_rival"),
            make_knowledge_entry(|entry| {
                entry.entry_id = "kb_group".into();
                entry.owner = Some(owner("kb_mira"));
                entry.disclosure = Some(disclosure("group-private"));
            }),
        ];

        let mut scope = base_scope();
        scope.viewpoint = viewpoint("kb_mira");

        let filtered = authorized_scope_entries(&knowledge_entries, &scope);
        let entry_ids: Vec<&str> = filtered
            .iter()
            .map(|entry| entry.entry_id.as_str())
            .collect();
        assert_eq!(entry_ids, ["kb_shared", "kb_mine"]);
    }

    #[test]
    fn filter_knowledge_entries_without_viewpoint_keeps_only_shared_entries() {
        let knowledge_entries = [
            make_knowledge_entry(|entry| {
                entry.entry_id = "kb_shared".into();
            }),
            owner_private_entry("kb_mine", "kb_mira"),
        ];

        let filtered = authorized_scope_entries(&knowledge_entries, &base_scope());
        let entry_ids: Vec<&str> = filtered
            .iter()
            .map(|entry| entry.entry_id.as_str())
            .collect();
        assert_eq!(entry_ids, ["kb_shared"]);
    }

    #[test]
    fn served_scope_filter_applies_cognition_after_authorization() {
        let entries = [
            cognition_entry(false),
            make_knowledge_entry(|entry| {
                entry.entry_id = "kb_plain".into();
            }),
        ];

        // Baseline without cognition is unchanged.
        let baseline = authorized_scope_entries(&entries, &base_scope());
        let baseline_ids: Vec<&str> = baseline
            .iter()
            .map(|entry| entry.entry_id.as_str())
            .collect();
        assert_eq!(baseline_ids, ["kb_mira", "kb_plain"]);

        // A current selection filters on carried cognition after authorization.
        let mut selected_scope = base_scope();
        selected_scope.cognition = Some(
            serde_json::from_value(json!({
                "view": "current",
                "filter": {
                    "holder": {
                        "kind": "knowledge_entry",
                        "namespace": "world_a",
                        "id": "kb_mira"
                    }
                },
                "selection_policy": { "policy_id": "pol_scope", "policy_version": "v1" }
            }))
            .expect("current cognition selection"),
        );
        let selected = authorized_scope_entries(&entries, &selected_scope);
        let selected_ids: Vec<&str> = selected
            .iter()
            .map(|entry| entry.entry_id.as_str())
            .collect();
        assert_eq!(selected_ids, ["kb_mira"]);

        // The recorded-at source is unavailable here: reject at the Scope field
        // path instead of silently returning the baseline set.
        let mut recorded_at_scope = base_scope();
        recorded_at_scope.cognition = Some(
            serde_json::from_value(json!({
                "view": "recorded_at",
                "filter": {
                    "holder": {
                        "kind": "knowledge_entry",
                        "namespace": "world_a",
                        "id": "kb_mira"
                    }
                }
            }))
            .expect("recorded-at cognition selection"),
        );
        match filter_knowledge_entries_by_scope(&entries, &recorded_at_scope) {
            SpokeResult::Reject(reject) => {
                assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
                let details = reject.details.expect("details");
                assert_eq!(
                    details.get("reason").and_then(Value::as_str),
                    Some("unsupported_cognition_requirement")
                );
                assert_eq!(
                    details.get("field").and_then(Value::as_str),
                    Some("scope.cognition.view")
                );
            }
            SpokeResult::Ok(_) => panic!("recorded-at view must reject"),
        }
    }

    /// Some closed-grammar violations are decodable in the typed filter
    /// (`{absent: false}` on a categorical predicate, a `q` range with no
    /// bound), so the served filter must reject the whole call at the actual
    /// Scope field path instead of silently matching nothing — including when
    /// the authorized source is empty.
    #[test]
    fn served_scope_filter_rejects_grammar_invalid_typed_cognition_filters() {
        let current_selection = |filter: Value| -> Scope {
            serde_json::from_value(json!({
                "scope_id": "world_1",
                "cognition": {
                    "view": "current",
                    "filter": filter,
                    "selection_policy": { "policy_id": "pol_scope", "policy_version": "v1" }
                }
            }))
            .expect("typed filter shape decodes")
        };

        for (filter, field) in [
            (
                json!({ "truth": { "absent": false } }),
                "scope.cognition.filter.truth.absent",
            ),
            (json!({ "q": {} }), "scope.cognition.filter.q"),
        ] {
            let scope = current_selection(filter);
            for entries in [Vec::new(), vec![cognition_entry(false)]] {
                match filter_knowledge_entries_by_scope(&entries, &scope) {
                    SpokeResult::Reject(reject) => {
                        assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
                        let details = reject.details.expect("details");
                        assert_eq!(
                            details.get("reason").and_then(Value::as_str),
                            Some("unsupported_cognition_requirement")
                        );
                        assert_eq!(
                            details.get("field").and_then(Value::as_str),
                            Some(field)
                        );
                    }
                    SpokeResult::Ok(_) => panic!("{field} must reject, not match"),
                }
            }
        }

        // Valid controls: an unrefined Scope and a grammar-valid `absent: true`
        // predicate still serve the baseline (truth-absent) belief entry.
        let entries = [cognition_entry(false)];
        assert_eq!(authorized_scope_entries(&entries, &base_scope()).len(), 1);
        let absent_true = current_selection(json!({ "truth": { "absent": true } }));
        assert_eq!(authorized_scope_entries(&entries, &absent_true).len(), 1);
    }

    // === P2/T2 authorization-first cognition carriers ===

    fn cognition_entry(wire_owner: bool) -> KnowledgeEntry {
        let mut wire = json!({
            "schema_version": 1,
            "entry_id": "kb_mira",
            "entry_type": "character",
            "canonical_name": "Mira Vale",
            "status": "confirmed",
            "body": {},
            "extensions": {},
            "modules": {
                "cognition": {
                    "records": [{
                        "kind": "belief",
                        "record_id": "rec_scope",
                        "holder": {
                            "kind": "knowledge_entry",
                            "namespace": "world_a",
                            "id": "kb_mira"
                        },
                        "target": {
                            "carrier": {
                                "kind": "knowledge_entry",
                                "namespace": "world_a",
                                "id": "kb_box"
                            },
                            "target_id": "t_marble",
                            "kind": "proposition"
                        },
                        "assessor": {
                            "kind": "principal",
                            "namespace": "world_a",
                            "id": "assessor_self"
                        },
                        "assessor_role": "holder_self_report",
                        "basis": {
                            "kind": "evidence",
                            "evidence": [{
                                "kind": "knowledge_entry",
                                "namespace": "world_a",
                                "id": "kb_box"
                            }]
                        },
                        "assessment_context": {
                            "context_id": "ctx_main",
                            "fork_id": "fork_alpha"
                        },
                        "effective_at": "2026-07-23T09:00:00Z",
                        "assessed_at": "2026-07-23T09:05:00Z",
                        "view": "current",
                        "order": 1,
                        "q": {
                            "value": 0.9,
                            "assertion": "belief_attribution",
                            "method": "direct_report",
                            "calibration": "uncalibrated"
                        },
                        "extensions": {}
                    }],
                    "world_facts": [],
                    "extensions": {}
                }
            }
        });
        if wire_owner {
            if let Some(object) = wire.as_object_mut() {
                object.insert("owner".into(), json!("kb_mira"));
                object.insert("disclosure".into(), json!("owner-private"));
            }
        }
        serde_json::from_value(wire).expect("valid cognition KnowledgeEntry")
    }

    fn cognition_record_ids(
        result: SpokeResult<Vec<CognitionRecord>>,
    ) -> Vec<String> {
        match result {
            SpokeResult::Ok(records) => records
                .iter()
                .map(|record| match record {
                    CognitionRecord::BeliefRecord(record) => record.record_id.as_str().to_owned(),
                    CognitionRecord::EpistemicAssessment(record) => {
                        record.record_id.as_str().to_owned()
                    }
                    CognitionRecord::PerceptionRecord(record) => {
                        record.record_id.as_str().to_owned()
                    }
                })
                .collect(),
            SpokeResult::Reject(reject) => panic!("extraction rejected: {reject:?}"),
        }
    }

    #[test]
    fn cognition_records_follow_authorization_before_selection() {
        let entries = vec![cognition_entry(true)];

        let authorized = authorize_knowledge_entries_by_scope(
            &entries,
            &Scope {
                viewpoint: Some(ScopeViewpoint::try_from("kb_mira".to_owned()).unwrap()),
                ..base_scope()
            },
        );
        let visible = cognition_record_ids(cognition_records_of_authorized_entries(&authorized));
        assert_eq!(visible, ["rec_scope"]);

        let withheld = authorize_knowledge_entries_by_scope(
            &entries,
            &Scope {
                viewpoint: Some(ScopeViewpoint::try_from("kb_rival".to_owned()).unwrap()),
                ..base_scope()
            },
        );
        let hidden = cognition_record_ids(cognition_records_of_authorized_entries(&withheld));
        assert!(hidden.is_empty());
    }

    #[test]
    fn cognition_records_ignore_entries_without_carrier() {
        let entries = vec![make_knowledge_entry(|_| {})];
        let authorized = authorize_knowledge_entries_by_scope(&entries, &base_scope());
        let records = cognition_record_ids(cognition_records_of_authorized_entries(&authorized));
        assert!(records.is_empty());
    }

    #[test]
    fn cognition_records_reject_malformed_carrier() {
        let mut entry = make_knowledge_entry(|_| {});
        entry.modules.insert(
            "cognition".parse().expect("module key"),
            serde_json::from_value(json!({ "records": [], "extensions": {} }))
                .expect("module value"),
        );

        match cognition_records_of_authorized_entries(&[&entry]) {
            SpokeResult::Reject(reject) => {
                assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
                let details = reject.details.expect("details");
                assert_eq!(
                    details.get("reason").and_then(Value::as_str),
                    Some("invalid_cognition_record")
                );
                assert_eq!(
                    details.get("field").and_then(Value::as_str),
                    Some("modules.cognition")
                );
            }
            SpokeResult::Ok(_) => panic!("malformed cognition module must reject"),
        }
    }
}
