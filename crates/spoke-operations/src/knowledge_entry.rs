//! KnowledgeEntry status transition, active-uniqueness, and ownership helpers.

use crate::result::{spoke_ok, spoke_ok_unit, spoke_reject, SpokeRejectCode, SpokeResult};
use serde_json::{json, Map, Value};
use spoke_schemas::knowledge_entry::KnowledgeEntry;
use std::collections::{HashMap, HashSet};

const CORE_KNOWLEDGE_ENTRY_STATUSES: &[&str] = &[
    "provisional",
    "confirmed",
    "deprecated",
    "merged",
    "deleted",
];

const ACTIVE_KNOWLEDGE_ENTRY_STATUSES: &[&str] = &["provisional", "confirmed"];

fn allowed_transitions() -> HashMap<&'static str, HashSet<&'static str>> {
    HashMap::from([
        (
            "provisional",
            HashSet::from([
                "confirmed",
                "deprecated",
                "merged",
                "deleted",
                "provisional",
            ]),
        ),
        (
            "confirmed",
            HashSet::from(["deprecated", "merged", "deleted", "confirmed"]),
        ),
        (
            "deprecated",
            HashSet::from(["confirmed", "deleted", "deprecated"]),
        ),
        ("merged", HashSet::from(["merged"])),
        ("deleted", HashSet::from(["deleted"])),
    ])
}

fn is_core_knowledge_entry_status(status: &str) -> bool {
    CORE_KNOWLEDGE_ENTRY_STATUSES.contains(&status)
}

fn is_active_knowledge_entry_status(status: &str) -> bool {
    ACTIVE_KNOWLEDGE_ENTRY_STATUSES.contains(&status)
}

/// Returns whether a KnowledgeEntry status transition is allowed by the cross-product table.
#[must_use]
pub fn is_valid_knowledge_entry_status_transition(from: &str, to: &str) -> bool {
    if !is_core_knowledge_entry_status(from) || !is_core_knowledge_entry_status(to) {
        return false;
    }

    allowed_transitions()
        .get(from)
        .is_some_and(|targets| targets.contains(to))
}

/// Apply a KnowledgeEntry status transition; returns updated status on success without mutating input.
pub fn transition_knowledge_entry_status(
    knowledge_entry: &KnowledgeEntry,
    to: &str,
) -> SpokeResult<KnowledgeEntry> {
    if !is_core_knowledge_entry_status(to) {
        let mut details = Map::new();
        details.insert("status".into(), Value::String(to.to_owned()));
        return spoke_reject(
            SpokeRejectCode::InvalidKnowledgeEntryStatus,
            format!("Invalid knowledge entry status: {to}"),
            Some(details),
        );
    }

    if !is_core_knowledge_entry_status(&knowledge_entry.status) {
        let mut details = Map::new();
        details.insert(
            "status".into(),
            Value::String(knowledge_entry.status.clone()),
        );
        return spoke_reject(
            SpokeRejectCode::InvalidKnowledgeEntryStatus,
            format!(
                "Invalid current knowledge entry status: {}",
                knowledge_entry.status
            ),
            Some(details),
        );
    }

    if !is_valid_knowledge_entry_status_transition(&knowledge_entry.status, to) {
        let mut details = Map::new();
        details.insert("from".into(), Value::String(knowledge_entry.status.clone()));
        details.insert("to".into(), Value::String(to.to_owned()));
        return spoke_reject(
            SpokeRejectCode::InvalidKnowledgeEntryStatusTransition,
            format!(
                "Disallowed knowledge entry status transition: {} -> {to}",
                knowledge_entry.status
            ),
            Some(details),
        );
    }

    let mut updated = knowledge_entry.clone();
    updated.status = to.to_owned();
    spoke_ok(updated)
}

/// Input for [`assert_unique_active_knowledge_entry`].
pub struct AssertUniqueActiveKnowledgeEntryInput<'a> {
    pub scope_key: &'a str,
    pub entry_type: &'a str,
    pub canonical_name: &'a str,
    pub candidate: &'a KnowledgeEntry,
    pub existing: &'a [KnowledgeEntry],
}

/// Reject duplicate active triple among caller-supplied KnowledgeEntries; scope_key is opaque.
pub fn assert_unique_active_knowledge_entry(
    input: AssertUniqueActiveKnowledgeEntryInput<'_>,
) -> SpokeResult<()> {
    if input.candidate.entry_type != input.entry_type
        || input.candidate.canonical_name.as_str() != input.canonical_name
    {
        let mut details = Map::new();
        details.insert("entry_type".into(), json!(input.entry_type));
        details.insert("canonical_name".into(), json!(input.canonical_name));
        details.insert(
            "candidate_entry_type".into(),
            json!(input.candidate.entry_type),
        );
        details.insert(
            "candidate_canonical_name".into(),
            json!(input.candidate.canonical_name.as_str()),
        );
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            "entry_type and canonical_name must match candidate wire fields",
            Some(details),
        );
    }

    if !is_active_knowledge_entry_status(&input.candidate.status) {
        return spoke_ok_unit();
    }

    for knowledge_entry in input.existing {
        if !is_active_knowledge_entry_status(&knowledge_entry.status) {
            continue;
        }
        if knowledge_entry.entry_type != input.entry_type {
            continue;
        }
        if knowledge_entry.canonical_name.as_str() != input.canonical_name {
            continue;
        }
        if knowledge_entry.entry_id == input.candidate.entry_id {
            continue;
        }

        let mut details = Map::new();
        details.insert("scope_key".into(), json!(input.scope_key));
        details.insert("entry_type".into(), json!(input.entry_type));
        details.insert("canonical_name".into(), json!(input.canonical_name));
        details.insert(
            "conflicting_entry_id".into(),
            json!(knowledge_entry.entry_id),
        );
        return spoke_reject(
            SpokeRejectCode::DuplicateActiveKnowledgeEntry,
            format!(
                "Duplicate active knowledge entry for ({}, {}, {})",
                input.scope_key, input.entry_type, input.canonical_name
            ),
            Some(details),
        );
    }

    spoke_ok_unit()
}

/// Sole core disclosure value: the entry is visible only to its own holder.
///
/// Additional disclosure strings are Domain Profile vocabulary and stay open.
const OWNER_PRIVATE_DISCLOSURE: &str = "owner-private";

/// Read the holder entry id governing a KnowledgeEntry.
///
/// `None` means ownership is unspecified — not a reserved world owner and not world
/// consensus. The identifier is borrowed from the entry; no lookup or normalization occurs.
#[must_use]
pub fn get_knowledge_entry_owner(knowledge_entry: &KnowledgeEntry) -> Option<&str> {
    knowledge_entry.owner.as_deref().map(String::as_str)
}

/// Core disclosure predicate for a reader viewpoint.
///
/// Absent disclosure is shared within the caller's already pre-scoped KB; `owner-private`
/// is visible only to a viewpoint that exactly equals the owner; unknown open-vocabulary
/// values and a private entry without an owner are excluded. Comparison is exact —
/// identifiers are not normalized, and no holder lookup or audience expansion occurs.
#[must_use]
pub fn knowledge_entry_visible_to_viewpoint(
    knowledge_entry: &KnowledgeEntry,
    viewpoint: Option<&str>,
) -> bool {
    let Some(disclosure) = knowledge_entry.disclosure.as_deref() else {
        return true;
    };

    if disclosure.as_str() != OWNER_PRIVATE_DISCLOSURE {
        return false;
    }

    let (Some(owner), Some(viewpoint)) = (get_knowledge_entry_owner(knowledge_entry), viewpoint)
    else {
        return false;
    };

    owner == viewpoint
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result::SpokeResult;
    use spoke_schemas::knowledge_entry::{
        KnowledgeEntryBody, KnowledgeEntryCanonicalName, KnowledgeEntryDisclosure,
        KnowledgeEntryOwner,
    };
    use std::collections::HashMap;
    use std::num::NonZeroU64;

    fn make_knowledge_entry(status: &str, overrides: impl FnOnce(&mut KnowledgeEntry)) -> KnowledgeEntry {
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
            status: status.into(),
            updated_at: None,
        };
        overrides(&mut entry);
        entry
    }

    #[test]
    fn allows_documented_status_transitions() {
        for (from, to) in [
            ("provisional", "confirmed"),
            ("provisional", "deprecated"),
            ("provisional", "merged"),
            ("provisional", "deleted"),
            ("confirmed", "deprecated"),
            ("confirmed", "merged"),
            ("confirmed", "deleted"),
            ("deprecated", "confirmed"),
            ("deprecated", "deleted"),
            ("provisional", "provisional"),
            ("confirmed", "confirmed"),
            ("deprecated", "deprecated"),
            ("merged", "merged"),
            ("deleted", "deleted"),
        ] {
            assert!(is_valid_knowledge_entry_status_transition(from, to));
        }
    }

    #[test]
    fn rejects_disallowed_status_transitions() {
        for (from, to) in [
            ("deprecated", "merged"),
            ("merged", "confirmed"),
            ("deleted", "provisional"),
            ("confirmed", "provisional"),
            ("merged", "deleted"),
            ("deleted", "confirmed"),
            ("provisional", "bogus"),
            ("bogus", "confirmed"),
        ] {
            assert!(!is_valid_knowledge_entry_status_transition(from, to));
        }
    }

    #[test]
    fn updates_status_on_allowed_transition_without_mutating_input() {
        let knowledge_entry = make_knowledge_entry("provisional", |_| {});
        let result = transition_knowledge_entry_status(&knowledge_entry, "confirmed");

        assert!(result.is_ok());
        if let SpokeResult::Ok(value) = result {
            assert_eq!(value.status, "confirmed");
            assert_eq!(knowledge_entry.status, "provisional");
        }
    }

    #[test]
    fn accepts_no_op_same_status_transition() {
        let knowledge_entry = make_knowledge_entry("merged", |_| {});
        let result = transition_knowledge_entry_status(&knowledge_entry, "merged");

        assert!(result.is_ok());
        if let SpokeResult::Ok(value) = result {
            assert_eq!(value.status, "merged");
        }
    }

    #[test]
    fn rejects_invalid_target_status() {
        let result = transition_knowledge_entry_status(&make_knowledge_entry("provisional", |_| {}), "bogus");

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidKnowledgeEntryStatus);
            assert_eq!(
                reject.details,
                Some(Map::from_iter([("status".into(), json!("bogus"))]))
            );
        }
    }

    #[test]
    fn rejects_invalid_current_status() {
        let result = transition_knowledge_entry_status(&make_knowledge_entry("bogus", |_| {}), "confirmed");

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidKnowledgeEntryStatus);
            assert_eq!(
                reject.details,
                Some(Map::from_iter([("status".into(), json!("bogus"))]))
            );
        }
    }

    #[test]
    fn rejects_disallowed_transition_with_from_to_details() {
        let result = transition_knowledge_entry_status(&make_knowledge_entry("deprecated", |_| {}), "merged");

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(
                reject.code,
                SpokeRejectCode::InvalidKnowledgeEntryStatusTransition
            );
            assert_eq!(
                reject.details,
                Some(Map::from_iter([
                    ("from".into(), json!("deprecated")),
                    ("to".into(), json!("merged")),
                ]))
            );
        }
    }

    #[test]
    fn rejects_terminal_outbound_transition() {
        let result =
            transition_knowledge_entry_status(&make_knowledge_entry("deleted", |_| {}), "provisional");

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(
                reject.code,
                SpokeRejectCode::InvalidKnowledgeEntryStatusTransition
            );
            assert_eq!(
                reject.details,
                Some(Map::from_iter([
                    ("from".into(), json!("deleted")),
                    ("to".into(), json!("provisional")),
                ]))
            );
        }
    }

    const BASE_INPUT: (&str, &str, &str) = ("world_1", "character", "Mira Vale");

    #[test]
    fn accepts_when_no_conflicting_active_knowledge_entry_exists() {
        let candidate = make_knowledge_entry("provisional", |entry| {
            entry.entry_id = "kb_new".into();
        });
        let existing = [make_knowledge_entry("confirmed", |entry| {
            entry.entry_id = "kb_other".into();
            entry.entry_type = "location".into();
            entry.canonical_name =
                KnowledgeEntryCanonicalName::try_from("Harbor".to_owned()).unwrap();
        })];

        let result = assert_unique_active_knowledge_entry(AssertUniqueActiveKnowledgeEntryInput {
            scope_key: BASE_INPUT.0,
            entry_type: BASE_INPUT.1,
            canonical_name: BASE_INPUT.2,
            candidate: &candidate,
            existing: &existing,
        });

        assert!(result.is_ok());
    }

    #[test]
    fn rejects_duplicate_active_triple_for_different_entry_id() {
        let candidate = make_knowledge_entry("provisional", |entry| {
            entry.entry_id = "kb_new".into();
        });
        let existing = [make_knowledge_entry("confirmed", |entry| {
            entry.entry_id = "kb_existing".into();
        })];

        let result = assert_unique_active_knowledge_entry(AssertUniqueActiveKnowledgeEntryInput {
            scope_key: BASE_INPUT.0,
            entry_type: BASE_INPUT.1,
            canonical_name: BASE_INPUT.2,
            candidate: &candidate,
            existing: &existing,
        });

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::DuplicateActiveKnowledgeEntry);
            assert_eq!(
                reject.details,
                Some(Map::from_iter([
                    ("scope_key".into(), json!("world_1")),
                    ("entry_type".into(), json!("character")),
                    ("canonical_name".into(), json!("Mira Vale")),
                    ("conflicting_entry_id".into(), json!("kb_existing")),
                ]))
            );
        }
    }

    #[test]
    fn allows_same_entry_id_update_in_place() {
        let candidate = make_knowledge_entry("confirmed", |entry| {
            entry.entry_id = "kb_1".into();
        });
        let existing = [make_knowledge_entry("confirmed", |entry| {
            entry.entry_id = "kb_1".into();
        })];

        let result = assert_unique_active_knowledge_entry(AssertUniqueActiveKnowledgeEntryInput {
            scope_key: BASE_INPUT.0,
            entry_type: BASE_INPUT.1,
            canonical_name: BASE_INPUT.2,
            candidate: &candidate,
            existing: &existing,
        });

        assert!(result.is_ok());
    }

    #[test]
    fn ignores_inactive_existing_knowledge_entries() {
        let candidate = make_knowledge_entry("provisional", |entry| {
            entry.entry_id = "kb_new".into();
        });
        let existing = [
            make_knowledge_entry("deprecated", |entry| {
                entry.entry_id = "kb_deprecated".into();
            }),
            make_knowledge_entry("merged", |entry| {
                entry.entry_id = "kb_merged".into();
            }),
            make_knowledge_entry("deleted", |entry| {
                entry.entry_id = "kb_deleted".into();
            }),
        ];

        let result = assert_unique_active_knowledge_entry(AssertUniqueActiveKnowledgeEntryInput {
            scope_key: BASE_INPUT.0,
            entry_type: BASE_INPUT.1,
            canonical_name: BASE_INPUT.2,
            candidate: &candidate,
            existing: &existing,
        });

        assert!(result.is_ok());
    }

    #[test]
    fn passes_when_candidate_is_inactive() {
        let candidate = make_knowledge_entry("deprecated", |entry| {
            entry.entry_id = "kb_new".into();
        });
        let existing = [make_knowledge_entry("confirmed", |entry| {
            entry.entry_id = "kb_existing".into();
        })];

        let result = assert_unique_active_knowledge_entry(AssertUniqueActiveKnowledgeEntryInput {
            scope_key: BASE_INPUT.0,
            entry_type: BASE_INPUT.1,
            canonical_name: BASE_INPUT.2,
            candidate: &candidate,
            existing: &existing,
        });

        assert!(result.is_ok());
    }

    #[test]
    fn rejects_when_entry_type_or_canonical_name_do_not_match_candidate_wire_fields() {
        let candidate = make_knowledge_entry("provisional", |entry| {
            entry.entry_id = "kb_new".into();
        });

        let type_mismatch = assert_unique_active_knowledge_entry(
            AssertUniqueActiveKnowledgeEntryInput {
                scope_key: BASE_INPUT.0,
                entry_type: "location",
                canonical_name: BASE_INPUT.2,
                candidate: &candidate,
                existing: &[],
            },
        );
        assert!(type_mismatch.is_reject());
        if let SpokeResult::Reject(reject) = type_mismatch {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        }

        let name_mismatch = assert_unique_active_knowledge_entry(
            AssertUniqueActiveKnowledgeEntryInput {
                scope_key: BASE_INPUT.0,
                entry_type: BASE_INPUT.1,
                canonical_name: "Other Name",
                candidate: &candidate,
                existing: &[],
            },
        );
        assert!(name_mismatch.is_reject());
        if let SpokeResult::Reject(reject) = name_mismatch {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        }
    }

    fn owner(id: &str) -> KnowledgeEntryOwner {
        KnowledgeEntryOwner::try_from(id).expect("non-empty owner entry id")
    }

    fn disclosure(value: &str) -> KnowledgeEntryDisclosure {
        KnowledgeEntryDisclosure::try_from(value).expect("non-empty disclosure value")
    }

    #[test]
    fn ownership_returns_holder_entry_id_when_present() {
        let knowledge_entry = make_knowledge_entry("confirmed", |entry| {
            entry.owner = Some(owner("kb_mira"));
        });

        assert_eq!(get_knowledge_entry_owner(&knowledge_entry), Some("kb_mira"));
    }

    #[test]
    fn ownership_returns_none_when_unspecified() {
        let knowledge_entry = make_knowledge_entry("confirmed", |_| {});

        assert_eq!(get_knowledge_entry_owner(&knowledge_entry), None);
    }

    #[test]
    fn ownership_includes_entry_without_disclosure_for_any_viewpoint() {
        let knowledge_entry = make_knowledge_entry("confirmed", |_| {});

        assert!(knowledge_entry_visible_to_viewpoint(
            &knowledge_entry,
            Some("kb_mira")
        ));
        assert!(knowledge_entry_visible_to_viewpoint(
            &knowledge_entry,
            Some("kb_other")
        ));
        assert!(knowledge_entry_visible_to_viewpoint(&knowledge_entry, None));
    }

    #[test]
    fn ownership_includes_entry_with_owner_but_no_disclosure() {
        let knowledge_entry = make_knowledge_entry("confirmed", |entry| {
            entry.owner = Some(owner("kb_mira"));
        });

        assert!(knowledge_entry_visible_to_viewpoint(
            &knowledge_entry,
            Some("kb_other")
        ));
        assert!(knowledge_entry_visible_to_viewpoint(&knowledge_entry, None));
    }

    #[test]
    fn ownership_includes_owner_private_entry_for_its_own_viewpoint() {
        let knowledge_entry = make_knowledge_entry("confirmed", |entry| {
            entry.owner = Some(owner("kb_mira"));
            entry.disclosure = Some(disclosure("owner-private"));
        });

        assert!(knowledge_entry_visible_to_viewpoint(
            &knowledge_entry,
            Some("kb_mira")
        ));
    }

    #[test]
    fn ownership_excludes_owner_private_entry_for_foreign_viewpoint() {
        let knowledge_entry = make_knowledge_entry("confirmed", |entry| {
            entry.owner = Some(owner("kb_mira"));
            entry.disclosure = Some(disclosure("owner-private"));
        });

        assert!(!knowledge_entry_visible_to_viewpoint(
            &knowledge_entry,
            Some("kb_other")
        ));
    }

    #[test]
    fn ownership_excludes_owner_private_entry_without_viewpoint() {
        let knowledge_entry = make_knowledge_entry("confirmed", |entry| {
            entry.owner = Some(owner("kb_mira"));
            entry.disclosure = Some(disclosure("owner-private"));
        });

        assert!(!knowledge_entry_visible_to_viewpoint(&knowledge_entry, None));
    }

    #[test]
    fn ownership_excludes_owner_private_entry_without_owner() {
        let knowledge_entry = make_knowledge_entry("confirmed", |entry| {
            entry.disclosure = Some(disclosure("owner-private"));
        });

        assert!(!knowledge_entry_visible_to_viewpoint(
            &knowledge_entry,
            Some("kb_mira")
        ));
        assert!(!knowledge_entry_visible_to_viewpoint(&knowledge_entry, None));
    }

    #[test]
    fn ownership_excludes_unknown_disclosure_value_even_for_matching_owner() {
        let knowledge_entry = make_knowledge_entry("confirmed", |entry| {
            entry.owner = Some(owner("kb_mira"));
            entry.disclosure = Some(disclosure("group-private"));
        });

        assert!(!knowledge_entry_visible_to_viewpoint(
            &knowledge_entry,
            Some("kb_mira")
        ));
        assert!(!knowledge_entry_visible_to_viewpoint(&knowledge_entry, None));
    }

    #[test]
    fn ownership_compares_identifiers_exactly() {
        let knowledge_entry = make_knowledge_entry("confirmed", |entry| {
            entry.owner = Some(owner("kb_mira"));
            entry.disclosure = Some(disclosure("owner-private"));
        });

        assert!(!knowledge_entry_visible_to_viewpoint(
            &knowledge_entry,
            Some(" kb_mira")
        ));
        assert!(!knowledge_entry_visible_to_viewpoint(
            &knowledge_entry,
            Some("KB_MIRA")
        ));
    }
}
