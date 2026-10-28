//! Regression: receipt-shaped memory responses stay constructible and
//! wire-identical.
//!
//! The mutating-op response schemas intersected the whole `MemoryReceipt`
//! union (`allOf` over the root `oneOf` of committed/pending/failed) with the
//! per-op pins. typify cannot intersect a union, so it emitted
//! `pub enum CommittedReceipt {}` / `OneOfVariant1 {}` — unconstructible, and
//! a typed `MemoryPort` provider could not answer a committed receipt.
//!
//! The schemas now intersect the specific receipt variant
//! (`...memory-receipt.schema.json#/definitions/CommittedReceipt` and friends).
//! The accepted wire documents are unchanged: the pins already fix `status` to
//! exactly one variant, so `(Committed ∪ Pending ∪ Failed) ∩ pins` equals
//! `Variant ∩ pins`. These tests pin both halves of that claim — the generated
//! Rust types construct explicitly, and the JSON is the same document the
//! schema examples describe.

use serde_json::{json, Value};

use spoke_schemas::generated::ops::memory_capture_response as capture;
use spoke_schemas::generated::ops::memory_exports_create_response as exports;
use spoke_schemas::{MemoryCaptureResponse, MemoryExportsCreateResponse};

/// `§15.3` committed `memory.capture` receipt — the wire document a provider
/// answers with.
const COMMITTED_CAPTURE: &str = r#"{
  "schema_version": 1,
  "receipt_id": "rcpt_01J8ZQ9W2N0EXAMPLE0000099",
  "op": "memory.capture",
  "space_id": "spc_local_default",
  "requester_principal_id": "prn_nexus_host_local",
  "idempotency_key": "cap-9182-1400-mira",
  "payload_digest": "034a5c6d74a6c8fa3fa25d0e41dd2b10d1a110464c5e9e3f56e7c1a22cf48696",
  "status": "committed",
  "result": {
    "candidate_ids": ["cand_01J8ZQ9X7P0EXAMPLE0000101"],
    "classified": 1
  },
  "created_at": "2026-10-28T09:20:01Z",
  "completed_at": "2026-10-28T09:20:01Z",
  "extensions": {}
}"#;

/// Asynchronous `memory.exports.create` pending receipt (§6.10 pending shape:
/// no `result`, no `completed_at`).
const PENDING_EXPORT: &str = r#"{
  "schema_version": 1,
  "receipt_id": "rcpt_01J8ZR6C3S0EXAMPLE0000401",
  "op": "memory.exports.create",
  "space_id": "spc_local_default",
  "requester_principal_id": "prn_nexus_cli_operator",
  "idempotency_key": "exp-2026-10-28T10:00Z",
  "payload_digest": "58c03d12c3e307e4483a49f144efb87a81f4c1b16724ac8fefe8b1c173e3e136",
  "status": "pending",
  "created_at": "2026-10-28T10:00:00Z",
  "extensions": {}
}"#;

/// Failed receipt (`result` absent, `error` present, `details.retry_class`
/// required by the §14.2 memory error envelope).
const FAILED_CAPTURE: &str = r#"{
  "schema_version": 1,
  "receipt_id": "rcpt_01J8ZR7D4T0EXAMPLE0000501",
  "op": "memory.capture",
  "space_id": "spc_local_default",
  "requester_principal_id": "prn_nexus_host_local",
  "idempotency_key": "cap-failed-1",
  "payload_digest": "1111111111111111111111111111111111111111111111111111111111111111",
  "status": "failed",
  "error": {
    "code": "INVALID_INPUT",
    "message": "capture content is empty",
    "details": {"retry_class": "terminal"},
    "extensions": {}
  },
  "created_at": "2026-10-28T09:20:01Z",
  "completed_at": "2026-10-28T09:20:01Z",
  "extensions": {}
}"#;

fn value(fixture: &str) -> Value {
    serde_json::from_str(fixture).expect("fixture is valid JSON")
}

fn timestamp(raw: &str) -> chrono::DateTime<chrono::Utc> {
    raw.parse().expect("timestamp parses")
}

#[test]
fn committed_capture_receipt_is_constructible_and_wire_identical() {
    let receipt = capture::CommittedReceipt {
        completed_at: capture::Shared0(timestamp("2026-10-28T09:20:01Z")),
        created_at: timestamp("2026-10-28T09:20:01Z"),
        extensions: Default::default(),
        idempotency_key: "cap-9182-1400-mira".parse().expect("idempotency key"),
        op: capture::CommittedReceiptOp::MemoryCapture,
        payload_digest: "034a5c6d74a6c8fa3fa25d0e41dd2b10d1a110464c5e9e3f56e7c1a22cf48696"
            .parse()
            .expect("payload digest"),
        receipt_id: "rcpt_01J8ZQ9W2N0EXAMPLE0000099"
            .parse()
            .expect("receipt id"),
        requester_principal_id: "prn_nexus_host_local".parse().expect("principal"),
        result: capture::CommittedReceiptResult {
            candidate_ids: vec!["cand_01J8ZQ9X7P0EXAMPLE0000101".to_string()],
            classified: 1,
        },
        schema_version: std::num::NonZeroU64::new(1).expect("schema version"),
        space_id: "spc_local_default".parse().expect("space id"),
        status: capture::CommittedReceiptStatus::Committed,
    };
    let typed = MemoryCaptureResponse::Variant0(capture::OneOf::Variant0(receipt));

    assert_eq!(
        serde_json::to_value(&typed).expect("committed receipt serializes"),
        value(COMMITTED_CAPTURE),
        "the constructed receipt must be the §15.3 wire document"
    );

    // Round-trip: the same document parses back into the committed branch and
    // re-serializes identically.
    let parsed: MemoryCaptureResponse =
        serde_json::from_str(COMMITTED_CAPTURE).expect("committed receipt parses");
    assert!(
        matches!(
            parsed,
            MemoryCaptureResponse::Variant0(capture::OneOf::Variant0(_))
        ),
        "the committed branch is selected"
    );
    assert_eq!(
        serde_json::to_value(&parsed).expect("parsed receipt serializes"),
        value(COMMITTED_CAPTURE)
    );
}

#[test]
fn pending_exports_create_receipt_is_constructible_and_wire_identical() {
    let receipt = exports::OneOf::Variant1 {
        created_at: exports::Shared0(timestamp("2026-10-28T10:00:00Z")),
        extensions: exports::Shared2(Default::default()),
        idempotency_key: Some(
            "exp-2026-10-28T10:00Z"
                .parse()
                .expect("idempotency key"),
        ),
        op: exports::OneOfVariant1Op::MemoryExportsCreate,
        payload_digest: "58c03d12c3e307e4483a49f144efb87a81f4c1b16724ac8fefe8b1c173e3e136"
            .parse()
            .expect("payload digest"),
        receipt_id: "rcpt_01J8ZR6C3S0EXAMPLE0000401"
            .parse()
            .expect("receipt id"),
        requester_principal_id: "prn_nexus_cli_operator".parse().expect("principal"),
        schema_version: exports::Shared1(
            std::num::NonZeroU64::new(1).expect("schema version"),
        ),
        space_id: "spc_local_default".parse().expect("space id"),
        status: exports::OneOfVariant1Status::Pending,
    };
    let typed = MemoryExportsCreateResponse::Variant0(receipt);

    assert_eq!(
        serde_json::to_value(&typed).expect("pending receipt serializes"),
        value(PENDING_EXPORT),
        "the constructed pending receipt must be the §6.10 wire document"
    );

    let parsed: MemoryExportsCreateResponse =
        serde_json::from_str(PENDING_EXPORT).expect("pending receipt parses");
    assert!(
        matches!(
            parsed,
            MemoryExportsCreateResponse::Variant0(exports::OneOf::Variant1 { .. })
        ),
        "the pending branch is selected"
    );
    assert_eq!(
        serde_json::to_value(&parsed).expect("parsed receipt serializes"),
        value(PENDING_EXPORT)
    );
}

#[test]
fn failed_capture_receipt_round_trips() {
    let parsed: MemoryCaptureResponse =
        serde_json::from_str(FAILED_CAPTURE).expect("failed receipt parses");
    assert!(
        matches!(
            parsed,
            MemoryCaptureResponse::Variant0(capture::OneOf::Variant1 { .. })
        ),
        "the failed branch is selected"
    );
    assert_eq!(
        serde_json::to_value(&parsed).expect("failed receipt serializes"),
        value(FAILED_CAPTURE)
    );
    // The pins stay closed: a committed receipt document never parses as the
    // failed branch and vice versa (one receipt shape per op branch).
    assert!(serde_json::from_value::<MemoryCaptureResponse>(json!({
        "schema_version": 1,
        "receipt_id": "rcpt_x",
        "op": "memory.capture",
        "space_id": "spc_local_default",
        "requester_principal_id": "prn_nexus_host_local",
        "payload_digest": "2222222222222222222222222222222222222222222222222222222222222222",
        "status": "committed",
        "error": {
            "code": "INVALID_INPUT",
            "message": "m",
            "details": {"retry_class": "terminal"},
            "extensions": {}
        },
        "created_at": "2026-10-28T09:20:01Z",
        "completed_at": "2026-10-28T09:20:01Z",
        "extensions": {}
    }))
    .is_err());
}
