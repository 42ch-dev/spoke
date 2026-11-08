//! Regression pins for the generated memory transfer / receipt-polling types.
//!
//! Two representation defects surfaced while building the SQLite reference
//! provider. `imports.activate`'s completion result is a closed three-field
//! object, so an extra provider field breaks `receipts.get` decoding; and the
//! export-chunk union must accept the header chunk a provider emits.

use serde_json::{json, Value};
use spoke_schemas::data::memory_export_chunk::HeaderChunk;
use spoke_schemas::{MemoryExportChunk, MemoryImportsStageRequest, MemoryReceiptsGetResponse};

fn header_chunk() -> Value {
    json!({
        "export_id": "exp_tw_regression_0001",
        "chunk_ordinal": 0,
        "chunk_count": 1,
        "bundle_digest": "b".repeat(64),
        "chunk_digest": "c".repeat(64),
        "bytes": 1234,
        "header": {
            "schema_version": 1,
            "bundle_format_version": 1,
            "memory_protocol_version": 1,
            "export_id": "exp_tw_regression_0001",
            "space_id": "spc_tw_harbor",
            "created_at": "2026-07-23T06:00:00Z",
            "source": {
                "authority_host_id": "host_tw_primary",
                "authority_generation": 1,
                "engine_policy_version": "spoke-memory/1.0.0-deterministic"
            },
            "digest": "d".repeat(64),
            "extensions": {}
        },
        "subjects": [],
        "records": [],
        "candidates": [],
        "profiles": [],
        "grants": [],
        "receipt_summaries": []
    })
}

#[test]
fn header_chunk_decodes_as_the_named_type_and_the_union() {
    let value = header_chunk();
    let named: HeaderChunk =
        serde_json::from_value(value.clone()).expect("named header chunk decodes");
    let union: MemoryExportChunk =
        serde_json::from_value(value.clone()).expect("chunk union decodes");
    match union {
        MemoryExportChunk::HeaderChunk(chunk) => {
            assert_eq!(chunk.chunk_count.get(), named.chunk_count.get())
        }
        MemoryExportChunk::TailChunk(_) => panic!("header chunk decoded as a tail chunk"),
    }
}

#[test]
fn header_chunk_is_representable_as_an_imports_stage_request_member() {
    let request = json!({
        "space_id": "spc_tw_import_target",
        "chunk": header_chunk(),
        "conflict_policy": "fail",
        "idempotency_key": "stage-regression-0",
    });
    let decoded: MemoryImportsStageRequest =
        serde_json::from_value(request.clone()).expect("stage request with a header chunk decodes");
    let reencoded = serde_json::to_value(&decoded).expect("stage request re-encodes");
    assert_eq!(reencoded["space_id"], "spc_tw_import_target");
    assert_eq!(reencoded["chunk"]["chunk_ordinal"], 0);
}

#[test]
fn header_chunk_round_trips_explicit_empty_arrays() {
    // §6.13 seals the chunk's exact JSON minus `chunk_digest`: an explicitly
    // present `[]` is part of those bytes and must survive the typed
    // transport. Absence stays absent; an explicit `null` or an unknown
    // member stays rejected.
    let chunk = header_chunk();
    let request = json!({
        "space_id": "spc_tw_import_target",
        "chunk": chunk.clone(),
        "conflict_policy": "fail",
        "idempotency_key": "stage-regression-empty-arrays",
    });
    let decoded: MemoryImportsStageRequest =
        serde_json::from_value(request.clone()).expect("sealed chunk decodes");
    let reencoded = serde_json::to_value(&decoded).expect("sealed chunk re-encodes");
    assert_eq!(
        reencoded["chunk"], request["chunk"],
        "the typed round-trip preserves every explicit empty array"
    );

    let union: MemoryExportChunk =
        serde_json::from_value(chunk.clone()).expect("chunk union decodes");
    assert_eq!(
        serde_json::to_value(&union).expect("chunk union re-encodes"),
        chunk
    );

    let mut with_null = chunk.clone();
    with_null["profiles"] = json!(null);
    assert!(
        serde_json::from_value::<HeaderChunk>(with_null).is_err(),
        "an explicit null array stays invalid"
    );

    let mut with_unknown = chunk;
    with_unknown["unexpected"] = json!(1);
    assert!(
        serde_json::from_value::<HeaderChunk>(with_unknown).is_err(),
        "an unknown member stays rejected"
    );
}

#[test]
fn content_round_trips_absent_and_explicit_empty_cognition() {
    // §7.1 admission measures the complete supplied `Content`: absent and `[]`
    // are different wire values, so the typed carrier keeps them distinct.
    use spoke_schemas::memory_capture_request::Content;

    let absent: Content =
        serde_json::from_value(json!({"text": "Memory"})).expect("absent cognition decodes");
    assert_eq!(
        serde_json::to_value(&absent).expect("absent content re-encodes"),
        json!({"text": "Memory"})
    );

    let empty: Content = serde_json::from_value(json!({"text": "Memory", "cognition": []}))
        .expect("explicit empty cognition decodes");
    assert_eq!(
        serde_json::to_value(&empty).expect("explicit empty content re-encodes"),
        json!({"text": "Memory", "cognition": []})
    );

    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/cognition/memory-lifecycle-vectors.json"
    );
    let raw = std::fs::read_to_string(path).expect("lifecycle vectors readable");
    let file: Value = serde_json::from_str(&raw).expect("lifecycle vectors parse");
    let nonempty = file["vectors"]
        .as_array()
        .expect("vectors array")
        .iter()
        .find(|vector| vector["id"] == "lifecycle-admission-cognition-valid")
        .expect("non-empty valid cognition vector")["content"]
        .clone();
    let typed: Content = serde_json::from_value(nonempty.clone()).expect("non-empty content decodes");
    assert_eq!(
        serde_json::to_value(&typed).expect("non-empty content re-encodes"),
        nonempty
    );

    assert!(
        serde_json::from_value::<Content>(json!({"text": "Memory", "cognition": null})).is_err(),
        "an explicit null cognition stays invalid"
    );
    assert!(
        serde_json::from_value::<Content>(json!({"text": "Memory", "unexpected": 1})).is_err(),
        "an unknown member stays rejected"
    );
}

#[test]
fn committed_activate_receipt_with_nested_counts_decodes() {
    let receipt = json!({
        "receipt": {
            "schema_version": 1,
            "receipt_id": "rcpt_regression_0001",
            "op": "memory.imports.activate",
            "space_id": "spc_tw_import_target",
            "requester_principal_id": "prn_tw_cli_operator",
            "idempotency_key": "act-regression",
            "payload_digest": "a".repeat(64),
            "status": "committed",
            "result": {
                "import_id": "imp_tw_regression_0001",
                "activated": {
                    "subjects": 2,
                    "records": 3,
                    "candidates": 1,
                    "profiles": 1,
                    "reflections": 1,
                    "receipt_summaries": 1
                },
                "authority_generation": 2
            },
            "created_at": "2026-07-23T06:00:00Z",
            "completed_at": "2026-07-23T06:00:00Z",
            "extensions": {}
        }
    });
    let response: MemoryReceiptsGetResponse =
        serde_json::from_value(receipt).expect("polled activate receipt decodes");
    let value = serde_json::to_value(&response).expect("response re-encodes");
    assert_eq!(value["receipt"]["result"]["activated"]["records"], 3);
}

/// One shared presence vector over every generated `Content` clone: the six
/// already-correct carriers plus the nine read/review clones corrected here.
/// Absent, explicit `[]`, and a valid non-empty recorded-at-time array must
/// stay distinct; an explicit `null` stays rejected.
#[test]
fn every_generated_content_clone_distinguishes_absent_empty_and_nonempty_cognition() {
    use serde::{de::DeserializeOwned, Serialize};
    use spoke_schemas::generated::{data, ops};

    fn check<T: DeserializeOwned + Serialize>(
        label: &str,
        absent: &Value,
        empty: &Value,
        nonempty: &Value,
    ) {
        for (case, expected) in [
            ("absent", absent),
            ("explicit empty", empty),
            ("non-empty", nonempty),
        ] {
            let typed: T = serde_json::from_value(expected.clone())
                .unwrap_or_else(|error| panic!("{label} {case} decodes: {error}"));
            assert_eq!(
                serde_json::to_value(&typed).expect("content re-encodes"),
                *expected,
                "{label} {case} keeps its cognition presence"
            );
        }
        let mut with_null = empty.clone();
        with_null["cognition"] = Value::Null;
        assert!(
            serde_json::from_value::<T>(with_null).is_err(),
            "{label}: an explicit null cognition stays invalid"
        );
    }

    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/cognition/memory-lifecycle-vectors.json"
    );
    let raw = std::fs::read_to_string(path).expect("lifecycle vectors readable");
    let file: Value = serde_json::from_str(&raw).expect("lifecycle vectors parse");
    let cognition = file["vectors"]
        .as_array()
        .expect("vectors array")
        .iter()
        .find(|vector| vector["id"] == "lifecycle-admission-cognition-valid")
        .expect("non-empty valid cognition vector")["content"]["cognition"]
        .clone();
    assert!(
        cognition.as_array().is_some_and(|records| !records.is_empty()),
        "the shared non-empty vector carries recorded cognition"
    );

    let absent = json!({"text": "Memory"});
    let empty = json!({"text": "Memory", "cognition": []});
    let nonempty = json!({"text": "Memory", "cognition": cognition});

    check::<data::retained_record::Content>("data/retained-record", &absent, &empty, &nonempty);
    check::<data::memory_candidate::Content>("data/memory-candidate", &absent, &empty, &nonempty);
    check::<data::memory_export_bundle::Shared57>("data/memory-export-bundle", &absent, &empty, &nonempty);
    check::<data::memory_export_chunk::Shared64>("data/memory-export-chunk", &absent, &empty, &nonempty);
    check::<ops::memory_records_get_response::Shared57>("ops/memory-records-get-response", &absent, &empty, &nonempty);
    check::<ops::memory_records_list_response::Shared57>("ops/memory-records-list-response", &absent, &empty, &nonempty);
    check::<ops::memory_recall_response::Shared57>("ops/memory-recall-response", &absent, &empty, &nonempty);
    check::<ops::memory_candidates_get_response::Content>("ops/memory-candidates-get-response", &absent, &empty, &nonempty);
    check::<ops::memory_candidates_list_response::Content>("ops/memory-candidates-list-response", &absent, &empty, &nonempty);
    check::<ops::memory_candidates_review_request::Content>("ops/memory-candidates-review-request content_override", &absent, &empty, &nonempty);
    check::<ops::memory_candidates_review_request::Shared52>("ops/memory-candidates-review-request fragments", &absent, &empty, &nonempty);
    check::<ops::memory_capture_request::Content>("ops/memory-capture-request", &absent, &empty, &nonempty);
    check::<ops::memory_exports_get_response::Shared66>("ops/memory-exports-get-response", &absent, &empty, &nonempty);
    check::<ops::memory_imports_stage_request::Shared57>("ops/memory-imports-stage-request", &absent, &empty, &nonempty);
    check::<ops::memory_records_create_request::Content>("ops/memory-records-create-request", &absent, &empty, &nonempty);
    check::<ops::memory_records_update_request::Content>("ops/memory-records-update-request content", &absent, &empty, &nonempty);
    check::<ops::memory_records_update_request::Shared52>("ops/memory-records-update-request kind+content", &absent, &empty, &nonempty);
}
