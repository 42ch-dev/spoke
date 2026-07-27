//! Beat-assist pure helpers over caller-supplied TimelineEvent and Relation slices.

use crate::result::{spoke_ok, spoke_reject, SpokeRejectCode, SpokeResult};
use crate::util::trimmed_string_field;
use serde_json::{json, Map, Value};
use spoke_schemas::timeline_event::TimelineEventExtensionsKey;
use spoke_schemas::{Relation, TimelineEvent};
use std::collections::{HashMap, HashSet};

fn is_non_empty_trimmed_string(value: &str) -> bool {
    !value.trim().is_empty()
}

fn read_timeline_entry_id(event: &TimelineEvent) -> Option<String> {
    let spoke_key = TimelineEventExtensionsKey::try_from("spoke").ok()?;
    let spoke_map = event.extensions.get(&spoke_key)?;
    trimmed_string_field(&Value::Object(spoke_map.clone()), "timeline_entry_id")
}

/// Keep TimelineEvents where `timeline_scale` is exactly `"moment"`; input order preserved.
#[must_use]
pub fn filter_timeline_events_by_moment_scale(
    timeline_events: &[TimelineEvent],
) -> Vec<&TimelineEvent> {
    timeline_events
        .iter()
        .filter(|event| event.timeline_scale.as_deref() == Some("moment"))
        .collect()
}

/// Order TimelineEvents by an explicit `timeline_event_id` list; unknown or duplicate ids reject.
pub fn order_timeline_events_by_ids(
    timeline_events: &[TimelineEvent],
    ordered_ids: &[String],
) -> SpokeResult<Vec<TimelineEvent>> {
    let mut seen_ordered_ids = HashSet::new();
    for id in ordered_ids {
        if !seen_ordered_ids.insert(id.clone()) {
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                "orderedIds contains duplicate timeline_event_id values",
                None,
            );
        }
    }

    let by_id: HashMap<&str, &TimelineEvent> = timeline_events
        .iter()
        .map(|event| (event.timeline_event_id.as_str(), event))
        .collect();

    let mut unknown_timeline_event_ids = Vec::new();
    for id in ordered_ids {
        if !by_id.contains_key(id.as_str()) {
            unknown_timeline_event_ids.push(id.clone());
        }
    }
    if !unknown_timeline_event_ids.is_empty() {
        let mut details = Map::new();
        details.insert(
            "unknown_timeline_event_ids".into(),
            json!(unknown_timeline_event_ids),
        );
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            "orderedIds contains timeline_event_id values not present in timelineEvents",
            Some(details),
        );
    }

    let ordered_id_set: HashSet<&str> = ordered_ids.iter().map(String::as_str).collect();
    let mut ordered = Vec::with_capacity(timeline_events.len());
    for id in ordered_ids {
        ordered.push(by_id[id.as_str()].clone());
    }
    for event in timeline_events {
        if !ordered_id_set.contains(event.timeline_event_id.as_str()) {
            ordered.push(event.clone());
        }
    }

    spoke_ok(ordered)
}

/// Options for [`order_timeline_events_by_precedes`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrderTimelineEventsByPrecedesOptions {
    pub relation_type: Option<String>,
}

/// Topologically order linked TimelineEvents via `precedes` Relations on dual KE ids.
pub fn order_timeline_events_by_precedes(
    timeline_events: &[TimelineEvent],
    relations: &[Relation],
    options: Option<&OrderTimelineEventsByPrecedesOptions>,
) -> SpokeResult<Vec<TimelineEvent>> {
    let relation_type = options
        .and_then(|opts| opts.relation_type.as_deref())
        .unwrap_or("precedes");

    let mut linked_events: Vec<TimelineEvent> = Vec::new();
    let mut unlinked_events = Vec::new();
    let mut entry_id_to_event_id = HashMap::new();
    let mut event_id_to_entry_id = HashMap::new();

    for event in timeline_events {
        let Some(entry_id) = read_timeline_entry_id(event) else {
            unlinked_events.push(event.clone());
            continue;
        };
        linked_events.push(event.clone());
        entry_id_to_event_id.insert(entry_id.clone(), event.timeline_event_id.clone());
        event_id_to_entry_id.insert(event.timeline_event_id.clone(), entry_id);
    }

    let linked_event_ids: HashSet<String> = linked_events
        .iter()
        .map(|event| event.timeline_event_id.clone())
        .collect();
    let mut in_degree: HashMap<&str, usize> = linked_event_ids
        .iter()
        .map(|event_id| (event_id.as_str(), 0))
        .collect();
    let mut adjacency: HashMap<&str, Vec<String>> = linked_event_ids
        .iter()
        .map(|event_id| (event_id.as_str(), Vec::new()))
        .collect();

    for relation in relations {
        if relation.relation_type != relation_type {
            continue;
        }
        if !is_non_empty_trimmed_string(&relation.from_id)
            || !is_non_empty_trimmed_string(&relation.to_id)
        {
            continue;
        }

        let from_entry_id = relation.from_id.trim();
        let to_entry_id = relation.to_id.trim();
        let Some(from_event_id) = entry_id_to_event_id.get(from_entry_id) else {
            continue;
        };
        let Some(to_event_id) = entry_id_to_event_id.get(to_entry_id) else {
            continue;
        };
        if from_event_id == to_event_id {
            continue;
        }

        adjacency
            .get_mut(from_event_id.as_str())
            .expect("linked event id")
            .push(to_event_id.clone());
        let degree = in_degree
            .get_mut(to_event_id.as_str())
            .expect("linked event id");
        *degree += 1;
    }

    let mut ready: Vec<String> = linked_event_ids
        .iter()
        .filter(|event_id| in_degree.get(event_id.as_str()).copied().unwrap_or(0) == 0)
        .cloned()
        .collect();
    ready.sort();

    let mut sorted_linked_ids = Vec::with_capacity(linked_event_ids.len());
    while let Some(current) = ready.first().cloned() {
        ready.remove(0);
        sorted_linked_ids.push(current.clone());

        let neighbors = adjacency
            .get(current.as_str())
            .cloned()
            .unwrap_or_default();
        for neighbor in neighbors {
            let degree = in_degree
                .get_mut(neighbor.as_str())
                .expect("linked event id");
            *degree -= 1;
            if *degree == 0 {
                ready.push(neighbor);
            }
        }
        ready.sort();
    }

    if sorted_linked_ids.len() != linked_event_ids.len() {
        let mut cycle_entry_ids: Vec<String> = linked_event_ids
            .iter()
            .filter(|event_id| in_degree.get(event_id.as_str()).copied().unwrap_or(0) > 0)
            .filter_map(|event_id| event_id_to_entry_id.get(event_id).cloned())
            .collect();
        cycle_entry_ids.sort();

        let mut details = Map::new();
        details.insert("precedes_cycle".into(), json!(true));
        details.insert("entry_ids".into(), json!(cycle_entry_ids));
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            "precedes relation graph contains a cycle among linked timeline events",
            Some(details),
        );
    }

    let event_by_id: HashMap<&str, &TimelineEvent> = timeline_events
        .iter()
        .map(|event| (event.timeline_event_id.as_str(), event))
        .collect();

    let mut ordered = Vec::with_capacity(timeline_events.len());
    for event_id in &sorted_linked_ids {
        ordered.push(event_by_id[event_id.as_str()].clone());
    }
    ordered.extend(unlinked_events);

    spoke_ok(ordered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result::SpokeResult;
    use serde_json::Value;
    use spoke_schemas::timeline_event::TimelineEventCanonicalName;
    use std::collections::HashMap;
    use std::num::NonZeroU64;
    use std::path::Path;

    fn fixture_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/toy-world")
    }

    fn load_fixture<T: serde::de::DeserializeOwned>(filename: &str) -> T {
        let path = fixture_root().join(filename);
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read fixture {}: {error}", path.display()));
        serde_json::from_str(&raw)
            .unwrap_or_else(|error| panic!("failed to parse fixture {}: {error}", path.display()))
    }

    fn make_timeline_event(overrides: impl FnOnce(&mut TimelineEvent)) -> TimelineEvent {
        let mut event = TimelineEvent {
            canonical_name: TimelineEventCanonicalName::try_from("Event".to_owned()).unwrap(),
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

    fn spoke_extension(timeline_entry_id: &str) -> HashMap<TimelineEventExtensionsKey, Map<String, Value>> {
        let mut spoke_map = Map::new();
        spoke_map.insert(
            "timeline_entry_id".into(),
            Value::String(timeline_entry_id.into()),
        );
        HashMap::from([(
            TimelineEventExtensionsKey::try_from("spoke").unwrap(),
            spoke_map,
        )])
    }

    fn make_relation(overrides: impl FnOnce(&mut Relation)) -> Relation {
        let mut relation = Relation {
            created_at: None,
            extensions: HashMap::new(),
            from_id: "kb_a".into(),
            label: None,
            metadata: Map::new(),
            relation_id: "rel_1".into(),
            relation_type: "precedes".into(),
            schema_version: NonZeroU64::new(1).unwrap(),
            to_id: "kb_b".into(),
            updated_at: None,
        };
        overrides(&mut relation);
        relation
    }

    #[test]
    fn filter_keeps_only_moment_scale_events_in_input_order() {
        let events = [
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_1".into();
                event.timeline_scale = Some("moment".into());
            }),
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_2".into();
                event.timeline_scale = Some("narrative".into());
            }),
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_3".into();
                event.timeline_scale = Some("moment".into());
            }),
            make_timeline_event(|event| event.timeline_event_id = "evt_4".into()),
        ];

        let filtered = filter_timeline_events_by_moment_scale(&events);
        assert_eq!(
            filtered
                .iter()
                .map(|event| event.timeline_event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["evt_1", "evt_3"]
        );
    }

    #[test]
    fn filter_returns_empty_for_empty_input() {
        assert!(filter_timeline_events_by_moment_scale(&[]).is_empty());
    }

    #[test]
    fn order_by_ids_orders_explicit_list_and_appends_stable_tail() {
        let events = [
            make_timeline_event(|event| event.timeline_event_id = "evt_a".into()),
            make_timeline_event(|event| event.timeline_event_id = "evt_b".into()),
            make_timeline_event(|event| event.timeline_event_id = "evt_c".into()),
        ];

        let result = order_timeline_events_by_ids(&events, &["evt_c".into(), "evt_a".into()]);
        let SpokeResult::Ok(ordered) = result else {
            panic!("expected ok result");
        };
        assert_eq!(
            ordered
                .iter()
                .map(|event| event.timeline_event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["evt_c", "evt_a", "evt_b"]
        );
    }

    #[test]
    fn order_by_ids_rejects_unknown_timeline_event_ids() {
        let events = [make_timeline_event(|event| {
            event.timeline_event_id = "evt_a".into();
        })];
        let result = order_timeline_events_by_ids(
            &events,
            &["evt_a".into(), "evt_missing".into()],
        );
        let SpokeResult::Reject(reject) = result else {
            panic!("expected reject result");
        };
        assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        assert_eq!(
            reject.details.as_ref().and_then(|details| details.get("unknown_timeline_event_ids")),
            Some(&json!(["evt_missing"]))
        );
    }

    #[test]
    fn order_by_ids_rejects_duplicate_ids() {
        let events = [make_timeline_event(|event| {
            event.timeline_event_id = "evt_a".into();
        })];
        let result = order_timeline_events_by_ids(&events, &["evt_a".into(), "evt_a".into()]);
        assert!(result.is_reject());
    }

    #[test]
    fn order_by_precedes_orders_harbor_moment_beats() {
        let events = [
            load_fixture::<TimelineEvent>("evt_tw_harbor_berth_confirm.json"),
            load_fixture::<TimelineEvent>("evt_tw_harbor_dawn.json"),
            load_fixture::<TimelineEvent>("evt_tw_harbor_customs_gate.json"),
            load_fixture::<TimelineEvent>("evt_tw_harbor_market_square.json"),
        ];
        let relations = [
            load_fixture::<Relation>("rel_tw_harbor_precedes_dawn_to_market.json"),
            load_fixture::<Relation>("rel_tw_harbor_precedes_market_to_customs.json"),
            load_fixture::<Relation>("rel_tw_harbor_precedes_customs_to_berth.json"),
        ];

        let result = order_timeline_events_by_precedes(&events, &relations, None);
        let SpokeResult::Ok(ordered) = result else {
            panic!("expected ok result");
        };
        assert_eq!(
            ordered
                .iter()
                .map(|event| event.timeline_event_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "evt_tw_harbor_dawn",
                "evt_tw_harbor_market_square",
                "evt_tw_harbor_customs_gate",
                "evt_tw_harbor_berth_confirm",
            ]
        );
    }

    #[test]
    fn order_by_precedes_appends_unlinked_events_in_input_order() {
        let events = [
            make_timeline_event(|event| event.timeline_event_id = "evt_unlinked_1".into()),
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_linked_a".into();
                event.extensions = spoke_extension("kb_a");
            }),
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_linked_b".into();
                event.extensions = spoke_extension("kb_b");
            }),
            make_timeline_event(|event| event.timeline_event_id = "evt_unlinked_2".into()),
        ];
        let relations = [make_relation(|relation| {
            relation.relation_id = "rel_a_b".into();
            relation.from_id = "kb_a".into();
            relation.to_id = "kb_b".into();
        })];

        let result = order_timeline_events_by_precedes(&events, &relations, None);
        let SpokeResult::Ok(ordered) = result else {
            panic!("expected ok result");
        };
        assert_eq!(
            ordered
                .iter()
                .map(|event| event.timeline_event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["evt_linked_a", "evt_linked_b", "evt_unlinked_1", "evt_unlinked_2"]
        );
    }

    #[test]
    fn order_by_precedes_breaks_ready_queue_ties_lexicographically() {
        let events = [
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_z".into();
                event.extensions = spoke_extension("kb_z");
            }),
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_a".into();
                event.extensions = spoke_extension("kb_a");
            }),
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_m".into();
                event.extensions = spoke_extension("kb_m");
            }),
        ];

        let result = order_timeline_events_by_precedes(&events, &[], None);
        let SpokeResult::Ok(ordered) = result else {
            panic!("expected ok result");
        };
        assert_eq!(
            ordered
                .iter()
                .map(|event| event.timeline_event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["evt_a", "evt_m", "evt_z"]
        );
    }

    #[test]
    fn order_by_precedes_ignores_relations_outside_input_link_map() {
        let events = [
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_a".into();
                event.extensions = spoke_extension("kb_a");
            }),
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_b".into();
                event.extensions = spoke_extension("kb_b");
            }),
        ];
        let relations = [make_relation(|relation| {
            relation.relation_id = "rel_external".into();
            relation.from_id = "kb_a".into();
            relation.to_id = "kb_outside".into();
        })];

        let result = order_timeline_events_by_precedes(&events, &relations, None);
        let SpokeResult::Ok(ordered) = result else {
            panic!("expected ok result");
        };
        assert_eq!(
            ordered
                .iter()
                .map(|event| event.timeline_event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["evt_a", "evt_b"]
        );
    }

    #[test]
    fn order_by_precedes_rejects_cycles() {
        let events = [
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_a".into();
                event.extensions = spoke_extension("kb_a");
            }),
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_b".into();
                event.extensions = spoke_extension("kb_b");
            }),
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_c".into();
                event.extensions = spoke_extension("kb_c");
            }),
        ];
        let relations = [
            make_relation(|relation| {
                relation.relation_id = "rel_a_b".into();
                relation.from_id = "kb_a".into();
                relation.to_id = "kb_b".into();
            }),
            make_relation(|relation| {
                relation.relation_id = "rel_b_c".into();
                relation.from_id = "kb_b".into();
                relation.to_id = "kb_c".into();
            }),
            make_relation(|relation| {
                relation.relation_id = "rel_c_a".into();
                relation.from_id = "kb_c".into();
                relation.to_id = "kb_a".into();
            }),
        ];

        let result = order_timeline_events_by_precedes(&events, &relations, None);
        let SpokeResult::Reject(reject) = result else {
            panic!("expected reject result");
        };
        assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        assert_eq!(
            reject.details,
            Some(Map::from_iter([
                ("precedes_cycle".into(), json!(true)),
                ("entry_ids".into(), json!(["kb_a", "kb_b", "kb_c"])),
            ]))
        );
    }

    #[test]
    fn order_by_precedes_keeps_all_events_sharing_timeline_entry_id() {
        let events = [
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_b".into();
                event.extensions = spoke_extension("kb_shared");
            }),
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_a".into();
                event.extensions = spoke_extension("kb_shared");
            }),
        ];

        let result = order_timeline_events_by_precedes(&events, &[], None);
        let SpokeResult::Ok(ordered) = result else {
            panic!("expected ok result");
        };
        assert_eq!(
            ordered
                .iter()
                .map(|event| event.timeline_event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["evt_a", "evt_b"]
        );
    }

    #[test]
    fn order_by_precedes_honors_relation_type_override() {
        let events = [
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_a".into();
                event.extensions = spoke_extension("kb_a");
            }),
            make_timeline_event(|event| {
                event.timeline_event_id = "evt_b".into();
                event.extensions = spoke_extension("kb_b");
            }),
        ];
        let relations = [make_relation(|relation| {
            relation.relation_id = "rel_custom".into();
            relation.relation_type = "follows".into();
            relation.from_id = "kb_b".into();
            relation.to_id = "kb_a".into();
        })];

        let default_result = order_timeline_events_by_precedes(&events, &relations, None);
        let SpokeResult::Ok(default_ordered) = default_result else {
            panic!("expected ok result");
        };
        assert_eq!(
            default_ordered
                .iter()
                .map(|event| event.timeline_event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["evt_a", "evt_b"]
        );

        let custom_result = order_timeline_events_by_precedes(
            &events,
            &relations,
            Some(&OrderTimelineEventsByPrecedesOptions {
                relation_type: Some("follows".into()),
            }),
        );
        let SpokeResult::Ok(custom_ordered) = custom_result else {
            panic!("expected ok result");
        };
        assert_eq!(
            custom_ordered
                .iter()
                .map(|event| event.timeline_event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["evt_b", "evt_a"]
        );
    }
}
