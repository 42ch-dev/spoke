//! Relation validation helpers.

use crate::result::{spoke_ok_unit, spoke_reject, SpokeRejectCode, SpokeResult};
use serde_json::{json, Map, Value};
use spoke_schemas::Relation;

fn is_non_empty_trimmed_string(value: &str) -> bool {
    !value.trim().is_empty()
}

/// Validate Relation shape and lifecycle rules before persist.
pub fn validate_relate_request(relation: &Relation) -> SpokeResult<()> {
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

    spoke_ok_unit()
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
        assert!(validate_relate_request(&make_relation(|_| {})).is_ok());
    }

    #[test]
    fn rejects_self_edge() {
        let result = validate_relate_request(&make_relation(|relation| {
            relation.from_id = "kb_1".into();
            relation.to_id = "kb_1".into();
        }));

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::RelationSelfEdge);
        }
    }

    #[test]
    fn rejects_self_edge_when_ids_differ_only_by_surrounding_whitespace() {
        let result = validate_relate_request(&make_relation(|relation| {
            relation.from_id = "kb_1".into();
            relation.to_id = "kb_1 ".into();
        }));

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::RelationSelfEdge);
        }
    }

    #[test]
    fn rejects_missing_from_id() {
        let result = validate_relate_request(&make_relation(|relation| {
            relation.from_id = "   ".into();
        }));

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::RelationMissingEndpoint);
        }
    }

    #[test]
    fn rejects_missing_to_id() {
        let result = validate_relate_request(&make_relation(|relation| {
            relation.to_id = String::new();
        }));

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::RelationMissingEndpoint);
        }
    }
}
