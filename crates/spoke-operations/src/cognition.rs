//! Cognition wire-shape guards — Rust parity for the TypeScript C5 validators
//! (`packages/spoke-operations/src/cognition/validate.ts`) over the frozen
//! `schemas/data/cognition.schema.json` family.
//!
//! These guards check required/variant fields, closed/open keys, enums,
//! reference/tuple shape, target-chain/order consistency, non-empty
//! evidence/prior basis, category/assertion/metric compatibility, view
//! placement, filter grammar and projection selection trace only. They never
//! resolve references, judge evidence sufficiency, compute probabilities or
//! perform I/O. Fictional leap-second timestamps follow compass D18:
//! offset-adjusted structurally possible UTC month-end placement, no real
//! historical IERS lookup.

use crate::result::{spoke_ok_unit, spoke_reject, SpokeRejectCode, SpokeResult};
use chrono::{Datelike, Duration, NaiveDate, Timelike};
use serde_json::{json, Map, Value};
use spoke_schemas::data::cognition::{
    AcquisitionPath, BeliefAccess, BeliefAccessPredicate, BeliefContentType,
    BeliefContentTypePredicate, BeliefContext, BeliefContextPredicate, BeliefSourcePredicate,
    CognitionContext, CognitionFilter, CognitionFilterCategory, CognitionProjection,
    CognitionProjectionItem, CognitionProjectionItemAssessmentsItem,
    CognitionProjectionItemPresence, CognitionProjectionItemSelection,
    CognitionProjectionItemSelectionSelectedRecordId, CognitionReference, CognitionSelection,
    CognitionTarget, CognitiveView, ForkIdPredicate, Representation, RepresentationPredicate,
    Shared1, Truth, TruthPredicate,
};
use spoke_schemas::CognitionRecord;
use std::collections::HashMap;

/// Placement of a standard record: holder-side current authority
/// (`view = "current"`) or recorded-at-time Memory content
/// (`view = "recorded_at"`). `None` means placement-agnostic shape
/// validation (both views are enum-valid).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Placement {
    Authority,
    Memory,
}

// Closed vocabularies (cognition.schema.json).
const REFERENCE_KINDS: &[&str] = &[
    "knowledge_entry",
    "timeline_event",
    "memory_record",
    "memory_subject",
    "principal",
    "world",
    "external",
];
const TARGET_KINDS: &[&str] = &["proposition", "question", "perceived_content", "episode"];
const CALIBRATION_STATUSES: &[&str] = &["calibrated", "uncalibrated", "unknown"];
const ACQUISITION_PATHS: &[&str] = &[
    "Narration",
    "Perception",
    "Memory",
    "Testimony",
    "Inference",
    "Imagination",
    "Unknown",
];
const UNCERTAINTY_KINDS: &[&str] = &["subjective_credible", "statistical_confidence", "other"];
const ASSESSOR_ROLES: &[&str] = &["holder_self_report", "external_judgment", "product_assessment"];
const TRUTHS: &[&str] = &["True", "False", "Unknown"];
const ACCESSES: &[&str] = &["Private", "Shared", "Public"];
const REPRESENTATIONS: &[&str] = &["Explicit", "Implicit"];
const CONTENT_TYPES: &[&str] = &[
    "Location",
    "Contents/Physical State",
    "Identity/Relation",
    "Epistemic",
    "Desire/Intention",
    "Emotion",
    "Trait/Value",
    "Action/Event",
];
const BELIEF_CONTEXTS: &[&str] = &["Deceptive", "Temporal", "Counterfactual", "Neutral"];
const EPISTATES: &[&str] = &["known", "ignorant", "undetermined"];
const CATEGORIES: &[&str] = &["epistemic", "perception", "belief"];
const VIEWS: &[&str] = &["current", "recorded_at"];
const PRESENCES: &[&str] = &["assessed", "no_record"];
const EPISODE_KINDS: &[&str] = &["timeline_event", "memory_record", "external"];

// Fixed metric key → estimand mapping; the key is present only when its
// distinct estimate exists, and no cross-estimand arithmetic is inferred.
const METRIC_KEYS_ALL: &[&str] = &["b", "p", "f", "k", "r"];

fn key_estimand(key: &str) -> Option<&'static str> {
    match key {
        "b" => Some("holder_credence"),
        "p" => Some("capture"),
        "f" => Some("correspondence"),
        "k" => Some("knowledge_criterion"),
        "r" => Some("recording_fidelity"),
        _ => None,
    }
}

// Category-compatible metric keys: Belief permits b/r; Perception permits
// p/f/r; epistemic permits k/r.
fn category_metric_keys(category: &str) -> Option<&'static [&'static str]> {
    match category {
        "belief" => Some(&["b", "r"]),
        "perception" => Some(&["p", "f", "r"]),
        "epistemic" => Some(&["k", "r"]),
        _ => None,
    }
}

// q.assertion per epistemic state (no-record is a projection state, not an
// enum value).
fn epistemic_assertion(state: &str) -> Option<&'static str> {
    match state {
        "known" => Some("knowledge_criterion"),
        "ignorant" => Some("ignorance_criterion"),
        "undetermined" => Some("assessor_indeterminacy"),
        _ => None,
    }
}

// Closed-envelope property tables (additionalProperties: false).
const REFERENCE_KEYS: &[&str] = &["kind", "id", "namespace"];
const TARGET_KEYS: &[&str] = &["carrier", "target_id", "kind", "text", "about"];
const CONTEXT_KEYS: &[&str] = &["context_id", "fork_id"];
const CRITERION_KEYS: &[&str] = &["criterion_id", "expression"];
const BASIS_KEYS: &[&str] = &["kind", "evidence", "description", "acquisition"];
const UNCERTAINTY_KEYS: &[&str] = &["kind", "lower", "upper", "level", "description"];
const Q_KEYS: &[&str] = &["value", "assertion", "method", "calibration", "uncertainty"];
const METRIC_KEYS: &[&str] = &[
    "value",
    "estimand",
    "assessor",
    "assessor_role",
    "basis",
    "context",
    "effective_at",
    "assessed_at",
    "method",
    "criterion",
    "calibration",
    "uncertainty",
];
const METRICS_KEYS: &[&str] = &["b", "p", "f", "k", "r"];
const BELIEF_KEYS: &[&str] = &[
    "kind",
    "record_id",
    "holder",
    "target",
    "assessor",
    "assessor_role",
    "basis",
    "assessment_context",
    "effective_at",
    "assessed_at",
    "view",
    "criterion",
    "q",
    "metrics",
    "extensions",
    "order",
    "truth",
    "access",
    "representation",
    "content_type",
    "source",
    "context",
];
const EPISTEMIC_KEYS: &[&str] = &[
    "kind",
    "record_id",
    "holder",
    "target",
    "assessor",
    "assessor_role",
    "basis",
    "assessment_context",
    "effective_at",
    "assessed_at",
    "view",
    "state",
    "criterion",
    "classification_policy",
    "q",
    "metrics",
    "extensions",
];
const PERCEPTION_KEYS: &[&str] = &[
    "kind",
    "record_id",
    "holder",
    "target",
    "assessor",
    "assessor_role",
    "basis",
    "assessment_context",
    "effective_at",
    "assessed_at",
    "view",
    "episode",
    "content",
    "access",
    "criterion",
    "q",
    "metrics",
    "extensions",
];
const WORLD_FACT_KEYS: &[&str] = &[
    "fact_id",
    "holder",
    "target",
    "order",
    "truth",
    "access",
    "representation",
    "content_type",
    "source",
    "context",
    "assessment_context",
    "effective_at",
    "recorded_at",
    "extensions",
];
const MODULE_KEYS: &[&str] = &["records", "world_facts", "extensions"];
const OBSERVATION_MODULE_KEYS: &[&str] = &["observers", "access"];
const FILTER_KEYS: &[&str] = &[
    "holder",
    "target",
    "assessment_context",
    "category",
    "state",
    "q",
    "order",
    "truth",
    "access",
    "representation",
    "content_type",
    "source",
    "belief_context",
];
const TARGET_PREDICATE_KEYS: &[&str] = &["carrier", "target_id", "kind"];
const CONTEXT_PREDICATE_KEYS: &[&str] = &["context_id", "fork_id"];
const QRANGE_KEYS: &[&str] = &["min", "max"];
const POLICY_KEYS: &[&str] = &["policy_id", "policy_version"];
const SELECTION_TRACE_KEYS: &[&str] = &["policy", "input_record_ids", "selected_record_id"];
const PROJECTION_KEYS: &[&str] = &["view", "items", "extensions"];
const PROJECTION_ITEM_KEYS: &[&str] = &[
    "holder",
    "target",
    "assessment_context",
    "presence",
    "assessments",
    "selection",
];

fn is_plain_object(value: &Value) -> bool {
    value.is_object()
}

// Mirrors the JavaScript `String.prototype.trim` character set exactly, so
// prior-basis and uncertainty-description non-emptiness agrees with the TS
// guard: Unicode White_Space plus U+FEFF, but not U+0085 (which
// `char::is_whitespace` alone would include and JS does not trim).
fn is_js_trim_whitespace(c: char) -> bool {
    c == '\u{FEFF}' || (c.is_whitespace() && c != '\u{0085}')
}

fn is_non_empty_trimmed_string(value: &str) -> bool {
    !value.trim_matches(is_js_trim_whitespace).is_empty()
}

fn has_only_keys(value: &Map<String, Value>, keys: &[&str]) -> bool {
    value.keys().all(|key| keys.contains(&key.as_str()))
}

// Required identity realm for every reference kind: `^[a-z][a-z0-9_-]*$`.
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

fn is_extension_map(value: &Value) -> bool {
    let Some(entries) = value.as_object() else {
        return false;
    };
    entries
        .iter()
        .all(|(key, entry)| is_namespace(key) && is_plain_object(entry))
}

// Finite declared probability in the closed 0..1 range; 0 and 1 are admitted
// only as deliberately assigned values, never label defaults.
fn is_probability01(value: &Value) -> bool {
    value
        .as_f64()
        .is_some_and(|number| number.is_finite() && (0.0..=1.0).contains(&number))
}

// Complete (kind, namespace, id) tuple identity; shape only — no resolution.
//
// Every `*_problem` below returns the relative path of the first member that
// fails that grammar, or `None` when the value passes. A failure with no
// member path (`Problem::Here`) names the checked value itself, so a
// cross-field failure (reversed `q` bounds, a predicate with two arms) or a
// non-object keeps its enclosing member's path. Member names are rendered
// where they are discovered: an ordinary identifier stays dotted
// (`holder.id`), any other name keeps its JSON-quoted bracket form
// (`q[""]`, `q["a.b"]`), so an empty member never collides with the enclosing
// object's own failure. Paths are built only on failure, and one traversal
// walks the original value: no probe copies and no second schema map.

/// Failure location relative to the checked value.
enum Problem {
    /// The checked value itself failed.
    Here,
    /// Rendered relative path of the failing member.
    At(String),
}

/// A failure at a known schema member, which is always an identifier.
fn at(name: &str) -> Problem {
    Problem::At(name.to_owned())
}

/// Render an arbitrary member name: identifiers stay plain, everything else
/// keeps the JSON-quoted bracket form (`[""]`, `["a.b"]`).
fn member_path(name: &str) -> String {
    let mut chars = name.chars();
    let identifier = matches!(chars.next(), Some('_' | 'a'..='z' | 'A'..='Z'))
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric());
    if identifier {
        name.to_owned()
    } else {
        // A borrowed one-element array serializes to the exact bracketed token.
        serde_json::to_string(&[name]).expect("a string member name serializes")
    }
}

fn child_path(name: &str, problem: Problem) -> Problem {
    match problem {
        Problem::Here => Problem::At(name.to_owned()),
        // A rendered non-identifier member already carries its brackets.
        Problem::At(path) if path.starts_with('[') => Problem::At(format!("{name}{path}")),
        Problem::At(path) => Problem::At(format!("{name}.{path}")),
    }
}

fn cognition_reference_problem(value: &Value) -> Option<Problem> {
    let Some(reference) = value.as_object() else {
        return Some(Problem::Here);
    };
    if let Some(unknown) = first_unknown_key(reference, &REFERENCE_KEYS) {
        return Some(Problem::At(member_path(unknown)));
    }
    if !reference
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| REFERENCE_KINDS.contains(&kind))
    {
        return Some(at("kind"));
    }
    if !reference
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty())
    {
        return Some(at("id"));
    }
    if !reference
        .get("namespace")
        .and_then(Value::as_str)
        .is_some_and(is_namespace)
    {
        return Some(at("namespace"));
    }
    None
}

fn is_cognition_reference(value: &Value) -> bool {
    cognition_reference_problem(value).is_none()
}

fn first_unknown_key<'a>(value: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    value
        .keys()
        .find(|key| !keys.contains(&key.as_str()))
        .map(String::as_str)
}

/// The relative problem of an optional member, prefixed with its key.
fn child_problem(
    value: &Map<String, Value>,
    key: &str,
    check: impl Fn(&Value) -> Option<Problem>,
) -> Option<Problem> {
    let child = value.get(key)?;
    check(child).map(|problem| child_path(key, problem))
}

// Target chain never exceeds 3; `depth` counts nested `about` links.
fn is_cognition_target(value: &Value, depth: u32) -> bool {
    if depth > 2 {
        return false;
    }
    let Some(target) = value.as_object() else {
        return false;
    };
    if !has_only_keys(target, &TARGET_KEYS) {
        return false;
    }
    if !is_cognition_reference(target.get("carrier").unwrap_or(&Value::Null)) {
        return false;
    }
    if !target
        .get("target_id")
        .and_then(Value::as_str)
        .is_some_and(|target_id| !target_id.is_empty())
    {
        return false;
    }
    if !target
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| TARGET_KINDS.contains(&kind))
    {
        return false;
    }
    if let Some(text) = target.get("text") {
        if !text.is_string() {
            return false;
        }
    }
    if let Some(about) = target.get("about") {
        if !is_cognition_target(about, depth + 1) {
            return false;
        }
    }
    true
}

fn target_chain_length(value: &Map<String, Value>) -> usize {
    match value.get("about").and_then(Value::as_object) {
        Some(about) => 1 + target_chain_length(about),
        None => 1,
    }
}

fn is_cognition_context(value: &Value) -> bool {
    let Some(context) = value.as_object() else {
        return false;
    };
    if !has_only_keys(context, &CONTEXT_KEYS) {
        return false;
    }
    if !context
        .get("context_id")
        .and_then(Value::as_str)
        .is_some_and(|context_id| !context_id.is_empty())
    {
        return false;
    }
    match context.get("fork_id") {
        None => true,
        Some(fork_id) => fork_id.as_str().is_some_and(|fork_id| !fork_id.is_empty()),
    }
}

fn is_cognition_criterion(value: &Value) -> bool {
    let Some(criterion) = value.as_object() else {
        return false;
    };
    if !has_only_keys(criterion, &CRITERION_KEYS) {
        return false;
    }
    if !criterion
        .get("criterion_id")
        .and_then(Value::as_str)
        .is_some_and(|criterion_id| !criterion_id.is_empty())
    {
        return false;
    }
    match criterion.get("expression") {
        None => true,
        Some(expression) => expression.is_string(),
    }
}

// Every completed standard assessment/metric has a non-empty evidence basis
// or an explicit declared prior basis; a declared basis may remain
// uncalibrated and supplies no default probability, interval or calibration.
fn is_assessment_basis(value: &Value) -> bool {
    let Some(basis) = value.as_object() else {
        return false;
    };
    if !has_only_keys(basis, &BASIS_KEYS) {
        return false;
    }
    if let Some(acquisition) = basis.get("acquisition") {
        if !acquisition
            .as_str()
            .is_some_and(|path| ACQUISITION_PATHS.contains(&path))
        {
            return false;
        }
    }
    match basis.get("kind").and_then(Value::as_str) {
        Some("evidence") => {
            basis.get("description").is_none()
                && basis
                    .get("evidence")
                    .and_then(Value::as_array)
                    .is_some_and(|evidence| {
                        !evidence.is_empty() && evidence.iter().all(is_cognition_reference)
                    })
        }
        Some("prior") => {
            basis.get("evidence").is_none()
                && basis
                    .get("description")
                    .and_then(Value::as_str)
                    .is_some_and(is_non_empty_trimmed_string)
        }
        _ => false,
    }
}

// 0 <= lower <= upper <= 1; level strictly inside (0, 1); kind "other"
// requires an explicit description. Unquantified uncertainty is an absent
// `uncertainty`, never a fabricated zero-width interval.
fn is_probability_uncertainty(value: &Value) -> bool {
    let Some(uncertainty) = value.as_object() else {
        return false;
    };
    if !has_only_keys(uncertainty, &UNCERTAINTY_KEYS) {
        return false;
    }
    let Some(kind) = uncertainty.get("kind").and_then(Value::as_str) else {
        return false;
    };
    if !UNCERTAINTY_KINDS.contains(&kind) {
        return false;
    }
    let lower = uncertainty.get("lower").and_then(Value::as_f64);
    let upper = uncertainty.get("upper").and_then(Value::as_f64);
    let Some(lower) = lower.filter(|value| is_probability01_value(*value)) else {
        return false;
    };
    let Some(upper) = upper.filter(|value| is_probability01_value(*value)) else {
        return false;
    };
    if lower > upper {
        return false;
    }
    if let Some(level) = uncertainty.get("level") {
        let level_ok = level
            .as_f64()
            .is_some_and(|level| level.is_finite() && level > 0.0 && level < 1.0);
        if !level_ok {
            return false;
        }
    }
    if kind == "other" {
        return uncertainty
            .get("description")
            .and_then(Value::as_str)
            .is_some_and(is_non_empty_trimmed_string);
    }
    match uncertainty.get("description") {
        None => true,
        Some(description) => description.is_string(),
    }
}

fn is_probability01_value(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

struct CognitionTimestamp {
    /// Local date-time with the leap second (if any) already clamped to :59.
    naive: chrono::NaiveDateTime,
    leap_second: bool,
    /// Declared fixed offset east of UTC in minutes (`+` = local ahead).
    offset_east_minutes: i32,
}

fn take_digits(bytes: &[u8], start: usize, count: usize) -> Option<u32> {
    let mut value = 0u32;
    for offset in 0..count {
        let byte = *bytes.get(start + offset)?;
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value * 10 + u32::from(byte - b'0');
    }
    Some(value)
}

fn byte_is(bytes: &[u8], position: usize, expected: u8) -> bool {
    bytes.get(position) == Some(&expected)
}

// Strict RFC 3339 with mandatory offset: `YYYY-MM-DD[Tt]HH:MM:SS[.frac]
// ([Zz]|[+-]HH:MM)`, calendar-valid, offset hour <= 23, offset minute <= 59,
// seconds <= 60 (leap second admitted structurally, checked by `is_timestamp`).
fn parse_cognition_timestamp(value: &str) -> Option<CognitionTimestamp> {
    let bytes = value.as_bytes();
    if bytes.len() < 20 {
        return None;
    }
    if !byte_is(bytes, 4, b'-')
        || !byte_is(bytes, 7, b'-')
        || !(byte_is(bytes, 10, b'T') || byte_is(bytes, 10, b't'))
        || !byte_is(bytes, 13, b':')
        || !byte_is(bytes, 16, b':')
    {
        return None;
    }
    let year = take_digits(bytes, 0, 4)?;
    let month = take_digits(bytes, 5, 2)?;
    let day = take_digits(bytes, 8, 2)?;
    let hour = take_digits(bytes, 11, 2)?;
    let minute = take_digits(bytes, 14, 2)?;
    let second = take_digits(bytes, 17, 2)?;

    let mut index = 19;
    if byte_is(bytes, index, b'.') {
        index += 1;
        let fraction_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == fraction_start {
            return None;
        }
    }
    let offset_east_minutes = match bytes.get(index) {
        Some(b'Z') | Some(b'z') => {
            index += 1;
            0
        }
        Some(sign @ (b'+' | b'-')) => {
            let offset_hour = take_digits(bytes, index + 1, 2)?;
            if !byte_is(bytes, index + 3, b':') {
                return None;
            }
            let offset_minute = take_digits(bytes, index + 4, 2)?;
            index += 6;
            if offset_hour > 23 || offset_minute > 59 {
                return None;
            }
            let total = (offset_hour * 60 + offset_minute) as i32;
            if *sign == b'-' {
                -total
            } else {
                total
            }
        }
        _ => return None,
    };
    if index != bytes.len() {
        return None;
    }

    let date = NaiveDate::from_ymd_opt(year as i32, month, day)?;
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let naive = date.and_hms_opt(hour, minute, second.min(59))?;
    Some(CognitionTimestamp {
        naive,
        leap_second: second == 60,
        offset_east_minutes,
    })
}

fn is_timestamp(value: &Value) -> bool {
    let Some(timestamp) = value.as_str() else {
        return false;
    };
    if timestamp != timestamp.trim() {
        return false;
    }
    let Some(parsed) = parse_cognition_timestamp(timestamp) else {
        return false;
    };
    if !parsed.leap_second {
        return true;
    }
    // Fictional leap seconds (compass D18) need a UTC month boundary after
    // offset adjustment, not a historical IERS lookup.
    let utc = parsed.naive - Duration::minutes(i64::from(parsed.offset_east_minutes));
    utc.hour() == 23
        && utc.minute() == 59
        && (utc + Duration::seconds(1)).day() == 1
}

// A single metric whose estimand is fixed by its enclosing metrics key.
fn is_cognition_metric(value: &Value, estimand: &str) -> bool {
    let Some(metric) = value.as_object() else {
        return false;
    };
    if !has_only_keys(metric, &METRIC_KEYS) {
        return false;
    }
    if !is_probability01(metric.get("value").unwrap_or(&Value::Null)) {
        return false;
    }
    if metric.get("estimand").and_then(Value::as_str) != Some(estimand) {
        return false;
    }
    if !metric
        .get("assessor")
        .is_some_and(is_cognition_reference)
    {
        return false;
    }
    if !metric
        .get("assessor_role")
        .and_then(Value::as_str)
        .is_some_and(|role| ASSESSOR_ROLES.contains(&role))
    {
        return false;
    }
    if !metric.get("basis").is_some_and(is_assessment_basis) {
        return false;
    }
    if !metric.get("context").is_some_and(is_cognition_context) {
        return false;
    }
    if !metric.get("effective_at").is_some_and(is_timestamp) {
        return false;
    }
    if !metric.get("assessed_at").is_some_and(is_timestamp) {
        return false;
    }
    if !metric
        .get("method")
        .and_then(Value::as_str)
        .is_some_and(|method| !method.is_empty())
    {
        return false;
    }
    if !metric
        .get("calibration")
        .and_then(Value::as_str)
        .is_some_and(|status| CALIBRATION_STATUSES.contains(&status))
    {
        return false;
    }
    if let Some(uncertainty) = metric.get("uncertainty") {
        if !is_probability_uncertainty(uncertainty) {
            return false;
        }
    }
    // criterion is required for knowledge_criterion/correspondence estimands.
    if estimand == "knowledge_criterion" || estimand == "correspondence" {
        return metric.get("criterion").is_some_and(is_cognition_criterion);
    }
    match metric.get("criterion") {
        None => true,
        Some(criterion) => is_cognition_criterion(criterion),
    }
}

// Category-compatible metric keys only, each with its fixed estimand; absent
// keys are ordinary missing data, never zero or unsupported features.
fn is_cognition_metrics(value: &Value, category: &str) -> bool {
    let Some(metrics) = value.as_object() else {
        return false;
    };
    if !has_only_keys(metrics, &METRICS_KEYS) {
        return false;
    }
    let Some(allowed) = category_metric_keys(category) else {
        return false;
    };
    for &key in METRIC_KEYS_ALL {
        let Some(metric) = metrics.get(key) else {
            continue;
        };
        if !allowed.contains(&key) {
            return false;
        }
        let Some(estimand) = key_estimand(key) else {
            return false;
        };
        if !is_cognition_metric(metric, estimand) {
            return false;
        }
    }
    true
}

fn same_json(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right.iter())
                    .all(|(left, right)| same_json(left, right))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .all(|(key, value)| right.get(key).is_some_and(|other| same_json(value, other)))
        }
        _ => match (a.as_f64(), b.as_f64()) {
            (Some(left), Some(right)) => left == right,
            _ => a == b,
        },
    }
}

// Missing properties are `undefined` in the TS guard and compare distinct
// from an explicit null; mirror that with Option-level equality.
fn same_json_opt(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(left), Some(right)) => same_json(left, right),
        _ => false,
    }
}

fn duplicate_estimate(record: &Map<String, Value>, category: &str) -> bool {
    let (Some(metrics), Some(q)) = (
        record.get("metrics").and_then(Value::as_object),
        record.get("q").and_then(Value::as_object),
    ) else {
        return false;
    };
    let key = match category {
        "perception" => Some("p"),
        "epistemic"
            if matches!(
                record.get("state").and_then(Value::as_str),
                Some("known") | Some("ignorant")
            ) =>
        {
            Some("k")
        }
        _ => None,
    };
    let Some(key) = key else {
        return false;
    };
    let Some(metric) = metrics.get(key).and_then(Value::as_object) else {
        return false;
    };
    ["value", "method", "calibration", "uncertainty"]
        .iter()
        .all(|field| same_json_opt(q.get(*field), metric.get(*field)))
        && ["assessor", "assessor_role", "basis", "effective_at", "assessed_at"]
            .iter()
            .all(|field| same_json_opt(record.get(*field), metric.get(*field)))
        && same_json_opt(record.get("assessment_context"), metric.get("context"))
        && (key != "k" || same_json_opt(record.get("criterion"), metric.get("criterion")))
}

// Shared required members of the three completed standard record variants.
// The holder is an actor/group and never kind = world.
fn is_assessment_members(
    value: &Map<String, Value>,
    category: &str,
    assertion: &str,
    placement: Option<Placement>,
) -> bool {
    if !value
        .get("record_id")
        .and_then(Value::as_str)
        .is_some_and(|record_id| !record_id.is_empty())
    {
        return false;
    }
    let holder_ok = value.get("holder").is_some_and(|holder| {
        is_cognition_reference(holder)
            && holder
                .get("kind")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind != "world")
    });
    if !holder_ok {
        return false;
    }
    if !is_cognition_target(value.get("target").unwrap_or(&Value::Null), 0) {
        return false;
    }
    if !value.get("assessor").is_some_and(is_cognition_reference) {
        return false;
    }
    if !value
        .get("assessor_role")
        .and_then(Value::as_str)
        .is_some_and(|role| ASSESSOR_ROLES.contains(&role))
    {
        return false;
    }
    if !value.get("basis").is_some_and(is_assessment_basis) {
        return false;
    }
    if !value
        .get("assessment_context")
        .is_some_and(is_cognition_context)
    {
        return false;
    }
    if !value.get("effective_at").is_some_and(is_timestamp) {
        return false;
    }
    if !value.get("assessed_at").is_some_and(is_timestamp) {
        return false;
    }
    let view_ok = match value.get("view").and_then(Value::as_str) {
        Some(view) if VIEWS.contains(&view) => match placement {
            None => true,
            Some(Placement::Authority) => view == "current",
            Some(Placement::Memory) => view == "recorded_at",
        },
        _ => false,
    };
    if !view_ok {
        return false;
    }
    if let Some(criterion) = value.get("criterion") {
        if !is_cognition_criterion(criterion) {
            return false;
        }
    }
    if !value
        .get("q")
        .is_some_and(|q| is_declared_probability(q, assertion))
    {
        return false;
    }
    if let Some(metrics) = value.get("metrics") {
        if !is_cognition_metrics(metrics, category) {
            return false;
        }
    }
    !duplicate_estimate(value, category) && is_extension_map(value.get("extensions").unwrap_or(&Value::Null))
}

fn is_declared_probability(value: &Value, assertion: &str) -> bool {
    let Some(q) = value.as_object() else {
        return false;
    };
    if !has_only_keys(q, &Q_KEYS) {
        return false;
    }
    if !is_probability01(q.get("value").unwrap_or(&Value::Null)) {
        return false;
    }
    if q.get("assertion").and_then(Value::as_str) != Some(assertion) {
        return false;
    }
    if !q
        .get("method")
        .and_then(Value::as_str)
        .is_some_and(|method| !method.is_empty())
    {
        return false;
    }
    if !q
        .get("calibration")
        .and_then(Value::as_str)
        .is_some_and(|status| CALIBRATION_STATUSES.contains(&status))
    {
        return false;
    }
    match q.get("uncertainty") {
        None => true,
        Some(uncertainty) => is_probability_uncertainty(uncertainty),
    }
}

// Retained optional Belief dimensions; every optional dimension is a closed
// enum. Absence is always valid; presence must be a member.
fn optional_enum_member(value: &Map<String, Value>, key: &str, members: &[&str]) -> bool {
    match value.get(key) {
        None => true,
        Some(entry) => entry.as_str().is_some_and(|member| members.contains(&member)),
    }
}

fn is_belief_record(value: &Map<String, Value>, placement: Option<Placement>) -> bool {
    if !has_only_keys(value, &BELIEF_KEYS) {
        return false;
    }
    if value.get("kind").and_then(Value::as_str) != Some("belief") {
        return false;
    }
    let Some(order) = value.get("order").and_then(Value::as_f64) else {
        return false;
    };
    if order.fract() != 0.0 || !(0.0..=3.0).contains(&order) {
        return false;
    }
    if !optional_enum_member(value, "truth", &TRUTHS)
        || !optional_enum_member(value, "access", &ACCESSES)
        || !optional_enum_member(value, "representation", REPRESENTATIONS)
        || !optional_enum_member(value, "content_type", CONTENT_TYPES)
        || !optional_enum_member(value, "source", ACQUISITION_PATHS)
        || !optional_enum_member(value, "context", BELIEF_CONTEXTS)
    {
        return false;
    }
    if !is_cognition_target(value.get("target").unwrap_or(&Value::Null), 0) {
        return false;
    }
    let Some(target) = value.get("target").and_then(Value::as_object) else {
        return false;
    };
    if target.get("kind").and_then(Value::as_str) != Some("proposition") {
        return false;
    }
    let chain = target_chain_length(target);
    if chain > 3 {
        return false;
    }
    // order 1: about absent; order 2/3: about present and chain depth equals
    // the order. order 0 keeps the shared vocabulary without a chain rule.
    if order == 1.0 && target.contains_key("about") {
        return false;
    }
    if (order == 2.0 || order == 3.0)
        && (!target.contains_key("about") || chain as f64 != order)
    {
        return false;
    }
    is_assessment_members(value, "belief", "belief_attribution", placement)
}

fn is_selection_policy_identity(value: &Value) -> bool {
    selection_policy_problem(value).is_none()
}

fn is_epistemic_assessment(value: &Map<String, Value>, placement: Option<Placement>) -> bool {
    if !has_only_keys(value, &EPISTEMIC_KEYS) {
        return false;
    }
    let Some(state) = value.get("state").and_then(Value::as_str) else {
        return false;
    };
    if value.get("kind").and_then(Value::as_str) != Some("epistemic")
        || !EPISTATES.contains(&state)
        || !value.get("criterion").is_some_and(is_cognition_criterion)
    {
        return false;
    }
    let target_ok = value.get("target").is_some_and(|target| {
        target.as_object().is_some_and(|target| {
            matches!(
                target.get("kind").and_then(Value::as_str),
                Some("proposition") | Some("question")
            )
        })
    });
    if !target_ok {
        return false;
    }
    if !value
        .get("classification_policy")
        .is_some_and(is_selection_policy_identity)
    {
        return false;
    }
    let Some(assertion) = epistemic_assertion(state) else {
        return false;
    };
    is_assessment_members(value, "epistemic", assertion, placement)
}

// Actual capture constraints if known; open — unknown keys round-trip verbatim.
fn is_observation_access(value: &Value) -> bool {
    let Some(access) = value.as_object() else {
        return false;
    };
    if let Some(position) = access.get("position") {
        if !position.is_string() && !position.is_object() {
            return false;
        }
    }
    for field in ["line_of_sight", "hearing_range"] {
        if let Some(field_value) = access.get(field) {
            if !field_value.is_boolean() && !field_value.is_object() {
                return false;
            }
        }
    }
    match access.get("modality") {
        None => true,
        Some(modality) => modality
            .as_array()
            .is_some_and(|entries| entries.iter().all(Value::is_string)),
    }
}

fn is_perception_record(value: &Map<String, Value>, placement: Option<Placement>) -> bool {
    if !has_only_keys(value, &PERCEPTION_KEYS) {
        return false;
    }
    if value.get("kind").and_then(Value::as_str) != Some("perception") {
        return false;
    }
    let episode_ok = value.get("episode").is_some_and(|episode| {
        is_cognition_reference(episode)
            && episode
                .get("kind")
                .and_then(Value::as_str)
                .is_some_and(|kind| EPISODE_KINDS.contains(&kind))
    });
    if !episode_ok {
        return false;
    }
    if !value.get("content").is_some() {
        return false;
    }
    if !is_cognition_target(value.get("target").unwrap_or(&Value::Null), 0) {
        return false;
    }
    let Some(target) = value.get("target").and_then(Value::as_object) else {
        return false;
    };
    if !matches!(
        target.get("kind").and_then(Value::as_str),
        Some("perceived_content") | Some("episode")
    ) {
        return false;
    }
    // Perception target and episode share the complete carrier reference.
    let carrier_ok = target
        .get("carrier")
        .and_then(Value::as_object)
        .zip(value.get("episode").and_then(Value::as_object))
        .is_some_and(|(carrier, episode)| {
            ["kind", "namespace", "id"].iter().all(|field| {
                carrier.get(*field).and_then(Value::as_str)
                    == episode.get(*field).and_then(Value::as_str)
            })
        });
    if !carrier_ok {
        return false;
    }
    if let Some(access) = value.get("access") {
        if !is_observation_access(access) {
            return false;
        }
    }
    is_assessment_members(value, "perception", "actual_capture", placement)
}

// Wire-shape gate for a closed standard cognition record (Belief, epistemic
// or Perception variant). When `placement` is supplied, the record `view`
// must match it; without a placement both views are shape-valid.
fn validate_placed_record(value: &Value, placement: Option<Placement>) -> bool {
    let Some(record) = value.as_object() else {
        return false;
    };
    match record.get("kind").and_then(Value::as_str) {
        Some("belief") => is_belief_record(record, placement),
        Some("epistemic") => is_epistemic_assessment(record, placement),
        Some("perception") => is_perception_record(record, placement),
        _ => false,
    }
}

/// Wire-shape gate for a closed standard cognition record (Belief, epistemic
/// or Perception variant), placement-agnostic on `view`.
pub fn validate_cognition_record(value: &Value) -> SpokeResult<()> {
    gate(
        validate_placed_record(value, None),
        "cognition record failed wire-shape validation",
    )
}

// `record_id` is unique within one carrier (§3) and nowhere else, so the same
// id may still identify different records in another carrier.
fn all_distinct(mut ids: Vec<&str>) -> bool {
    let total = ids.len();
    ids.sort_unstable();
    ids.dedup();
    ids.len() == total
}

// One carrier's record list: every entry is a placed record of that carrier's
// view and the ids are distinct, so the selection trace can name one record.
fn carrier_records_valid(records: &[Value], view: &str) -> bool {
    let mut ids = Vec::with_capacity(records.len());
    for record in records {
        if !validate_placed_record(record, None)
            || record.get("view").and_then(Value::as_str) != Some(view)
        {
            return false;
        }
        let Some(record_id) = record.get("record_id").and_then(Value::as_str) else {
            return false;
        };
        ids.push(record_id);
    }
    all_distinct(ids)
}

/// Wire-shape gate over Content-like values: when `cognition` is present,
/// every entry must be a valid `CognitionRecord` with `view = "recorded_at"`
/// and a distinct `record_id` within this carrier; when absent the guard
/// accepts. No byte measurement, no persistence.
pub fn validate_cognition_content(value: &Value) -> SpokeResult<()> {
    let Some(content) = value.as_object() else {
        return gate(false, "Content must be a non-null plain object");
    };
    let ok = match content.get("cognition") {
        None => true,
        Some(records) => records
            .as_array()
            .is_some_and(|records| carrier_records_valid(records, "recorded_at")),
    };
    gate(ok, "Content cognition entries must be recorded-at-time records")
}

fn is_world_fact_record(value: &Value) -> bool {
    let Some(fact) = value.as_object() else {
        return false;
    };
    if !has_only_keys(fact, &WORLD_FACT_KEYS) {
        return false;
    }
    // Narrated order-0 facts: the holder is the designated world authority.
    if !fact
        .get("fact_id")
        .and_then(Value::as_str)
        .is_some_and(|fact_id| !fact_id.is_empty())
    {
        return false;
    }
    let holder_ok = fact.get("holder").is_some_and(|holder| {
        is_cognition_reference(holder)
            && holder
                .get("kind")
                .and_then(Value::as_str)
                == Some("world")
    });
    if !holder_ok {
        return false;
    }
    if !is_cognition_target(fact.get("target").unwrap_or(&Value::Null), 0) {
        return false;
    }
    let Some(target) = fact.get("target").and_then(Value::as_object) else {
        return false;
    };
    if target.get("kind").and_then(Value::as_str) != Some("proposition") {
        return false;
    }
    if fact.get("order").and_then(Value::as_f64) != Some(0.0) {
        return false;
    }
    if !optional_enum_member(fact, "truth", &TRUTHS)
        || !optional_enum_member(fact, "access", &ACCESSES)
        || !optional_enum_member(fact, "representation", REPRESENTATIONS)
        || !optional_enum_member(fact, "content_type", CONTENT_TYPES)
        || !optional_enum_member(fact, "source", ACQUISITION_PATHS)
        || !optional_enum_member(fact, "context", BELIEF_CONTEXTS)
    {
        return false;
    }
    if !fact
        .get("assessment_context")
        .is_some_and(is_cognition_context)
    {
        return false;
    }
    if !fact.get("effective_at").is_some_and(is_timestamp) {
        return false;
    }
    if !fact.get("recorded_at").is_some_and(is_timestamp) {
        return false;
    }
    if !is_extension_map(fact.get("extensions").unwrap_or(&Value::Null)) {
        return false;
    }
    target_chain_length(target) <= 3
}

/// Wire-shape gate for the KnowledgeEntry `modules.cognition` carrier: closed
/// `{records, world_facts, extensions}` with distinct-id current-view actor
/// records and order-0 world facts whose holder is kind = "world".
pub fn validate_cognition_module(value: &Value) -> SpokeResult<()> {
    let Some(module) = value.as_object() else {
        return gate(false, "cognition module must be a non-null plain object");
    };
    if !has_only_keys(module, &MODULE_KEYS) {
        return gate(false, "cognition module has unknown property");
    }
    let records_ok = module
        .get("records")
        .and_then(Value::as_array)
        .is_some_and(|records| carrier_records_valid(records, "current"));
    let world_facts_ok = module
        .get("world_facts")
        .and_then(Value::as_array)
        .is_some_and(|facts| facts.iter().all(is_world_fact_record));
    let ok = records_ok
        && world_facts_ok
        && is_extension_map(module.get("extensions").unwrap_or(&Value::Null));
    gate(ok, "cognition module failed wire-shape validation")
}

/// Wire-shape gate for the TimelineEvent `modules.observation` carrier.
/// `observers` is required and `[]` is the explicit empty could-perceive set
/// (distinct from a missing module and from an actual Perception record);
/// `access` is open and unknown keys round-trip verbatim.
pub fn validate_observation_module(value: &Value) -> SpokeResult<()> {
    let Some(module) = value.as_object() else {
        return gate(false, "observation module must be a non-null plain object");
    };
    if !has_only_keys(module, &OBSERVATION_MODULE_KEYS) {
        return gate(false, "observation module has unknown property");
    }
    let observers_ok = module
        .get("observers")
        .and_then(Value::as_array)
        .is_some_and(|observers| observers.iter().all(is_cognition_reference));
    let access_ok = match module.get("access") {
        None => true,
        Some(access) => is_observation_access(access),
    };
    gate(
        observers_ok && access_ok,
        "observation module failed wire-shape validation",
    )
}

// Categorical predicate: exactly one of {eq: <enum member>} or {absent: true}.
fn categorical_predicate_problem(value: &Value, members: &[&str]) -> Option<Problem> {
    let Some(predicate) = value.as_object() else {
        return Some(Problem::Here);
    };
    // A genuine unknown member outranks the arm count: name it where it is.
    if let Some(unknown) = predicate
        .keys()
        .find(|key| key.as_str() != "eq" && key.as_str() != "absent")
    {
        return Some(Problem::At(member_path(unknown)));
    }
    if predicate.len() != 1 {
        return Some(Problem::Here);
    }
    let (key, arm) = predicate.iter().next().expect("one member checked above");
    match key.as_str() {
        "absent" => (arm.as_bool() != Some(true)).then(|| at("absent")),
        "eq" => {
            if arm.as_str().is_some_and(|member| members.contains(&member)) {
                None
            } else {
                Some(at("eq"))
            }
        }
        _ => Some(Problem::At(member_path(key))),
    }
}

// Closed fork predicate: exactly one of {eq: <non-empty fork id>} or
// {absent: true}. An omitted predicate is a wildcard and valid.
fn fork_id_predicate_problem(value: &Value) -> Option<Problem> {
    let Some(predicate) = value.as_object() else {
        return Some(Problem::Here);
    };
    // A genuine unknown member outranks the arm count: name it where it is.
    if let Some(unknown) = predicate
        .keys()
        .find(|key| key.as_str() != "eq" && key.as_str() != "absent")
    {
        return Some(Problem::At(member_path(unknown)));
    }
    if predicate.len() != 1 {
        return Some(Problem::Here);
    }
    let (key, arm) = predicate.iter().next().expect("one member checked above");
    match key.as_str() {
        "absent" => (arm.as_bool() != Some(true)).then(|| at("absent")),
        "eq" => {
            if arm.as_str().is_some_and(|fork_id| !fork_id.is_empty()) {
                None
            } else {
                Some(at("eq"))
            }
        }
        _ => Some(Problem::At(member_path(key))),
    }
}

// Inclusive range over the declared foundational q.value; at least one bound
// is required and min <= max. Uncalibrated but declared values participate
// identically — the validator checks the grammar, not the semantics.
fn q_range_problem(value: &Value) -> Option<Problem> {
    let Some(range) = value.as_object() else {
        return Some(Problem::Here);
    };
    if let Some(unknown) = first_unknown_key(range, &QRANGE_KEYS) {
        return Some(Problem::At(member_path(unknown)));
    }
    // A present bound must be a number in 0..=1; only an absent key is
    // skipped. Presence is checked separately so a present `null`/string
    // bound cannot silently behave as an absent one (TS `!== undefined`).
    let min = range
        .get("min")
        .and_then(Value::as_f64)
        .filter(|bound| is_probability01_value(*bound));
    let max = range
        .get("max")
        .and_then(Value::as_f64)
        .filter(|bound| is_probability01_value(*bound));
    if range.contains_key("min") && min.is_none() {
        return Some(at("min"));
    }
    if range.contains_key("max") && max.is_none() {
        return Some(at("max"));
    }
    match (min, max) {
        (None, None) => Some(Problem::Here),
        (Some(min), Some(max)) => (min > max).then_some(Problem::Here),
        _ => None,
    }
}

// Filter target predicate: closed carrier/target_id/kind envelope.
fn target_predicate_problem(value: &Value) -> Option<Problem> {
    let Some(target) = value.as_object() else {
        return Some(Problem::Here);
    };
    if let Some(unknown) = first_unknown_key(target, &TARGET_PREDICATE_KEYS) {
        return Some(Problem::At(member_path(unknown)));
    }
    if let Some(problem) = child_problem(target, "carrier", cognition_reference_problem) {
        return Some(problem);
    }
    if target
        .get("target_id")
        .is_some_and(|target_id| !target_id.as_str().is_some_and(|id| !id.is_empty()))
    {
        return Some(at("target_id"));
    }
    if target
        .get("kind")
        .is_some_and(|kind| !kind.as_str().is_some_and(|kind| TARGET_KINDS.contains(&kind)))
    {
        return Some(at("kind"));
    }
    None
}

// Filter assessment-context predicate: closed context_id/fork_id envelope.
fn context_predicate_problem(value: &Value) -> Option<Problem> {
    let Some(context) = value.as_object() else {
        return Some(Problem::Here);
    };
    if let Some(unknown) = first_unknown_key(context, &CONTEXT_PREDICATE_KEYS) {
        return Some(Problem::At(member_path(unknown)));
    }
    if context
        .get("context_id")
        .is_some_and(|context_id| !context_id.as_str().is_some_and(|id| !id.is_empty()))
    {
        return Some(at("context_id"));
    }
    child_problem(context, "fork_id", fork_id_predicate_problem)
}

fn cognition_filter_problem(value: &Value) -> Option<Problem> {
    let Some(filter) = value.as_object() else {
        return Some(Problem::Here);
    };
    if let Some(unknown) = first_unknown_key(filter, &FILTER_KEYS) {
        return Some(Problem::At(member_path(unknown)));
    }
    if let Some(problem) = child_problem(filter, "holder", cognition_reference_problem) {
        return Some(problem);
    }
    if let Some(problem) = child_problem(filter, "target", target_predicate_problem) {
        return Some(problem);
    }
    if let Some(problem) = child_problem(filter, "assessment_context", context_predicate_problem) {
        return Some(problem);
    }
    if filter
        .get("category")
        .is_some_and(|category| !category.as_str().is_some_and(|c| CATEGORIES.contains(&c)))
    {
        return Some(at("category"));
    }
    if filter
        .get("state")
        .is_some_and(|state| !state.as_str().is_some_and(|s| EPISTATES.contains(&s)))
    {
        return Some(at("state"));
    }
    if let Some(problem) = child_problem(filter, "q", q_range_problem) {
        return Some(problem);
    }
    if filter.get("order").is_some_and(|order| {
        !order
            .as_f64()
            .is_some_and(|order| order.fract() == 0.0 && (0.0..=3.0).contains(&order))
    }) {
        return Some(at("order"));
    }
    for (field, members) in [
        ("truth", TRUTHS),
        ("access", ACCESSES),
        ("representation", REPRESENTATIONS),
        ("content_type", CONTENT_TYPES),
        ("source", ACQUISITION_PATHS),
        ("belief_context", BELIEF_CONTEXTS),
    ] {
        let Some(predicate) = filter.get(field) else {
            continue;
        };
        if let Some(problem) = categorical_predicate_problem(predicate, members) {
            return Some(child_path(field, problem));
        }
    }
    None
}

fn cognition_reject(problem: Problem, message: &str) -> SpokeResult<()> {
    let details = match problem {
        Problem::Here => None,
        Problem::At(path) => {
            let mut details = Map::new();
            details.insert("field".to_owned(), Value::from(path));
            Some(details)
        }
    };
    spoke_reject(SpokeRejectCode::InvalidInput, message, details)
}

/// Wire-shape gate for the closed CognitionFilter grammar. Bare numeric keys
/// and schema-unknown predicates are not standard probability filters and
/// fail the closed envelope. Pure grammar validation only — predicates are
/// not evaluated against any record set here. A rejection carries
/// `details.field` with the failing member's path relative to the filter; a
/// root failure omits it.
pub fn validate_cognition_filter(value: &Value) -> SpokeResult<()> {
    match cognition_filter_problem(value) {
        None => spoke_ok_unit(),
        Some(problem) => cognition_reject(problem, "cognition filter failed wire-shape validation"),
    }
}

const SELECTION_KEYS: &[&str] = &["view", "filter", "selection_policy"];

fn selection_policy_problem(value: &Value) -> Option<Problem> {
    let Some(policy) = value.as_object() else {
        return Some(Problem::Here);
    };
    if let Some(unknown) = first_unknown_key(policy, &POLICY_KEYS) {
        return Some(Problem::At(member_path(unknown)));
    }
    if !policy
        .get("policy_id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty())
    {
        return Some(at("policy_id"));
    }
    if !policy
        .get("policy_version")
        .and_then(Value::as_str)
        .is_some_and(|version| !version.is_empty())
    {
        return Some(at("policy_version"));
    }
    None
}

fn cognition_selection_problem(value: &Value) -> Option<Problem> {
    let Some(selection) = value.as_object() else {
        return Some(Problem::Here);
    };
    if let Some(unknown) = first_unknown_key(selection, &SELECTION_KEYS) {
        return Some(Problem::At(member_path(unknown)));
    }
    let filter = cognition_filter_problem(selection.get("filter").unwrap_or(&Value::Null));
    if let Some(problem) = filter {
        return Some(child_path("filter", problem));
    }
    match selection.get("view").and_then(Value::as_str) {
        Some("current") => {
            let Some(policy) = selection.get("selection_policy") else {
                return Some(at("selection_policy"));
            };
            selection_policy_problem(policy)
                .map(|problem| child_path("selection_policy", problem))
        }
        Some("recorded_at") => selection
            .contains_key("selection_policy")
            .then(|| at("selection_policy")),
        _ => Some(at("view")),
    }
}

/// Wire-shape gate for the CognitionSelection union: `view = "current"`
/// requires an explicitly identified `selection_policy`; `view =
/// "recorded_at"` carries a filter only. The guard verifies the policy
/// identity is present — it supplies no policy algorithm. A rejection
/// carries `details.field` with the failing member's path relative to the
/// selection; a root failure omits it.
pub fn validate_cognition_selection(value: &Value) -> SpokeResult<()> {
    match cognition_selection_problem(value) {
        None => spoke_ok_unit(),
        Some(problem) => cognition_reject(
            problem,
            "cognition selection failed wire-shape validation",
        ),
    }
}

fn is_selection_trace(value: &Value) -> bool {
    let Some(trace) = value.as_object() else {
        return false;
    };
    if !has_only_keys(trace, &SELECTION_TRACE_KEYS) {
        return false;
    }
    if !trace.get("policy").is_some_and(is_selection_policy_identity) {
        return false;
    }
    let Some(input_record_ids) = trace.get("input_record_ids").and_then(Value::as_array) else {
        return false;
    };
    if !input_record_ids.iter().all(Value::is_string) {
        return false;
    }
    let Some(selected_record_id) = trace
        .get("selected_record_id")
        .and_then(Value::as_str)
        .filter(|selected_record_id| !selected_record_id.is_empty())
    else {
        return false;
    };
    input_record_ids.iter().any(|id| id.as_str() == Some(selected_record_id))
}

// One item and its assessments describe one source group (§5.3): the same
// complete holder reference, target identity (carrier tuple, kind, target_id
// — never text/about) and assessment context. Ids are compared per item only,
// because the same `record_id` may legitimately live in another carrier.
fn is_projection_group_member(item: &Map<String, Value>, record: &Value) -> bool {
    let item_target = item.get("target");
    let record_target = record.get("target");
    ["carrier", "kind", "target_id"].iter().all(|key| {
        record_target.and_then(|target| target.get(*key))
            == item_target.and_then(|target| target.get(*key))
    }) && record.get("holder") == item.get("holder")
        && record.get("assessment_context") == item.get("assessment_context")
}

fn is_cognition_projection_item(
    value: &Value,
    placement: Placement,
    with_selection: bool,
) -> bool {
    let Some(item) = value.as_object() else {
        return false;
    };
    if !has_only_keys(item, &PROJECTION_ITEM_KEYS) {
        return false;
    }
    if !item.get("holder").is_some_and(is_cognition_reference) {
        return false;
    }
    if !is_cognition_target(item.get("target").unwrap_or(&Value::Null), 0) {
        return false;
    }
    if !item
        .get("assessment_context")
        .is_some_and(is_cognition_context)
    {
        return false;
    }
    let Some(presence) = item.get("presence").and_then(Value::as_str) else {
        return false;
    };
    if !PRESENCES.contains(&presence) {
        return false;
    }
    let Some(assessments) = item.get("assessments").and_then(Value::as_array) else {
        return false;
    };
    if !assessments
        .iter()
        .all(|record| validate_placed_record(record, Some(placement)))
    {
        return false;
    }
    if (presence == "no_record") != assessments.is_empty() {
        return false;
    }
    let record_ids: Vec<&str> = assessments
        .iter()
        .filter_map(|record| record.get("record_id").and_then(Value::as_str))
        .collect();
    // The item and its assessments describe one source group with distinct
    // ids, so the trace can name exactly one record.
    if record_ids.len() != assessments.len()
        || !assessments
            .iter()
            .all(|record| is_projection_group_member(item, record))
        || !all_distinct(record_ids.clone())
    {
        return false;
    }
    // The current selection trace is required iff view = current and presence
    // = assessed; recorded-at-time projection has no current selection. Its
    // candidates are exactly this item's assessments, so the selected id
    // always names a record the item carries.
    if with_selection {
        let Some(trace) = item
            .get("selection")
            .filter(|selection| is_selection_trace(selection))
        else {
            return false;
        };
        let Some(input_ids) = trace.get("input_record_ids").and_then(Value::as_array) else {
            return false;
        };
        let Some(selected) = trace.get("selected_record_id").and_then(Value::as_str) else {
            return false;
        };
        return record_ids
            .iter()
            .all(|id| input_ids.iter().any(|input| input.as_str() == Some(*id)))
            && input_ids
                .iter()
                .all(|input| input.as_str().is_some_and(|id| record_ids.contains(&id)))
            && record_ids.contains(&selected);
    }
    item.get("selection").is_none()
}

/// Wire-shape gate for an attribution-preserving CognitionProjection. Every
/// assessment must carry the projection's own view and match its item's
/// holder, target identity and assessment context. A current `assessed` item
/// must carry a selection trace whose candidates are exactly that item's
/// assessment records and whose `selected_record_id` names one of them.
pub fn validate_cognition_projection(value: &Value) -> SpokeResult<()> {
    let Some(projection) = value.as_object() else {
        return gate(false, "cognition projection must be a non-null plain object");
    };
    if !has_only_keys(projection, &PROJECTION_KEYS) {
        return gate(false, "cognition projection has unknown property");
    }
    let Some(view) = projection.get("view").and_then(Value::as_str) else {
        return gate(false, "cognition projection view is required");
    };
    if !VIEWS.contains(&view) {
        return gate(false, "cognition projection view is invalid");
    }
    if !is_extension_map(projection.get("extensions").unwrap_or(&Value::Null)) {
        return gate(false, "cognition projection has invalid extensions");
    }
    let Some(items) = projection.get("items").and_then(Value::as_array) else {
        return gate(false, "cognition projection items must be an array");
    };
    let placement = if view == "current" {
        Placement::Authority
    } else {
        Placement::Memory
    };
    let ok = items.iter().all(|item| {
        let with_selection = view == "current"
            && item
                .get("presence")
                .and_then(Value::as_str)
                == Some("assessed");
        is_cognition_projection_item(item, placement, with_selection)
    });
    gate(ok, "cognition projection failed wire-shape validation")
}

fn gate(ok: bool, message: &str) -> SpokeResult<()> {
    if ok {
        spoke_ok_unit()
    } else {
        spoke_reject(SpokeRejectCode::InvalidInput, message, None)
    }
}

// === C6 shared selection predicates and traceable projection ===
// (cognition-data-contract.md §5). Pure in-memory evaluation over typed
// records: one-record conjunction, complete (kind, namespace, id) reference
// identity, categorical absent/eq arms, inclusive q ranges over declared
// (uncalibrated) values, and an attribution-preserving projection whose
// current view is chosen only by an explicitly injected product policy. No
// storage/reference I/O, no registry resolution, no probability synthesis,
// no averaging/latest-wins merging and no built-in policy.

/// Explicitly injected product policy for current assessed projections.
/// The library supplies no policy algorithm: it verifies only that the
/// returned id is one of the traced input record ids.
pub trait CurrentViewPolicy {
    fn select(&self, assessments: &[&CognitionRecord]) -> Option<String>;
}

fn record_parts(
    record: &CognitionRecord,
) -> (
    &'static str,
    &CognitionReference,
    &CognitionTarget,
    &CognitionContext,
    f64,
    &str,
) {
    match record {
        CognitionRecord::BeliefRecord(record) => (
            "belief",
            &record.holder,
            &record.target,
            &record.assessment_context,
            record.q.value,
            record.record_id.as_str(),
        ),
        CognitionRecord::EpistemicAssessment(record) => (
            "epistemic",
            &record.holder,
            &record.target,
            &record.assessment_context,
            record.q.value,
            record.record_id.as_str(),
        ),
        CognitionRecord::PerceptionRecord(record) => (
            "perception",
            &record.holder,
            &record.target,
            &record.assessment_context,
            record.q.value,
            record.record_id.as_str(),
        ),
    }
}

fn record_view(record: &CognitionRecord) -> &CognitiveView {
    match record {
        CognitionRecord::BeliefRecord(record) => &record.view,
        CognitionRecord::EpistemicAssessment(record) => &record.view,
        CognitionRecord::PerceptionRecord(record) => &record.view,
    }
}

// Reference identity is the complete (kind, namespace, id) tuple; bare ids
// and equal text never establish identity across namespaces.
fn same_reference(a: &CognitionReference, b: &CognitionReference) -> bool {
    a.kind == b.kind && a.namespace == b.namespace && a.id == b.id
}

// Categorical predicate: `Eq` matches only a present value; `Absent(true)`
// matches only an explicitly absent optional field. A missing field is
// never equal to any value (missing data is not False/Unknown/zero).
trait CategoricalPredicate<V> {
    fn predicate_matches(&self, value: Option<&V>) -> bool;
}

macro_rules! impl_categorical_predicate {
    ($predicate:ty, $value:ty) => {
        impl CategoricalPredicate<$value> for $predicate {
            fn predicate_matches(&self, value: Option<&$value>) -> bool {
                match self {
                    Self::Eq(expected) => value == Some(expected),
                    Self::Absent(true) => value.is_none(),
                    Self::Absent(false) => false,
                }
            }
        }
    };
}

impl_categorical_predicate!(TruthPredicate, Truth);
impl_categorical_predicate!(BeliefAccessPredicate, BeliefAccess);
impl_categorical_predicate!(RepresentationPredicate, Representation);
impl_categorical_predicate!(BeliefContentTypePredicate, BeliefContentType);
impl_categorical_predicate!(BeliefSourcePredicate, AcquisitionPath);
impl_categorical_predicate!(BeliefContextPredicate, BeliefContext);

fn category_str(category: &CognitionFilterCategory) -> &'static str {
    match category {
        CognitionFilterCategory::Epistemic => "epistemic",
        CognitionFilterCategory::Perception => "perception",
        CognitionFilterCategory::Belief => "belief",
    }
}

/// One-record conjunction over the closed CognitionFilter grammar. Every
/// supplied predicate must hold on the same record; predicates never join
/// across records or targets.
pub fn cognition_record_matches_filter(record: &CognitionRecord, filter: &CognitionFilter) -> bool {
    let (kind, holder, target, context, q, _record_id) = record_parts(record);
    if let Some(expected) = &filter.holder {
        if !same_reference(holder, expected) {
            return false;
        }
    }
    if let Some(target_filter) = &filter.target {
        if let Some(carrier) = &target_filter.carrier {
            if !same_reference(&target.carrier, carrier) {
                return false;
            }
        }
        if let Some(target_id) = &target_filter.target_id {
            if target.target_id.as_str() != target_id.as_str() {
                return false;
            }
        }
        if let Some(target_kind) = &target_filter.kind {
            if target.kind != *target_kind {
                return false;
            }
        }
    }
    if let Some(context_filter) = &filter.assessment_context {
        if let Some(context_id) = &context_filter.context_id {
            if context.context_id.as_str() != context_id.as_str() {
                return false;
            }
        }
        if let Some(fork) = &context_filter.fork_id {
            // Closed fork predicate: `Eq` matches one fork identity; `Absent`
            // matches only the context identity with no fork. An omitted
            // predicate is a wildcard over forked and no-fork contexts.
            let matches = match fork {
                ForkIdPredicate::Eq(expected) => context
                    .fork_id
                    .as_ref()
                    .is_some_and(|value| value.as_str() == expected.as_str()),
                ForkIdPredicate::Absent(true) => context.fork_id.is_none(),
                ForkIdPredicate::Absent(false) => false,
            };
            if !matches {
                return false;
            }
        }
    }
    if let Some(category) = &filter.category {
        if kind != category_str(category) {
            return false;
        }
    }
    if let Some(state) = &filter.state {
        match record {
            CognitionRecord::EpistemicAssessment(record) if &record.state == state => {}
            _ => return false,
        }
    }
    if let Some(order) = &filter.order {
        match record {
            CognitionRecord::BeliefRecord(record) if *record.order == **order => {}
            _ => return false,
        }
    }
    if let Some(range) = &filter.q {
        // Inclusive comparisons over the declared foundational q.value;
        // uncalibrated but declared values participate identically.
        if let Some(min) = range.min {
            if q < min {
                return false;
            }
        }
        if let Some(max) = range.max {
            if q > max {
                return false;
            }
        }
    }
    // Belief-only dimensions exist only on Belief records: an epistemic or
    // Perception record has no absent Belief field, and its own `access` is
    // an ObservationAccess, never BeliefAccess.
    let belief = match record {
        CognitionRecord::BeliefRecord(record) => Some(record),
        _ => None,
    };
    if belief.is_none()
        && (filter.truth.is_some()
            || filter.access.is_some()
            || filter.representation.is_some()
            || filter.content_type.is_some()
            || filter.source.is_some()
            || filter.belief_context.is_some())
    {
        return false;
    }
    if let Some(predicate) = &filter.truth {
        if !predicate.predicate_matches(belief.and_then(|record| record.truth.as_ref())) {
            return false;
        }
    }
    if let Some(predicate) = &filter.access {
        if !predicate.predicate_matches(belief.and_then(|record| record.access.as_ref())) {
            return false;
        }
    }
    if let Some(predicate) = &filter.representation {
        if !predicate.predicate_matches(belief.and_then(|record| record.representation.as_ref())) {
            return false;
        }
    }
    if let Some(predicate) = &filter.content_type {
        if !predicate.predicate_matches(belief.and_then(|record| record.content_type.as_ref())) {
            return false;
        }
    }
    if let Some(predicate) = &filter.source {
        if !predicate.predicate_matches(belief.and_then(|record| record.source.as_ref())) {
            return false;
        }
    }
    if let Some(predicate) = &filter.belief_context {
        if !predicate.predicate_matches(belief.and_then(|record| record.context.as_ref())) {
            return false;
        }
    }
    true
}

fn unsupported_requirement<T>(message: &str) -> SpokeResult<T> {
    let mut details = Map::new();
    details.insert(
        "reason".to_owned(),
        Value::from("unsupported_cognition_requirement"),
    );
    spoke_reject(SpokeRejectCode::InvalidInput, message, Some(details))
}

fn filter_is_valid(filter: &CognitionFilter) -> bool {
    serde_json::to_value(filter)
        .map(|value| cognition_filter_problem(&value).is_none())
        .unwrap_or(false)
}

fn selection_is_valid(selection: &CognitionSelection) -> bool {
    let (filter, policy) = match selection {
        CognitionSelection::Current {
            filter,
            selection_policy,
        } => (filter, Some(selection_policy)),
        CognitionSelection::RecordedAt { filter } => (filter, None),
    };
    filter_is_valid(filter)
        && policy.is_none_or(|policy| {
            !policy.policy_id.is_empty() && !policy.policy_version.is_empty()
        })
}

/// Filters an authorized source record set by the closed filter grammar.
/// Records are returned in input order. A filter that fails the wire-shape
/// gate (unknown predicates, bare numerics, metric filters) rejects the
/// whole request as an unsupported cognition requirement — nothing is
/// stripped.
pub fn select_cognition_records<'a>(
    records: &'a [CognitionRecord],
    filter: &CognitionFilter,
) -> SpokeResult<Vec<&'a CognitionRecord>> {
    if !filter_is_valid(filter) {
        return unsupported_requirement("cognition filter failed wire-shape validation");
    }
    SpokeResult::Ok(records
        .iter()
        .filter(|record| cognition_record_matches_filter(record, filter))
        .collect())
}

fn reference_wire(reference: &CognitionReference) -> Value {
    json!([
        serde_json::to_value(&reference.kind).expect("reference kind serializes"),
        reference.namespace.as_str(),
        reference.id.as_str(),
    ])
}

// Compound item identity: holder tuple + target identity (carrier tuple,
// kind, target_id — never text) + assessment context identity.
fn group_key(record: &CognitionRecord) -> String {
    let (_, holder, target, context, _, _) = record_parts(record);
    serde_json::to_string(&json!([
        reference_wire(holder),
        [
            reference_wire(&target.carrier),
            serde_json::to_value(&target.kind).expect("target kind serializes"),
            target.target_id.as_str(),
        ],
        [
            context.context_id.as_str(),
            context.fork_id.as_ref().map(|fork_id| fork_id.as_str()),
        ],
    ]))
    .expect("identity key serializes")
}

// The exact-source rule (§5.3): only the filter itself can exactly address a
// source key. That requires the complete holder reference, complete target
// identity, `assessment_context.context_id`, and an explicit fork predicate
// (`Eq` or `Absent(true)`). An omitted fork_id is a wildcard, so it never
// yields a no_record claim for a particular fork identity.
fn exact_source_context(filter: &CognitionFilter) -> Option<CognitionContext> {
    filter.holder.as_ref()?;
    let target = filter.target.as_ref()?;
    if target.carrier.is_none() || target.target_id.is_none() || target.kind.is_none() {
        return None;
    }
    let context = filter.assessment_context.as_ref()?;
    let context_id = context.context_id.as_ref()?;
    let fork_id = match context.fork_id.as_ref()? {
        ForkIdPredicate::Eq(fork_id) => Some(
            fork_id
                .as_str()
                .parse()
                .expect("fork predicate eq is non-empty"),
        ),
        ForkIdPredicate::Absent(true) => None,
        ForkIdPredicate::Absent(false) => return None,
    };
    Some(CognitionContext {
        context_id: context_id
            .as_str()
            .parse()
            .expect("context id predicate is non-empty"),
        fork_id,
    })
}

fn assessment_item(record: &CognitionRecord) -> CognitionProjectionItemAssessmentsItem {
    match record {
        CognitionRecord::BeliefRecord(record) => {
            CognitionProjectionItemAssessmentsItem::BeliefRecord(record.clone())
        }
        CognitionRecord::EpistemicAssessment(record) => {
            CognitionProjectionItemAssessmentsItem::EpistemicAssessment(record.clone())
        }
        CognitionRecord::PerceptionRecord(record) => {
            CognitionProjectionItemAssessmentsItem::PerceptionRecord(record.clone())
        }
    }
}

/// Attribution-preserving projection (§5.3). Context selection comes only
/// from the filter: matching records from every requested context are
/// grouped by compound item identity in input order, and each item keeps its
/// own `assessment_context` — there is no projection-global effective context
/// and no caller-context intersection. Every retained disagreement stays in
/// `assessments` (never averaged or latest-wins merged). A current view calls
/// the injected policy independently per compound group and writes the policy
/// identity plus all input ids into the trace; recorded-at-time projection
/// takes no callback. A `no_record` item is emitted only under the
/// exact-source rule, and only when the source has no record for that
/// compound key before optional predicates. A wildcard or partially addressed
/// empty result is an ordinary empty projection.
pub fn project_cognition_records(
    records: &[CognitionRecord],
    selection: &CognitionSelection,
    current_view_policy: Option<&dyn CurrentViewPolicy>,
) -> SpokeResult<CognitionProjection> {
    if !selection_is_valid(selection) {
        return unsupported_requirement("cognition selection failed wire-shape validation");
    }
    let (view, filter, policy_identity) = match selection {
        CognitionSelection::Current {
            filter,
            selection_policy,
        } => (CognitiveView::Current, filter, Some(selection_policy)),
        CognitionSelection::RecordedAt { filter } => (CognitiveView::RecordedAt, filter, None),
    };
    let matched: Vec<&CognitionRecord> = records
        .iter()
        .filter(|record| {
            record_view(record) == &view && cognition_record_matches_filter(record, filter)
        })
        .collect();
    let mut group_order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<&CognitionRecord>> = HashMap::new();
    for record in matched {
        let key = group_key(record);
        match groups.get_mut(&key) {
            Some(group) => group.push(record),
            None => {
                group_order.push(key.clone());
                groups.insert(key, vec![record]);
            }
        }
    }
    let mut items: Vec<CognitionProjectionItem> = Vec::new();
    for key in &group_order {
        let group = &groups[key];
        let first = group[0];
        let (_, holder, target, context, _, _) = record_parts(first);
        let mut item = CognitionProjectionItem {
            assessment_context: context.clone(),
            assessments: group.iter().map(|record| assessment_item(record)).collect(),
            holder: holder.clone(),
            presence: CognitionProjectionItemPresence::Assessed,
            selection: None,
            target: target.clone(),
        };
        if view == CognitiveView::Current {
            let Some(policy) = current_view_policy else {
                return spoke_reject(
                    SpokeRejectCode::InvalidInput,
                    "current projection requires a current view policy",
                    None,
                );
            };
            let selected = policy.select(group);
            let Some(selected) = selected.filter(|id| {
                group
                    .iter()
                    .any(|record| record_parts(record).5 == id.as_str())
            }) else {
                return spoke_reject(
                    SpokeRejectCode::InvalidInput,
                    "current view policy must select one of the traced input record ids",
                    None,
                );
            };
            item.selection = Some(CognitionProjectionItemSelection {
                input_record_ids: group
                    .iter()
                    .map(|record| record_parts(record).5.to_owned())
                    .collect(),
                policy: policy_identity.expect("current selection carries a policy identity").clone(),
                selected_record_id: selected
                    .parse::<CognitionProjectionItemSelectionSelectedRecordId>()
                    .expect("selected id validated as a record id"),
            });
        }
        items.push(item);
    }
    if let Some(source_context) = exact_source_context(filter) {
        // The lookup ignores optional category/dimension/q predicates: a
        // source record excluded by those means "no item", never "no
        // cognition was recorded".
        let holder = filter.holder.as_ref().expect("addressed above");
        let target = filter.target.as_ref().expect("addressed above");
        let target_carrier = target.carrier.as_ref().expect("addressed above");
        let target_id = target.target_id.as_ref().expect("addressed above");
        let target_kind = target.kind.expect("addressed above");
        let source_has_record = records.iter().any(|record| {
            let (_, record_holder, record_target, record_context, _, _) = record_parts(record);
            record_view(record) == &view
                && same_reference(record_holder, holder)
                && same_reference(&record_target.carrier, target_carrier)
                && record_target.target_id.as_str() == target_id.as_str()
                && record_target.kind == target_kind
                && record_context.context_id.as_str() == source_context.context_id.as_str()
                && record_context.fork_id.as_ref().map(|fork_id| fork_id.as_str())
                    == source_context
                        .fork_id
                        .as_ref()
                        .map(|fork_id| fork_id.as_str())
        });
        if !source_has_record {
            items.push(CognitionProjectionItem {
                assessment_context: source_context.clone(),
                assessments: Vec::new(),
                holder: holder.clone(),
                presence: CognitionProjectionItemPresence::NoRecord,
                selection: None,
                target: CognitionTarget {
                    about: None,
                    carrier: target_carrier.clone(),
                    kind: target_kind,
                    target_id: target_id
                        .as_str()
                        .parse()
                        .expect("addressed target id is non-empty"),
                    text: None,
                },
            });
        }
    }
    SpokeResult::Ok(CognitionProjection {
        extensions: Shared1(HashMap::new()),
        items,
        view,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashSet;
    use std::path::Path;

    fn load_vectors() -> Vec<Value> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/cognition/validator-vectors.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let document: Value = serde_json::from_str(&raw)
            .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()));
        document
            .get("vectors")
            .and_then(Value::as_array)
            .expect("validator-vectors.json must carry a vectors array")
            .clone()
    }

    // Timeline-event-observation vectors carry a real TimelineEvent host;
    // deserialize it with the generated spoke_schemas type, then assert raw
    // `modules.observation` presence against the vector metadata and invoke
    // the observation guard only when present. Rust deserialization and TS
    // AJV are not claimed as full host-validation parity.
    fn run_timeline_observation_vector(vector: &Value) -> bool {
        let input = vector.get("input").expect("vector input");
        let host: spoke_schemas::TimelineEvent = serde_json::from_value(input.clone())
            .expect("timeline-event-observation input must deserialize as the generated TimelineEvent host");
        let modules = host
            .modules
            .keys()
            .map(|key| key.to_string())
            .collect::<Vec<_>>();
        let _ = modules;
        let raw_modules = input
            .get("modules")
            .and_then(Value::as_object)
            .expect("timeline-event-observation input must carry a modules object");
        let observation = raw_modules.get("observation");
        match vector
            .get("observation_state")
            .and_then(Value::as_str)
            .expect("timeline-event-observation vector must carry observation_state")
        {
            "absent" => observation.is_none(),
            "empty" => {
                assert_eq!(
                    observation,
                    Some(&json!({ "observers": [] })),
                    "empty observation_state must present exactly {{\"observers\": []}}"
                );
                validate_observation_module(observation.expect("present observation")).is_ok()
            }
            "malformed" => {
                assert_eq!(
                    observation,
                    Some(&json!({})),
                    "malformed observation_state must present exactly {{}}"
                );
                validate_observation_module(observation.expect("present observation")).is_ok()
            }
            other => panic!("unsupported observation state: {other}"),
        }
    }

    fn run_vector(vector: &Value) -> bool {
        let input = vector.get("input").expect("vector input");
        match vector.get("shape").and_then(Value::as_str).expect("vector shape") {
            "cognition-content" => validate_cognition_content(input).is_ok(),
            "cognition-record" => validate_cognition_record(input).is_ok(),
            "cognition-module" => validate_cognition_module(input).is_ok(),
            "observation-module" => validate_observation_module(input).is_ok(),
            "cognition-filter" => validate_cognition_filter(input).is_ok(),
            "cognition-selection" => validate_cognition_selection(input).is_ok(),
            "cognition-projection" => validate_cognition_projection(input).is_ok(),
            "mind-state" => crate::mind_state::validate_mind_state(input).is_ok(),
            "timeline-event-observation" => run_timeline_observation_vector(vector),
            other => panic!("unsupported vector shape: {other}"),
        }
    }

    /// C5 is the sole authority for the relative member at which a filter or
    /// selection is rejected; the Connect boundaries only prefix the carrier
    /// location. Each case below fails for exactly one member.
    #[test]
    fn cognition_diagnostic_names_the_failing_member() {
        use crate::result::SpokeResult;

        fn relative_field(validation: SpokeResult<()>) -> Option<String> {
            match validation {
                SpokeResult::Ok(()) => None,
                SpokeResult::Reject(reject) => reject
                    .details
                    .and_then(|details| details.get("field").cloned())
                    .and_then(|field| field.as_str().map(str::to_owned)),
            }
        }

        let reference = json!({
            "kind": "knowledge_entry",
            "namespace": "world_a",
            "id": "kb_mira",
        });
        let mut invalid_id = reference.clone();
        invalid_id["id"] = json!(7);
        assert_eq!(
            relative_field(validate_cognition_filter(
                &json!({ "holder": invalid_id })
            )),
            Some("holder.id".to_owned())
        );
        let mut invalid_namespace = reference.clone();
        invalid_namespace["namespace"] = json!(42);
        assert_eq!(
            relative_field(validate_cognition_filter(&json!({
                "target": { "carrier": invalid_namespace }
            }))),
            Some("target.carrier.namespace".to_owned())
        );
        let mut missing_kind = reference;
        missing_kind.as_object_mut().expect("object").remove("kind");
        assert_eq!(
            relative_field(validate_cognition_filter(
                &json!({ "holder": missing_kind })
            )),
            Some("holder.kind".to_owned())
        );
        assert_eq!(
            relative_field(validate_cognition_filter(&json!({ "q_min": 0.5 }))),
            Some("q_min".to_owned())
        );
        assert_eq!(
            relative_field(validate_cognition_filter(&json!({
                "q": { "min": 0.9, "max": 0.1 }
            }))),
            Some("q".to_owned())
        );
        // An unknown closed-envelope member outranks the arm count.
        assert_eq!(
            relative_field(validate_cognition_filter(&json!({
                "truth": { "eq": "True", "extra": 1 }
            }))),
            Some("truth.extra".to_owned())
        );
        assert_eq!(
            relative_field(validate_cognition_filter(&json!({
                "assessment_context": { "fork_id": { "eq": "fork_1", "extra": 1 } }
            }))),
            Some("assessment_context.fork_id.extra".to_owned())
        );
        // Two valid arms remain a cross-field failure at the enclosing member.
        assert_eq!(
            relative_field(validate_cognition_filter(&json!({
                "assessment_context": { "fork_id": { "eq": "fork_1", "absent": true } }
            }))),
            Some("assessment_context.fork_id".to_owned())
        );
        // An empty member is a member, never the enclosing object's failure.
        assert_eq!(
            relative_field(validate_cognition_filter(&json!({ "": 1 }))),
            Some("[\"\"]".to_owned())
        );
        assert_eq!(
            relative_field(validate_cognition_filter(&json!({
                "q": { "min": 0.5, "": 1 }
            }))),
            Some("q[\"\"]".to_owned())
        );
        assert_eq!(
            relative_field(validate_cognition_selection(&json!({ "": 1 }))),
            Some("[\"\"]".to_owned())
        );
        // Delimiters, quotes and backslashes keep the actual member identity.
        assert_eq!(
            relative_field(validate_cognition_filter(&json!({
                "q": { "min": 0.5, "a.b": 1 }
            }))),
            Some("q[\"a.b\"]".to_owned())
        );
        assert_eq!(
            relative_field(validate_cognition_filter(&json!({
                "q": { "min": 0.5, "a\"b": 1 }
            }))),
            Some("q[\"a\\\"b\"]".to_owned())
        );
        assert_eq!(
            relative_field(validate_cognition_filter(&json!({
                "q": { "min": 0.5, "a\\b": 1 }
            }))),
            Some("q[\"a\\\\b\"]".to_owned())
        );
        // A failed root omits the relative field.
        assert_eq!(relative_field(validate_cognition_filter(&json!(7))), None);
        assert_eq!(
            relative_field(validate_cognition_selection(&json!({
                "view": "current",
                "filter": {},
                "selection_policy": { "policy_id": "p" },
            }))),
            Some("selection_policy.policy_version".to_owned())
        );
        assert_eq!(
            relative_field(validate_cognition_selection(&json!({
                "view": "recorded_at",
                "filter": {},
                "selection_policy": { "policy_id": "p", "policy_version": "v1" },
            }))),
            Some("selection_policy".to_owned())
        );
        assert_eq!(
            relative_field(validate_cognition_selection(&json!({
                "view": "recorded_at",
                "filter": {},
            }))),
            None
        );
    }

    #[test]
    fn cognition_shared_vectors_execute_every_id() {
        let vectors = load_vectors();
        let mut executed = HashSet::new();
        for vector in &vectors {
            let id = vector.get("id").and_then(Value::as_str).expect("vector id");
            let expected_accept = match vector.get("expected").and_then(Value::as_str) {
                Some("accept") => true,
                Some("reject") => false,
                other => panic!("vector {id} has unsupported expected result: {other:?}"),
            };
            let actual = run_vector(vector);
            assert_eq!(
                actual, expected_accept,
                "vector {id} mismatch: expected {}, got {actual}",
                if expected_accept { "accept" } else { "reject" }
            );
            assert!(executed.insert(id.to_owned()), "duplicate vector id {id}");
        }
        assert_eq!(
            executed.len(),
            vectors.len(),
            "every shared vector id must be executed exactly once"
        );
    }
}

#[cfg(test)]
mod select_tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::Path;

    enum StubPolicy {
        Fixed(Option<String>),
        FirstInput,
    }

    impl CurrentViewPolicy for StubPolicy {
        fn select(&self, assessments: &[&CognitionRecord]) -> Option<String> {
            match self {
                Self::Fixed(selected) => selected.clone(),
                Self::FirstInput => {
                    assessments.first().map(|record| record_parts(record).5.to_owned())
                }
            }
        }
    }

    fn load_selection_vectors() -> Vec<Value> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/cognition/selection-vectors.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let document: Value = serde_json::from_str(&raw)
            .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()));
        document
            .get("vectors")
            .and_then(Value::as_array)
            .expect("selection-vectors.json must carry a vectors array")
            .clone()
    }

    fn vector_records(vector: &Value) -> Vec<CognitionRecord> {
        vector
            .get("records")
            .and_then(Value::as_array)
            .expect("selection vector must carry a records array")
            .iter()
            .map(|record| {
                serde_json::from_value::<CognitionRecord>(record.clone())
                    .expect("selection vector record must deserialize as CognitionRecord")
            })
            .collect()
    }

    fn run_select_vector(vector: &Value) {
        let id = vector.get("id").and_then(Value::as_str).expect("vector id");
        let records = vector_records(vector);
        let expected = vector.get("expected").expect("vector expected");
        let Some(filter_value) = vector.get("filter").cloned() else {
            panic!("vector {id} must carry a filter");
        };
        let Ok(filter) = serde_json::from_value::<CognitionFilter>(filter_value) else {
            // A filter outside the closed grammar cannot be constructed;
            // both languages reject the whole request as an unsupported
            // cognition requirement.
            assert_eq!(
                expected.get("error").and_then(Value::as_str),
                Some("INVALID_INPUT"),
                "vector {id} filter must fail deserialization only for error vectors"
            );
            return;
        };
        let result = select_cognition_records(&records, &filter);
        if let Some(code) = expected.get("error").and_then(Value::as_str) {
            match result {
                SpokeResult::Reject(reject) => assert_eq!(
                    reject.code.as_str(),
                    code,
                    "vector {id} reject code mismatch"
                ),
                SpokeResult::Ok(_) => panic!("vector {id} must reject with {code}"),
            }
            return;
        }
        let SpokeResult::Ok(matched) = result else {
            panic!("vector {id} must succeed");
        };
        let actual: Vec<&str> = matched
            .iter()
            .map(|record| record_parts(record).5)
            .collect();
        let expected_ids: Vec<&str> = expected
            .get("matched_record_ids")
            .and_then(Value::as_array)
            .expect("vector {id} expected matched_record_ids")
            .iter()
            .map(|id| id.as_str().expect("expected record id string"))
            .collect();
        assert_eq!(actual, expected_ids, "vector {id} matched record ids");
    }

    fn projection_summary(projection: &CognitionProjection) -> Value {
        json!({
            "view": serde_json::to_value(&projection.view).expect("view serializes"),
            "items": projection
                .items
                .iter()
                .map(|item| {
                    let target = &item.target;
                    json!({
                        "presence": serde_json::to_value(&item.presence).expect("presence serializes"),
                        "holder": reference_wire(&item.holder),
                        "target": [
                            reference_wire(&target.carrier),
                            serde_json::to_value(&target.kind).expect("target kind serializes"),
                            target.target_id.as_str(),
                        ],
                        "context": [
                            item.assessment_context.context_id.as_str(),
                            item.assessment_context.fork_id.as_ref().map(|fork_id| fork_id.as_str()),
                        ],
                        "assessments": item.assessments.iter().map(|assessment| match assessment {
                            CognitionProjectionItemAssessmentsItem::BeliefRecord(record) => record.record_id.as_str(),
                            CognitionProjectionItemAssessmentsItem::EpistemicAssessment(record) => record.record_id.as_str(),
                            CognitionProjectionItemAssessmentsItem::PerceptionRecord(record) => record.record_id.as_str(),
                        }).collect::<Vec<_>>(),
                        "selected": item.selection.as_ref().map(|selection| selection.selected_record_id.as_str()),
                        "policy": item.selection.as_ref().map(|selection| json!([
                            selection.policy.policy_id.as_str(),
                            selection.policy.policy_version.as_str(),
                        ])),
                        "input_record_ids": item.selection.as_ref().map(|selection| selection
                            .input_record_ids
                            .iter()
                            .map(|record_id| record_id.as_str())
                            .collect::<Vec<_>>()),
                    })
                })
                .collect::<Vec<_>>(),
        })
    }

    fn run_project_vector(vector: &Value) {
        let id = vector.get("id").and_then(Value::as_str).expect("vector id");
        let records = vector_records(vector);
        let expected = vector.get("expected").expect("vector expected");
        let Some(selection_value) = vector.get("selection").cloned() else {
            panic!("vector {id} must carry a selection");
        };
        let policy_id = vector
            .get("policy_selected_record_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let Ok(selection) = serde_json::from_value::<CognitionSelection>(selection_value) else {
            assert_eq!(
                expected.get("error").and_then(Value::as_str),
                Some("INVALID_INPUT"),
                "vector {id} selection must fail deserialization only for error vectors"
            );
            return;
        };
        let policy = match vector.get("policy_selects").and_then(Value::as_str) {
            Some("first_input") => StubPolicy::FirstInput,
            _ => StubPolicy::Fixed(policy_id),
        };
        let result = project_cognition_records(&records, &selection, Some(&policy));
        if let Some(code) = expected.get("error").and_then(Value::as_str) {
            match result {
                SpokeResult::Reject(reject) => assert_eq!(
                    reject.code.as_str(),
                    code,
                    "vector {id} reject code mismatch"
                ),
                SpokeResult::Ok(_) => panic!("vector {id} must reject with {code}"),
            }
            return;
        }
        let SpokeResult::Ok(projection) = result else {
            panic!("vector {id} must succeed");
        };
        assert_eq!(
            projection_summary(&projection),
            *expected,
            "vector {id} projection summary mismatch"
        );
        assert!(
            validate_cognition_projection(
                &serde_json::to_value(&projection).expect("projection serializes")
            )
            .is_ok(),
            "vector {id} projector output must pass the projection gate"
        );
    }

    #[test]
    fn cognition_select_shared_vectors_execute_every_id() {
        let vectors = load_selection_vectors();
        let mut executed = HashSet::new();
        for vector in &vectors {
            let id = vector.get("id").and_then(Value::as_str).expect("vector id");
            match vector.get("case").and_then(Value::as_str).expect("vector case") {
                "select" => run_select_vector(vector),
                "project" => run_project_vector(vector),
                other => panic!("vector {id} has unsupported case: {other}"),
            }
            assert!(executed.insert(id.to_owned()), "duplicate vector id {id}");
        }
        assert_eq!(
            executed.len(),
            vectors.len(),
            "every shared selection vector id must be executed exactly once"
        );
    }
}
