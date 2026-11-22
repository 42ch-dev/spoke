//! `spoke-judgment` envelope validation and canonical comparison (spoke-judgment
//! wire contract): `validate_judgment` is the pure wire-shape gate — required
//! fields, the closed `Judgment` / `JudgmentRef` / `verdict` envelopes, raw
//! `minLength: 1` strings, the interoperable integer domain,
//! `verdict.confidence` in `[0, 1]` and the reserved `verdict.detail`
//! members. It runs on the untyped wire value so contract refusals (e.g.
//! `verdict.detail.target`) report `INVALID_INPUT` with `details.field`
//! instead of opaque decode errors.
//!
//! `judgments_equal_canonical` is the RFC 8785 comparison helper on the wire
//! JSON itself: `verdict.detail` presence, `extensions` and the `created_at`
//! spelling stay comparison-significant — typed decodes never normalize them.
//!
//! TypeScript parity: `packages/spoke-operations/src/judgment.ts`.

use crate::result::{spoke_ok, spoke_ok_unit, spoke_reject, SpokeRejectCode, SpokeResult};
use chrono::DateTime;
use serde_json::{Map, Value};

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

// Raw `minLength: 1`: whitespace-only strings are valid non-empty wire
// values; the raw string is measured, never trimmed (wire-wide rule).
fn is_non_empty_string(value: &str) -> bool {
    !value.is_empty()
}

fn is_plain_object(value: &Value) -> bool {
    value.is_object()
}

// Interoperable integer domain [0, 2^53-1]: exact in u64 and IEEE-754
// binary64 alike. Integral spellings (`1.0`) decode as the integer they
// spell; non-integral or out-of-domain values are INVALID_INPUT.
const MAX_INTEROPERABLE_INTEGER: u64 = (1 << 53) - 1;

fn wire_integer(value: &Value) -> Option<u64> {
    if let Some(v) = value.as_u64() {
        return (v <= MAX_INTEROPERABLE_INTEGER).then_some(v);
    }
    match value.as_f64() {
        Some(v) if v.fract() == 0.0 && (0.0..=(MAX_INTEROPERABLE_INTEGER as f64)).contains(&v) => {
            Some(v as u64)
        }
        _ => None,
    }
}

fn is_positive_integer(value: &Value) -> bool {
    wire_integer(value).is_some_and(|v| v >= 1)
}

// RFC 3339 `date-time` requires an explicit offset (`Z` or `±hh:mm`); chrono
// refuses offsetless spellings, keeps leap-second `23:59:60` accepted and
// refuses invalid calendar days. The grammar is matched against the raw wire
// string exactly as received: chrono `parse_from_rfc3339` also accepts a
// space as the date-time separator and Unicode offset signs such as U+2212,
// so the raw-string gates refuse whitespace-wrapped, space-separated and
// non-ASCII spellings to keep Rust and TypeScript on one strict form.
fn is_parseable_date_time(value: &str) -> bool {
    value.is_ascii()
        && value.trim() == value
        && matches!(value.as_bytes().get(10), Some(b'T' | b't'))
        && DateTime::parse_from_rfc3339(value).is_ok()
}

// Shared ExtensionMap grammar: namespace keys match `^[a-z][a-z0-9_-]*$`.
fn is_namespace(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    value[1..]
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
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
    if !is_non_empty_string(kind) {
        return invalid(
            &format!("{field}.kind"),
            &format!("{field}.kind must be a non-empty string"),
        );
    }
    let id = ref_.get("id").and_then(Value::as_str).unwrap_or("");
    if !is_non_empty_string(id) {
        return invalid(
            &format!("{field}.id"),
            &format!("{field}.id must be a non-empty string"),
        );
    }
    if let Some(namespace) = ref_.get("namespace") {
        let namespace = namespace.as_str().unwrap_or("");
        if !is_non_empty_string(namespace) {
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
/// Checks required-field presence, the closed envelopes, raw `minLength: 1`
/// strings, `schema_version`/`revision` positive integers in the
/// interoperable `[0, 2^53-1]` domain, `verdict` shape and the reserved
/// `verdict.detail` members. Pure envelope validation only — verdict
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
    if !is_non_empty_string(judgment_id) {
        return invalid("judgment_id", "judgment_id must be a non-empty string");
    }
    let judgment_kind = judgment
        .get("judgment_kind")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !is_non_empty_string(judgment_kind) {
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
    if !is_non_empty_string(outcome) {
        return invalid(
            "verdict.outcome",
            "verdict.outcome must be a non-empty string",
        );
    }
    if let Some(confidence) = verdict.get("confidence") {
        if !confidence
            .as_f64()
            .is_some_and(|c| c.is_finite() && (0.0..=1.0).contains(&c))
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
        // Fixed protocol probe order (contract §verdict core and detail):
        // bounded over the four reserved names, independent of wire member
        // order, so `details.field` is deterministic across stacks.
        for member in DETAIL_RESERVED_MEMBERS {
            if detail.contains_key(member) {
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
    let Some(extensions) = judgment.get("extensions").and_then(Value::as_object) else {
        return invalid("extensions", "extensions must be a present object");
    };
    // serde_json::Map is a BTreeMap: iteration is lexicographic key order, so
    // with several failing entries the first refusal names the
    // lexicographically-first offending member — the contract's sorted-order
    // probe, derived from the map's keys alone and matching TypeScript's
    // explicit sort.
    for (key, entry) in extensions {
        if !is_namespace(key) || !is_plain_object(entry) {
            return invalid(
                &format!("extensions.{key}"),
                "extensions entries need a namespace key and an object value",
            );
        }
    }

    spoke_ok_unit()
}

/// Whether two `Judgment` wire values are byte-identical under RFC 8785
/// canonical JSON — the contract's producer-equality rule. Compares the
/// canonical serialization of the wire JSON, never a typed decode: member
/// presence (`verdict.detail: {}` ≠ absent) and the `created_at` spelling
/// stay comparison-significant.
///
/// Rejects with `INVALID_INPUT` when either judgment cannot be serialized to
/// canonical bytes.
pub fn judgments_equal_canonical(left: &Value, right: &Value) -> SpokeResult<bool> {
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
        judgment["judgment_id"] = Value::String("".into());
        expect_invalid(validate_judgment(&judgment), "judgment_id");

        let mut judgment = valid_judgment();
        judgment["judgment_kind"] = Value::String("".into());
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
        judgment["target"] = serde_json::json!({"kind": "", "id": "ke-1"});
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
        judgment["verdict"]["outcome"] = Value::String("".into());
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
    fn validate_judgment_uses_raw_min_length_one() {
        // Raw minLength: 1 — whitespace-only strings are valid non-empty
        // wire values (a lone space, U+FEFF); only the empty string fails.
        let mut judgment = valid_judgment();
        judgment["judgment_id"] = Value::String(" ".into());
        judgment["judgment_kind"] = Value::String("\u{FEFF}".into());
        judgment["target"]["kind"] = Value::String(" ".into());
        judgment["target"]["namespace"] = Value::String(" ".into());
        judgment["verdict"]["outcome"] = Value::String(" ".into());
        assert!(matches!(validate_judgment(&judgment), SpokeResult::Ok(())));
    }

    #[test]
    fn validate_judgment_enforces_the_interoperable_integer_domain() {
        // Integral spellings decode as the integer they spell.
        let mut judgment = valid_judgment();
        judgment["schema_version"] = Value::from(2.0);
        judgment["evidence"][0]["revision"] = Value::from(1.0);
        assert!(matches!(validate_judgment(&judgment), SpokeResult::Ok(())));

        // The top of [0, 2^53-1] is exact on both stacks.
        let mut judgment = valid_judgment();
        judgment["evidence"][0]["revision"] = Value::from(9_007_199_254_740_991_u64);
        assert!(matches!(validate_judgment(&judgment), SpokeResult::Ok(())));

        // 2^53 is out of domain even though it is u64-safe.
        let mut judgment = valid_judgment();
        judgment["evidence"][0]["revision"] = Value::from(9_007_199_254_740_992_u64);
        expect_invalid(validate_judgment(&judgment), "evidence[0].revision");

        let mut judgment = valid_judgment();
        judgment["target"]["revision"] = Value::from(2.5);
        expect_invalid(validate_judgment(&judgment), "target.revision");
    }

    #[test]
    fn validate_judgment_requires_a_created_at_offset() {
        let mut judgment = valid_judgment();
        judgment["created_at"] = Value::String("2026-11-22T10:00:00".into());
        expect_invalid(validate_judgment(&judgment), "created_at");

        // Leap-second spellings stay accepted.
        let mut judgment = valid_judgment();
        judgment["created_at"] = Value::String("2026-06-30T23:59:60Z".into());
        assert!(matches!(validate_judgment(&judgment), SpokeResult::Ok(())));

        // Explicit non-UTC offsets are accepted.
        let mut judgment = valid_judgment();
        judgment["created_at"] = Value::String("2026-11-22T18:00:00+08:00".into());
        assert!(matches!(validate_judgment(&judgment), SpokeResult::Ok(())));

        // Invalid calendar days stay refused.
        let mut judgment = valid_judgment();
        judgment["created_at"] = Value::String("2026-02-30T10:00:00Z".into());
        expect_invalid(validate_judgment(&judgment), "created_at");
    }

    #[test]
    fn validate_judgment_matches_created_at_against_the_raw_wire_string() {
        // The grammar is matched against the raw wire string exactly as
        // received: leading or trailing whitespace refuses even though the
        // inner spelling is a valid timestamp.
        let mut judgment = valid_judgment();
        judgment["created_at"] = Value::String(" 2026-06-30T15:59:59Z ".into());
        expect_invalid(validate_judgment(&judgment), "created_at");

        let mut judgment = valid_judgment();
        judgment["created_at"] = Value::String("\t2026-06-30T15:59:59Z".into());
        expect_invalid(validate_judgment(&judgment), "created_at");

        // A non-`T`/`t` date-time separator refuses — chrono's RFC 3339
        // parse alone would accept the space at index 10.
        let mut judgment = valid_judgment();
        judgment["created_at"] = Value::String("2026-06-30 15:59:59Z".into());
        expect_invalid(validate_judgment(&judgment), "created_at");

        // Lowercase `t` and `z` spellings stay accepted.
        let mut judgment = valid_judgment();
        judgment["created_at"] = Value::String("2026-06-30t15:59:59z".into());
        assert!(matches!(validate_judgment(&judgment), SpokeResult::Ok(())));
    }

    #[test]
    fn validate_judgment_refuses_non_ascii_created_at_spelling() {
        // Only ASCII offset signs are in contract: chrono's RFC 3339 parse
        // alone accepts U+2212 MINUS SIGN as an offset sign.
        let mut judgment = valid_judgment();
        judgment["created_at"] = Value::String("2026-06-30T23:59:59\u{2212}08:00".into());
        expect_invalid(validate_judgment(&judgment), "created_at");

        // Any non-ASCII byte refuses, not only offset signs.
        let mut judgment = valid_judgment();
        judgment["created_at"] = Value::String("2026-06-30T15:59:59\u{2212}".into());
        expect_invalid(validate_judgment(&judgment), "created_at");

        // ASCII `+` and `-` offsets stay accepted.
        let mut judgment = valid_judgment();
        judgment["created_at"] = Value::String("2026-06-30T15:59:59-08:00".into());
        assert!(matches!(validate_judgment(&judgment), SpokeResult::Ok(())));
        let mut judgment = valid_judgment();
        judgment["created_at"] = Value::String("2026-06-30T15:59:59+08:00".into());
        assert!(matches!(validate_judgment(&judgment), SpokeResult::Ok(())));
    }

    #[test]
    fn extensions_multi_violation_reports_the_lexicographically_first_key() {
        // Sorted-order probe: with several failing entries, details.field
        // names the first offending member in lexicographic key order —
        // derived from the map's keys alone, deterministic across stacks.
        let mut judgment = valid_judgment();
        judgment["extensions"] = serde_json::json!({"zeta_ns": 7, "Alpha": {}, "9lives": {}});
        expect_invalid(validate_judgment(&judgment), "extensions.9lives");

        let mut judgment = valid_judgment();
        judgment["extensions"] = serde_json::json!({"zeta_ns": 7, "acme_audit": 7});
        expect_invalid(validate_judgment(&judgment), "extensions.acme_audit");

        // A valid member ahead of an invalid one is skipped, not reported.
        let mut judgment = valid_judgment();
        judgment["extensions"] = serde_json::json!({"a_ok": {}, "bad key": {}});
        expect_invalid(validate_judgment(&judgment), "extensions.bad key");
    }

    #[test]
    fn validate_judgment_enforces_the_extension_map_grammar() {
        let mut judgment = valid_judgment();
        judgment["extensions"] = serde_json::json!({"Acme": {}});
        expect_invalid(validate_judgment(&judgment), "extensions.Acme");

        let mut judgment = valid_judgment();
        judgment["extensions"] = serde_json::json!({"acme": 7});
        expect_invalid(validate_judgment(&judgment), "extensions.acme");

        let mut judgment = valid_judgment();
        judgment["extensions"] = serde_json::json!({"acme-prod_1": {"flag": true}});
        assert!(matches!(validate_judgment(&judgment), SpokeResult::Ok(())));
    }

    #[test]
    fn detail_reserved_probe_follows_the_fixed_protocol_order() {
        // Probe order target, evidence, judgment_kind, outcome is independent
        // of wire member order, so details.field matches across stacks.
        let mut judgment = valid_judgment();
        judgment["verdict"]["detail"] =
            serde_json::json!({"outcome": 1, "judgment_kind": 1, "evidence": 1, "target": 1});
        expect_invalid(validate_judgment(&judgment), "verdict.detail.target");

        let mut judgment = valid_judgment();
        judgment["verdict"]["detail"] =
            serde_json::json!({"outcome": 1, "judgment_kind": 1});
        expect_invalid(validate_judgment(&judgment), "verdict.detail.judgment_kind");
    }

    #[test]
    fn identical_judgments_compare_equal() {
        assert!(matches!(
            judgments_equal_canonical(&valid_judgment(), &valid_judgment()),
            SpokeResult::Ok(true)
        ));
    }

    #[test]
    fn different_judgments_compare_unequal() {
        let mut right = valid_judgment();
        right["judgment_id"] = Value::String("j-2".into());
        assert!(matches!(
            judgments_equal_canonical(&valid_judgment(), &right),
            SpokeResult::Ok(false)
        ));
    }

    #[test]
    fn member_order_does_not_change_the_canonical_bytes() {
        let left = serde_json::json!({
            "judgment_id": "j-1",
            "judgment_kind": "review",
            "schema_version": 1,
            "target": {"id": "ke-1", "kind": "knowledge_entry"},
            "evidence": [],
            "verdict": {"outcome": "accepted", "detail": {"a": 1, "b": 2}},
            "extensions": {},
        });
        let right = serde_json::json!({
            "extensions": {},
            "verdict": {"detail": {"b": 2, "a": 1}, "outcome": "accepted"},
            "evidence": [],
            "target": {"kind": "knowledge_entry", "id": "ke-1"},
            "schema_version": 1,
            "judgment_kind": "review",
            "judgment_id": "j-1",
        });
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
            judgments_equal_canonical(&valid_judgment(), &right),
            SpokeResult::Ok(false)
        ));
    }

    #[test]
    fn present_empty_detail_differs_from_absent_detail() {
        // Wire presence is comparison-significant: a present-and-empty
        // verdict.detail is a distinct member from an absent detail.
        let mut left = valid_judgment();
        left["verdict"].as_object_mut().unwrap().remove("detail");
        let mut right = left.clone();
        right["verdict"]["detail"] = serde_json::json!({});
        assert!(matches!(
            judgments_equal_canonical(&left, &right),
            SpokeResult::Ok(false)
        ));
    }

    #[test]
    fn created_at_spelling_stays_comparison_significant() {
        // `created_at` compares by its exact wire spelling; the same instant
        // spelled with a non-UTC offset is a different canonical string.
        let mut right = valid_judgment();
        right["created_at"] = Value::String("2026-11-22T18:00:00+08:00".into());
        assert!(matches!(
            judgments_equal_canonical(&valid_judgment(), &right),
            SpokeResult::Ok(false)
        ));
    }
}
