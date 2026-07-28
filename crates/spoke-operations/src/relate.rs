//! Relation validation helpers.

use crate::occ::assert_revision_match;
use crate::result::{spoke_ok_unit, spoke_reject, SpokeRejectCode, SpokeResult};
use serde_json::{json, Map, Value};
use spoke_schemas::Relation;

fn is_non_empty_trimmed_string(value: &str) -> bool {
    !value.trim().is_empty()
}

/// Context for [`validate_relate_request`].
pub struct ValidateRelateRequestContext<'a> {
    pub stored: Option<&'a Relation>,
}

impl<'a> ValidateRelateRequestContext<'a> {
    #[must_use]
    pub const fn new() -> Self {
        Self { stored: None }
    }
}

impl Default for ValidateRelateRequestContext<'_> {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_create_revision(candidate: &Relation) -> SpokeResult<()> {
    match candidate.revision {
        None | Some(0) => spoke_ok_unit(),
        Some(revision) if revision >= 1 => {
            let mut details = Map::new();
            details.insert("revision".into(), json!(revision));
            spoke_reject(
                SpokeRejectCode::InvalidInput,
                "Relation revision must be absent, undefined, or 0 on create",
                Some(details),
            )
        }
        // u64 cannot be negative; defensive parity with upsert create-revision guard.
        Some(revision) => {
            let mut details = Map::new();
            details.insert("revision".into(), json!(revision));
            spoke_reject(
                SpokeRejectCode::InvalidInput,
                "Relation revision must be a non-negative integer, 0, or omitted on create",
                Some(details),
            )
        }
    }
}

fn validate_update_path(candidate: &Relation, stored: &Relation) -> SpokeResult<()> {
    if candidate.relation_id != stored.relation_id {
        let mut details = Map::new();
        details.insert("candidate_relation_id".into(), json!(candidate.relation_id));
        details.insert("stored_relation_id".into(), json!(stored.relation_id));
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            "Candidate relation_id must match stored relation_id on update",
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

/// Validate Relation shape and lifecycle rules before persist; create vs update
/// inferred from stored presence. Mirrors validate_upsert_knowledge_entry.
pub fn validate_relate_request(
    relation: &Relation,
    context: ValidateRelateRequestContext<'_>,
) -> SpokeResult<()> {
    if !is_non_empty_trimmed_string(&relation.from_id) {
        let mut details = Map::new();
        details.insert("field".into(), Value::String("from_id".into()));
        return spoke_reject(
            SpokeRejectCode::RelationMissingEndpoint,
            "Relation from_id must be a non-empty trimmed string",
            Some(details),
        );
    }

    if !is_non_empty_trimmed_string(&relation.to_id) {
        let mut details = Map::new();
        details.insert("field".into(), Value::String("to_id".into()));
        return spoke_reject(
            SpokeRejectCode::RelationMissingEndpoint,
            "Relation to_id must be a non-empty trimmed string",
            Some(details),
        );
    }

    let from_id = relation.from_id.trim();
    let to_id = relation.to_id.trim();

    if from_id == to_id {
        let mut details = Map::new();
        details.insert("from_id".into(), json!(relation.from_id));
        details.insert("to_id".into(), json!(relation.to_id));
        return spoke_reject(
            SpokeRejectCode::RelationSelfEdge,
            "Relation from_id must not equal to_id",
            Some(details),
        );
    }

    if let Some(stored) = context.stored {
        return validate_update_path(relation, stored);
    }

    validate_create_revision(relation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result::SpokeResult;
    use std::collections::HashMap;
    use std::num::NonZeroU64;

    fn make_relation(overrides: impl FnOnce(&mut Relation)) -> Relation {
        let mut relation = Relation {
            created_at: None,
            extensions: HashMap::new(),
            from_id: "kb_1".into(),
            label: None,
            metadata: serde_json::Map::new(),
            relation_id: "rel_1".into(),
            relation_type: "related_to".into(),
            revision: None,
            schema_version: NonZeroU64::new(1).unwrap(),
            to_id: "kb_2".into(),
            updated_at: None,
        };
        overrides(&mut relation);
        relation
    }

    #[test]
    fn accepts_valid_relation() {
        assert!(
            validate_relate_request(&make_relation(|_| {}), ValidateRelateRequestContext::default())
                .is_ok()
        );
    }

    #[test]
    fn rejects_self_edge() {
        let result = validate_relate_request(
            &make_relation(|relation| {
                relation.from_id = "kb_1".into();
                relation.to_id = "kb_1".into();
            }),
            ValidateRelateRequestContext::default(),
        );

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::RelationSelfEdge);
        }
    }

    #[test]
    fn rejects_self_edge_when_ids_differ_only_by_surrounding_whitespace() {
        let result = validate_relate_request(
            &make_relation(|relation| {
                relation.from_id = "kb_1".into();
                relation.to_id = "kb_1 ".into();
            }),
            ValidateRelateRequestContext::default(),
        );

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::RelationSelfEdge);
        }
    }

    #[test]
    fn rejects_missing_from_id() {
        let result = validate_relate_request(
            &make_relation(|relation| {
                relation.from_id = "   ".into();
            }),
            ValidateRelateRequestContext::default(),
        );

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::RelationMissingEndpoint);
        }
    }

    #[test]
    fn rejects_missing_to_id() {
        let result = validate_relate_request(
            &make_relation(|relation| {
                relation.to_id = String::new();
            }),
            ValidateRelateRequestContext::default(),
        );

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::RelationMissingEndpoint);
        }
    }
}
