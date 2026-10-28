//! Per-op request validators and pure policy predicates.
//!
//! Covers only the conditional wire rules the JSON-Schema codegen cannot
//! derive (draft-07 `if`/`then`/`else`), plus provider-limit-dependent
//! checks (`limit` vs `max_page_size`, content vs `max_content_bytes`) which
//! call sites parameterize from `memory.describe.limits` (§7.1):
//!
//! - list/recall page bounds (`limit ≥ 1` is type-enforced by
//!   `NonZeroU64`; the upper bound is provider-declared);
//! - `ReadSelection` / `ProvenanceFilter` conditional requirements (§6.12,
//!   §6.13) — non-empty `context_ids` / `source_context_ids` where the
//!   generated types allow programmatically-built empty vectors;
//! - `memory.subjects.fence` archive-on-context → `INVALID_INPUT` (§7.7);
//! - `memory.grants.grant` restricted-shape `context_ids` minItems (§7.9);
//! - `memory.candidates.review` fragment minItems (§7.15);
//! - `memory.records.update` kind-update shape must not carry content (§7.18);
//! - `memory.imports.stage` header chunk ordinal must be 0 (§7.31);
//! - content byte limits for `memory.capture` / `memory.records.create` /
//!   `memory.records.update` (§7 limits; measured by [`content_wire_bytes`]);
//! - the §8.2 freeze maintenance allowlist predicate.
//!
//! Everything else — idempotency-key presence and 1–128 shape on keyed
//! mutations (`subjects.ensure` exempt, §7.0.5), placement `context_id`
//! conditionality (§6.6), oneOf request shapes for `exports.create` /
//! `imports.stage` — is enforced by the generated types themselves.

use crate::result::{spoke_ok, spoke_reject, SpokeRejectCode, SpokeResult};
use serde_json::{Map, Value};
use spoke_schemas::{
    MemoryCandidatesListRequest, MemoryCandidatesReviewRequest, MemoryCaptureRequest,
    MemoryGrantsGrantRequest, MemoryGrantsListRequest, MemoryImportsStageRequest,
    MemoryRecallRequest, MemoryRecordsCreateRequest, MemoryRecordsListRequest,
    MemoryRecordsUpdateRequest, MemoryReflectionsGenerateRequest, MemoryReflectionsListRequest,
    MemoryReflectionsStateRequest, MemorySpacesListRequest, MemorySubjectsFenceRequest,
};
use std::num::NonZeroU64;

fn invalid_input(field: &str, message: impl Into<String>) -> SpokeResult<()> {
    let mut details = Map::new();
    details.insert("field".into(), Value::String(field.to_owned()));
    spoke_reject(SpokeRejectCode::InvalidInput, message, Some(details))
}

fn check_page_limit(limit: Option<NonZeroU64>, max_page_size: NonZeroU64) -> SpokeResult<()> {
    if limit.is_some_and(|limit| limit > max_page_size) {
        return invalid_input(
            "limit",
            format!("limit exceeds the provider max_page_size ({max_page_size})"),
        );
    }
    spoke_ok(())
}

/// Byte size of a `Content` request field against `max_content_bytes`
/// (§7.1): UTF-8 bytes of `text` plus the compact serialization of
/// `structured` when present.
#[must_use]
pub fn content_wire_bytes(text: &str, structured: Option<&Value>) -> usize {
    text.len() + structured.map_or(0, |value| serde_json::to_vec(value).map_or(0, |bytes| bytes.len()))
}

fn check_content_limit(byte_len: usize, max_content_bytes: usize) -> SpokeResult<()> {
    if byte_len > max_content_bytes {
        return invalid_input(
            "content",
            format!("content exceeds the provider max_content_bytes ({max_content_bytes})"),
        );
    }
    spoke_ok(())
}

/// §8.2 freeze maintenance allowlist: while a space is frozen it admits
/// exactly all read-only ops (MEM-R011), `memory.spaces.unfreeze`,
/// `memory.grants.revoke`, and the five transfer ops. `op` is the exact op
/// string (§13, e.g. `"memory.exports.create"`).
#[must_use]
pub fn is_freeze_allowlisted(op: &str) -> bool {
    matches!(
        op,
        // Read-only ops (MEM-R011).
        "memory.describe"
            | "memory.spaces.list"
            | "memory.subjects.get"
            | "memory.grants.list"
            | "memory.candidates.list"
            | "memory.candidates.get"
            | "memory.records.get"
            | "memory.records.list"
            | "memory.recall"
            | "memory.profile.get"
            | "memory.reflections.state"
            | "memory.reflections.list"
            | "memory.receipts.get"
            | "memory.exports.get"
            // §8.2 maintenance mutations.
            | "memory.spaces.unfreeze"
            | "memory.grants.revoke"
            | "memory.exports.create"
            | "memory.imports.stage"
            | "memory.imports.activate"
            | "memory.imports.discard"
    )
}

/// §7.2 `memory.spaces.list`: page limit vs the provider's `max_page_size`.
pub fn validate_memory_spaces_list(
    request: &MemorySpacesListRequest,
    max_page_size: NonZeroU64,
) -> SpokeResult<()> {
    check_page_limit(request.limit, max_page_size)
}

/// §7.8 `memory.grants.list`: page limit vs the provider's `max_page_size`.
pub fn validate_memory_grants_list(
    request: &MemoryGrantsListRequest,
    max_page_size: NonZeroU64,
) -> SpokeResult<()> {
    check_page_limit(request.limit, max_page_size)
}

/// §7.12 `memory.candidates.list`: page limit vs the provider's
/// `max_page_size`.
pub fn validate_memory_candidates_list(
    request: &MemoryCandidatesListRequest,
    max_page_size: NonZeroU64,
) -> SpokeResult<()> {
    check_page_limit(request.limit, max_page_size)
}

/// §7.7 `memory.subjects.fence`: `archive` on a context target is
/// `INVALID_INPUT` (context targets support `fence`/`unfence` only).
pub fn validate_memory_subjects_fence(request: &MemorySubjectsFenceRequest) -> SpokeResult<()> {
    use spoke_schemas::memory_subjects_fence_request::{Action, Target};
    match (&request.target, &request.action) {
        (Target::Context(_), Action::Archive) => {
            invalid_input("target", "archive is not valid on a context target")
        }
        _ => spoke_ok(()),
    }
}

/// §7.11 `memory.capture`: content byte limit vs the provider's
/// `max_content_bytes`. `capture_scope` conditionality (§6.6) is
/// type-enforced by `PlacementScope`.
pub fn validate_memory_capture(
    request: &MemoryCaptureRequest,
    max_content_bytes: usize,
) -> SpokeResult<()> {
    check_content_limit(
        content_wire_bytes(&request.content.text, request.content.structured.as_ref()),
        max_content_bytes,
    )
}

/// §7.9 `memory.grants.grant`: the restricted-shape `context_ids` must be
/// non-empty (minItems 1). `shared_access` without `context_ids` is
/// type-enforced (only the restricted shape carries the field).
pub fn validate_memory_grants_grant(request: &MemoryGrantsGrantRequest) -> SpokeResult<()> {
    match request {
        MemoryGrantsGrantRequest::Variant0 { context_ids, .. } if context_ids.is_empty() => {
            invalid_input("context_ids", "context_ids must be non-empty when present")
        }
        _ => spoke_ok(()),
    }
}

/// §7.15 `memory.candidates.review`: a fragment decision must name at least
/// one fragment (minItems 1). The oneOf decision shape is type-enforced.
pub fn validate_memory_candidates_review(
    request: &MemoryCandidatesReviewRequest,
) -> SpokeResult<()> {
    use spoke_schemas::memory_candidates_review_request::Decision;
    match &request.decision {
        Decision::Fragment { fragments } if fragments.is_empty() => {
            invalid_input("decision.fragments", "fragment decision requires at least one fragment")
        }
        _ => spoke_ok(()),
    }
}

/// §7.16 `memory.records.create`: content byte limit. Placement
/// conditionality is type-enforced by `PlacementScope`.
pub fn validate_memory_records_create(
    request: &MemoryRecordsCreateRequest,
    max_content_bytes: usize,
) -> SpokeResult<()> {
    check_content_limit(
        content_wire_bytes(&request.content.text, request.content.structured.as_ref()),
        max_content_bytes,
    )
}

/// §7.18 `memory.records.update`: the kind-update shape must not carry
/// content (oneOf is deliberate per shape); the content-update shape is
/// bounds-checked against `max_content_bytes`.
pub fn validate_memory_records_update(
    request: &MemoryRecordsUpdateRequest,
    max_content_bytes: usize,
) -> SpokeResult<()> {
    match request {
        MemoryRecordsUpdateRequest::Variant0 { content, .. } => check_content_limit(
            content_wire_bytes(&content.text, content.structured.as_ref()),
            max_content_bytes,
        ),
        MemoryRecordsUpdateRequest::Variant1 { content: Some(_), .. } => {
            invalid_input("content", "content is not valid on a kind-update request")
        }
        MemoryRecordsUpdateRequest::Variant1 { .. } => spoke_ok(()),
    }
}

/// §7.20 `memory.records.list`: selection/filter invariants (§6.12, §6.13)
/// plus the page limit.
pub fn validate_memory_records_list(
    request: &MemoryRecordsListRequest,
    max_page_size: NonZeroU64,
) -> SpokeResult<()> {
    use spoke_schemas::memory_records_list_request::ReadSelection;
    if let ReadSelection::SelectedContexts { context_ids, .. } = &request.selection {
        if context_ids.is_empty() {
            return invalid_input(
                "selection.context_ids",
                "selected_contexts requires at least one context_id",
            );
        }
    }
    if let Some(filter) = &request.filter {
        if filter.source_context_ids.is_empty() {
            return invalid_input(
                "filter.source_context_ids",
                "provenance filter requires at least one source context",
            );
        }
    }
    check_page_limit(request.limit, max_page_size)
}

/// §7.22 `memory.recall`: selection/filter invariants and the
/// `budget.max_items` hard cap (§9: ≥ 1 is type-enforced, ≤ `max_page_size`
/// is provider-declared).
pub fn validate_memory_recall(
    request: &MemoryRecallRequest,
    max_page_size: NonZeroU64,
) -> SpokeResult<()> {
    use spoke_schemas::memory_recall_request::ReadSelection;
    if let ReadSelection::SelectedContexts { context_ids, .. } = &request.selection {
        if context_ids.is_empty() {
            return invalid_input(
                "selection.context_ids",
                "selected_contexts requires at least one context_id",
            );
        }
    }
    if let Some(filter) = &request.filter {
        if filter.source_context_ids.is_empty() {
            return invalid_input(
                "filter.source_context_ids",
                "provenance filter requires at least one source context",
            );
        }
    }
    if request.budget.max_items > max_page_size {
        return invalid_input(
            "max_items",
            format!("budget.max_items exceeds the provider max_page_size ({max_page_size})"),
        );
    }
    spoke_ok(())
}

/// §7.25 `memory.reflections.state`: selection/filter invariants.
pub fn validate_memory_reflections_state(
    request: &MemoryReflectionsStateRequest,
) -> SpokeResult<()> {
    use spoke_schemas::memory_reflections_state_request::ReadSelection;
    if let ReadSelection::SelectedContexts { context_ids, .. } = &request.selection {
        if context_ids.is_empty() {
            return invalid_input(
                "selection.context_ids",
                "selected_contexts requires at least one context_id",
            );
        }
    }
    if let Some(filter) = &request.filter {
        if filter.source_context_ids.is_empty() {
            return invalid_input(
                "filter.source_context_ids",
                "provenance filter requires at least one source context",
            );
        }
    }
    spoke_ok(())
}

/// §7.26 `memory.reflections.list`: optional selection/filter invariants plus
/// the page limit.
pub fn validate_memory_reflections_list(
    request: &MemoryReflectionsListRequest,
    max_page_size: NonZeroU64,
) -> SpokeResult<()> {
    use spoke_schemas::memory_reflections_list_request::ReadSelection;
    if let Some(ReadSelection::SelectedContexts { context_ids, .. }) = &request.selection {
        if context_ids.is_empty() {
            return invalid_input(
                "selection.context_ids",
                "selected_contexts requires at least one context_id",
            );
        }
    }
    if let Some(filter) = &request.filter {
        if filter.source_context_ids.is_empty() {
            return invalid_input(
                "filter.source_context_ids",
                "provenance filter requires at least one source context",
            );
        }
    }
    check_page_limit(request.limit, max_page_size)
}

/// §7.27 `memory.reflections.generate`: selection/filter invariants.
pub fn validate_memory_reflections_generate(
    request: &MemoryReflectionsGenerateRequest,
) -> SpokeResult<()> {
    use spoke_schemas::memory_reflections_generate_request::ReadSelection;
    if let ReadSelection::SelectedContexts { context_ids, .. } = &request.selection {
        if context_ids.is_empty() {
            return invalid_input(
                "selection.context_ids",
                "selected_contexts requires at least one context_id",
            );
        }
    }
    if let Some(filter) = &request.filter {
        if filter.source_context_ids.is_empty() {
            return invalid_input(
                "filter.source_context_ids",
                "provenance filter requires at least one source context",
            );
        }
    }
    spoke_ok(())
}

/// §7.31 `memory.imports.stage`: the header-chunk shape (`Variant0`) must
/// carry `chunk_ordinal == 0`; tail ordinals ≥ 1 are type-enforced by
/// `NonZeroU64`. `conflict_policy` / `adopt_grants` chunk-0 scoping is
/// type-enforced by the shape split.
pub fn validate_memory_imports_stage(request: &MemoryImportsStageRequest) -> SpokeResult<()> {
    match request {
        MemoryImportsStageRequest::Variant0 { chunk, .. } if chunk.chunk_ordinal != 0 => {
            invalid_input("chunk.chunk_ordinal", "header chunk must carry chunk_ordinal 0")
        }
        _ => spoke_ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SpokeRejectCode, SpokeResult};
    use serde_json::json;
    use spoke_schemas::{
        MemoryCandidatesReviewRequest, MemoryCaptureRequest, MemoryGrantsGrantRequest,
        MemoryImportsStageRequest, MemoryRecallRequest, MemoryRecordsCreateRequest,
        MemoryRecordsListRequest, MemoryRecordsUpdateRequest, MemorySpacesListRequest,
        MemorySubjectsFenceRequest,
    };
    use std::num::NonZeroU64;

    fn max_page() -> NonZeroU64 {
        NonZeroU64::new(100).expect("non-zero")
    }

    fn expect_invalid<T>(result: SpokeResult<T>, field: &str) {
        match result {
            SpokeResult::Reject(reject) => {
                assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
                assert_eq!(
                    reject.details.as_ref().and_then(|d| d.get("field")).and_then(|v| v.as_str()),
                    Some(field)
                );
            }
            SpokeResult::Ok(_) => panic!("expected reject for field {field}"),
        }
    }

    // ---- §8.2 freeze allowlist ----

    #[test]
    fn freeze_allowlist_admits_reads_and_maintenance_ops() {
        for op in [
            "memory.describe",
            "memory.spaces.list",
            "memory.subjects.get",
            "memory.grants.list",
            "memory.candidates.list",
            "memory.candidates.get",
            "memory.records.get",
            "memory.records.list",
            "memory.recall",
            "memory.profile.get",
            "memory.reflections.state",
            "memory.reflections.list",
            "memory.receipts.get",
            "memory.exports.get",
            "memory.spaces.unfreeze",
            "memory.grants.revoke",
            "memory.exports.create",
            "memory.imports.stage",
            "memory.imports.activate",
            "memory.imports.discard",
        ] {
            assert!(is_freeze_allowlisted(op), "{op} must be allowlisted");
        }
    }

    #[test]
    fn freeze_allowlist_rejects_other_mutations_and_unknown_ops() {
        for op in [
            "memory.spaces.freeze",
            "memory.subjects.ensure",
            "memory.subjects.fence",
            "memory.grants.grant",
            "memory.capture",
            "memory.candidates.dismiss",
            "memory.candidates.review",
            "memory.records.create",
            "memory.records.update",
            "memory.records.delete",
            "memory.records.promote",
            "memory.profile.update",
            "memory.reflections.generate",
            "memory.extract",
        ] {
            assert!(!is_freeze_allowlisted(op), "{op} must not be allowlisted");
        }
    }

    // ---- page bounds ----

    #[test]
    fn spaces_list_rejects_limit_above_max_page_size() {
        let ok: MemorySpacesListRequest =
            serde_json::from_value(json!({"limit": 100})).expect("valid request");
        assert!(validate_memory_spaces_list(&ok, max_page()).is_ok());

        let too_big: MemorySpacesListRequest =
            serde_json::from_value(json!({"limit": 101})).expect("valid request");
        expect_invalid(validate_memory_spaces_list(&too_big, max_page()), "limit");
    }

    #[test]
    fn recall_rejects_budget_max_items_above_max_page_size() {
        let request: MemoryRecallRequest = serde_json::from_value(json!({
            "space_id": "sp_1",
            "subject_id": "s1",
            "selection": {"mode": "shared_only"},
            "budget": {"max_items": 101},
        }))
        .expect("valid request");
        expect_invalid(validate_memory_recall(&request, max_page()), "max_items");

        let request: MemoryRecallRequest = serde_json::from_value(json!({
            "space_id": "sp_1",
            "subject_id": "s1",
            "selection": {"mode": "shared_only"},
            "budget": {"max_items": 100},
        }))
        .expect("valid request");
        assert!(validate_memory_recall(&request, max_page()).is_ok());
    }

    // ---- selection / filter invariants (§6.12, §6.13) ----

    #[test]
    fn recall_rejects_empty_selected_contexts() {
        let mut request: MemoryRecallRequest = serde_json::from_value(json!({
            "space_id": "sp_1",
            "subject_id": "s1",
            "selection": {"mode": "selected_contexts", "context_ids": ["c1"]},
            "budget": {"max_items": 10},
        }))
        .expect("valid request");
        request.selection = spoke_schemas::memory_recall_request::ReadSelection::SelectedContexts {
            context_ids: Vec::new(),
            include_shared: None,
        };
        expect_invalid(validate_memory_recall(&request, max_page()), "selection.context_ids");
    }

    #[test]
    fn records_list_rejects_empty_provenance_filter() {
        let mut request: MemoryRecordsListRequest = serde_json::from_value(json!({
            "space_id": "sp_1",
            "subject_id": "s1",
            "selection": {"mode": "subject_all"},
            "filter": {"source_context_ids": ["c1"]},
        }))
        .expect("valid request");
        request.filter =
            Some(spoke_schemas::memory_records_list_request::ProvenanceFilter {
                source_context_ids: Vec::new(),
            });
        expect_invalid(validate_memory_records_list(&request, max_page()), "filter.source_context_ids");
    }

    #[test]
    fn reflections_state_rejects_empty_selected_contexts() {
        let mut request = serde_json::from_value::<spoke_schemas::MemoryReflectionsStateRequest>(json!({
            "space_id": "sp_1",
            "subject_id": "s1",
            "selection": {"mode": "subject_all"},
        }))
        .expect("valid request");
        request.selection =
            spoke_schemas::memory_reflections_state_request::ReadSelection::SelectedContexts {
                context_ids: Vec::new(),
                include_shared: None,
            };
        expect_invalid(validate_memory_reflections_state(&request), "selection.context_ids");
    }

    #[test]
    fn reflections_generate_rejects_empty_provenance_filter() {
        let mut request = serde_json::from_value::<spoke_schemas::MemoryReflectionsGenerateRequest>(
            json!({
                "space_id": "sp_1",
                "subject_id": "s1",
                "selection": {"mode": "shared_only"},
                "expected_state_digest": "d".repeat(64),
                "idempotency_key": "k1",
            }),
        )
        .expect("valid request");
        request.filter = Some(spoke_schemas::memory_reflections_generate_request::ProvenanceFilter {
            source_context_ids: Vec::new(),
        });
        expect_invalid(validate_memory_reflections_generate(&request), "filter.source_context_ids");
    }

    #[test]
    fn reflections_list_validates_optional_selection_and_limit() {
        let mut request = serde_json::from_value::<spoke_schemas::MemoryReflectionsListRequest>(json!({
            "space_id": "sp_1",
            "subject_id": "s1",
            "limit": 50,
        }))
        .expect("valid request");
        assert!(validate_memory_reflections_list(&request, max_page()).is_ok());

        request.limit = NonZeroU64::new(101);
        expect_invalid(validate_memory_reflections_list(&request, max_page()), "limit");
    }

    // ---- per-op conditionals ----

    #[test]
    fn subjects_fence_rejects_archive_on_context_target() {
        let request: MemorySubjectsFenceRequest = serde_json::from_value(json!({
            "space_id": "sp_1",
            "subject_id": "s1",
            "target": {"type": "context", "context_id": "c1"},
            "action": "archive",
            "expected_revision": 1,
            "idempotency_key": "k1",
        }))
        .expect("valid request");
        expect_invalid(validate_memory_subjects_fence(&request), "target");

        let fence: MemorySubjectsFenceRequest = serde_json::from_value(json!({
            "space_id": "sp_1",
            "subject_id": "s1",
            "target": {"type": "context", "context_id": "c1"},
            "action": "fence",
            "expected_revision": 1,
            "idempotency_key": "k1",
        }))
        .expect("valid request");
        assert!(validate_memory_subjects_fence(&fence).is_ok());

        let archive_subject: MemorySubjectsFenceRequest = serde_json::from_value(json!({
            "space_id": "sp_1",
            "subject_id": "s1",
            "target": {"type": "subject"},
            "action": "archive",
            "expected_revision": 1,
            "idempotency_key": "k1",
        }))
        .expect("valid request");
        assert!(validate_memory_subjects_fence(&archive_subject).is_ok());
    }

    #[test]
    fn grants_grant_rejects_empty_context_ids_on_restricted_shape() {
        let mut request: MemoryGrantsGrantRequest = serde_json::from_value(json!({
            "space_id": "sp_1",
            "principal_id": "p2",
            "capabilities": ["capture"],
            "context_ids": ["c1"],
            "idempotency_key": "k1",
        }))
        .expect("valid restricted-shape request");
        match &mut request {
            MemoryGrantsGrantRequest::Variant0 { context_ids, .. } => context_ids.clear(),
            MemoryGrantsGrantRequest::Variant1 { .. } => panic!("expected restricted shape"),
        }
        expect_invalid(validate_memory_grants_grant(&request), "context_ids");
    }

    #[test]
    fn candidates_review_rejects_empty_fragment_list() {
        let mut request: MemoryCandidatesReviewRequest = serde_json::from_value(json!({
            "space_id": "sp_1",
            "candidate_id": "cand_1",
            "expected_revision": 1,
            "decision": {"action": "drop"},
            "idempotency_key": "k1",
        }))
        .expect("valid request");
        request.decision =
            spoke_schemas::memory_candidates_review_request::Decision::Fragment {
                fragments: Vec::new(),
            };
        expect_invalid(validate_memory_candidates_review(&request), "decision.fragments");
    }

    #[test]
    fn records_update_rejects_content_on_kind_update_shape() {
        let request: MemoryRecordsUpdateRequest = serde_json::from_value(json!({
            "space_id": "sp_1",
            "record_id": "r1",
            "expected_revision": 1,
            "kind": "note",
            "content": {"text": "must not ride a kind update"},
            "idempotency_key": "k1",
        }))
        .expect("valid kind-update-shape request");
        match &request {
            MemoryRecordsUpdateRequest::Variant1 { content: Some(_), .. } => {}
            other => panic!("expected kind-update shape carrying content, got {other:?}"),
        }
        expect_invalid(validate_memory_records_update(&request, 1024), "content");
    }

    #[test]
    fn imports_stage_rejects_non_zero_header_chunk_ordinal() {
        let request: MemoryImportsStageRequest = serde_json::from_value(json!({
            "space_id": "sp_1",
            "chunk": {
                "export_id": "e1",
                "chunk_ordinal": 1,
                "chunk_count": 2,
                "bundle_digest": "b".repeat(64),
                "chunk_digest": "c".repeat(64),
                "bytes": 10,
                "header": {
                    "schema_version": 1,
                    "bundle_format_version": 1,
                    "memory_protocol_version": 1,
                    "export_id": "e1",
                    "space_id": "sp_1",
                    "created_at": "2026-01-01T00:00:00Z",
                    "source": {
                        "authority_host_id": "host_1",
                        "authority_generation": 1,
                        "engine_policy_version": "v1",
                    },
                    "digest": "d".repeat(64),
                },
            },
            "conflict_policy": "fail",
            "idempotency_key": "k1",
        }))
        .expect("valid first-chunk-shape request");
        expect_invalid(validate_memory_imports_stage(&request), "chunk.chunk_ordinal");
    }

    // ---- content byte limits ----

    #[test]
    fn content_wire_bytes_counts_text_and_structured() {
        assert_eq!(content_wire_bytes("héllo", None), 6);
        let structured = json!({"a": 1});
        let structured_len =
            serde_json::to_vec(&structured).expect("serialize").len();
        assert_eq!(
            content_wire_bytes("ab", Some(&structured)),
            2 + structured_len
        );
    }

    #[test]
    fn capture_rejects_oversize_content() {
        let request: MemoryCaptureRequest = serde_json::from_value(json!({
            "space_id": "sp_1",
            "subject_id": "s1",
            "content": {"text": "x".repeat(2048)},
            "capture_scope": {"visibility": "shared"},
            "idempotency_key": "k1",
        }))
        .expect("valid request");
        expect_invalid(validate_memory_capture(&request, 1024), "content");

        let small: MemoryCaptureRequest = serde_json::from_value(json!({
            "space_id": "sp_1",
            "subject_id": "s1",
            "content": {"text": "fine"},
            "capture_scope": {"visibility": "context", "context_id": "c1"},
            "idempotency_key": "k1",
        }))
        .expect("valid request");
        assert!(validate_memory_capture(&small, 1024).is_ok());
    }

    #[test]
    fn records_create_rejects_oversize_content() {
        let request: MemoryRecordsCreateRequest = serde_json::from_value(json!({
            "space_id": "sp_1",
            "subject_id": "s1",
            "kind": "note",
            "content": {"text": "x".repeat(2048)},
            "scope": {"visibility": "shared"},
            "idempotency_key": "k1",
        }))
        .expect("valid request");
        expect_invalid(validate_memory_records_create(&request, 1024), "content");
    }
}
