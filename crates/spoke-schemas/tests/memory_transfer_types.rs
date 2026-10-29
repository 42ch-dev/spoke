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
        serde_json::from_value(request).expect("stage request with a header chunk decodes");
    let reencoded = serde_json::to_value(&decoded).expect("stage request re-encodes");
    assert_eq!(reencoded["space_id"], "spc_tw_import_target");
    assert_eq!(reencoded["chunk"]["chunk_ordinal"], 0);
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
