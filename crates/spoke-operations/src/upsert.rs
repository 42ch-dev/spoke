//! KnowledgeEntry upsert validation helpers.

use crate::occ::assert_revision_match;
use crate::result::{spoke_ok_unit, spoke_reject, SpokeRejectCode, SpokeResult};
use serde_json::{json, Map, Value};
use spoke_schemas::knowledge_entry::KnowledgeEntry;

const TERMINAL_KNOWLEDGE_ENTRY_STATUSES: &[&str] = &["merged", "deleted"];

/// Explicit upsert path when caller supplies stored presence separately from inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertMode {
    Create,
    Update,
}

/// Context for [`validate_upsert_knowledge_entry`].
pub struct ValidateUpsertKnowledgeEntryContext<'a> {
    pub stored: Option<&'a KnowledgeEntry>,
    pub mode: Option<UpsertMode>,
}

impl<'a> ValidateUpsertKnowledgeEntryContext<'a> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            stored: None,
            mode: None,
        }
    }
}

impl Default for ValidateUpsertKnowledgeEntryContext<'_> {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_required_knowledge_entry_fields(candidate: &KnowledgeEntry) -> SpokeResult<()> {
    if candidate.schema_version.get() < 1 {
        let mut details = Map::new();
        details.insert(
            "schema_version".into(),
            json!(candidate.schema_version.get()),
        );
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            "KnowledgeEntry schema_version must be an integer >= 1",
            Some(details),
        );
    }

    if candidate.entry_id.is_empty() {
        let mut details = Map::new();
        details.insert("field".into(), Value::String("entry_id".into()));
        return spoke_reject(
            SpokeRejectCode::MissingRequiredField,
            "KnowledgeEntry entry_id must be a non-empty string",
            Some(details),
        );
    }

    if candidate.entry_type.is_empty() {
        let mut details = Map::new();
        details.insert("field".into(), Value::String("entry_type".into()));
        return spoke_reject(
            SpokeRejectCode::MissingRequiredField,
            "KnowledgeEntry entry_type must be a non-empty string",
            Some(details),
        );
    }

    if candidate.canonical_name.as_str().trim().is_empty() {
        return spoke_reject(
            SpokeRejectCode::EmptyCanonicalName,
            "KnowledgeEntry canonical_name must be non-empty",
            None,
        );
    }

    spoke_ok_unit()
}

fn validate_create_revision(candidate: &KnowledgeEntry) -> SpokeResult<()> {
    match candidate.revision {
        None | Some(0) => spoke_ok_unit(),
        Some(revision) if revision >= 1 => {
            let mut details = Map::new();
            details.insert("revision".into(), json!(revision));
            spoke_reject(
                SpokeRejectCode::InvalidInput,
                "KnowledgeEntry revision must be absent, undefined, or 0 on create",
                Some(details),
            )
        }
        Some(revision) => {
            let mut details = Map::new();
            details.insert("revision".into(), json!(revision));
            spoke_reject(
                SpokeRejectCode::InvalidInput,
                "KnowledgeEntry revision must be a non-negative integer, 0, or omitted on create",
                Some(details),
            )
        }
    }
}

fn validate_create_path(candidate: &KnowledgeEntry) -> SpokeResult<()> {
    let required = validate_required_knowledge_entry_fields(candidate);
    if required.is_reject() {
        return required;
    }

    validate_create_revision(candidate)
}

fn validate_update_path(candidate: &KnowledgeEntry, stored: &KnowledgeEntry) -> SpokeResult<()> {
    let required = validate_required_knowledge_entry_fields(candidate);
    if required.is_reject() {
        return required;
    }

    if candidate.entry_id != stored.entry_id {
        let mut details = Map::new();
        details.insert("candidate_entry_id".into(), json!(candidate.entry_id));
        details.insert("stored_entry_id".into(), json!(stored.entry_id));
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            "Candidate entry_id must match stored entry_id on update",
            Some(details),
        );
    }

    if TERMINAL_KNOWLEDGE_ENTRY_STATUSES.contains(&stored.status.as_str()) {
        let mut details = Map::new();
        details.insert("status".into(), Value::String(stored.status.clone()));
        return spoke_reject(
            SpokeRejectCode::KnowledgeEntryTerminalStatus,
            format!("Stored KnowledgeEntry has terminal status: {}", stored.status),
            Some(details),
        );
    }

    let Some(revision) = candidate.revision else {
        let mut details = Map::new();
        details.insert("field".into(), Value::String("revision".into()));
        return spoke_reject(
            SpokeRejectCode::MissingRequiredField,
            "Candidate revision is required on update",
            Some(details),
        );
    };

    assert_revision_match(revision, stored.revision.unwrap_or(0))
}

/// Validate KnowledgeEntry upsert before persist; create vs update inferred from stored presence.
pub fn validate_upsert_knowledge_entry(
    candidate: &KnowledgeEntry,
    context: ValidateUpsertKnowledgeEntryContext<'_>,
) -> SpokeResult<()> {
    let ValidateUpsertKnowledgeEntryContext { stored, mode } = context;

    if mode == Some(UpsertMode::Update) && stored.is_none() {
        let mut details = Map::new();
        details.insert("entry_id".into(), json!(candidate.entry_id));
        return spoke_reject(
            SpokeRejectCode::KnowledgeEntryNotFound,
            "Update path requires a stored KnowledgeEntry",
            Some(details),
        );
    }

    if mode == Some(UpsertMode::Create) && stored.is_some() {
        let stored = stored.expect("checked above");
        let mut details = Map::new();
        details.insert("entry_id".into(), json!(stored.entry_id));
        return spoke_reject(
            SpokeRejectCode::KnowledgeEntryAlreadyExists,
            "Create path must not include a stored KnowledgeEntry",
            Some(details),
        );
    }

    if let Some(stored) = stored {
        validate_update_path(candidate, stored)
    } else {
        validate_create_path(candidate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result::SpokeResult;
    use spoke_schemas::knowledge_entry::{KnowledgeEntryBody, KnowledgeEntryCanonicalName};
    use std::collections::HashMap;
    use std::num::NonZeroU64;

    fn make_knowledge_entry(overrides: impl FnOnce(&mut KnowledgeEntry)) -> KnowledgeEntry {
        let mut entry = KnowledgeEntry {
            body: KnowledgeEntryBody::default(),
            canonical_name: KnowledgeEntryCanonicalName::try_from("Mira Vale".to_owned()).unwrap(),
            created_at: None,
            entry_id: "kb_new".into(),
            entry_type: "character".into(),
            extensions: HashMap::new(),
            modules: HashMap::new(),
            revision: None,
            schema_version: NonZeroU64::new(1).unwrap(),
            source_anchor: None,
            status: "provisional".into(),
            updated_at: None,
        };
        overrides(&mut entry);
        entry
    }

    #[test]
    fn accepts_valid_create_without_revision() {
        let candidate = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_new".into();
        });

        assert!(validate_upsert_knowledge_entry(
            &candidate,
            ValidateUpsertKnowledgeEntryContext::default()
        )
        .is_ok());
    }

    #[test]
    fn accepts_valid_create_with_revision_zero() {
        let candidate = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_new".into();
            entry.revision = Some(0);
        });

        assert!(validate_upsert_knowledge_entry(
            &candidate,
            ValidateUpsertKnowledgeEntryContext::default()
        )
        .is_ok());
    }

    #[test]
    fn rejects_create_with_revision_at_least_one() {
        let candidate = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_new".into();
            entry.revision = Some(1);
        });

        let result = validate_upsert_knowledge_entry(
            &candidate,
            ValidateUpsertKnowledgeEntryContext::default(),
        );

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        }
    }

    #[test]
    fn rejects_create_with_whitespace_only_canonical_name() {
        let candidate = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_new".into();
            entry.canonical_name =
                KnowledgeEntryCanonicalName::try_from("   ".to_owned()).unwrap();
        });

        let result = validate_upsert_knowledge_entry(
            &candidate,
            ValidateUpsertKnowledgeEntryContext::default(),
        );

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::EmptyCanonicalName);
        }
    }

    #[test]
    fn accepts_valid_update_with_matching_revision() {
        let stored = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_1".into();
            entry.revision = Some(2);
            entry.status = "confirmed".into();
        });
        let candidate = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_1".into();
            entry.revision = Some(2);
            entry.status = "confirmed".into();
        });

        assert!(validate_upsert_knowledge_entry(
            &candidate,
            ValidateUpsertKnowledgeEntryContext {
                stored: Some(&stored),
                mode: None,
            }
        )
        .is_ok());
    }

    #[test]
    fn rejects_update_without_revision() {
        let stored = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_1".into();
            entry.revision = Some(1);
        });
        let candidate = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_1".into();
            entry.revision = None;
        });

        let result = validate_upsert_knowledge_entry(
            &candidate,
            ValidateUpsertKnowledgeEntryContext {
                stored: Some(&stored),
                mode: None,
            },
        );

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::MissingRequiredField);
        }
    }

    #[test]
    fn rejects_update_when_stored_revision_is_stale() {
        let stored = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_1".into();
            entry.revision = Some(3);
        });
        let candidate = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_1".into();
            entry.revision = Some(2);
        });

        let result = validate_upsert_knowledge_entry(
            &candidate,
            ValidateUpsertKnowledgeEntryContext {
                stored: Some(&stored),
                mode: None,
            },
        );

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::StoredRevisionStale);
        }
    }

    #[test]
    fn rejects_update_when_stored_has_terminal_status() {
        let stored = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_1".into();
            entry.revision = Some(1);
            entry.status = "merged".into();
        });
        let candidate = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_1".into();
            entry.revision = Some(1);
            entry.status = "merged".into();
        });

        let result = validate_upsert_knowledge_entry(
            &candidate,
            ValidateUpsertKnowledgeEntryContext {
                stored: Some(&stored),
                mode: None,
            },
        );

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::KnowledgeEntryTerminalStatus);
        }
    }

    #[test]
    fn rejects_update_path_without_stored_via_explicit_mode() {
        let candidate = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_1".into();
            entry.revision = Some(1);
        });

        let result = validate_upsert_knowledge_entry(
            &candidate,
            ValidateUpsertKnowledgeEntryContext {
                stored: None,
                mode: Some(UpsertMode::Update),
            },
        );

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::KnowledgeEntryNotFound);
        }
    }

    #[test]
    fn rejects_create_path_when_stored_is_provided_via_explicit_mode() {
        let stored = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_1".into();
            entry.revision = Some(0);
        });
        let candidate = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_1".into();
        });

        let result = validate_upsert_knowledge_entry(
            &candidate,
            ValidateUpsertKnowledgeEntryContext {
                stored: Some(&stored),
                mode: Some(UpsertMode::Create),
            },
        );

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::KnowledgeEntryAlreadyExists);
        }
    }

    #[test]
    fn rejects_entry_id_mismatch_on_update() {
        let stored = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_1".into();
            entry.revision = Some(1);
        });
        let candidate = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_2".into();
            entry.revision = Some(1);
        });

        let result = validate_upsert_knowledge_entry(
            &candidate,
            ValidateUpsertKnowledgeEntryContext {
                stored: Some(&stored),
                mode: None,
            },
        );

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        }
    }

    #[test]
    fn rejects_update_when_candidate_has_empty_entry_type() {
        let stored = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_1".into();
            entry.revision = Some(1);
        });
        let candidate = make_knowledge_entry(|entry| {
            entry.entry_id = "kb_1".into();
            entry.revision = Some(1);
            entry.entry_type = String::new();
        });

        let result = validate_upsert_knowledge_entry(
            &candidate,
            ValidateUpsertKnowledgeEntryContext {
                stored: Some(&stored),
                mode: None,
            },
        );

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::MissingRequiredField);
        }
    }
}
