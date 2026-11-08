//! Regression pins for the generated cognition wire types.
//!
//! `CognitionFilter.q` admits an inclusive range with either one bound or both
//! (data contract §5.1). The frozen carrier must carry a two-sided range: an
//! earlier `anyOf`-only schema shaped QRange as a one-bound enum with
//! `deny_unknown_fields`, so `{"min":0.2,"max":0.8}` could not decode at all.

use serde_json::{json, Value};
use spoke_schemas::data::cognition::QRange;

fn range(min: Option<f64>, max: Option<f64>) -> Value {
    let mut value = json!({});
    if let Some(min) = min {
        value["min"] = json!(min);
    }
    if let Some(max) = max {
        value["max"] = json!(max);
    }
    value
}

#[test]
fn two_bound_range_decodes_with_both_bounds() {
    let decoded: QRange =
        serde_json::from_value(range(Some(0.2), Some(0.8))).expect("two-sided range decodes");

    assert_eq!(decoded.min, Some(0.2));
    assert_eq!(decoded.max, Some(0.8));
}

#[test]
fn single_bound_ranges_decode() {
    let min_only: QRange =
        serde_json::from_value(range(Some(0.2), None)).expect("min-only range decodes");
    let max_only: QRange =
        serde_json::from_value(range(None, Some(0.8))).expect("max-only range decodes");

    assert_eq!(min_only.min, Some(0.2));
    assert_eq!(min_only.max, None);
    assert_eq!(max_only.min, None);
    assert_eq!(max_only.max, Some(0.8));
}

#[test]
fn range_round_trips_through_the_generated_carrier() {
    let decoded: QRange = serde_json::from_value(range(Some(0.2), Some(0.8)))
        .expect("two-sided range decodes");
    let encoded = serde_json::to_value(&decoded).expect("range encodes");

    assert_eq!(encoded, range(Some(0.2), Some(0.8)));
}

// QC1-F001 (I-000060): ObservationAccess is an open object (data contract
// §4.2: unknown access keys round-trip verbatim). The generated Rust carrier
// must not silently drop unknown keys on decode/encode.
// QC1-F002 (I-000061): a recursive `about` target that is absent must stay
// absent; serialization may not introduce `"about": null` (data contract §1).

use spoke_schemas::data::cognition::ObservationAccess;
use spoke_schemas::{MemoryExportsGetResponse, MemoryImportsStageRequest};

fn access_with_unknown_keys() -> Value {
    json!({
        "position": "balcony",
        "line_of_sight": true,
        "occluded_by": { "id": "screen" },
        "stale_air": true,
    })
}

fn order3_belief_record() -> Value {
    json!({
        "kind": "belief",
        "record_id": "rec_belief_3",
        "holder": { "kind": "principal", "namespace": "system_a", "id": "alice" },
        "target": {
            "carrier": { "kind": "knowledge_entry", "namespace": "system_a", "id": "ke_alice" },
            "target_id": "t_ho_1",
            "kind": "proposition",
            "about": {
                "carrier": { "kind": "knowledge_entry", "namespace": "system_a", "id": "ke_alice" },
                "target_id": "t_ho_2",
                "kind": "proposition",
                "about": {
                    "carrier": { "kind": "knowledge_entry", "namespace": "system_a", "id": "ke_alice" },
                    "target_id": "t_prop_1",
                    "kind": "proposition"
                }
            }
        },
        "assessor": { "kind": "principal", "namespace": "system_a", "id": "alice" },
        "assessor_role": "holder_self_report",
        "basis": {
            "kind": "evidence",
            "evidence": [
                { "kind": "knowledge_entry", "namespace": "system_a", "id": "ke_evt_1" }
            ]
        },
        "assessment_context": { "context_id": "ctx_default" },
        "effective_at": "2026-07-23T09:00:00Z",
        "assessed_at": "2026-07-23T09:05:00Z",
        "view": "current",
        "order": 3,
        "q": {
            "value": 0.9,
            "assertion": "belief_attribution",
            "method": "elicitation_v1",
            "calibration": "calibrated"
        },
        "extensions": {}
    })
}

fn perception_record_with_unknown_access() -> Value {
    json!({
        "kind": "perception",
        "record_id": "rec_perc_1",
        "holder": { "kind": "principal", "namespace": "system_a", "id": "alice" },
        "target": {
            "carrier": { "kind": "timeline_event", "namespace": "system_a", "id": "evt_1" },
            "target_id": "t_content_1",
            "kind": "perceived_content"
        },
        "assessor": { "kind": "principal", "namespace": "system_a", "id": "alice" },
        "assessor_role": "holder_self_report",
        "basis": {
            "kind": "evidence",
            "evidence": [
                { "kind": "timeline_event", "namespace": "system_a", "id": "evt_1" }
            ]
        },
        "assessment_context": { "context_id": "ctx_default" },
        "effective_at": "2026-07-23T09:00:00Z",
        "assessed_at": "2026-07-23T09:05:00Z",
        "view": "current",
        "episode": { "kind": "timeline_event", "namespace": "system_a", "id": "evt_1" },
        "content": { "saw": "a figure" },
        "access": access_with_unknown_keys(),
        "q": {
            "value": 1,
            "assertion": "actual_capture",
            "method": "capture_log",
            "calibration": "unknown"
        },
        "extensions": {}
    })
}

fn tail_chunk_with_cognition_records() -> Value {
    json!({
        "export_id": "exp_wire_qc1",
        "chunk_ordinal": 1,
        "chunk_count": 1,
        "bundle_digest": "b".repeat(64),
        "chunk_digest": "c".repeat(64),
        "bytes": 2048,
        "records": [
            {
                "schema_version": 1,
                "record_id": "rec_wire_qc1_1",
                "space_id": "spc_wire_qc1",
                "subject_id": "toy:character:mira",
                "kind": "fact",
                "content": {
                    "text": "Wire fidelity regression record.",
                    "cognition": [
                        order3_belief_record(),
                        perception_record_with_unknown_access(),
                    ]
                },
                "scope": { "visibility": "shared" },
                "provenance": {
                    "source_context_ids": [],
                    "anchors": [],
                    "captured_by": "prn_wire_qc1",
                    "captured_at": "2026-07-23T06:05:00Z"
                },
                "created_at": "2026-07-23T06:06:12Z",
                "updated_at": "2026-07-23T06:06:12Z",
                "revision": 1,
                "extensions": {}
            }
        ]
    })
}

fn export_response_with_cognition_records() -> Value {
    json!({ "chunk": tail_chunk_with_cognition_records() })
}

fn import_stage_request_with_cognition_records() -> Value {
    json!({
        "space_id": "spc_wire_qc1_target",
        "chunk": tail_chunk_with_cognition_records(),
        "idempotency_key": "wire-qc1-stage-0",
    })
}

fn record_access(encoded: &Value) -> &Value {
    &encoded["chunk"]["records"][0]["content"]["cognition"][1]["access"]
}

fn record_target(encoded: &Value) -> &Value {
    &encoded["chunk"]["records"][0]["content"]["cognition"][0]["target"]
}

#[test]
fn observation_access_round_trips_unknown_keys() {
    let decoded: ObservationAccess =
        serde_json::from_value(access_with_unknown_keys()).expect("open access object decodes");
    let encoded = serde_json::to_value(&decoded).expect("access re-encodes");

    assert_eq!(encoded["position"], "balcony");
    assert_eq!(encoded["occluded_by"], json!({ "id": "screen" }));
    assert_eq!(encoded["stale_air"], true);
}

#[test]
fn export_carrier_preserves_unknown_nested_access_keys() {
    let decoded: MemoryExportsGetResponse =
        serde_json::from_value(export_response_with_cognition_records())
            .expect("export response with cognition records decodes");
    let encoded = serde_json::to_value(&decoded).expect("export response re-encodes");

    let access = record_access(&encoded);
    assert_eq!(access["occluded_by"], json!({ "id": "screen" }));
    assert_eq!(access["stale_air"], true);
}

#[test]
fn import_carrier_preserves_unknown_nested_access_keys() {
    let decoded: MemoryImportsStageRequest =
        serde_json::from_value(import_stage_request_with_cognition_records())
            .expect("stage request with cognition records decodes");
    let encoded = serde_json::to_value(&decoded).expect("stage request re-encodes");

    let access = record_access(&encoded);
    assert_eq!(access["occluded_by"], json!({ "id": "screen" }));
    assert_eq!(access["stale_air"], true);
}

#[test]
fn export_recursive_about_stays_absent_not_null() {
    let decoded: MemoryExportsGetResponse =
        serde_json::from_value(export_response_with_cognition_records())
            .expect("export response with cognition records decodes");
    let encoded = serde_json::to_value(&decoded).expect("export response re-encodes");

    // Order-3 chain: t_ho_1 -> t_ho_2 -> t_prop_1 (leaf without `about`).
    let target = record_target(&encoded);
    assert_eq!(target["target_id"], "t_ho_1");
    assert_eq!(target["about"]["target_id"], "t_ho_2");
    let leaf = &target["about"]["about"];
    assert_eq!(leaf["target_id"], "t_prop_1");
    assert!(
        leaf.get("about").is_none(),
        "absent leaf `about` must stay absent, got: {leaf}"
    );

    // No `"about": null` may appear anywhere in the re-encoded output.
    let serialized = serde_json::to_string(&encoded).expect("encoded string");
    assert!(
        !serialized.contains("\"about\":null"),
        "serialization introduced a forbidden null `about`: {serialized}"
    );
}
