//! Promote acceptance validation and apply.

use crate::result::{spoke_ok, spoke_ok_unit, spoke_reject, SpokeRejectCode, SpokeResult};
use crate::util::validate_revision_wire;
use serde_json::{json, Map, Value};
use spoke_schemas::knowledge_entry::KnowledgeEntry;
use spoke_schemas::promote_request::PromoteRequest;

const TERMINAL_KNOWLEDGE_ENTRY_STATUSES: &[&str] = &["merged", "deleted"];

fn validate_revision(candidate_wire: &Value) -> SpokeResult<()> {
    validate_revision_wire(candidate_wire.get("revision"))
}

fn validate_knowledge_entry_shape(candidate: &KnowledgeEntry) -> SpokeResult<()> {
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

    if TERMINAL_KNOWLEDGE_ENTRY_STATUSES.contains(&candidate.status.as_str()) {
        let mut details = Map::new();
        details.insert("status".into(), Value::String(candidate.status.clone()));
        return spoke_reject(
            SpokeRejectCode::CandidateTerminalStatus,
            format!("Candidate KnowledgeEntry has terminal status: {}", candidate.status),
            Some(details),
        );
    }

    if candidate.status != "provisional" {
        let mut details = Map::new();
        details.insert("status".into(), Value::String(candidate.status.clone()));
        return spoke_reject(
            SpokeRejectCode::CandidateNotProvisional,
            format!(
                "Candidate KnowledgeEntry status must be provisional (got {})",
                candidate.status
            ),
            Some(details),
        );
    }

    spoke_ok_unit()
}

fn request_target_equals_candidate(
    candidate: &KnowledgeEntry,
    target_id: Option<&str>,
) -> bool {
    target_id.is_some_and(|target| target == candidate.entry_id)
}

fn validate_promote_lifecycle(request: &PromoteRequest) -> SpokeResult<()> {
    let candidate_wire = match serde_json::to_value(&request.candidate) {
        Ok(wire) => wire,
        Err(_) => {
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                "PromoteRequest candidate is not serializable",
                None,
            );
        }
    };

    if let SpokeResult::Reject(reject) = validate_revision(&candidate_wire) {
        return SpokeResult::Reject(reject);
    }

    let candidate = match promote_candidate_as_data(&request.candidate) {
        SpokeResult::Ok(candidate) => candidate,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };

    let shape_result = validate_knowledge_entry_shape(&candidate);
    if shape_result.is_reject() {
        return shape_result;
    }

    if request_target_equals_candidate(&candidate, request.target_entry_id.as_deref()) {
        let mut details = Map::new();
        details.insert("entry_id".into(), Value::String(candidate.entry_id.clone()));
        if let Some(target) = &request.target_entry_id {
            details.insert("target_entry_id".into(), Value::String(target.clone()));
        }
        return spoke_reject(
            SpokeRejectCode::MergeTargetSelf,
            "target_entry_id must not equal candidate.entry_id",
            Some(details),
        );
    }

    spoke_ok_unit()
}

fn next_revision(candidate: &KnowledgeEntry) -> SpokeResult<u64> {
    match candidate.revision {
        None => spoke_ok(1),
        Some(revision) if revision == u64::MAX => {
            let mut details = Map::new();
            details.insert("revision".into(), json!(revision));
            spoke_reject(
                SpokeRejectCode::InvalidInput,
                "KnowledgeEntry revision cannot be incremented past u64::MAX",
                Some(details),
            )
        }
        Some(revision) => spoke_ok(revision + 1),
    }
}

fn promote_candidate_as_data(
    candidate: &spoke_schemas::promote_request::KnowledgeEntry,
) -> SpokeResult<KnowledgeEntry> {
    let wire = match serde_json::to_value(candidate) {
        Ok(wire) => wire,
        Err(_) => {
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                "PromoteRequest candidate is not serializable",
                None,
            );
        }
    };

    if let SpokeResult::Reject(reject) = validate_revision(&wire) {
        return SpokeResult::Reject(reject);
    }

    match serde_json::from_value::<KnowledgeEntry>(wire) {
        Ok(entry) => spoke_ok(entry),
        Err(error) => spoke_reject(
            SpokeRejectCode::InvalidInput,
            format!("Invalid promote candidate KnowledgeEntry: {error}"),
            None,
        ),
    }
}

/// Validate promote request wire JSON before typed deserialize (maps revision/shape errors to rejects).
pub fn validate_promote_request_wire(wire: &Value) -> SpokeResult<()> {
    let candidate_wire = match wire.get("candidate") {
        Some(candidate) => candidate,
        None => {
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                "PromoteRequest candidate is required",
                None,
            );
        }
    };

    if let SpokeResult::Reject(reject) = validate_revision(candidate_wire) {
        return SpokeResult::Reject(reject);
    }

    let request: PromoteRequest = match serde_json::from_value(wire.clone()) {
        Ok(request) => request,
        Err(error) => {
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                format!("Invalid PromoteRequest wire JSON: {error}"),
                None,
            );
        }
    };

    validate_promote_lifecycle(&request)
}

/// Validate promote request shape and lifecycle rules.
pub fn validate_promote_request(request: &PromoteRequest) -> SpokeResult<()> {
    validate_promote_lifecycle(request)
}

/// Return promoted KnowledgeEntry view (`status: confirmed`, revision bumped); does not persist.
pub fn apply_promote_acceptance(request: &PromoteRequest) -> SpokeResult<KnowledgeEntry> {
    if let SpokeResult::Reject(reject) = validate_promote_lifecycle(request) {
        return SpokeResult::Reject(reject);
    }

    let mut promoted = match promote_candidate_as_data(&request.candidate) {
        SpokeResult::Ok(candidate) => candidate,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };
    promoted.status = "confirmed".into();
    let next = match next_revision(&promoted) {
        SpokeResult::Ok(revision) => revision,
        SpokeResult::Reject(reject) => return SpokeResult::Reject(reject),
    };
    promoted.revision = Some(next);
    spoke_ok(promoted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use spoke_schemas::knowledge_entry::{KnowledgeEntryBody, KnowledgeEntryCanonicalName};
    use std::collections::HashMap;
    use std::num::NonZeroU64;

    fn make_candidate(overrides: impl FnOnce(&mut KnowledgeEntry)) -> KnowledgeEntry {
        let mut candidate = KnowledgeEntry {
            body: KnowledgeEntryBody::default(),
            canonical_name: KnowledgeEntryCanonicalName::try_from("Mira Vale".to_owned()).unwrap(),
            created_at: None,
            entry_id: "kb_1".into(),
            entry_type: "character".into(),
            extensions: HashMap::new(),
            modules: HashMap::new(),
            revision: None,
            schema_version: NonZeroU64::new(1).unwrap(),
            source_anchor: None,
            status: "provisional".into(),
            updated_at: None,
        };
        overrides(&mut candidate);
        candidate
    }

    fn make_request(overrides: impl FnOnce(&mut PromoteRequest)) -> PromoteRequest {
        let candidate = make_candidate(|_| {});
        let candidate = serde_json::from_value(serde_json::to_value(candidate).unwrap()).unwrap();
        let mut request = PromoteRequest {
            candidate,
            extensions: HashMap::new(),
            target_entry_id: None,
        };
        overrides(&mut request);
        request
    }

    fn candidate_wire(overrides: impl FnOnce(&mut Map<String, Value>)) -> Value {
        let mut candidate = Map::from_iter([
            ("schema_version".into(), json!(1)),
            ("entry_id".into(), json!("kb_1")),
            ("entry_type".into(), json!("character")),
            ("canonical_name".into(), json!("Mira Vale")),
            ("status".into(), json!("provisional")),
            ("body".into(), json!({})),
            ("extensions".into(), json!({})),
        ]);
        overrides(&mut candidate);
        json!({
            "candidate": Value::Object(candidate),
            "extensions": {},
        })
    }

    #[test]
    fn accepts_valid_provisional_candidate() {
        assert!(validate_promote_request(&make_request(|_| {})).is_ok());
    }

    #[test]
    fn rejects_deleted_candidate() {
        let result = validate_promote_request(&make_request(|request| {
            request.candidate.status = "deleted".into();
        }));
        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::CandidateTerminalStatus);
        }
    }

    #[test]
    fn rejects_merged_candidate() {
        let result = validate_promote_request(&make_request(|request| {
            request.candidate.status = "merged".into();
        }));
        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::CandidateTerminalStatus);
        }
    }

    #[test]
    fn rejects_empty_canonical_name() {
        let result = validate_promote_request(&make_request(|request| {
            request.candidate.canonical_name =
                spoke_schemas::promote_request::KnowledgeEntryCanonicalName::try_from(
                    "   ".to_owned(),
                )
                .unwrap();
        }));
        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::EmptyCanonicalName);
        }
    }

    #[test]
    fn rejects_merge_target_equal_to_candidate() {
        let result = validate_promote_request(&make_request(|request| {
            request.target_entry_id = Some("kb_1".into());
        }));
        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::MergeTargetSelf);
        }
    }

    #[test]
    fn rejects_non_provisional_candidate() {
        let result = validate_promote_request(&make_request(|request| {
            request.candidate.status = "confirmed".into();
        }));
        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::CandidateNotProvisional);
        }
    }

    #[test]
    fn rejects_empty_entry_type() {
        let result = validate_promote_request(&make_request(|request| {
            request.candidate.entry_type.clear();
        }));
        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::MissingRequiredField);
        }
    }

    #[test]
    fn rejects_string_revision_f001() {
        let wire = candidate_wire(|candidate| {
            candidate.insert("revision".into(), json!("2"));
        });
        let result = validate_promote_request_wire(&wire);
        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        }
    }

    #[test]
    fn rejects_negative_revision_f003() {
        let wire = candidate_wire(|candidate| {
            candidate.insert("revision".into(), json!(-1));
        });
        let result = validate_promote_request_wire(&wire);
        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        }
    }

    #[test]
    fn promotes_provisional_candidate_to_confirmed() {
        let result = apply_promote_acceptance(&make_request(|_| {}));
        assert!(result.is_ok());
        if let SpokeResult::Ok(value) = result {
            assert_eq!(value.status, "confirmed");
            assert_eq!(value.canonical_name.as_str(), "Mira Vale");
        }
    }

    #[test]
    fn bumps_revision_from_none_to_one() {
        let result = apply_promote_acceptance(&make_request(|request| {
            request.candidate.revision = None;
        }));
        assert!(result.is_ok());
        if let SpokeResult::Ok(value) = result {
            assert_eq!(value.revision, Some(1));
        }
    }

    #[test]
    fn bumps_revision_from_two_to_three() {
        let result = apply_promote_acceptance(&make_request(|request| {
            request.candidate.revision = Some(2);
        }));
        assert!(result.is_ok());
        if let SpokeResult::Ok(value) = result {
            assert_eq!(value.revision, Some(3));
        }
    }

    #[test]
    fn rejects_promote_when_revision_is_max() {
        let result = apply_promote_acceptance(&make_request(|request| {
            request.candidate.revision = Some(u64::MAX);
        }));

        assert!(result.is_reject());
        if let SpokeResult::Reject(reject) = result {
            assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        }
    }

    #[test]
    fn does_not_set_updated_at() {
        let result = apply_promote_acceptance(&make_request(|_| {}));
        assert!(result.is_ok());
        if let SpokeResult::Ok(value) = result {
            assert!(value.updated_at.is_none());
        }
    }
}
