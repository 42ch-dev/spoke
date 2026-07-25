//! Scope matching and filtering helpers.

use spoke_schemas::knowledge_entry::KnowledgeEntry;
use spoke_schemas::{Scope, TimelineEvent};

/// KnowledgeEntry passes optional Scope refinements (AND when present).
#[must_use]
pub fn knowledge_entry_matches_scope(knowledge_entry: &KnowledgeEntry, scope: &Scope) -> bool {
    if !scope.entry_ids.is_empty() && !scope.entry_ids.contains(&knowledge_entry.entry_id) {
        return false;
    }

    if !scope.entry_types.is_empty()
        && !scope.entry_types.contains(&knowledge_entry.entry_type)
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

    true
}

/// Filter KnowledgeEntries by optional Scope refinements.
#[must_use]
pub fn filter_knowledge_entries_by_scope<'a>(
    knowledge_entries: &'a [KnowledgeEntry],
    scope: &Scope,
) -> Vec<&'a KnowledgeEntry> {
    knowledge_entries
        .iter()
        .filter(|knowledge_entry| knowledge_entry_matches_scope(knowledge_entry, scope))
        .collect()
}

/// TimelineEvent passes optional Scope refinements (AND when present).
#[must_use]
pub fn timeline_event_matches_scope(timeline_event: &TimelineEvent, scope: &Scope) -> bool {
    if !scope.timeline_event_ids.is_empty()
        && !scope
            .timeline_event_ids
            .contains(&timeline_event.timeline_event_id)
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
    use spoke_schemas::knowledge_entry::{KnowledgeEntryBody, KnowledgeEntryCanonicalName, SourceAnchor};
    use spoke_schemas::timeline_event::TimelineEventCanonicalName;
    use std::collections::HashMap;
    use std::num::NonZeroU64;

    fn base_scope() -> Scope {
        Scope {
            entry_ids: Vec::new(),
            entry_types: Vec::new(),
            fork_id: None,
            scope_id: "world_1".into(),
            source_id: None,
            timeline_event_ids: Vec::new(),
            timeline_scale: None,
        }
    }

    fn make_knowledge_entry(overrides: impl FnOnce(&mut KnowledgeEntry)) -> KnowledgeEntry {
        let mut entry = KnowledgeEntry {
            body: KnowledgeEntryBody::default(),
            canonical_name: KnowledgeEntryCanonicalName::try_from("Mira Vale".to_owned()).unwrap(),
            created_at: None,
            entry_id: "kb_1".into(),
            entry_type: "character".into(),
            extensions: HashMap::new(),
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

    #[test]
    fn knowledge_entry_passes_when_only_scope_id_is_set() {
        let knowledge_entry = make_knowledge_entry(|entry| {
            entry.source_anchor = Some(SourceAnchor {
                extensions: HashMap::new(),
                label: None,
                mime_type: None,
                schema_version: NonZeroU64::new(1).unwrap(),
                source_id: "manuscript_1".into(),
                span: None,
            });
        });

        assert!(knowledge_entry_matches_scope(
            &knowledge_entry,
            &base_scope()
        ));
    }

    #[test]
    fn knowledge_entry_matches_entry_ids_refinement() {
        let knowledge_entry = make_knowledge_entry(|entry| {
            entry.source_anchor = Some(SourceAnchor {
                extensions: HashMap::new(),
                label: None,
                mime_type: None,
                schema_version: NonZeroU64::new(1).unwrap(),
                source_id: "manuscript_1".into(),
                span: None,
            });
        });

        let mut matching_scope = base_scope();
        matching_scope.entry_ids = vec!["kb_1".into(), "kb_2".into()];
        assert!(knowledge_entry_matches_scope(
            &knowledge_entry,
            &matching_scope
        ));

        matching_scope.entry_ids = vec!["kb_2".into()];
        assert!(!knowledge_entry_matches_scope(
            &knowledge_entry,
            &matching_scope
        ));
    }

    #[test]
    fn knowledge_entry_matches_entry_types_refinement() {
        let knowledge_entry = make_knowledge_entry(|entry| {
            entry.source_anchor = Some(SourceAnchor {
                extensions: HashMap::new(),
                label: None,
                mime_type: None,
                schema_version: NonZeroU64::new(1).unwrap(),
                source_id: "manuscript_1".into(),
                span: None,
            });
        });

        let mut matching_scope = base_scope();
        matching_scope.entry_types = vec!["character".into()];
        assert!(knowledge_entry_matches_scope(
            &knowledge_entry,
            &matching_scope
        ));

        matching_scope.entry_types = vec!["location".into()];
        assert!(!knowledge_entry_matches_scope(
            &knowledge_entry,
            &matching_scope
        ));
    }

    #[test]
    fn knowledge_entry_matches_source_id_refinement() {
        let knowledge_entry = make_knowledge_entry(|entry| {
            entry.source_anchor = Some(SourceAnchor {
                extensions: HashMap::new(),
                label: None,
                mime_type: None,
                schema_version: NonZeroU64::new(1).unwrap(),
                source_id: "manuscript_1".into(),
                span: None,
            });
        });

        let mut matching_scope = base_scope();
        matching_scope.source_id = Some("manuscript_1".into());
        assert!(knowledge_entry_matches_scope(
            &knowledge_entry,
            &matching_scope
        ));

        matching_scope.source_id = Some("other".into());
        assert!(!knowledge_entry_matches_scope(
            &knowledge_entry,
            &matching_scope
        ));
    }

    #[test]
    fn knowledge_entry_ignores_timeline_event_refinements() {
        let knowledge_entry = make_knowledge_entry(|entry| {
            entry.source_anchor = Some(SourceAnchor {
                extensions: HashMap::new(),
                label: None,
                mime_type: None,
                schema_version: NonZeroU64::new(1).unwrap(),
                source_id: "manuscript_1".into(),
                span: None,
            });
        });

        let mut scope = base_scope();
        scope.timeline_event_ids = vec!["evt_missing".into()];
        scope.timeline_scale = Some("brief".into());

        assert!(knowledge_entry_matches_scope(
            &knowledge_entry,
            &scope
        ));
    }

    #[test]
    fn knowledge_entry_requires_all_present_refinements() {
        let knowledge_entry = make_knowledge_entry(|entry| {
            entry.source_anchor = Some(SourceAnchor {
                extensions: HashMap::new(),
                label: None,
                mime_type: None,
                schema_version: NonZeroU64::new(1).unwrap(),
                source_id: "manuscript_1".into(),
                span: None,
            });
        });

        let mut scope = base_scope();
        scope.entry_ids = vec!["kb_1".into()];
        scope.entry_types = vec!["location".into()];

        assert!(!knowledge_entry_matches_scope(
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
        scope.entry_types = vec!["character".into()];

        let filtered = filter_knowledge_entries_by_scope(&knowledge_entries, &scope);
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
        matching_scope.timeline_event_ids = vec!["evt_1".into()];
        assert!(timeline_event_matches_scope(
            &timeline_event,
            &matching_scope
        ));

        matching_scope.timeline_event_ids = vec!["evt_2".into()];
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
        matching_scope.timeline_scale = Some("narrative".into());
        assert!(timeline_event_matches_scope(
            &timeline_event,
            &matching_scope
        ));

        matching_scope.timeline_scale = Some("brief".into());
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
        scope.entry_ids = vec!["kb_missing".into()];
        scope.entry_types = vec!["character".into()];
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
            spoke_schemas::ScopeForkId::try_from("fork_mainline_a".to_owned()).unwrap(),
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
            spoke_schemas::ScopeForkId::try_from("fork_what_if_b".to_owned()).unwrap(),
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
            spoke_schemas::ScopeForkId::try_from("fork_mainline_a".to_owned()).unwrap(),
        );

        assert!(!timeline_event_matches_scope(&timeline_event, &scope));
    }

    #[test]
    fn timeline_event_does_not_filter_on_parent_fork_id() {
        let timeline_event = make_timeline_event(|event| {
            event.timeline_event_id = "evt_fork".into();
            event.parent_fork_id = Some(
                spoke_schemas::timeline_event::TimelineEventParentForkId::try_from(
                    "fork_mainline_a".to_owned(),
                )
                .unwrap(),
            );
        });

        let mut scope = base_scope();
        scope.fork_id = Some(
            spoke_schemas::ScopeForkId::try_from("fork_mainline_a".to_owned()).unwrap(),
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
        scope.timeline_scale = Some("narrative".into());

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
            spoke_schemas::ScopeForkId::try_from("fork_mainline_a".to_owned()).unwrap(),
        );

        let filtered = filter_timeline_events_by_scope(&timeline_events, &scope);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].timeline_event_id, "evt_1");
    }
}
