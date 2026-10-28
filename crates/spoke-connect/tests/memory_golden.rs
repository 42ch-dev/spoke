//! Cross-language memory invoke vectors (memory protocol §13).
//!
//! The fixture is shared byte-for-byte with the TypeScript harness
//! (`packages/spoke-connect-ts/tests/fixtures/memory-invoke-golden.json`), so a
//! shape drift on either codegen side fails here or in the TS twin.
//!
//! Each vector carries the exact wire request and response objects for one op.
//! This test decodes them into the generated typed pair and re-encodes them:
//! the typed layer must accept and reproduce the shared wire document, which is
//! what the client `MemoryPort` adapter and the `MemoryPortService` seam rely on
//! (payload = the op's request object, no wrapper).

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use spoke_connect::core::required_capability;

const FIXTURE: &str = include_str!("fixtures/memory-invoke-golden.json");
const TS_FIXTURE: &str =
    include_str!("../../../packages/spoke-connect-ts/tests/fixtures/memory-invoke-golden.json");

fn round_trip<T>(label: &str, value: &Value) -> Value
where
    T: DeserializeOwned + Serialize,
{
    let typed: T = serde_json::from_value(value.clone())
        .unwrap_or_else(|error| panic!("{label} does not decode into its generated type: {error}"));
    serde_json::to_value(&typed)
        .unwrap_or_else(|error| panic!("{label} does not re-encode: {error}"))
}

#[test]
fn the_fixture_is_shared_with_the_typescript_harness() {
    assert_eq!(
        FIXTURE, TS_FIXTURE,
        "the Rust and TS harnesses must consume the same vectors byte-for-byte"
    );
}

#[test]
fn every_vector_targets_a_gated_memory_op() {
    let fixture: Value = serde_json::from_str(FIXTURE).expect("fixture parses");
    let vectors = fixture["vectors"]
        .as_array()
        .expect("fixture carries a vectors array");
    assert!(!vectors.is_empty(), "the fixture carries no vectors");
    for vector in vectors {
        let op = vector["op"].as_str().expect("vector op");
        assert!(
            op.starts_with("memory."),
            "vector op {op} is a memory op"
        );
        assert!(
            required_capability(op).is_some(),
            "{op} must have a dispatch row (Task 1 table)"
        );
    }
}

#[test]
fn vectors_round_trip_through_the_generated_types() {
    let fixture: Value = serde_json::from_str(FIXTURE).expect("fixture parses");
    let vectors = fixture["vectors"]
        .as_array()
        .expect("fixture carries a vectors array");

    for vector in vectors {
        let op = vector["op"].as_str().expect("vector op");
        let request = &vector["request"];
        let response = &vector["response"];

        let (request_wire, response_wire) = match op {
            "memory.describe" => (
                round_trip::<spoke_schemas::MemoryDescribeRequest>(op, request),
                round_trip::<spoke_schemas::MemoryDescribeResponse>(op, response),
            ),
            "memory.capture" => (
                round_trip::<spoke_schemas::MemoryCaptureRequest>(op, request),
                round_trip::<spoke_schemas::MemoryCaptureResponse>(op, response),
            ),
            "memory.candidates.review" => (
                round_trip::<spoke_schemas::MemoryCandidatesReviewRequest>(op, request),
                round_trip::<spoke_schemas::MemoryCandidatesReviewResponse>(op, response),
            ),
            "memory.recall" => (
                round_trip::<spoke_schemas::MemoryRecallRequest>(op, request),
                round_trip::<spoke_schemas::MemoryRecallResponse>(op, response),
            ),
            "memory.profile.update" => (
                round_trip::<spoke_schemas::MemoryProfileUpdateRequest>(op, request),
                round_trip::<spoke_schemas::MemoryProfileUpdateResponse>(op, response),
            ),
            "memory.exports.create" => (
                round_trip::<spoke_schemas::MemoryExportsCreateRequest>(op, request),
                round_trip::<spoke_schemas::MemoryExportsCreateResponse>(op, response),
            ),
            "memory.exports.get" => (
                round_trip::<spoke_schemas::MemoryExportsGetRequest>(op, request),
                round_trip::<spoke_schemas::MemoryExportsGetResponse>(op, response),
            ),
            "memory.imports.stage" => (
                round_trip::<spoke_schemas::MemoryImportsStageRequest>(op, request),
                round_trip::<spoke_schemas::MemoryImportsStageResponse>(op, response),
            ),
            "memory.receipts.get" => (
                round_trip::<spoke_schemas::MemoryReceiptsGetRequest>(op, request),
                round_trip::<spoke_schemas::MemoryReceiptsGetResponse>(op, response),
            ),
            other => panic!("vector {other} has no typed round-trip arm"),
        };

        assert_eq!(&request_wire, request, "{op} request changed shape");
        assert_eq!(&response_wire, response, "{op} response changed shape");
    }
}
