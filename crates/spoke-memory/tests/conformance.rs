//! Conformance-vector corpus runner (memory protocol §17).
//!
//! Loads every `fixtures/memory-conformance/*.json` vector file, checks the
//! corpus envelope, and executes the `surface: "engine"` cases through the
//! reference engine policy surfaces (plus the P2 pure helpers). Cases marked
//! `surface: "wire"` declare provider/transaction expectations that only the
//! Connect loopback runner can execute; they are carried here so both runners
//! share one vector set.
//!
//! The corpus is never silently skipped: an engine case whose scenario has no
//! handler fails the run, every minimum-set scenario must be present, and each
//! `outcome: "variable"` case runs twice so a non-deterministic policy is
//! caught instead of matching a fixed reference by luck.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use spoke_memory::classify::{classify_candidate, CandidateMaterial};
use spoke_memory::recall::{recall, RecallRecord, RecallRequest};
use spoke_memory::reflect::{is_stale, ReflectionBasis, SourceRef};
use spoke_memory::transfer::{
    adopt_grants, partition_bundle, reassemble, staged_report, verify_reassembly, verify_staging,
};
use spoke_operations::{
    is_freeze_allowlisted, jcs_sha256_hex, validate_memory_subjects_fence, SpokeRejectCode,
    SpokeResult,
};
use spoke_schemas::data::provenance_filter::ProvenanceFilter;
use spoke_schemas::data::read_selection::ReadSelection;
use spoke_schemas::data::retained_record::Content;
use spoke_schemas::ops::memory_subjects_fence_request::MemorySubjectsFenceRequest;

/// Minimum-set scenarios the plan's Global Constraints and the prototype r3
/// addendum require. Each must appear in the corpus.
const REQUIRED_SCENARIOS: [&str; 13] = [
    "candidate-consumption",
    "cas-conflict",
    "classification",
    "cursor-invalidation",
    "grant-coverage",
    "idempotency-replay",
    "profile-admission",
    "recall-budget",
    "reflection-staleness",
    "space-freeze",
    "staging-import",
    "subject-fencing",
    "transfer-chunks",
];

/// Scenarios with an engine handler; `run_engine_case` panics otherwise.
/// Scenarios whose expectations are provider-enforced (grants, CAS,
/// idempotency, fencing enforcement, cursor invalidation, admission,
/// staleness triggers, staging over the wire) deliberately have no handler.
const ENGINE_SCENARIOS: [&str; 8] = [
    "candidate-consumption",
    "classification",
    "recall-budget",
    "reflection-staleness",
    "space-freeze",
    "staging-import",
    "subject-fencing",
    "transfer-chunks",
];

/// Closed `expect` key set for wire cases (interpreted by the loopback runner).
const WIRE_EXPECT_KEYS: [&str; 16] = [
    "error_code",
    "details",
    "success",
    "state",
    "absent",
    "present",
    "sizes",
    "minimums",
    "atLeast",
    "contains",
    "excludes",
    "unchanged",
    "verified",
    "uniform",
    "receipt_status",
    "same_receipt_as",
];

/// Closed key set of a `setup` / `then` step.
const STEP_KEYS: [&str; 5] = ["op", "request", "expect", "stage_from", "hold"];

/// `expect` values that must be arrays of small objects.
const WIRE_EXPECT_LISTS: [&str; 10] = [
    "state",
    "absent",
    "present",
    "sizes",
    "minimums",
    "atLeast",
    "contains",
    "excludes",
    "unchanged",
    "uniform",
];

/// Cross-step response namespaces: `setup.<n>`, `op`, `then.<n>`.
const RESPONSE_NAMESPACES: [&str; 3] = ["setup", "op", "then"];

/// Harness capabilities a step may request instead of a protocol op.
const HARNESS_OPS: [&str; 2] = ["harness.save_snapshot", "harness.restore_snapshot"];

/// Provider classes a policy-bearing case may declare.
const PROVIDER_CLASSES: [&str; 2] = ["cooperative", "third-party"];

/// Policy directives whose violation handling is bound to the provider class.
const POLICY_DIRECTIVES: [&str; 3] = [
    "remap_policy_check",
    "pinned_basis_check",
    "policy_version_check",
];

/// Resolves a `<namespace>[.<index>][.<path>]` reference, returning the path
/// remainder (`""` for a bare step reference such as `setup.1` or `op`).
fn split_step_reference<'a>(case_id: &str, reference: &'a str, setup_len: usize) -> &'a str {
    let (namespace, rest) = reference.split_once('.').unwrap_or((reference, ""));
    assert!(
        RESPONSE_NAMESPACES.contains(&namespace),
        "case {case_id}: reference {reference} lacks a response namespace"
    );
    if namespace == "op" {
        return rest;
    }
    let (index, path) = match rest.split_once('.') {
        Some((index, path)) => (index, path),
        None => (rest, ""),
    };
    let index: usize = index
        .parse()
        .unwrap_or_else(|_| panic!("case {case_id}: reference {reference} index"));
    let bound = if namespace == "setup" {
        setup_len
    } else {
        usize::MAX
    };
    assert!(
        index < bound,
        "case {case_id}: reference {reference} index outside its namespace"
    );
    path
}

/// A bare step reference: `setup.<n>`, `op` or `then.<n>`.
fn validate_step_reference(case_id: &str, reference: &str, setup_len: usize) {
    split_step_reference(case_id, reference, setup_len);
}

/// A value reference: `<step>.<path>` — the path is mandatory.
fn validate_value_reference(case_id: &str, reference: &str, setup_len: usize) {
    let path = split_step_reference(case_id, reference, setup_len);
    assert!(
        !path.is_empty(),
        "case {case_id}: value reference {reference} lacks a path"
    );
}

/// Dispatch-layer codes that appear at the Connect boundary (§4.1).
const DISPATCH_CODES: [&str; 2] = ["capability_missing", "op_unsupported"];

struct Scenario {
    id: String,
    value: Value,
}

struct Case<'a> {
    id: String,
    scenario: &'a str,
    value: &'a Value,
}

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/memory-conformance")
}

fn load_scenarios() -> Vec<Scenario> {
    let root = corpus_root();
    let mut files: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap_or_else(|error| panic!("conformance corpus at {}: {error}", root.display()))
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension()?.to_str()? == "json").then_some(path)
        })
        .collect();
    files.sort();

    files
        .into_iter()
        .map(|path| {
            let raw = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let value: Value = serde_json::from_str(&raw)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let id = value["id"]
                .as_str()
                .unwrap_or_else(|| panic!("{}: scenario id", path.display()))
                .to_string();
            Scenario { id, value }
        })
        .collect()
}

fn engine_cases(scenarios: &[Scenario]) -> Vec<Case<'_>> {
    scenarios
        .iter()
        .flat_map(|scenario| {
            scenario.value["cases"]
                .as_array()
                .map(|cases| {
                    cases
                        .iter()
                        .filter(|case| case["surface"] == "engine")
                        .map(|case| Case {
                            id: case["id"].as_str().unwrap_or("<missing id>").to_string(),
                            scenario: scenario.id.as_str(),
                            value: case,
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .collect()
}

fn load_asset(relative: &str) -> Value {
    let path = corpus_root().join(relative);
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("asset {}: {error}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|error| panic!("asset {}: {error}", path.display()))
}

fn parse_timestamp(raw: &str) -> DateTime<Utc> {
    raw.parse::<DateTime<Utc>>()
        .unwrap_or_else(|error| panic!("timestamp {raw}: {error}"))
}

fn string_list(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| item.as_str().expect("string list item").to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn string_map(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .map(|entries| {
            entries
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        value.as_str().expect("string map value").to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Walking setter for `dotted.paths[0]`-style vector tamper targets.
fn set_path(value: &mut Value, path: &str, new_value: Value) {
    let (head, rest) = match path.split_once('.') {
        Some((head, rest)) => (head.to_string(), Some(rest)),
        None => (path.to_string(), None),
    };
    match value {
        Value::Object(entries) => match rest {
            Some(rest) => set_path(
                entries
                    .get_mut(&head)
                    .unwrap_or_else(|| panic!("tamper path {path}: no {head}")),
                rest,
                new_value,
            ),
            None => {
                entries.insert(head, new_value);
            }
        },
        Value::Array(items) => {
            let index: usize = head
                .parse()
                .unwrap_or_else(|error| panic!("tamper path {path}: {error}"));
            let item = items
                .get_mut(index)
                .unwrap_or_else(|| panic!("tamper path {path}: no index {index}"));
            match rest {
                Some(rest) => set_path(item, rest, new_value),
                None => *item = new_value,
            }
        }
        other => panic!("tamper path {path}: {other} is not traversable"),
    }
}

/// `{"$repeat": {"unit": "...", "times": n}}` keeps oversized-content vectors
/// readable; plain strings pass through unchanged.
fn expand_text(value: &Value) -> String {
    if let Some(repeat) = value.get("$repeat") {
        let unit = repeat["unit"].as_str().expect("$repeat.unit");
        let times = repeat["times"].as_u64().expect("$repeat.times") as usize;
        return unit.repeat(times);
    }
    value.as_str().expect("material text").to_string()
}

fn with_basis<T>(spec: &Value, run: impl FnOnce(&ReflectionBasis<'_>) -> T) -> T {
    let selection: ReadSelection =
        serde_json::from_value(spec["selection"].clone()).expect("basis selection");
    let filter: Option<ProvenanceFilter> = match spec.get("filter") {
        None | Some(Value::Null) => None,
        Some(value) => Some(serde_json::from_value(value.clone()).expect("basis filter")),
    };
    let sources_json = spec["sources"].as_array().expect("basis sources");
    let source_ids: Vec<String> = sources_json
        .iter()
        .map(|source| {
            source["record_id"]
                .as_str()
                .expect("source record_id")
                .to_string()
        })
        .collect();
    let revisions: Vec<u64> = sources_json
        .iter()
        .map(|source| source["revision"].as_u64().expect("source revision"))
        .collect();
    let refs: Vec<SourceRef<'_>> = source_ids
        .iter()
        .zip(&revisions)
        .map(|(record_id, revision)| SourceRef {
            record_id,
            revision: *revision,
        })
        .collect();
    let profiles: Vec<(String, u64)> = spec["profile_revisions"]
        .as_object()
        .map(|entries| {
            entries
                .iter()
                .map(|(kind, revision)| {
                    (kind.clone(), revision.as_u64().expect("profile revision"))
                })
                .collect()
        })
        .unwrap_or_default();
    let profile_refs: Vec<(&str, u64)> = profiles
        .iter()
        .map(|(kind, revision)| (kind.as_str(), *revision))
        .collect();
    let basis = ReflectionBasis {
        selection: &selection,
        filter: filter.as_ref(),
        sources: &refs,
        profile_revisions: &profile_refs,
    };
    run(&basis)
}

fn run_engine_case(case: &Case<'_>) -> Value {
    match case.scenario {
        "space-freeze" => run_space_freeze(case),
        "subject-fencing" => run_subject_fencing(case),
        "candidate-consumption" => run_candidate_consumption(case),
        "transfer-chunks" => run_transfer_chunks(case),
        "staging-import" => run_staging_import(case),
        "reflection-staleness" => run_reflection_staleness(case),
        "classification" => run_classification(case),
        "recall-budget" => run_recall_budget(case),
        other => panic!("case {}: no engine handler for scenario {other}", case.id),
    }
}

fn run_space_freeze(case: &Case<'_>) -> Value {
    let op = case.value["input"]["op"].as_str().expect("op");
    let expected = case.value["expect"]["allowlisted"]
        .as_bool()
        .expect("expect.allowlisted");
    let allowlisted = is_freeze_allowlisted(op);
    assert_eq!(
        allowlisted, expected,
        "case {}: allowlist for {op}",
        case.id
    );
    json!({ "allowlisted": allowlisted })
}

fn run_subject_fencing(case: &Case<'_>) -> Value {
    let request: MemorySubjectsFenceRequest =
        serde_json::from_value(case.value["input"]["request"].clone()).expect("fence request");
    let expect = &case.value["expect"];
    let result = validate_memory_subjects_fence(&request);
    let code = match &result {
        SpokeResult::Ok(()) => Value::Null,
        SpokeResult::Reject(reject) => json!(reject.code.as_str()),
    };
    assert_eq!(
        result.is_ok(),
        expect["ok"].as_bool().expect("expect.ok"),
        "case {}: fence validation",
        case.id
    );
    if let Some(expected_code) = expect["error_code"].as_str() {
        assert_eq!(code, json!(expected_code), "case {}: reject code", case.id);
    }
    json!({ "ok": result.is_ok(), "code": code })
}

fn run_candidate_consumption(case: &Case<'_>) -> Value {
    let content = &case.value["input"]["proposed_content"];
    let digest = match jcs_sha256_hex(content) {
        SpokeResult::Ok(digest) => digest,
        SpokeResult::Reject(reject) => panic!("case {}: {}", case.id, reject.message),
    };
    assert_eq!(
        digest,
        case.value["expect"]["content_digest"]
            .as_str()
            .expect("expect.content_digest"),
        "case {}: content_digest",
        case.id
    );
    json!({ "content_digest": digest })
}

fn run_transfer_chunks(case: &Case<'_>) -> Value {
    let input = &case.value["input"];
    let expect = &case.value["expect"];
    let bundle = load_asset(input["bundle_file"].as_str().expect("bundle_file"));
    let limit = input["chunk_limit"].as_u64().expect("chunk_limit") as usize;
    let mut chunks = partition_bundle(&bundle, limit).expect("partition bundle");

    if let Some(tamper) = input.get("tamper") {
        let ordinal = tamper["chunk_ordinal"].as_u64().expect("chunk_ordinal") as usize;
        let chunk = chunks
            .get_mut(ordinal)
            .unwrap_or_else(|| panic!("case {}: no chunk {ordinal}", case.id));
        set_path(
            chunk,
            tamper["path"].as_str().expect("tamper path"),
            tamper["value"].clone(),
        );
    }

    let verification = verify_reassembly(&chunks);
    let verified = verification.bundle_digest_ok && verification.errors.is_empty();
    assert_eq!(
        verified,
        expect["verification_ok"]
            .as_bool()
            .expect("expect.verification_ok"),
        "case {}: errors {:?}",
        case.id,
        verification.errors
    );

    if let Some(minimum) = expect["min_chunk_count"].as_u64() {
        assert!(
            chunks.len() as u64 >= minimum,
            "case {}: {} chunks, expected at least {minimum}",
            case.id,
            chunks.len()
        );
    }
    if let Some(maximum) = expect["max_chunk_count"].as_u64() {
        assert!(
            chunks.len() as u64 <= maximum,
            "case {}: {} chunks, expected at most {maximum}",
            case.id,
            chunks.len()
        );
    }
    if let Some(needle) = expect["errors_contain"].as_str() {
        assert!(
            verification
                .errors
                .iter()
                .any(|error| error.contains(needle)),
            "case {}: no error contains {needle:?}: {:?}",
            case.id,
            verification.errors
        );
    }

    let reassembled_equals_input = match reassemble(&chunks) {
        Ok(reassembled) => Some(reassembled == bundle),
        Err(_) => None,
    };
    if let Some(expected) = expect["reassembled_equals_input"].as_bool() {
        assert_eq!(
            reassembled_equals_input,
            Some(expected),
            "case {}: reassembled bundle",
            case.id
        );
    }

    json!({
        "chunk_count": chunks.len(),
        "verification_ok": verified,
        "errors": verification.errors,
        "reassembled_equals_input": reassembled_equals_input,
    })
}

fn run_staging_import(case: &Case<'_>) -> Value {
    let input = &case.value["input"];
    let expect = &case.value["expect"];
    assert_eq!(
        input["kind"].as_str(),
        Some("staged-report"),
        "case {}: staging kind",
        case.id
    );

    let bundle = load_asset(input["bundle_file"].as_str().expect("bundle_file"));
    let limit = input["chunk_limit"].as_u64().expect("chunk_limit") as usize;
    let chunks = partition_bundle(&bundle, limit).expect("partition bundle");
    let renames = string_map(input.get("subject_renames"));
    let record_ids = string_map(input.get("record_ids"));
    let candidate_ids = string_map(input.get("candidate_ids"));

    let outcome = verify_staging(&chunks, &renames, &record_ids, &candidate_ids);
    let adopt_strings = string_list(&input["adopt_grants"]);
    let adopt: Vec<&str> = adopt_strings.iter().map(String::as_str).collect();
    let (adopted, dropped) = adopt_grants(&bundle, &adopt).expect("adopt_grants");
    let reassembled = outcome
        .bundle
        .as_ref()
        .unwrap_or_else(|| panic!("case {}: staging produced no bundle", case.id));
    let report = staged_report(
        reassembled,
        &outcome.maps,
        adopted.len(),
        dropped,
        &outcome.verification,
    )
    .expect("staged report");

    let verified = outcome.verification.bundle_digest_ok && outcome.verification.errors.is_empty();
    assert_eq!(
        verified,
        expect["verification_ok"]
            .as_bool()
            .expect("expect.verification_ok"),
        "case {}: errors {:?}",
        case.id,
        outcome.verification.errors
    );
    if let Some(needle) = expect["errors_contain"].as_str() {
        assert!(
            outcome
                .verification
                .errors
                .iter()
                .any(|error| error.contains(needle)),
            "case {}: no error contains {needle:?}: {:?}",
            case.id,
            outcome.verification.errors
        );
    }
    if let Some(counts) = expect.get("counts") {
        assert_eq!(report["counts"], *counts, "case {}: counts", case.id);
    }
    if let Some(expected) = expect["grants_adopted"].as_u64() {
        assert_eq!(
            report["grants_adopted"].as_u64(),
            Some(expected),
            "case {}: grants_adopted",
            case.id
        );
    }
    if let Some(expected) = expect["grants_dropped"].as_u64() {
        assert_eq!(
            report["grants_dropped"].as_u64(),
            Some(expected),
            "case {}: grants_dropped",
            case.id
        );
    }
    for field in ["subject_id_map", "record_id_map", "candidate_id_map"] {
        if let Some(expected) = expect.get(field) {
            assert_eq!(report[field], *expected, "case {}: {field}", case.id);
        }
    }

    json!({
        "report": report,
        "verification_ok": verified,
        "errors": outcome.verification.errors,
    })
}

fn run_reflection_staleness(case: &Case<'_>) -> Value {
    let input = &case.value["input"];
    let stale = with_basis(&input["pinned"], |pinned| {
        with_basis(&input["current"], |current| is_stale(pinned, current))
    });
    assert_eq!(
        stale,
        case.value["expect"]["stale"]
            .as_bool()
            .expect("expect.stale"),
        "case {}: staleness",
        case.id
    );
    json!({ "stale": stale })
}

fn run_classification(case: &Case<'_>) -> Value {
    let material_json = &case.value["input"]["material"];
    let text = expand_text(&material_json["text"]);
    let material = CandidateMaterial {
        kind: material_json["kind"].as_str().expect("kind"),
        text: &text,
        has_structured_content: material_json["has_structured_content"]
            .as_bool()
            .expect("has_structured_content"),
        occurred_at: material_json["occurred_at"].as_str().map(parse_timestamp),
        source_context_count: material_json["source_context_count"]
            .as_u64()
            .expect("source_context_count") as usize,
        anchor_count: material_json["anchor_count"]
            .as_u64()
            .expect("anchor_count") as usize,
    };
    let wire = serde_json::to_value(classify_candidate(&material)).expect("classification wire");
    let expect = &case.value["expect"];

    assert_eq!(
        wire["disposition"], expect["disposition"],
        "case {}: disposition",
        case.id
    );
    assert_eq!(
        wire["rationale_codes"], expect["rationale_codes"],
        "case {}: rationale_codes",
        case.id
    );
    assert_eq!(
        wire["confidence"], expect["confidence"],
        "case {}: confidence",
        case.id
    );
    wire
}

fn run_recall_budget(case: &Case<'_>) -> Value {
    let input = &case.value["input"];
    let expect = &case.value["expect"];
    let records: Vec<RecallRecord> = input["records"]
        .as_array()
        .expect("records")
        .iter()
        .map(|record| RecallRecord {
            record_id: record["record_id"].as_str().expect("record_id").to_string(),
            kind: record["kind"].as_str().expect("kind").to_string(),
            content: serde_json::from_value::<Content>(record["content"].clone())
                .expect("record content"),
            created_at: parse_timestamp(record["created_at"].as_str().expect("created_at")),
        })
        .collect();
    let request_json = &input["request"];
    let request = RecallRequest {
        query: request_json["query"].as_str(),
        max_items: request_json["max_items"].as_u64().expect("max_items") as usize,
        max_bytes: request_json["max_bytes"]
            .as_u64()
            .map(|bytes| bytes as usize),
    };

    let page = recall(&records, &request);
    let ids: Vec<&str> = page
        .records
        .iter()
        .map(|record| record.record_id.as_str())
        .collect();

    assert_eq!(json!(ids), expect["record_ids"], "case {}: ids", case.id);
    assert_eq!(
        page.total_authorized as u64,
        expect["total_authorized"]
            .as_u64()
            .expect("total_authorized"),
        "case {}: total_authorized",
        case.id
    );
    assert_eq!(
        page.truncated,
        expect["truncated"].as_bool().expect("truncated"),
        "case {}: truncated",
        case.id
    );
    assert_eq!(
        page.overflow_record_id,
        expect["overflow_record_id"].as_str(),
        "case {}: overflow_record_id",
        case.id
    );

    json!({
        "record_ids": ids,
        "total_authorized": page.total_authorized,
        "truncated": page.truncated,
        "overflow_record_id": page.overflow_record_id,
    })
}

/// Every `"$ref:<namespace>…"` string inside a request payload must name a
/// response namespace — an un-namespaced index would be ambiguous between the
/// setup, op-under-test and then-step sequences.
fn validate_request_refs(case_id: &str, value: &Value, setup_len: usize) {
    match value {
        Value::String(text) => {
            if let Some(reference) = text.strip_prefix("$ref:") {
                validate_value_reference(case_id, reference, setup_len);
            }
        }
        Value::Array(items) => {
            for item in items {
                validate_request_refs(case_id, item, setup_len);
            }
        }
        Value::Object(entries) => {
            for item in entries.values() {
                validate_request_refs(case_id, item, setup_len);
            }
        }
        _ => {}
    }
}

/// `stage_from` must define the deterministic per-chunk key scheme so a runner
/// can build valid `memory.imports.stage` requests without inventing one.
fn validate_stage_from(case_id: &str, stage_from: &Value) {
    assert!(
        stage_from["asset"].as_str().is_some(),
        "case {case_id}: stage_from.asset"
    );
    assert!(
        stage_from["chunk_limit"].as_u64().is_some(),
        "case {case_id}: stage_from.chunk_limit"
    );
    let pattern = stage_from["idempotency_key_pattern"]
        .as_str()
        .unwrap_or_else(|| panic!("case {case_id}: stage_from.idempotency_key_pattern"));
    assert!(
        pattern.contains("{export_id}") && pattern.contains("{ordinal}"),
        "case {case_id}: stage_from.idempotency_key_pattern needs {{export_id}} and {{ordinal}}"
    );
    if let Some(replay) = stage_from.get("replay") {
        assert!(
            replay["chunk"].as_u64().is_some(),
            "case {case_id}: stage_from.replay.chunk"
        );
    }
}

/// Ids a staged bundle contributes to each translation map.
fn bundle_map_ids(bundle: &Value, map: &str) -> Vec<String> {
    let (field, id_field) = match map {
        "record_id_map" => ("records", "record_id"),
        "candidate_id_map" => ("candidates", "candidate_id"),
        "subject_id_map" => ("subjects", "subject_id"),
        other => panic!("unknown remap map {other}"),
    };
    bundle
        .get(field)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get(id_field).and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The asset a `stage_from` step staged, located from a `<step>.<path>` reference.
fn staged_asset_for(case: &Value, reference: &str) -> Option<Value> {
    let (step_reference, _) = reference.split_once(".result.")?;
    let index: usize = step_reference.strip_prefix("setup.")?.parse().ok()?;
    let step = case.get("setup")?.as_array()?.get(index)?;
    let asset = step.get("stage_from")?.get("asset")?.as_str()?;
    Some(load_asset(asset))
}

fn validate_remap_policy(case_id: &str, case: &Value, remap: &Value, setup_len: usize) {
    let report = remap["report"]
        .as_str()
        .unwrap_or_else(|| panic!("case {case_id}: remap_policy_check.report"));
    validate_value_reference(case_id, report, setup_len);
    assert_eq!(
        remap["policy"].as_str(),
        Some("identity"),
        "case {case_id}: remap_policy_check.policy"
    );
    let maps = remap["maps"]
        .as_array()
        .unwrap_or_else(|| panic!("case {case_id}: remap_policy_check.maps"));
    assert!(
        !maps.is_empty(),
        "case {case_id}: empty remap_policy_check.maps"
    );
    for map in maps {
        let name = map
            .as_str()
            .unwrap_or_else(|| panic!("case {case_id}: remap map name"));
        assert!(
            ["record_id_map", "candidate_id_map", "subject_id_map"].contains(&name),
            "case {case_id}: unknown remap map {name}"
        );
    }

    // An identity check over an empty map proves nothing: the exact source ids the
    // fixture imports must be declared, and when the staged asset is reachable the
    // declared set must equal the asset's ids.
    let expect_keys = remap["expect_keys"]
        .as_object()
        .unwrap_or_else(|| panic!("case {case_id}: remap_policy_check.expect_keys"));
    assert!(!expect_keys.is_empty(), "case {case_id}: empty expect_keys");
    let staged = staged_asset_for(case, report);
    if report.contains(".result.staged_report") {
        assert!(
            staged.is_some(),
            "case {case_id}: staged_report reference must resolve to a staged asset"
        );
    }
    for (map, declared) in expect_keys {
        let declared: Vec<String> = declared
            .as_array()
            .unwrap_or_else(|| panic!("case {case_id}: expect_keys.{map}"))
            .iter()
            .map(|id| {
                id.as_str()
                    .unwrap_or_else(|| panic!("case {case_id}: expect_keys.{map} entry"))
                    .to_string()
            })
            .collect();
        assert!(
            !declared.is_empty(),
            "case {case_id}: expect_keys.{map} is empty"
        );
        assert!(
            maps.iter().any(|name| name.as_str() == Some(map.as_str())),
            "case {case_id}: expect_keys.{map} is not in maps"
        );
        if let Some(bundle) = &staged {
            let mut expected = bundle_map_ids(bundle, map);
            let mut declared_sorted = declared.clone();
            expected.sort();
            declared_sorted.sort();
            assert_eq!(
                declared_sorted, expected,
                "case {case_id}: expect_keys.{map} must list exactly the staged ids"
            );
        }
    }
    assert!(
        remap["inconclusive_on_violation"].as_bool().is_some(),
        "case {case_id}: remap_policy_check.inconclusive_on_violation"
    );
}

fn validate_step(case_id: &str, step: &Value, setup_len: usize) {
    for key in step.as_object().expect("step object").keys() {
        assert!(
            STEP_KEYS.contains(&key.as_str()),
            "case {case_id}: unknown step key {key}"
        );
    }
    let op = step["op"].as_str().expect("step op");
    assert!(
        op.starts_with("memory.") || op.starts_with("harness."),
        "case {case_id}: step op {op}"
    );
    if op.starts_with("harness.") {
        assert!(
            HARNESS_OPS.contains(&op),
            "case {case_id}: unknown harness capability {op}"
        );
        assert!(
            step["request"]["space_id"].as_str().is_some(),
            "case {case_id}: {op} needs space_id"
        );
    }
    assert!(step["request"].is_object(), "case {case_id}: step request");
    validate_request_refs(case_id, &step["request"], setup_len);
    if let Some(stage_from) = step.get("stage_from") {
        validate_stage_from(case_id, stage_from);
    }
    if let Some(hold) = step.get("hold") {
        assert_eq!(
            hold.as_str(),
            Some("pending"),
            "case {case_id}: hold must be \"pending\""
        );
    }
    if let Some(step_expect) = step.get("expect").and_then(Value::as_object) {
        validate_wire_expect(case_id, step_expect, setup_len);
    }
}

/// Shapes of the closed wire `expect` vocabulary, including cross-step namespaces.
fn validate_wire_expect(case_id: &str, expect: &serde_json::Map<String, Value>, setup_len: usize) {
    assert!(!expect.is_empty(), "case {case_id}: empty expect");
    for key in expect.keys() {
        assert!(
            WIRE_EXPECT_KEYS.contains(&key.as_str()),
            "case {case_id}: unknown expect key {key}"
        );
    }
    if let Some(code) = expect.get("error_code").and_then(Value::as_str) {
        assert!(
            SpokeRejectCode::try_from_str(code).is_some() || DISPATCH_CODES.contains(&code),
            "case {case_id}: unknown error code {code}"
        );
    }
    for list_key in WIRE_EXPECT_LISTS {
        if let Some(entries) = expect.get(list_key) {
            assert!(
                entries.as_array().is_some(),
                "case {case_id}: {list_key} must be an array"
            );
        }
    }
    if let Some(bounds) = expect.get("atLeast").and_then(Value::as_array) {
        for bound in bounds {
            assert!(
                bound["path"].as_str().is_some(),
                "case {case_id}: atLeast.path"
            );
            assert!(
                bound["equals"].as_f64().is_some(),
                "case {case_id}: atLeast.equals must be numeric"
            );
        }
    }
    if let Some(changes) = expect.get("unchanged").and_then(Value::as_array) {
        for change in changes {
            assert!(
                change["path"].as_str().is_some(),
                "case {case_id}: unchanged.path"
            );
            assert!(
                change["from_path"].as_str().is_some(),
                "case {case_id}: unchanged.from_path"
            );
            validate_step_reference(
                case_id,
                change["from"]
                    .as_str()
                    .unwrap_or_else(|| panic!("case {case_id}: unchanged.from")),
                setup_len,
            );
        }
    }
    if let Some(target) = expect.get("same_receipt_as").and_then(Value::as_str) {
        validate_step_reference(case_id, target, setup_len);
    }
    if let Some(verbatim) = expect.get("verified") {
        let verified = verbatim
            .as_object()
            .unwrap_or_else(|| panic!("case {case_id}: verified must be an object"));
        assert!(!verified.is_empty(), "case {case_id}: empty verified");
        for (name, value) in verified {
            assert!(
                value.as_bool().is_some(),
                "case {case_id}: verified.{name} must be a boolean"
            );
        }
    }
    if let Some(details) = expect.get("details") {
        assert!(
            details.is_object(),
            "case {case_id}: details must be an object"
        );
    }
}

#[test]
fn conformance_corpus_is_well_formed() {
    let scenarios = load_scenarios();
    let scenario_ids: BTreeSet<&str> = scenarios
        .iter()
        .map(|scenario| scenario.id.as_str())
        .collect();

    for required in REQUIRED_SCENARIOS {
        assert!(
            scenario_ids.contains(required),
            "missing minimum-set scenario {required}"
        );
    }

    let mut case_ids: BTreeSet<String> = BTreeSet::new();
    let mut engine_count = 0usize;
    let mut wire_count = 0usize;
    let mut variable_count = 0usize;

    for scenario in &scenarios {
        assert!(
            scenario.value["title"]
                .as_str()
                .is_some_and(|title| !title.is_empty()),
            "scenario {}: title",
            scenario.id
        );
        let refs = scenario.value["protocol_refs"]
            .as_array()
            .unwrap_or_else(|| panic!("scenario {}: protocol_refs", scenario.id));
        assert!(!refs.is_empty(), "scenario {}: protocol_refs", scenario.id);

        let cases = scenario.value["cases"]
            .as_array()
            .unwrap_or_else(|| panic!("scenario {}: cases", scenario.id));
        assert!(!cases.is_empty(), "scenario {}: no cases", scenario.id);

        for case in cases {
            let id = case["id"]
                .as_str()
                .unwrap_or_else(|| panic!("scenario {}: case id", scenario.id));
            assert!(case_ids.insert(id.to_string()), "duplicate case id {id}");
            assert!(
                case["invariant"]
                    .as_str()
                    .is_some_and(|value| !value.is_empty()),
                "case {id}: invariant"
            );
            assert!(
                case["what"].as_str().is_some_and(|value| !value.is_empty()),
                "case {id}: what"
            );
            let outcome = case["outcome"].as_str();
            assert!(
                matches!(outcome, Some("identical") | Some("variable")),
                "case {id}: outcome"
            );
            if outcome == Some("variable") {
                variable_count += 1;
            }

            match case["surface"].as_str() {
                Some("engine") => {
                    assert!(
                        ENGINE_SCENARIOS.contains(&scenario.id.as_str()),
                        "case {id}: engine case in scenario {} without a handler",
                        scenario.id
                    );
                    assert!(case["input"].is_object(), "case {id}: input");
                    assert!(case["expect"].is_object(), "case {id}: expect");
                    engine_count += 1;
                }
                Some("wire") => {
                    assert!(case["op"].as_str().is_some(), "case {id}: op");
                    assert!(case["request"].is_object(), "case {id}: request");
                    let setup_len = case
                        .get("setup")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len);
                    let expect = case["expect"]
                        .as_object()
                        .unwrap_or_else(|| panic!("case {id}: expect"));
                    validate_wire_expect(id, expect, setup_len);
                    validate_request_refs(id, &case["request"], setup_len);

                    if let Some(advance) = case.get("advance") {
                        let instants = advance["instants"]
                            .as_array()
                            .unwrap_or_else(|| panic!("case {id}: advance.instants"));
                        assert!(!instants.is_empty(), "case {id}: empty advance.instants");
                        for instant in instants {
                            validate_step_reference(
                                id,
                                instant["from"]
                                    .as_str()
                                    .unwrap_or_else(|| panic!("case {id}: advance.from")),
                                setup_len,
                            );
                            assert!(
                                instant["path"].as_str().is_some(),
                                "case {id}: advance instant path"
                            );
                            assert!(
                                instant["offset_seconds"].as_i64().is_some(),
                                "case {id}: advance instant offset_seconds"
                            );
                        }
                        if let Some(choose) = advance.get("choose") {
                            assert_eq!(choose.as_str(), Some("min"), "case {id}: advance.choose");
                            assert!(
                                instants.len() > 1,
                                "case {id}: advance.choose needs several instants"
                            );
                        }
                    }
                    if let Some(release) = case.get("release") {
                        let steps = release
                            .as_array()
                            .unwrap_or_else(|| panic!("case {id}: release must be an array"));
                        assert!(!steps.is_empty(), "case {id}: empty release");
                        for step in steps {
                            validate_step_reference(
                                id,
                                step.as_str()
                                    .unwrap_or_else(|| panic!("case {id}: release reference")),
                                setup_len,
                            );
                        }
                    }
                    if let Some(verify) = case.get("verify_export") {
                        validate_step_reference(
                            id,
                            verify["receipt"]
                                .as_str()
                                .unwrap_or_else(|| panic!("case {id}: verify_export.receipt")),
                            setup_len,
                        );
                    }
                    if let Some(inconclusive) = case.get("inconclusive_when") {
                        let conditions = inconclusive.as_array().unwrap_or_else(|| {
                            panic!("case {id}: inconclusive_when must be an array")
                        });
                        assert!(!conditions.is_empty(), "case {id}: empty inconclusive_when");
                        for condition in conditions {
                            assert!(
                                condition["path"].as_str().is_some(),
                                "case {id}: inconclusive_when.path"
                            );
                            assert_eq!(
                                condition["isEmpty"].as_bool(),
                                Some(true),
                                "case {id}: inconclusive_when.isEmpty"
                            );
                            validate_value_reference(
                                id,
                                condition["path"].as_str().expect("inconclusive path"),
                                setup_len,
                            );
                        }
                    }
                    if let Some(stage_from) = case.get("stage_from") {
                        validate_stage_from(id, stage_from);
                    }
                    if let Some(pinned) = case.get("pinned_basis_check") {
                        validate_value_reference(
                            id,
                            pinned["reflection"].as_str().unwrap_or_else(|| {
                                panic!("case {id}: pinned_basis_check.reflection")
                            }),
                            setup_len,
                        );
                        assert!(
                            pinned["inconclusive_on_mismatch"].as_bool().is_some(),
                            "case {id}: pinned_basis_check.inconclusive_on_mismatch"
                        );
                    }
                    if let Some(remap) = case.get("remap_policy_check") {
                        validate_remap_policy(id, case, remap, setup_len);
                    }
                    let declares_policy = POLICY_DIRECTIVES
                        .iter()
                        .any(|directive| case.get(*directive).is_some());
                    let provider_class = case.get("provider_class").map(|value| {
                        value
                            .as_str()
                            .unwrap_or_else(|| panic!("case {id}: provider_class"))
                    });
                    if declares_policy {
                        let class = provider_class.unwrap_or_else(|| {
                            panic!("case {id}: policy directives require provider_class")
                        });
                        assert!(
                            PROVIDER_CLASSES.contains(&class),
                            "case {id}: unknown provider_class {class}"
                        );
                        // A cooperative provider is the corpus's own dual-provider
                        // run: a declared-policy violation must FAIL, never skip.
                        let expected_inconclusive = class == "third-party";
                        for directive in POLICY_DIRECTIVES {
                            let Some(config) = case.get(directive) else {
                                continue;
                            };
                            let flag = config
                                .get("inconclusive_on_violation")
                                .or_else(|| config.get("inconclusive_on_mismatch"))
                                .unwrap_or_else(|| {
                                    panic!("case {id}: {directive} lacks a violation flag")
                                });
                            assert_eq!(
                                flag.as_bool(),
                                Some(expected_inconclusive),
                                "case {id}: {directive} flag disagrees with provider_class {class}"
                            );
                        }
                    } else {
                        assert!(
                            provider_class.is_none(),
                            "case {id}: provider_class without policy directives"
                        );
                    }
                    if let Some(policy) = case.get("policy_version_check") {
                        validate_value_reference(
                            id,
                            policy["observed"].as_str().unwrap_or_else(|| {
                                panic!("case {id}: policy_version_check.observed")
                            }),
                            setup_len,
                        );
                        validate_value_reference(
                            id,
                            policy["pinned"].as_str().unwrap_or_else(|| {
                                panic!("case {id}: policy_version_check.pinned")
                            }),
                            setup_len,
                        );
                        assert!(
                            policy["inconclusive_on_mismatch"].as_bool().is_some(),
                            "case {id}: policy_version_check.inconclusive_on_mismatch"
                        );
                    }
                    for steps_key in ["setup", "then"] {
                        if let Some(steps) = case.get(steps_key).and_then(Value::as_array) {
                            for step in steps {
                                validate_step(id, step, setup_len);
                            }
                        }
                    }
                    wire_count += 1;
                }
                other => panic!("case {id}: surface {other:?}"),
            }
        }
    }

    // Both halves of the corpus must stay populated: engine cases are executed
    // here, wire cases are executed by the loopback runner.
    assert!(engine_count >= 30, "engine cases: {engine_count}");
    assert!(wire_count >= 40, "wire cases: {wire_count}");
    assert!(variable_count >= 10, "variable cases: {variable_count}");
}

#[test]
fn conformance_engine_cases_pass() {
    let scenarios = load_scenarios();
    let cases = engine_cases(&scenarios);
    assert!(!cases.is_empty(), "no engine cases found");

    for case in &cases {
        let first = run_engine_case(case);
        if case.value["outcome"] == "variable" {
            // Determinism per implementation: a policy that alternates results
            // must not pass by matching the reference output once.
            let second = run_engine_case(case);
            assert_eq!(
                first, second,
                "case {}: variable case is not deterministic",
                case.id
            );
        }
    }
}
