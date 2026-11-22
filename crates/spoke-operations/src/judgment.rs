//! `spoke-judgment` envelope validation and canonical comparison (spoke-judgment
//! wire contract): `validate_judgment` is the pure wire-shape gate — required
//! fields, the closed `Judgment` / `JudgmentRef` / `verdict` envelopes,
//! non-empty strings, `verdict.confidence` in `[0, 1]`, positive `revision`s
//! and the reserved `verdict.detail` members. It runs on the untyped wire value
//! so contract refusals (e.g. `verdict.detail.target`) report `INVALID_INPUT`
//! with `details.field` instead of opaque decode errors.
//!
//! `judgments_equal_canonical` is the RFC 8785 comparison helper: two producers
//! of the same `Judgment` emit byte-identical canonical bytes; `verdict.detail`
//! and `extensions` are included, never normalized by product rules.
//!
//! TypeScript parity: `packages/spoke-operations/src/judgment.ts`.

use crate::result::{spoke_ok, spoke_ok_unit, spoke_reject, SpokeRejectCode, SpokeResult};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::{Map, Value};
use spoke_schemas::judgment::Judgment;

// Closed-envelope property tables (judgment.schema.json, additionalProperties:
// false on Judgment, JudgmentRef and Verdict).
const JUDGMENT_KEYS: [&str; 9] = [
    "schema_version",
    "judgment_id",
    "judgment_kind",
    "target",
    "evidence",
    "verdict",
    "basis",
    "created_at",
    "extensions",
];

const JUDGMENT_REF_KEYS: [&str; 4] = ["kind", "id", "namespace", "revision"];

const VERDICT_KEYS: [&str; 3] = ["outcome", "confidence", "detail"];

// Reserved verdict.detail direct member names — they duplicate protocol-level
// envelope locations (contract §verdict core and detail).
const DETAIL_RESERVED_MEMBERS: [&str; 4] = ["target", "evidence", "judgment_kind", "outcome"];

fn is_non_empty_trimmed_string(value: &str) -> bool {
    !value.trim().is_empty()
}

fn is_plain_object(value: &Value) -> bool {
    value.is_object()
}

fn is_positive_integer(value: &Value) -> bool {
    value.as_u64().is_some_and(|v| v >= 1)
}

fn is_parseable_date_time(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }

    DateTime::parse_from_rfc3339(trimmed).is_ok()
        || trimmed.parse::<DateTime<Utc>>().is_ok()
        || NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S%.f").is_ok()
        || NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S").is_ok()
}

/// Every shape violation rejects `INVALID_INPUT` with `details.field` set to
/// the logical field path (dotted members, bracketed array indices, e.g.
/// `target.revision`, `evidence[0].revision`, `verdict.detail.target`). `field`
/// is the only protocol-required member of `details`; the `message` carries no
/// stable machine semantics.
fn invalid(field: &str, message: &str) -> SpokeResult<()> {
    let mut details = Map::new();
    details.insert("field".into(), Value::String(field.into()));
    spoke_reject(SpokeRejectCode::InvalidInput, message, Some(details))
}

fn has_only_keys(value: &Map<String, Value>, allowed: &[&str]) -> bool {
    value.keys().all(|key| allowed.contains(&key.as_str()))
}

fn validate_judgment_ref(ref_: &Value, field: &str, require_revision: bool) -> SpokeResult<()> {
    let Some(ref_) = ref_.as_object() else {
        return invalid(field, &format!("{field} must be a JudgmentRef object"));
    };
    if !has_only_keys(ref_, &JUDGMENT_REF_KEYS) {
        return invalid(
            field,
            &format!("{field} carries a member outside the JudgmentRef shape"),
        );
    }
    let kind = ref_.get("kind").and_then(Value::as_str).unwrap_or("");
    if !is_non_empty_trimmed_string(kind) {
        return invalid(
            &format!("{field}.kind"),
            &format!("{field}.kind must be a non-empty string"),
        );
    }
    let id = ref_.get("id").and_then(Value::as_str).unwrap_or("");
    if !is_non_empty_trimmed_string(id) {
        return invalid(
            &format!("{field}.id"),
            &format!("{field}.id must be a non-empty string"),
        );
    }
    if let Some(namespace) = ref_.get("namespace") {
        let namespace = namespace.as_str().unwrap_or("");
        if !is_non_empty_trimmed_string(namespace) {
            return invalid(
                &format!("{field}.namespace"),
                &format!("{field}.namespace must be a non-empty string when present"),
            );
        }
    }
    let revision = ref_.get("revision");
    if require_revision && revision.is_none() {
        return invalid(
            &format!("{field}.revision"),
            &format!("{field}.revision is required on evidence references"),
        );
    }
    if let Some(revision) = revision {
        if !is_positive_integer(revision) {
            return invalid(
                &format!("{field}.revision"),
                &format!("{field}.revision must be a positive integer"),
            );
        }
    }
    spoke_ok_unit()
}

/// Wire-shape gate for `Judgment` (optional capability spoke-judgment).
///
/// Checks required-field presence, the closed envelopes, non-empty strings,
/// `schema_version`/`revision` positive integers, `verdict` shape and the
/// reserved `verdict.detail` members. Pure envelope validation only — verdict
/// truth, evidence existence or content correctness, and product-specific
/// `detail` shape stay product-owned. Deterministic, no I/O.
pub fn validate_judgment(judgment: &Value) -> SpokeResult<()> {
    let Some(judgment) = judgment.as_object() else {
        return invalid("judgment", "judgment must be an object");
    };
    if !has_only_keys(judgment, &JUDGMENT_KEYS) {
        return invalid(
            "judgment",
            "judgment carries a member outside the Judgment shape",
        );
    }
    let schema_version = judgment.get("schema_version").unwrap_or(&Value::Null);
    if !is_positive_integer(schema_version) {
        return invalid("schema_version", "schema_version must be an integer >= 1");
    }
    let judgment_id = judgment
        .get("judgment_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !is_non_empty_trimmed_string(judgment_id) {
        return invalid("judgment_id", "judgment_id must be a non-empty string");
    }
    let judgment_kind = judgment
        .get("judgment_kind")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !is_non_empty_trimmed_string(judgment_kind) {
        return invalid(
            "judgment_kind",
            "judgment_kind must be a non-empty string",
        );
    }

    let target = match judgment.get("target") {
        Some(target) => target,
        None => return invalid("target", "target must be a JudgmentRef object"),
    };
    let target_result = validate_judgment_ref(target, "target", false);
    if target_result.is_reject() {
        return target_result;
    }

    let Some(evidence) = judgment.get("evidence").and_then(Value::as_array) else {
        return invalid("evidence", "evidence must be an array (may be empty)");
    };
    for (index, item) in evidence.iter().enumerate() {
        let result = validate_judgment_ref(item, &format!("evidence[{index}]"), true);
        if result.is_reject() {
            return result;
        }
    }

    let verdict = match judgment.get("verdict") {
        Some(verdict) => verdict,
        None => return invalid("verdict", "verdict must be an object"),
    };
    let Some(verdict) = verdict.as_object() else {
        return invalid("verdict", "verdict must be an object");
    };
    if !has_only_keys(verdict, &VERDICT_KEYS) {
        return invalid(
            "verdict",
            "verdict carries a member outside the Verdict shape",
        );
    }
    let outcome = verdict
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !is_non_empty_trimmed_string(outcome) {
        return invalid(
            "verdict.outcome",
            "verdict.outcome must be a non-empty string",
        );
    }
    if let Some(confidence) = verdict.get("confidence") {
        if !confidence
            .as_f64()
            .is_some_and(|c| (0.0..=1.0).contains(&c))
        {
            return invalid(
                "verdict.confidence",
                "verdict.confidence must be a number in [0, 1]",
            );
        }
    }
    if let Some(detail) = verdict.get("detail") {
        let Some(detail) = detail.as_object() else {
            return invalid("verdict.detail", "verdict.detail must be an object");
        };
        for member in detail.keys() {
            if DETAIL_RESERVED_MEMBERS.contains(&member.as_str()) {
                return invalid(
                    &format!("verdict.detail.{member}"),
                    &format!(
                        "verdict.detail.{member} duplicates a protocol envelope location"
                    ),
                );
            }
        }
    }

    if let Some(basis) = judgment.get("basis") {
        if !basis.is_string() {
            return invalid("basis", "basis must be a string when present");
        }
    }
    if let Some(created_at) = judgment.get("created_at") {
        if !created_at.is_string()
            || !is_parseable_date_time(created_at.as_str().unwrap_or(""))
        {
            return invalid(
                "created_at",
                "created_at must be an RFC 3339 timestamp when present",
            );
        }
    }
    if !judgment.get("extensions").is_some_and(is_plain_object) {
        return invalid("extensions", "extensions must be a present object");
    }

    spoke_ok_unit()
}

/// Whether two [`Judgment`]s are byte-identical under RFC 8785 canonical JSON —
/// the contract's producer-equality rule. Canonicalization covers the complete
/// envelope including `verdict.detail`, `extensions` and present optional
/// members.
///
/// Rejects with `INVALID_INPUT` when either judgment cannot be serialized to
/// canonical bytes.
pub fn judgments_equal_canonical(left: &Judgment, right: &Judgment) -> SpokeResult<bool> {
    let left = match serde_jcs::to_vec(left) {
        Ok(bytes) => bytes,
        Err(error) => {
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                format!("JCS canonicalization failed: {error}"),
                None,
            );
        }
    };
    let right = match serde_jcs::to_vec(right) {
        Ok(bytes) => bytes,
        Err(error) => {
            return spoke_reject(
                SpokeRejectCode::InvalidInput,
                format!("JCS canonicalization failed: {error}"),
                None,
            );
        }
    };
    spoke_ok(left == right)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_judgment() -> Value {
        serde_json::json!({
            "schema_version": 1,
            "judgment_id": "j-1",
            "judgment_kind": "consistency",
            "target": {"kind": "knowledge_entry", "id": "ke-1"},
            "evidence": [
                {"kind": "knowledge_entry", "id": "ke-2", "revision": 3},
                {"kind": "rule", "id": "r-1", "namespace": "prod", "revision": 1},
            ],
            "verdict": {
                "outcome": "consistent",
                "confidence": 0.9,
                "detail": {"reason": "all agree"},
            },
            "basis": "human review",
            "created_at": "2026-11-22T10:00:00Z",
            "extensions": {},
        })
    }

    fn typed(judgment: &Value) -> Judgment {
        serde_json::from_value(judgment.clone()).expect("fixture parses into Judgment")
    }

    fn expect_invalid(result: SpokeResult<()>, field: &str) {
        let SpokeResult::Reject(reject) = result else {
            panic!("expected INVALID_INPUT reject for {field}");
        };
        assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        let details = reject.details.expect("reject carries details");
        assert_eq!(details.get("field").and_then(Value::as_str), Some(field));
    }

    #[test]
    fn validate_judgment_accepts_valid_record() {
        assert!(matches!(validate_judgment(&valid_judgment()), SpokeResult::Ok(())));
    }

    #[test]
    fn validate_judgment_rejects_missing_and_empty_required_fields() {
        let mut judgment = valid_judgment();
        judgment.as_object_mut().unwrap().remove("judgment_id");
        expect_invalid(validate_judgment(&judgment), "judgment_id");

        let mut judgment = valid_judgment();
        judgment["judgment_id"] = Value::String(" ".into());
        expect_invalid(validate_judgment(&judgment), "judgment_id");

        let mut judgment = valid_judgment();
        judgment["judgment_kind"] = Value::String("   ".into());
        expect_invalid(validate_judgment(&judgment), "judgment_kind");

        let mut judgment = valid_judgment();
        judgment["schema_version"] = Value::from(0);
        expect_invalid(validate_judgment(&judgment), "schema_version");

        let mut judgment = valid_judgment();
        judgment["schema_version"] = Value::from(1.5);
        expect_invalid(validate_judgment(&judgment), "schema_version");
    }

    #[test]
    fn validate_judgment_rejects_bad_target_and_evidence_refs() {
        let mut judgment = valid_judgment();
        judgment["target"] = serde_json::json!({"kind": " ", "id": "ke-1"});
        expect_invalid(validate_judgment(&judgment), "target.kind");

        let mut judgment = valid_judgment();
        judgment["target"]["namespace"] = Value::String("".into());
        expect_invalid(validate_judgment(&judgment), "target.namespace");

        let mut judgment = valid_judgment();
        judgment["target"]["revision"] = Value::from(0);
        expect_invalid(validate_judgment(&judgment), "target.revision");

        let mut judgment = valid_judgment();
        judgment["target"]["extra"] = Value::Bool(true);
        expect_invalid(validate_judgment(&judgment), "target");

        let mut judgment = valid_judgment();
        judgment["evidence"] = serde_json::json!([{"kind": "rule", "id": "r-1"}]);
        expect_invalid(validate_judgment(&judgment), "evidence[0].revision");

        let mut judgment = valid_judgment();
        judgment["evidence"][0]["revision"] = Value::from(-1);
        expect_invalid(validate_judgment(&judgment), "evidence[0].revision");

        let mut judgment = valid_judgment();
        judgment["evidence"][1]["namespace"] = Value::String("".into());
        expect_invalid(validate_judgment(&judgment), "evidence[1].namespace");
    }

    #[test]
    fn validate_judgment_rejects_bad_verdict() {
        let mut judgment = valid_judgment();
        judgment["verdict"]["outcome"] = Value::String(" ".into());
        expect_invalid(validate_judgment(&judgment), "verdict.outcome");

        let mut judgment = valid_judgment();
        judgment["verdict"]["confidence"] = Value::from(1.5);
        expect_invalid(validate_judgment(&judgment), "verdict.confidence");

        let mut judgment = valid_judgment();
        judgment["verdict"]["confidence"] = Value::from(-0.1);
        expect_invalid(validate_judgment(&judgment), "verdict.confidence");

        let mut judgment = valid_judgment();
        judgment["verdict"]["confidence"] = Value::String("high".into());
        expect_invalid(validate_judgment(&judgment), "verdict.confidence");

        let mut judgment = valid_judgment();
        judgment["verdict"]["detail"] = serde_json::json!({"target": "x"});
        expect_invalid(validate_judgment(&judgment), "verdict.detail.target");

        let mut judgment = valid_judgment();
        judgment["verdict"]["extra"] = Value::Bool(true);
        expect_invalid(validate_judgment(&judgment), "verdict");
    }

    #[test]
    fn validate_judgment_rejects_envelope_and_optional_shape_errors() {
        expect_invalid(
            validate_judgment(&serde_json::json!(null)),
            "judgment",
        );
        expect_invalid(
            validate_judgment(&serde_json::json!([])),
            "judgment",
        );

        let mut judgment = valid_judgment();
        judgment["stray"] = Value::Bool(true);
        expect_invalid(validate_judgment(&judgment), "judgment");

        let mut judgment = valid_judgment();
        judgment.as_object_mut().unwrap().remove("extensions");
        expect_invalid(validate_judgment(&judgment), "extensions");

        let mut judgment = valid_judgment();
        judgment["extensions"] = serde_json::json!([]);
        expect_invalid(validate_judgment(&judgment), "extensions");

        let mut judgment = valid_judgment();
        judgment["evidence"] = Value::Null;
        expect_invalid(validate_judgment(&judgment), "evidence");

        let mut judgment = valid_judgment();
        judgment["basis"] = Value::from(7);
        expect_invalid(validate_judgment(&judgment), "basis");

        let mut judgment = valid_judgment();
        judgment["created_at"] = Value::String("not-a-date".into());
        expect_invalid(validate_judgment(&judgment), "created_at");
    }

    #[test]
    fn validate_judgment_accepts_boundary_confidence_and_empty_evidence() {
        let mut judgment = valid_judgment();
        judgment["verdict"]["confidence"] = Value::from(1.0);
        judgment["evidence"] = serde_json::json!([]);
        assert!(matches!(validate_judgment(&judgment), SpokeResult::Ok(())));
    }

    #[test]
    fn identical_judgments_compare_equal() {
        assert!(matches!(
            judgments_equal_canonical(&typed(&valid_judgment()), &typed(&valid_judgment())),
            SpokeResult::Ok(true)
        ));
    }

    #[test]
    fn different_judgments_compare_unequal() {
        let mut right = valid_judgment();
        right["judgment_id"] = Value::String("j-2".into());
        assert!(matches!(
            judgments_equal_canonical(&typed(&valid_judgment()), &typed(&right)),
            SpokeResult::Ok(false)
        ));
    }

    #[test]
    fn member_order_does_not_change_the_canonical_bytes() {
        let left = typed(&serde_json::json!({
            "judgment_id": "j-1",
            "judgment_kind": "review",
            "schema_version": 1,
            "target": {"id": "ke-1", "kind": "knowledge_entry"},
            "evidence": [],
            "verdict": {"outcome": "accepted", "detail": {"a": 1, "b": 2}},
            "extensions": {},
        }));
        let right = typed(&serde_json::json!({
            "extensions": {},
            "verdict": {"detail": {"b": 2, "a": 1}, "outcome": "accepted"},
            "evidence": [],
            "target": {"kind": "knowledge_entry", "id": "ke-1"},
            "schema_version": 1,
            "judgment_kind": "review",
            "judgment_id": "j-1",
        }));
        assert!(matches!(
            judgments_equal_canonical(&left, &right),
            SpokeResult::Ok(true)
        ));
    }

    #[test]
    fn detail_is_included_in_canonical_bytes() {
        let mut right = valid_judgment();
        right["verdict"]["detail"] = serde_json::json!({"reason": "changed"});
        assert!(matches!(
            judgments_equal_canonical(&typed(&valid_judgment()), &typed(&right)),
            SpokeResult::Ok(false)
        ));
    }
}
