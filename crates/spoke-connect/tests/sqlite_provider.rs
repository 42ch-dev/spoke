//! Typed-port tests for the SQLite reference provider — storage plus the
//! low-complexity op groups landed in sub-round 3a (`describe`, `spaces.*`,
//! `subjects.*`, `grants.*`, `receipts.get`).
//!
//! The provider is exercised through the `MemoryPort` boundary exactly as the
//! loopback host serves it, so these tests cover provider duties (admission,
//! CAS, idempotency, fencing, receipt retention, snapshot semantics) rather than
//! the wire encoding.

#![cfg(feature = "remote-adapter")]

#[path = "support/sqlite_memory.rs"]
mod sqlite_memory;

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use spoke_fixture_memory_engine::reflect::{basis_digests, ReflectionBasis, SourceRef};
use spoke_fixture_memory_engine::ENGINE_POLICY_VERSION;
use spoke_operations::{content_serialized_bytes, jcs_sha256_hex, MemoryPort, SpokeRejectCode, SpokeResult};
use spoke_schemas::data::cognition::CognitionFilter;
use spoke_schemas::data::provenance_filter::ProvenanceFilter;
use spoke_schemas::data::read_selection::ReadSelection;
use spoke_schemas::{
    MemoryDescribeRequest, MemoryExportsCreateRequest, MemoryGrantsGrantRequest,
    MemoryGrantsListRequest, MemoryGrantsRevokeRequest, MemoryImportsStageRequest,
    MemoryRecallRequest, MemoryReceiptsGetRequest, MemoryReceiptsGetResponse,
    MemoryRecordsListRequest, MemoryReflectionsGenerateRequest, MemoryReflectionsListRequest,
    MemoryReflectionsListResponse, MemoryReflectionsStateRequest, MemorySpacesFreezeRequest,
    MemorySpacesListRequest,
    MemorySpacesUnfreezeRequest, MemorySubjectsEnsureRequest, MemorySubjectsFenceRequest,
    MemorySubjectsGetRequest, Principal,
};
use sqlite_memory::SqliteMemoryProvider;

const SPACE: &str = "spc_sqlite_test";
const OTHER_SPACE: &str = "spc_sqlite_other";
const OPERATOR: &str = "prn_operator";
const READER: &str = "prn_reader";
const SUBJECT: &str = "toy:character:mira";
const CONTEXT: &str = "ctx_customs_gate";

fn principal(id: &str) -> Principal {
    serde_json::from_value(json!({ "principal_id": id, "extensions": {} }))
        .expect("valid principal")
}

fn wire<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("value serializes")
}

fn expect_ok<T>(result: SpokeResult<T>) -> T {
    match result {
        SpokeResult::Ok(value) => value,
        SpokeResult::Reject(reject) => panic!("expected success, got {reject:?}"),
    }
}

fn expect_reject<T: std::fmt::Debug>(result: SpokeResult<T>) -> SpokeRejectCode {
    match result {
        SpokeResult::Ok(value) => panic!("expected a rejection, got {value:?}"),
        SpokeResult::Reject(reject) => reject.code,
    }
}

fn expect_reject_with<T: std::fmt::Debug>(
    result: SpokeResult<T>,
) -> spoke_operations::SpokeReject {
    match result {
        SpokeResult::Ok(value) => panic!("expected a rejection, got {value:?}"),
        SpokeResult::Reject(reject) => reject,
    }
}

/// The consumer-visible detail members that disclose a receipt: its retained
/// outcome (`last_known_status`) or its body (`receipt`). Exact names only — a
/// harmless diagnostic such as `request_status_check` is not one of them.
const RECEIPT_DETAIL_MEMBERS: [&str; 2] = ["last_known_status", "receipt"];

/// Every path in an error's `details` that would disclose receipt evidence,
/// at any depth: a nested `last_known_status` under an otherwise ordinary
/// member counts, a top-level key that merely contains "status" does not.
fn receipt_detail_paths(details: Option<&serde_json::Map<String, Value>>) -> Vec<String> {
    fn walk(value: &Value, path: &str, found: &mut Vec<String>) {
        match value {
            Value::Object(members) => {
                for (member, nested) in members {
                    let child = format!("{path}.{member}");
                    if RECEIPT_DETAIL_MEMBERS.contains(&member.as_str()) {
                        found.push(child.clone());
                    }
                    walk(nested, &child, found);
                }
            }
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    walk(item, &format!("{path}[{index}]"), found);
                }
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    for (member, value) in details.into_iter().flatten() {
        let path = format!("details.{member}");
        if RECEIPT_DETAIL_MEMBERS.contains(&member.as_str()) {
            found.push(path.clone());
        }
        walk(value, &path, &mut found);
    }
    found
}

fn grant_in(
    space: &str,
    grant_id: &str,
    principal_id: &str,
    capabilities: &[&str],
    subject: Option<&str>,
    contexts: Option<&[&str]>,
    shared_access: Option<bool>,
) -> Value {
    let mut value = json!({
        "schema_version": 1,
        "grant_id": grant_id,
        "space_id": space,
        "principal_id": principal_id,
        "capabilities": capabilities,
        "created_at": "2026-07-23T05:30:00Z",
        "revision": 1,
        "extensions": {},
    });
    if let Some(subject) = subject {
        value["subject_id"] = json!(subject);
    }
    if let Some(contexts) = contexts {
        value["context_ids"] = json!(contexts);
        if let Some(shared) = shared_access {
            value["shared_access"] = json!(shared);
        }
    }
    value
}

fn grant(
    grant_id: &str,
    principal_id: &str,
    capabilities: &[&str],
    subject: Option<&str>,
    contexts: Option<&[&str]>,
    shared_access: Option<bool>,
) -> Value {
    grant_in(
        SPACE,
        grant_id,
        principal_id,
        capabilities,
        subject,
        contexts,
        shared_access,
    )
}

/// A provider with the operator's space-wide admin grant plus a subject-scoped
/// reader grant, the two corpus principals this round exercises.
fn provider() -> SqliteMemoryProvider {
    seeded(SqliteMemoryProvider::in_memory())
}

/// The seeded corpus, over any provider instance (in-memory or file-backed).
fn seeded(provider: SqliteMemoryProvider) -> SqliteMemoryProvider {
    provider.provision_space(SPACE, "host_tw_primary", &["personality"]);
    provider.provision_space(OTHER_SPACE, "host_tw_secondary", &[]);
    provider.provision_subject(SPACE, SUBJECT, Some(("toy", "character")));
    provider.provision_grant(
        SPACE,
        grant(
            "grt_operator_admin_0001",
            OPERATOR,
            &["admin"],
            None,
            None,
            None,
        ),
    );
    provider.provision_grant(
        SPACE,
        grant(
            "grt_reader_context_0002",
            READER,
            &[
                "capture",
                "review",
                "records_read",
                "records_write",
                "recall",
            ],
            Some(SUBJECT),
            Some(&[CONTEXT]),
            Some(false),
        ),
    );
    provider
}

#[tokio::test]
async fn describes_limits_and_engine_policy_version() {
    let provider = provider();
    let response = expect_ok(
        provider
            .describe(&principal(OPERATOR), MemoryDescribeRequest::default())
            .await,
    );
    let value = wire(&response);
    assert_eq!(value["engine_policy_version"], ENGINE_POLICY_VERSION);
    assert_eq!(value["memory_protocol_version"], 1);
    assert_eq!(value["limits"]["max_page_size"], 100);
    assert_eq!(value["limits"]["receipt_retention_seconds"], 604_800);
    assert!(
        value["limits"]["dedup_retention_seconds"].as_i64().unwrap()
            > value["limits"]["receipt_retention_seconds"]
                .as_i64()
                .unwrap()
    );
}

#[tokio::test]
async fn spaces_are_listed_only_for_principals_with_a_grant() {
    let provider = provider();
    let operator_view = expect_ok(
        provider
            .list_spaces(
                &principal(OPERATOR),
                serde_json::from_value(json!({ "limit": 10 })).expect("list request"),
            )
            .await,
    );
    let spaces = wire(&operator_view)["spaces"].clone();
    assert_eq!(spaces.as_array().expect("spaces array").len(), 1);
    assert_eq!(spaces[0]["space_id"], SPACE);
    assert_eq!(spaces[0]["pending_receipts"], 0);
    assert_eq!(spaces[0]["status"], "active");

    let stranger_view = expect_ok(
        provider
            .list_spaces(
                &principal("prn_stranger"),
                serde_json::from_value(json!({ "limit": 10 })).expect("list request"),
            )
            .await,
    );
    assert!(wire(&stranger_view)["spaces"]
        .as_array()
        .expect("spaces array")
        .is_empty());
}

#[tokio::test]
async fn freeze_requires_admin_and_cas_on_the_space_revision() {
    let provider = provider();
    let freeze = |revision: u64, key: &str| {
        serde_json::from_value::<MemorySpacesFreezeRequest>(json!({
            "space_id": SPACE,
            "expected_revision": revision,
            "idempotency_key": key,
        }))
        .expect("freeze request")
    };

    assert_eq!(
        expect_reject(
            provider
                .freeze_space(&principal(READER), freeze(1, "fz-1"))
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );

    let committed = expect_ok(
        provider
            .freeze_space(&principal(OPERATOR), freeze(1, "fz-2"))
            .await,
    );
    let receipt = wire(&committed);
    assert_eq!(receipt["status"], "committed");
    assert_eq!(receipt["result"]["status"], "frozen");

    // Replaying the identical request returns the same receipt (dedup).
    let replay = expect_ok(
        provider
            .freeze_space(&principal(OPERATOR), freeze(1, "fz-2"))
            .await,
    );
    assert_eq!(wire(&replay)["receipt_id"], receipt["receipt_id"]);

    // A different payload under the same key is a conflict, not a replay.
    assert_eq!(
        expect_reject(
            provider
                .freeze_space(&principal(OPERATOR), freeze(2, "fz-2"))
                .await
        ),
        SpokeRejectCode::IdempotencyConflict
    );

    // Stale revision on a fresh key: the space already moved to revision 2.
    assert_eq!(
        expect_reject(
            provider
                .freeze_space(&principal(OPERATOR), freeze(1, "fz-3"))
                .await
        ),
        SpokeRejectCode::RevisionConflict
    );

    let unfrozen = expect_ok(
        provider
            .unfreeze_space(
                &principal(OPERATOR),
                serde_json::from_value::<MemorySpacesUnfreezeRequest>(json!({
                    "space_id": SPACE,
                    "expected_revision": 2,
                    "idempotency_key": "ufz-1",
                }))
                .expect("unfreeze request"),
            )
            .await,
    );
    assert_eq!(wire(&unfrozen)["result"]["status"], "active");
}

#[tokio::test]
async fn subject_fencing_tracks_contexts_with_cas_and_validation() {
    let provider = provider();
    let ensure = expect_ok(
        provider
            .ensure_subject(
                &principal(READER),
                serde_json::from_value::<MemorySubjectsEnsureRequest>(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                }))
                .expect("ensure request"),
            )
            .await,
    );
    assert_eq!(wire(&ensure)["result"]["subject_id"], SUBJECT);
    assert_eq!(wire(&ensure)["result"]["created"], false);

    let fence = |context: &str, action: &str, revision: u64, key: &str| {
        serde_json::from_value::<MemorySubjectsFenceRequest>(json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "target": { "type": "context", "context_id": context },
            "action": action,
            "expected_revision": revision,
            "idempotency_key": key,
        }))
        .expect("fence request")
    };

    assert_eq!(
        expect_reject(
            provider
                .fence_subject(&principal(READER), fence(CONTEXT, "fence", 1, "fn-1"))
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );
    let fenced = expect_ok(
        provider
            .fence_subject(&principal(OPERATOR), fence(CONTEXT, "fence", 1, "fn-2"))
            .await,
    );
    assert_eq!(wire(&fenced)["status"], "committed");

    let subject = expect_ok(
        provider
            .get_subject(
                &principal(READER),
                serde_json::from_value::<MemorySubjectsGetRequest>(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                }))
                .expect("get request"),
            )
            .await,
    );
    let subject = wire(&subject)["subject"].clone();
    assert_eq!(subject["fenced_contexts"], json!([CONTEXT]));
    assert_eq!(subject["revision"], 2);

    // Archive is subject-only: a context target is INVALID_INPUT (§7.7).
    assert_eq!(
        expect_reject(
            provider
                .fence_subject(&principal(OPERATOR), fence(CONTEXT, "archive", 2, "fn-3"))
                .await
        ),
        SpokeRejectCode::InvalidInput
    );
    // Stale revision.
    assert_eq!(
        expect_reject(
            provider
                .fence_subject(&principal(OPERATOR), fence(CONTEXT, "unfence", 1, "fn-4"))
                .await
        ),
        SpokeRejectCode::RevisionConflict
    );

    let unfenced = expect_ok(
        provider
            .fence_subject(&principal(OPERATOR), fence(CONTEXT, "unfence", 2, "fn-5"))
            .await,
    );
    assert_eq!(wire(&unfenced)["status"], "committed");
}

#[tokio::test]
async fn grant_issuance_enforces_non_amplification() {
    let provider = provider();
    let issue = |capabilities: Vec<&str>, contexts: Vec<&str>, key: &str| {
        serde_json::from_value::<MemoryGrantsGrantRequest>(json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "principal_id": READER,
            "capabilities": capabilities,
            "context_ids": contexts,
            "shared_access": true,
            "idempotency_key": key,
        }))
        .expect("grant request")
    };

    // A non-admin granter cannot issue at all.
    assert_eq!(
        expect_reject(
            provider
                .grant(
                    &principal(READER),
                    issue(vec!["recall"], vec![CONTEXT], "gr-1")
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );

    // The space-wide admin may issue the restricted grant.
    let issued = expect_ok(
        provider
            .grant(
                &principal(OPERATOR),
                issue(vec!["recall"], vec![CONTEXT], "gr-2"),
            )
            .await,
    );
    let issued = wire(&issued);
    assert_eq!(issued["status"], "committed");

    let listed = expect_ok(
        provider
            .list_grants(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryGrantsListRequest>(json!({
                    "space_id": SPACE,
                    "limit": 10,
                }))
                .expect("list request"),
            )
            .await,
    );
    let grants = wire(&listed)["grants"].clone();
    assert_eq!(grants.as_array().expect("grants array").len(), 3);
}

#[tokio::test]
async fn grant_revocation_cas_and_replay() {
    let provider = provider();
    let revoke = |revision: u64, key: &str| {
        serde_json::from_value::<MemoryGrantsRevokeRequest>(json!({
            "space_id": SPACE,
            "grant_id": "grt_reader_context_0002",
            "expected_revision": revision,
            "idempotency_key": key,
        }))
        .expect("revoke request")
    };

    assert_eq!(
        expect_reject(
            provider
                .revoke_grant(&principal(OPERATOR), revoke(9, "rv-1"))
                .await
        ),
        SpokeRejectCode::RevisionConflict
    );

    let revoked = expect_ok(
        provider
            .revoke_grant(&principal(OPERATOR), revoke(1, "rv-2"))
            .await,
    );
    let revoked = wire(&revoked);
    assert_eq!(revoked["status"], "committed");

    // The revoked grant no longer covers the reader.
    assert_eq!(
        expect_reject(
            provider
                .ensure_subject(
                    &principal(READER),
                    serde_json::from_value::<MemorySubjectsEnsureRequest>(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                    }))
                    .expect("ensure request"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );
}

#[tokio::test]
async fn receipts_are_scoped_reads_and_expire_from_the_tombstone() {
    let provider = provider();
    let freeze = expect_ok(
        provider
            .freeze_space(
                &principal(OPERATOR),
                serde_json::from_value::<MemorySpacesFreezeRequest>(json!({
                    "space_id": SPACE,
                    "expected_revision": 1,
                    "idempotency_key": "fz-receipt",
                }))
                .expect("freeze request"),
            )
            .await,
    );
    let receipt = wire(&freeze);
    let receipt_id = receipt["receipt_id"].as_str().expect("receipt id");
    assert_eq!(receipt["status"], "committed");
    assert!(receipt.get("error").is_none());

    let read = |principal_id: &'static str| {
        serde_json::from_value::<MemoryReceiptsGetRequest>(json!({
            "space_id": SPACE,
            "receipt_id": receipt_id,
        }))
        .expect("receipt request")
    };

    // Another principal without admin cannot read the operator's receipt.
    assert_eq!(
        expect_reject(
            provider
                .get_receipt(&principal("prn_stranger"), read("prn_stranger"))
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );

    let own = expect_ok(
        provider
            .get_receipt(&principal(OPERATOR), read(OPERATOR))
            .await,
    );
    assert_eq!(wire(&own)["receipt"]["requester_principal_id"], OPERATOR);

    provider.advance_clock(604_801);
    assert_eq!(
        expect_reject(
            provider
                .get_receipt(&principal(OPERATOR), read(OPERATOR))
                .await
        ),
        SpokeRejectCode::ReceiptExpired
    );
}

#[tokio::test]
async fn fresh_receipted_work_survives_its_own_retention_window_after_advance() {
    // QC1-001: receipts and dedup tombstones stamp the provider's CURRENT
    // clock, so work created after a clock advance is not instantly expired.
    let provider = provider();
    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);

    let captured = expect_ok(
        provider
            .capture(
                &principal(READER),
                serde_json::from_value(capture_request(
                    "Fresh work after the advance.",
                    "cap-fresh",
                ))
                .expect("capture request"),
            )
            .await,
    );
    let captured = wire(&captured);
    assert_eq!(captured["status"], "committed");
    let receipt_id = captured["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();

    // Polling the fresh receipt succeeds immediately (no false eviction).
    let polled = expect_ok(
        provider
            .get_receipt(
                &principal(READER),
                serde_json::from_value::<MemoryReceiptsGetRequest>(json!({
                    "space_id": SPACE,
                    "receipt_id": receipt_id,
                }))
                .expect("receipt request"),
            )
            .await,
    );
    assert_eq!(wire(&polled)["receipt"]["status"], "committed");

    // The same-key replay still returns the same receipt.
    let replayed = expect_ok(
        provider
            .capture(
                &principal(READER),
                serde_json::from_value(capture_request(
                    "Fresh work after the advance.",
                    "cap-fresh",
                ))
                .expect("capture request"),
            )
            .await,
    );
    assert_eq!(wire(&replayed)["receipt_id"], receipt_id);
}

#[tokio::test]
async fn eviction_never_reissues_a_receipt_id() {
    // QC1-002 / qc2 W-1: the receipt-id sequence is monotonic in the document
    // (snapshots included), so after eviction new work draws a fresh id and
    // the old key answers RECEIPT_EXPIRED from the tombstone — never a
    // foreign receipt body.
    let provider = provider();
    let first = expect_ok(
        provider
            .capture(
                &principal(READER),
                serde_json::from_value(capture_request("First work.", "cap-evict-1"))
                    .expect("capture request"),
            )
            .await,
    );
    let first_id = wire(&first)["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();

    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);

    // Same key after eviction: RECEIPT_EXPIRED from the dedup tombstone.
    let replay = provider
        .capture(
            &principal(READER),
            serde_json::from_value(capture_request("First work.", "cap-evict-1"))
                .expect("capture request"),
        )
        .await;
    assert_eq!(expect_reject(replay), SpokeRejectCode::ReceiptExpired);

    // New work draws a distinct id and polls committed immediately.
    let second = expect_ok(
        provider
            .capture(
                &principal(READER),
                serde_json::from_value(capture_request("Second work.", "cap-evict-2"))
                    .expect("capture request"),
            )
            .await,
    );
    let second = wire(&second);
    let second_id = second["receipt_id"].as_str().expect("receipt id");
    assert_ne!(second_id, first_id);
    let polled = expect_ok(
        provider
            .get_receipt(
                &principal(READER),
                serde_json::from_value::<MemoryReceiptsGetRequest>(json!({
                    "space_id": SPACE,
                    "receipt_id": second_id,
                }))
                .expect("receipt request"),
            )
            .await,
    );
    assert_eq!(wire(&polled)["receipt"]["status"], "committed");
}

#[tokio::test]
async fn consumption_receipt_id_matches_and_polls_for_dismiss_and_review() {
    // QC1-004: dismiss/review allocate the receipt ONCE — the id written into
    // candidate.consumed_by.receipt_id is the returned receipt's id, and that
    // id is pollable via receipts.get.
    let provider = provider();
    let receipt_request = |receipt_id: &str| {
        serde_json::from_value::<MemoryReceiptsGetRequest>(json!({
            "space_id": SPACE,
            "receipt_id": receipt_id,
        }))
        .expect("receipt request")
    };

    // Dismiss path.
    let captured = expect_ok(
        provider
            .capture(
                &principal(READER),
                serde_json::from_value(capture_request("A dismissable note.", "cap-rcpt-1"))
                    .expect("capture request"),
            )
            .await,
    );
    let candidate_id = wire(&captured)["result"]["candidate_ids"][0]
        .as_str()
        .expect("candidate id")
        .to_string();
    let dismissed = expect_ok(
        provider
            .dismiss_candidate(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "candidate_id": candidate_id,
                    "expected_revision": 1,
                    "idempotency_key": "dis-rcpt-1",
                }))
                .expect("dismiss request"),
            )
            .await,
    );
    let dismissed = wire(&dismissed);
    let receipt_id = dismissed["receipt_id"].as_str().expect("receipt id");
    let fetched = expect_ok(
        provider
            .get_candidate(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "candidate_id": candidate_id,
                }))
                .expect("get request"),
            )
            .await,
    );
    assert_eq!(
        wire(&fetched)["candidate"]["consumed_by"]["receipt_id"],
        receipt_id
    );
    let polled = expect_ok(
        provider
            .get_receipt(&principal(READER), receipt_request(receipt_id))
            .await,
    );
    assert_eq!(wire(&polled)["receipt"]["status"], "committed");

    // Review path.
    let captured = expect_ok(
        provider
            .capture(
                &principal(READER),
                serde_json::from_value(capture_request("A promotable note.", "cap-rcpt-2"))
                    .expect("capture request"),
            )
            .await,
    );
    let candidate_id = wire(&captured)["result"]["candidate_ids"][0]
        .as_str()
        .expect("candidate id")
        .to_string();
    let reviewed = expect_ok(
        provider
            .review_candidate(
                &principal(READER),
                serde_json::from_value(review_request(
                    &candidate_id,
                    1,
                    "rev-rcpt-1",
                    json!({ "action": "promote" }),
                ))
                .expect("review request"),
            )
            .await,
    );
    let reviewed = wire(&reviewed);
    let receipt_id = reviewed["receipt_id"].as_str().expect("receipt id");
    let fetched = expect_ok(
        provider
            .get_candidate(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "candidate_id": candidate_id,
                }))
                .expect("get request"),
            )
            .await,
    );
    assert_eq!(
        wire(&fetched)["candidate"]["consumed_by"]["receipt_id"],
        receipt_id
    );
    let polled = expect_ok(
        provider
            .get_receipt(&principal(READER), receipt_request(receipt_id))
            .await,
    );
    assert_eq!(wire(&polled)["receipt"]["status"], "committed");
}

#[tokio::test]
async fn snapshot_restore_rolls_back_provider_state() {
    let provider = provider();
    provider.save_snapshot();
    let _ = expect_ok(
        provider
            .revoke_grant(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryGrantsRevokeRequest>(json!({
                    "space_id": SPACE,
                    "grant_id": "grt_reader_context_0002",
                    "expected_revision": 1,
                    "idempotency_key": "rv-snapshot",
                }))
                .expect("revoke request"),
            )
            .await,
    );
    // Coverage is gone after the revoke…
    assert_eq!(
        expect_reject(
            provider
                .ensure_subject(
                    &principal(READER),
                    serde_json::from_value::<MemorySubjectsEnsureRequest>(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                    }))
                    .expect("ensure request"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );
    // …and restored with the snapshot, dedup records included.
    assert!(provider.restore_snapshot());
    let listed = expect_ok(
        provider
            .list_grants(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryGrantsListRequest>(json!({
                    "space_id": SPACE,
                    "limit": 10,
                }))
                .expect("list request"),
            )
            .await,
    );
    let grants = wire(&listed)["grants"].clone();
    let reader_grant = grants
        .as_array()
        .expect("grants array")
        .iter()
        .find(|grant| grant["grant_id"] == "grt_reader_context_0002")
        .cloned()
        .expect("reader grant present");
    assert!(reader_grant.get("revoked_at").is_none());
    assert_eq!(reader_grant["revision"], 1);
}

// ── 3b: capture / candidates / records / recall ───────────────────────────

fn capture_request(text: &str, key: &str) -> Value {
    json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "content": { "text": text },
        "capture_scope": { "visibility": "context", "context_id": CONTEXT },
        "source_context_ids": [CONTEXT],
        "kind_hint": "fact",
        "idempotency_key": key,
    })
}

fn review_request(candidate_id: &str, revision: u64, key: &str, decision: Value) -> Value {
    json!({
        "space_id": SPACE,
        "candidate_id": candidate_id,
        "expected_revision": revision,
        "decision": decision,
        "idempotency_key": key,
    })
}

fn selection_all() -> Value {
    json!({ "mode": "subject_all" })
}

#[tokio::test]
async fn capture_classifies_and_review_is_atomic_with_placement_inheritance() {
    let provider = provider();
    let captured = expect_ok(
        provider
            .capture(
                &principal(READER),
                serde_json::from_value(capture_request(
                    "Mira told Bo the gate key is hidden under the third barrel.",
                    "cap-1",
                ))
                .expect("capture request"),
            )
            .await,
    );
    let captured = wire(&captured);
    assert_eq!(captured["status"], "committed");
    let candidate_ids = captured["result"]["candidate_ids"]
        .as_array()
        .expect("candidate ids");
    assert_eq!(candidate_ids.len(), 1);
    assert_eq!(captured["result"]["classified"], 1);
    let candidate_id = candidate_ids[0].as_str().expect("candidate id").to_string();

    let candidate = expect_ok(
        provider
            .get_candidate(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "candidate_id": candidate_id,
                }))
                .expect("get request"),
            )
            .await,
    );
    let candidate = wire(&candidate)["candidate"].clone();
    assert_eq!(candidate["status"], "pending");
    assert_eq!(candidate["capture_scope"]["context_id"], CONTEXT);
    assert!(candidate.get("content_digest").is_none());

    // A refused review is atomic: no records, candidate untouched.
    let conflict = review_request(&candidate_id, 9, "rev-1", json!({ "action": "promote" }));
    assert_eq!(
        expect_reject(
            provider
                .review_candidate(
                    &principal(READER),
                    serde_json::from_value(conflict).expect("review request"),
                )
                .await
        ),
        SpokeRejectCode::RevisionConflict
    );
    let listed = expect_ok(
        provider
            .list_records(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": selection_all(),
                }))
                .expect("list request"),
            )
            .await,
    );
    assert_eq!(wire(&listed)["total_authorized"], 0);

    // A fragment review inherits the capture_scope and scrubs the shell.
    let fragment = review_request(
        &candidate_id,
        1,
        "rev-2",
        json!({
            "action": "fragment",
            "fragments": [
                { "kind": "fact", "content": { "text": "The gate key is hidden under the third barrel." } },
                { "kind": "event", "content": { "text": "Mira revealed the key location to Bo." } },
            ],
        }),
    );
    let reviewed = expect_ok(
        provider
            .review_candidate(
                &principal(READER),
                serde_json::from_value(fragment).expect("review request"),
            )
            .await,
    );
    let reviewed = wire(&reviewed);
    assert_eq!(reviewed["result"]["outcome"], "fragmented");
    let record_ids = reviewed["result"]["record_ids"]
        .as_array()
        .expect("record ids")
        .clone();
    assert_eq!(record_ids.len(), 2);

    let record = expect_ok(
        provider
            .get_record(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": record_ids[0],
                }))
                .expect("get request"),
            )
            .await,
    );
    assert_eq!(wire(&record)["record"]["scope"]["context_id"], CONTEXT);

    // Consumption scrubs the proposed content and keeps the digest.
    let consumed = expect_ok(
        provider
            .get_candidate(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "candidate_id": candidate_id,
                }))
                .expect("get request"),
            )
            .await,
    );
    let consumed = wire(&consumed)["candidate"].clone();
    assert_eq!(consumed["status"], "consumed");
    assert_eq!(consumed["revision"], 2);
    assert!(consumed.get("content_digest").is_some());
    assert!(consumed["proposed"].get("content").is_none());

    // Consumption is terminal.
    assert_eq!(
        expect_reject(
            provider
                .review_candidate(
                    &principal(READER),
                    serde_json::from_value(review_request(
                        &candidate_id,
                        2,
                        "rev-3",
                        json!({ "action": "promote" }),
                    ))
                    .expect("review request"),
                )
                .await
        ),
        SpokeRejectCode::CandidateConsumed
    );
}

#[tokio::test]
async fn capture_honours_drop_fence_and_freeze() {
    let provider = provider();
    // Engine policy: below the minimum text length the candidate is dropped.
    let dropped = expect_ok(
        provider
            .capture(
                &principal(READER),
                serde_json::from_value(capture_request("short", "cap-drop")).expect("capture"),
            )
            .await,
    );
    assert_eq!(wire(&dropped)["result"]["classified"], 0);

    let _ = expect_ok(
        provider
            .fence_subject(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "target": { "type": "context", "context_id": CONTEXT },
                    "action": "fence",
                    "expected_revision": 1,
                    "idempotency_key": "fn-3b",
                }))
                .expect("fence request"),
            )
            .await,
    );
    assert_eq!(
        expect_reject(
            provider
                .capture(
                    &principal(READER),
                    serde_json::from_value(capture_request(
                        "Mira counts the barrels by the gate.",
                        "cap-fenced",
                    ))
                    .expect("capture"),
                )
                .await
        ),
        SpokeRejectCode::SubjectFenced
    );

    let _ = expect_ok(
        provider
            .freeze_space(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "expected_revision": 1,
                    "idempotency_key": "fz-3b",
                }))
                .expect("freeze request"),
            )
            .await,
    );
    assert_eq!(
        expect_reject(
            provider
                .capture(
                    &principal(READER),
                    serde_json::from_value(capture_request(
                        "Mira counts the barrels by the gate.",
                        "cap-frozen",
                    ))
                    .expect("capture"),
                )
                .await
        ),
        SpokeRejectCode::SpaceFrozen
    );
}

#[tokio::test]
async fn records_tombstone_cas_and_list_filters() {
    let provider = provider();
    let created = expect_ok(
        provider
            .create_record(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "kind": "fact",
                    "content": { "text": "The gate key is under the third barrel." },
                    "scope": { "visibility": "context", "context_id": CONTEXT },
                    "idempotency_key": "rec-1",
                }))
                .expect("create request"),
            )
            .await,
    );
    let record_id = wire(&created)["result"]["record_id"]
        .as_str()
        .expect("record id")
        .to_string();

    assert_eq!(
        expect_reject(
            provider
                .update_record(
                    &principal(READER),
                    serde_json::from_value(json!({
                        "space_id": SPACE,
                        "record_id": record_id,
                        "expected_revision": 9,
                        "content": { "text": "stale" },
                        "idempotency_key": "rec-upd-stale",
                    }))
                    .expect("update request"),
                )
                .await
        ),
        SpokeRejectCode::RevisionConflict
    );
    let updated = expect_ok(
        provider
            .update_record(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": record_id,
                    "expected_revision": 1,
                    "content": { "text": "The gate key is under the second barrel." },
                    "idempotency_key": "rec-upd",
                }))
                .expect("update request"),
            )
            .await,
    );
    assert_eq!(wire(&updated)["result"]["revision"], 2);

    let deleted = expect_ok(
        provider
            .delete_record(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": record_id,
                    "expected_revision": 2,
                    "idempotency_key": "rec-del",
                }))
                .expect("delete request"),
            )
            .await,
    );
    assert_eq!(wire(&deleted)["result"]["tombstoned"], true);

    let tombstoned = expect_ok(
        provider
            .get_record(
                &principal(READER),
                serde_json::from_value(json!({ "space_id": SPACE, "record_id": record_id }))
                    .expect("get request"),
            )
            .await,
    );
    let tombstoned = wire(&tombstoned)["record"].clone();
    assert_eq!(tombstoned["content"]["text"], "");
    assert!(tombstoned.get("tombstone").is_some());

    let visible = expect_ok(
        provider
            .list_records(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": selection_all(),
                }))
                .expect("list request"),
            )
            .await,
    );
    assert_eq!(wire(&visible)["total_authorized"], 0);

    let with_tombstones = expect_ok(
        provider
            .list_records(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": selection_all(),
                    "include_tombstoned": true,
                }))
                .expect("list request"),
            )
            .await,
    );
    assert_eq!(wire(&with_tombstones)["total_authorized"], 1);
}

#[tokio::test]
async fn recall_ranks_truncates_and_surfaces_overflow() {
    let provider = provider();
    for (index, id) in ["rec_a", "rec_b", "rec_c"].iter().enumerate() {
        provider.provision_record(
            SPACE,
            json!({
                "schema_version": 1,
                "record_id": id,
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "kind": "fact",
                "content": { "text": format!("barrel tally {index}") },
                "scope": { "visibility": "context", "context_id": CONTEXT },
                "provenance": {
                    "source_context_ids": [CONTEXT],
                    "anchors": [],
                    "captured_by": OPERATOR,
                    "captured_at": "2026-07-23T06:00:00Z",
                },
                "created_at": "2026-07-23T06:00:00Z",
                "updated_at": "2026-07-23T06:00:00Z",
                "revision": 1,
                "extensions": {},
            }),
        );
    }

    let capped = expect_ok(
        provider
            .recall(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": selection_all(),
                    "budget": { "max_items": 2 },
                }))
                .expect("recall request"),
            )
            .await,
    );
    let capped = wire(&capped);
    assert_eq!(capped["total_authorized"], 3);
    assert_eq!(capped["truncated"], true);
    assert_eq!(capped["records"].as_array().expect("records").len(), 2);

    // §9 progress rule: an oversized first item is surfaced whole.
    let overflow = expect_ok(
        provider
            .recall(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": selection_all(),
                    "budget": { "max_items": 5, "max_bytes": 4 },
                }))
                .expect("recall request"),
            )
            .await,
    );
    let overflow = wire(&overflow);
    assert_eq!(overflow["records"].as_array().expect("records").len(), 1);
    assert!(overflow.get("overflow_record_id").is_some());
    assert_eq!(overflow["truncated"], true);
}

#[tokio::test]
async fn recall_cursors_invalidate_after_a_state_change() {
    let provider = provider();
    for id in ["rec_a", "rec_b"] {
        provider.provision_record(
            SPACE,
            json!({
                "schema_version": 1,
                "record_id": id,
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "kind": "fact",
                "content": { "text": "barrel tally" },
                "scope": { "visibility": "context", "context_id": CONTEXT },
                "provenance": {
                    "source_context_ids": [CONTEXT],
                    "anchors": [],
                    "captured_by": OPERATOR,
                    "captured_at": "2026-07-23T06:00:00Z",
                },
                "created_at": "2026-07-23T06:00:00Z",
                "updated_at": "2026-07-23T06:00:00Z",
                "revision": 1,
                "extensions": {},
            }),
        );
    }
    let first = expect_ok(
        provider
            .recall(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": selection_all(),
                    "budget": { "max_items": 1 },
                }))
                .expect("recall request"),
            )
            .await,
    );
    let cursor = wire(&first)["next_cursor"]
        .as_str()
        .expect("next cursor")
        .to_string();

    let _ = expect_ok(
        provider
            .create_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "kind": "fact",
                    "content": { "text": "a third tally appears" },
                    "scope": { "visibility": "context", "context_id": CONTEXT },
                    "idempotency_key": "rec-cursor",
                }))
                .expect("create request"),
            )
            .await,
    );

    assert_eq!(
        expect_reject(
            provider
                .recall(
                    &principal(OPERATOR),
                    serde_json::from_value(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                        "selection": selection_all(),
                        "budget": { "max_items": 1 },
                        "cursor": cursor,
                    }))
                    .expect("recall request"),
                )
                .await
        ),
        SpokeRejectCode::CursorInvalid
    );
}

// ── 3c: profile + reflections ─────────────────────────────────────────────

#[tokio::test]
async fn recall_cursor_walks_every_page_without_gaps_or_repeats() {
    // MEM-R026: the offset drops the leading ranked records BEFORE the page
    // budget; applied after the budget, page 2 was empty forever (and the
    // cursor never advanced).
    let provider = provider();
    for index in 0..5 {
        provider.provision_record(
            SPACE,
            json!({
                "schema_version": 1,
                "record_id": format!("rec_{index}"),
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "kind": "fact",
                "content": { "text": format!("barrel tally {index}") },
                "scope": { "visibility": "context", "context_id": CONTEXT },
                "provenance": {
                    "source_context_ids": [CONTEXT],
                    "anchors": [],
                    "captured_by": OPERATOR,
                    "captured_at": "2026-07-23T06:00:00Z",
                },
                "created_at": "2026-07-23T06:00:00Z",
                "updated_at": "2026-07-23T06:00:00Z",
                "revision": 1,
                "extensions": {},
            }),
        );
    }

    let mut seen: Vec<String> = Vec::new();
    let mut cursor = Value::Null;
    let mut pages = 0;
    loop {
        pages += 1;
        assert!(pages <= 3, "the cursor must advance every page, got {pages} pages");
        let mut request = json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": selection_all(),
            "budget": { "max_items": 2 },
        });
        if !cursor.is_null() {
            request["cursor"] = cursor.clone();
        }
        let page = expect_ok(
            provider
                .recall(
                    &principal(OPERATOR),
                    serde_json::from_value(request).expect("recall request"),
                )
                .await,
        );
        let page = wire(&page);
        seen.extend(
            page["records"]
                .as_array()
                .expect("records")
                .iter()
                .map(|record| {
                    record["record_id"]
                        .as_str()
                        .expect("record id")
                        .to_string()
                }),
        );
        match page.get("next_cursor") {
            Some(next) if pages < 3 => cursor = next.clone(),
            Some(next) => panic!("unexpected cursor after the last page: {next}"),
            None => break,
        }
    }
    assert_eq!(pages, 3, "five records at two per page walk three pages");
    seen.sort();
    assert_eq!(seen, ["rec_0", "rec_1", "rec_2", "rec_3", "rec_4"]);
}

#[tokio::test]
async fn recall_cursor_binds_every_mem_r026_input() {
    // MEM-R026: a cursor binds principal, space, subject, selection/filter,
    // snapshot marker and engine_policy_version. Reusing one across any of
    // those is CURSOR_INVALID even when the state epoch has not moved. The
    // probe principal is the space-wide admin, so every reused request clears
    // authorization and only the binding can reject it.
    let provider = provider();
    const OTHER_SUBJECT: &str = "toy:character:bo";
    provider.provision_subject(SPACE, OTHER_SUBJECT, Some(("toy", "character")));
    for index in 0..3 {
        provider.provision_record(
            SPACE,
            json!({
                "schema_version": 1,
                "record_id": format!("rec_binding_{index}"),
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "kind": "fact",
                "content": { "text": format!("barrel tally {index}") },
                "scope": { "visibility": "context", "context_id": CONTEXT },
                "provenance": {
                    "source_context_ids": [CONTEXT],
                    "anchors": [],
                    "captured_by": OPERATOR,
                    "captured_at": "2026-07-23T06:00:00Z",
                },
                "created_at": "2026-07-23T06:00:00Z",
                "updated_at": "2026-07-23T06:00:00Z",
                "revision": 1,
                "extensions": {},
            }),
        );
    }
    let request = |overrides: Value| {
        let mut request = json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": { "mode": "selected_contexts", "context_ids": [CONTEXT] },
            "budget": { "max_items": 1 },
        });
        for (key, value) in overrides.as_object().expect("overrides object") {
            request[key] = value.clone();
        }
        request
    };

    let first = expect_ok(
        provider
            .recall(
                &principal(OPERATOR),
                serde_json::from_value(request(json!({}))).expect("recall request"),
            )
            .await,
    );
    let cursor = wire(&first)["next_cursor"]
        .as_str()
        .expect("next cursor is an opaque string")
        .to_string();

    // The same bound request still pages: the offset is the pagination
    // position, not part of the binding.
    let second = expect_ok(
        provider
            .recall(
                &principal(OPERATOR),
                serde_json::from_value(request(json!({ "cursor": cursor })))
                    .expect("recall request"),
            )
            .await,
    );
    assert_eq!(
        wire(&second)["records"].as_array().expect("records").len(),
        1,
        "the bound cursor serves the next page"
    );
    assert_ne!(
        wire(&second)["records"], wire(&first)["records"],
        "the offset survived the binding"
    );

    for (label, principal_id, overrides) in [
        (
            "selection",
            OPERATOR,
            json!({ "cursor": cursor, "selection": { "mode": "subject_all" } }),
        ),
        (
            "filter",
            OPERATOR,
            json!({ "cursor": cursor, "filter": { "source_context_ids": [CONTEXT] } }),
        ),
        // The query ranks the page, so a cursor taken without one must not
        // resume a differently ranked list at the same offset.
        (
            "query",
            OPERATOR,
            json!({ "cursor": cursor, "query": "a different ranking" }),
        ),
        (
            "subject",
            OPERATOR,
            json!({ "cursor": cursor, "subject_id": OTHER_SUBJECT }),
        ),
        ("principal", READER, json!({ "cursor": cursor })),
    ] {
        let rejected = expect_reject(
            provider
                .recall(
                    &principal(principal_id),
                    serde_json::from_value(request(overrides)).expect("recall request"),
                )
                .await,
        );
        assert_eq!(
            rejected,
            SpokeRejectCode::CursorInvalid,
            "a cursor reused across a different {label} must be invalid"
        );
    }
}

#[tokio::test]
async fn polling_a_receipt_does_not_change_stored_state() {
    // MEM-R011: the read path never writes. Polling a pending receipt
    // observes it unchanged; only the separate provider worker settles it.
    let provider = provider();
    let created = expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "require_quiescent": false,
                    "idempotency_key": "exp-poll-readonly",
                }))
                .expect("export request"),
            )
            .await,
    );
    let created = wire(&created);
    assert_eq!(created["status"], "pending");
    let receipt_id = created["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();
    let request = || {
        serde_json::from_value::<MemoryReceiptsGetRequest>(json!({
            "space_id": SPACE,
            "receipt_id": receipt_id,
        }))
        .expect("receipt request")
    };

    // Polling alone changes nothing: the receipt stays pending in storage.
    for _ in 0..2 {
        let polled = expect_ok(provider.get_receipt(&principal(OPERATOR), request()).await);
        assert_eq!(wire(&polled)["receipt"]["status"], "pending");
    }
    let spaces = expect_ok(
        provider
            .list_spaces(
                &principal(OPERATOR),
                serde_json::from_value(json!({ "limit": 10 })).expect("list request"),
            )
            .await,
    );
    assert_eq!(
        wire(&spaces)["spaces"][0]["pending_receipts"],
        1,
        "the pending receipt must survive read-path polling untouched"
    );

    // The separate worker settles it; polling then observes the commit.
    provider.run_worker();
    let polled = expect_ok(provider.get_receipt(&principal(OPERATOR), request()).await);
    assert_eq!(wire(&polled)["receipt"]["status"], "committed");
}


const CONTEXT_SELECTION: &str = "ctx_customs_gate";

fn profile_update(kind: &str, revision: Option<u64>, key: &str) -> Value {
    let mut request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "profile_kind": kind,
        "document": { "format": "markdown", "body": "# Mira\n\nGuarded, loyal." },
        "idempotency_key": key,
    });
    request["expected_revision"] = match revision {
        Some(revision) => json!(revision),
        None => Value::Null,
    };
    request
}

fn context_selection() -> Value {
    json!({ "mode": "selected_contexts", "context_ids": [CONTEXT_SELECTION], "include_shared": false })
}

fn generate_request(expected: &str, key: &str) -> Value {
    json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "selection": context_selection(),
        "expected_state_digest": expected,
        "idempotency_key": key,
    })
}

fn seed_record(provider: &SqliteMemoryProvider, record_id: &str, text: &str) {
    provider.provision_record(
        SPACE,
        json!({
            "schema_version": 1,
            "record_id": record_id,
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "kind": "fact",
            "content": { "text": text },
            "scope": { "visibility": "context", "context_id": CONTEXT_SELECTION },
            "provenance": {
                "source_context_ids": [CONTEXT_SELECTION],
                "anchors": [],
                "captured_by": OPERATOR,
                "captured_at": "2026-07-23T06:00:00Z",
            },
            "created_at": "2026-07-23T06:00:00Z",
            "updated_at": "2026-07-23T06:00:00Z",
            "revision": 1,
            "extensions": {},
        }),
    );
}

async fn state_of(provider: &SqliteMemoryProvider) -> Value {
    let response = expect_ok(
        provider
            .get_reflection_state(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": context_selection(),
                }))
                .expect("state request"),
            )
            .await,
    );
    wire(&response)
}

#[tokio::test]
async fn profile_admission_is_honest_and_cas_holds() {
    let provider = provider();
    // Honest empty: an absent optional kind is an empty list (§4.2).
    let absent = expect_ok(
        provider
            .get_profile(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "profile_kinds": ["preferences"],
                }))
                .expect("profile request"),
            )
            .await,
    );
    assert!(wire(&absent)["profiles"]
        .as_array()
        .expect("profiles")
        .is_empty());

    // A principal without profile_write cannot write.
    assert_eq!(
        expect_reject(
            provider
                .update_profile(
                    &principal(READER),
                    serde_json::from_value(profile_update("personality", None, "prof-denied"))
                        .expect("profile update"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );

    // create-only (expected_revision null), then CAS.
    let created = expect_ok(
        provider
            .update_profile(
                &principal(OPERATOR),
                serde_json::from_value(profile_update("personality", None, "prof-1"))
                    .expect("profile update"),
            )
            .await,
    );
    assert_eq!(wire(&created)["result"]["revision"], 1);

    assert_eq!(
        expect_reject(
            provider
                .update_profile(
                    &principal(OPERATOR),
                    serde_json::from_value(profile_update("personality", None, "prof-2"))
                        .expect("profile update"),
                )
                .await
        ),
        SpokeRejectCode::RevisionConflict
    );
    let updated = expect_ok(
        provider
            .update_profile(
                &principal(OPERATOR),
                serde_json::from_value(profile_update("personality", Some(1), "prof-3"))
                    .expect("profile update"),
            )
            .await,
    );
    assert_eq!(wire(&updated)["result"]["revision"], 2);

    let present = expect_ok(
        provider
            .get_profile(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "profile_kinds": ["personality"],
                }))
                .expect("profile request"),
            )
            .await,
    );
    let profiles = wire(&present)["profiles"].clone();
    assert_eq!(profiles.as_array().expect("profiles").len(), 1);
    assert_eq!(profiles[0]["revision"], 2);
}

#[tokio::test]
async fn profile_update_stales_the_pinned_reflection() {
    let provider = provider();
    seed_record(
        &provider,
        "rec_1",
        "The gate key is under the third barrel.",
    );
    let _ = expect_ok(
        provider
            .update_profile(
                &principal(OPERATOR),
                serde_json::from_value(profile_update("personality", None, "prof-a"))
                    .expect("profile update"),
            )
            .await,
    );

    let before = state_of(&provider).await;
    assert_eq!(before["data_sufficiency"], "sufficient");
    assert_eq!(before["stale"], false);
    let digest = before["state_digest"].as_str().expect("digest").to_string();

    let generated = expect_ok(
        provider
            .generate_reflection(
                &principal(OPERATOR),
                serde_json::from_value(generate_request(&digest, "gen-1")).expect("generate"),
            )
            .await,
    );
    let generated = wire(&generated);
    assert_eq!(generated["status"], "pending");
    let receipt_id = generated["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();

    // The worker settles the pending generation separately from the read.
    provider.run_worker();
    let polled = expect_ok(
        provider
            .get_receipt(
                &principal(OPERATOR),
                serde_json::from_value(json!({ "space_id": SPACE, "receipt_id": receipt_id }))
                    .expect("receipt request"),
            )
            .await,
    );
    let polled = wire(&polled);
    assert_eq!(polled["receipt"]["status"], "committed");
    assert!(polled["receipt"]["result"]["reflection_id"]
        .as_str()
        .is_some());

    let listed = expect_ok(
        provider
            .list_reflections(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "limit": 10,
                }))
                .expect("list request"),
            )
            .await,
    );
    assert_eq!(wire(&listed)["reflections"][0]["stale"], false);

    // MEM-R013: a profile revision bump changes the basis ⇒ stale at read.
    let _ = expect_ok(
        provider
            .update_profile(
                &principal(OPERATOR),
                serde_json::from_value(profile_update("personality", Some(1), "prof-b"))
                    .expect("profile update"),
            )
            .await,
    );
    let after = state_of(&provider).await;
    assert_eq!(after["stale"], true);
    let listed = expect_ok(
        provider
            .list_reflections(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "limit": 10,
                }))
                .expect("list request"),
            )
            .await,
    );
    assert_eq!(wire(&listed)["reflections"][0]["stale"], true);
}

/// §6.9/§7 fail-closed pins: a reflection whose pinned `selection` or
/// `cognition_filter` cannot be decoded can never reproduce its basis, so the
/// existing `stale` member reports it instead of reading fresh — while a valid
/// pin keeps its current path.
///
/// The baseline is published unfiltered, so the pinned basis reproduces when a
/// corrupt `cognition_filter` degrades to `None`: only the fail-closed arm turns
/// that corrupt pin stale, instead of a digest mismatch. Both corrupt members
/// also surface through the existing typed-list refusal. `reflections.state`
/// carries the stale verdict because the list row embeds the typed pins.
#[tokio::test]
async fn unreadable_pinned_reflection_members_read_stale() {
    let path = temp_state_path("reflection-corrupt-pins");
    let provider = seeded(SqliteMemoryProvider::open(&path).expect("store opens"));
    provider.provision_profile(
        SPACE,
        json!({
            "schema_version": 1,
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "profile_kind": "personality",
            "document": {},
            "revision": 1,
        }),
    );
    provider.provision_record(
        SPACE,
        stored_record(
            "rec_pin",
            "fact",
            "2026-07-23T06:00:00Z",
            context_scope(CONTEXT_SELECTION),
            &[CONTEXT_SELECTION],
            json!({
                "text": "source rec_pin",
                "cognition": [belief_at("bel_alice", "alice", 0.9)],
            }),
        ),
    );
    let operator = principal(OPERATOR);

    let current = state_of(&provider).await;
    assert_eq!(current["basis"]["record_count"], json!(1), "{current}");
    assert_eq!(current["stale"], json!(false), "a valid pin is current");
    expect_ok(
        provider
            .generate_reflection(
                &operator,
                serde_json::from_value::<MemoryReflectionsGenerateRequest>(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": context_selection(),
                    "expected_state_digest": current["state_digest"],
                    "idempotency_key": "gen-pin-unfiltered",
                }))
                .expect("generate request"),
            )
            .await,
    );
    provider.run_worker();

    let list = || {
        provider.list_reflections(
            &operator,
            serde_json::from_value::<MemoryReflectionsListRequest>(json!({
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "limit": 10,
            }))
            .expect("list request"),
        )
    };

    // Positive controls: before any pin is corrupted, the published reflection
    // reads fresh through both the list row and the state member.
    let listed = wire(&expect_ok(list().await));
    assert_eq!(
        listed["reflections"].as_array().expect("reflections").len(),
        1,
        "{listed}"
    );
    assert_eq!(listed["reflections"][0]["stale"], json!(false), "{listed}");
    assert_eq!(state_of(&provider).await["stale"], json!(false));

    let valid_bytes = stored_document(&path);
    let reflection_id = {
        let document: Value =
            serde_json::from_str(&valid_bytes).expect("stored document is json");
        document["spaces"][SPACE]["reflections"]
            .as_object()
            .expect("stored reflections")
            .keys()
            .next()
            .expect("the published reflection")
            .clone()
    };
    let corrupt_member = |member: &str, value: Value| {
        let mut document: Value =
            serde_json::from_str(&valid_bytes).expect("stored document is json");
        document["spaces"][SPACE]["reflections"][reflection_id.as_str()]
            .as_object_mut()
            .expect("stored reflection is an object")
            .insert(member.to_string(), value);
        write_stored_document(
            &path,
            &serde_json::to_string(&document).expect("corrupt document serializes"),
        );
    };
    let typed_list_refusal = |result: SpokeResult<MemoryReflectionsListResponse>| {
        assert_eq!(expect_reject(result), SpokeRejectCode::InternalError);
    };

    // Exactly one stored member is corrupted per round, always from pristine.
    corrupt_member("selection", json!({ "kinds": 42 }));
    assert_eq!(
        state_of(&provider).await["stale"],
        json!(true),
        "an unreadable pinned selection is stale"
    );
    typed_list_refusal(list().await);

    corrupt_member("cognition_filter", json!({ "q": { "min": "high" } }));
    assert_eq!(
        state_of(&provider).await["stale"],
        json!(true),
        "an unreadable pinned cognition filter is stale"
    );
    typed_list_refusal(list().await);
}

#[tokio::test]
async fn reflection_generation_admission_gates_fail_closed() {
    let provider = provider();
    // Required kind absent ⇒ PROFILE_INCOMPLETE, before the digest check.
    assert_eq!(
        expect_reject(
            provider
                .generate_reflection(
                    &principal(OPERATOR),
                    serde_json::from_value(generate_request(&"0".repeat(64), "gen-a"))
                        .expect("generate"),
                )
                .await
        ),
        SpokeRejectCode::ProfileIncomplete
    );

    let _ = expect_ok(
        provider
            .update_profile(
                &principal(OPERATOR),
                serde_json::from_value(profile_update("personality", None, "prof-c"))
                    .expect("profile update"),
            )
            .await,
    );
    // No authorized records ⇒ INSUFFICIENT_DATA.
    assert_eq!(
        expect_reject(
            provider
                .generate_reflection(
                    &principal(OPERATOR),
                    serde_json::from_value(generate_request(&"0".repeat(64), "gen-b"))
                        .expect("generate"),
                )
                .await
        ),
        SpokeRejectCode::InsufficientData
    );

    seed_record(
        &provider,
        "rec_1",
        "The gate key is under the third barrel.",
    );
    // Stale expected digest ⇒ REVISION_CONFLICT.
    assert_eq!(
        expect_reject(
            provider
                .generate_reflection(
                    &principal(OPERATOR),
                    serde_json::from_value(generate_request(&"0".repeat(64), "gen-c"))
                        .expect("generate"),
                )
                .await
        ),
        SpokeRejectCode::RevisionConflict
    );
}

#[tokio::test]
async fn publish_pinned_generation_reads_stale_when_the_basis_moves() {
    let provider = provider();
    seed_record(
        &provider,
        "rec_1",
        "The gate key is under the third barrel.",
    );
    let _ = expect_ok(
        provider
            .update_profile(
                &principal(OPERATOR),
                serde_json::from_value(profile_update("personality", None, "prof-p"))
                    .expect("profile update"),
            )
            .await,
    );
    let digest = state_of(&provider).await["state_digest"]
        .as_str()
        .expect("digest")
        .to_string();
    let generated = expect_ok(
        provider
            .generate_reflection(
                &principal(OPERATOR),
                serde_json::from_value(generate_request(&digest, "gen-p")).expect("generate"),
            )
            .await,
    );
    let receipt_id = wire(&generated)["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();

    // The basis moves while generation is in flight.
    seed_record(&provider, "rec_2", "A second tally appears by the gate.");

    // The worker settles the pending generation separately from the read.
    provider.run_worker();
    let polled = expect_ok(
        provider
            .get_receipt(
                &principal(OPERATOR),
                serde_json::from_value(json!({ "space_id": SPACE, "receipt_id": receipt_id }))
                    .expect("receipt request"),
            )
            .await,
    );
    assert_eq!(wire(&polled)["receipt"]["status"], "committed");

    // Publish-pinned: published, and immediately reads stale.
    let listed = expect_ok(
        provider
            .list_reflections(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "limit": 10,
                }))
                .expect("list request"),
            )
            .await,
    );
    let reflections = wire(&listed)["reflections"].clone();
    assert_eq!(reflections.as_array().expect("reflections").len(), 1);
    assert_eq!(reflections[0]["stale"], true);
}

#[tokio::test]
async fn reflections_pinned_to_tombstoned_records_are_excluded() {
    let provider = provider();
    seed_record(
        &provider,
        "rec_1",
        "The gate key is under the third barrel.",
    );
    let _ = expect_ok(
        provider
            .update_profile(
                &principal(OPERATOR),
                serde_json::from_value(profile_update("personality", None, "prof-t"))
                    .expect("profile update"),
            )
            .await,
    );
    let digest = state_of(&provider).await["state_digest"]
        .as_str()
        .expect("digest")
        .to_string();
    let generated = expect_ok(
        provider
            .generate_reflection(
                &principal(OPERATOR),
                serde_json::from_value(generate_request(&digest, "gen-t")).expect("generate"),
            )
            .await,
    );
    let receipt_id = wire(&generated)["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();
    // The worker settles the pending generation separately from the read.
    provider.run_worker();
    let _ = expect_ok(
        provider
            .get_receipt(
                &principal(OPERATOR),
                serde_json::from_value(json!({ "space_id": SPACE, "receipt_id": receipt_id }))
                    .expect("receipt request"),
            )
            .await,
    );

    let before = expect_ok(
        provider
            .list_reflections(
                &principal(OPERATOR),
                serde_json::from_value(
                    json!({ "space_id": SPACE, "subject_id": SUBJECT, "limit": 10 }),
                )
                .expect("list request"),
            )
            .await,
    );
    assert_eq!(
        wire(&before)["reflections"].as_array().expect("list").len(),
        1
    );

    let _ = expect_ok(
        provider
            .delete_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": "rec_1",
                    "expected_revision": 1,
                    "idempotency_key": "rec-del-ref",
                }))
                .expect("delete request"),
            )
            .await,
    );
    let after = expect_ok(
        provider
            .list_reflections(
                &principal(OPERATOR),
                serde_json::from_value(
                    json!({ "space_id": SPACE, "subject_id": SUBJECT, "limit": 10 }),
                )
                .expect("list request"),
            )
            .await,
    );
    assert!(wire(&after)["reflections"]
        .as_array()
        .expect("list")
        .is_empty());
}

// ── Task 4: transfer surfaces ─────────────────────────────────────────────

use spoke_fixture_memory_engine::transfer::{bundle_digest, chunk_digest, reassemble, verify_reassembly};

async fn poll_receipt(provider: &SqliteMemoryProvider, space: &str, receipt_id: &str) -> Value {
    // The worker is separate from the read path (MEM-R011): settle the
    // deferred async work, then observe the receipt through the port.
    provider.run_worker();
    let response = expect_ok(
        provider
            .get_receipt(
                &principal(OPERATOR),
                serde_json::from_value(json!({ "space_id": space, "receipt_id": receipt_id }))
                    .expect("receipt request"),
            )
            .await,
    );
    wire(&response)["receipt"].clone()
}

/// The `memory.spaces.list` row for `SPACE`, read through the port.
async fn space_row(provider: &SqliteMemoryProvider) -> Value {
    let spaces = expect_ok(
        provider
            .list_spaces(
                &principal(OPERATOR),
                serde_json::from_value(json!({ "limit": 10 })).expect("list request"),
            )
            .await,
    );
    wire(&spaces)["spaces"]
        .as_array()
        .expect("spaces")
        .iter()
        .find(|space| space["space_id"] == json!(SPACE))
        .expect("space")
        .clone()
}

fn transfer_space() -> SqliteMemoryProvider {
    let provider = provider();
    provider.provision_grant(
        OTHER_SPACE,
        grant_in(
            OTHER_SPACE,
            "grt_operator_admin_other",
            OPERATOR,
            &["admin"],
            None,
            None,
            None,
        ),
    );
    provider.provision_subject(OTHER_SPACE, SUBJECT, Some(("toy", "character")));
    provider
}

#[tokio::test]
async fn export_chunks_round_trip_with_verifiable_digests() {
    let provider = transfer_space();
    seed_record(
        &provider,
        "rec_1",
        "The gate key is under the third barrel.",
    );
    seed_record(&provider, "rec_2", "A second tally appears by the gate.");

    let created = expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "require_quiescent": false,
                    "idempotency_key": "exp-1",
                }))
                .expect("export request"),
            )
            .await,
    );
    let created = wire(&created);
    assert_eq!(created["status"], "pending");
    let completed = poll_receipt(
        &provider,
        SPACE,
        created["receipt_id"].as_str().expect("receipt id"),
    )
    .await;
    assert_eq!(completed["status"], "committed");
    let export_id = completed["result"]["export_id"]
        .as_str()
        .expect("export id")
        .to_string();
    let chunk_count = completed["result"]["chunk_count"]
        .as_u64()
        .expect("chunk count");
    let advertised = completed["result"]["bundle_digest"]
        .as_str()
        .expect("bundle digest")
        .to_string();
    assert_eq!(completed["result"]["counts"]["records"], 2);

    let mut chunks = Vec::new();
    for ordinal in 0..chunk_count {
        let chunk = expect_ok(
            provider
                .get_export(
                    &principal(OPERATOR),
                    serde_json::from_value(json!({
                        "space_id": SPACE,
                        "export_id": export_id,
                        "chunk_ordinal": ordinal,
                    }))
                    .expect("export get"),
                )
                .await,
        );
        let chunk = wire(&chunk)["chunk"].clone();
        assert_eq!(chunk["bundle_digest"], json!(advertised));
        chunks.push(chunk);
    }
    // Every advertised chunk digest and the reassembled bundle must verify
    // against the provider's own chunk JSON (the typed response normalizes
    // optional fields, so a re-digest of the round-tripped value would drift).
    let raw_chunks = provider
        .export_chunks(SPACE, &export_id)
        .expect("provider chunks");
    assert_eq!(raw_chunks.len(), chunks.len());
    for (ordinal, chunk) in raw_chunks.iter().enumerate() {
        assert_eq!(
            chunk_digest(chunk).expect("chunk digest"),
            chunk["chunk_digest"],
            "chunk {ordinal} digest"
        );
        assert_eq!(chunk["bundle_digest"], json!(advertised));
    }
    let verification = verify_reassembly(&raw_chunks);
    assert!(
        verification.bundle_digest_ok && verification.errors.is_empty(),
        "chunk verification failed: {:?}",
        verification.errors
    );
    let bundle = reassemble(&raw_chunks).expect("reassemble");
    assert_eq!(bundle_digest(&bundle).expect("bundle digest"), advertised);
    assert_eq!(bundle["digest"], json!(advertised));
    assert_eq!(bundle["records"].as_array().expect("records").len(), 2);

    assert_eq!(
        expect_reject(
            provider
                .get_export(
                    &principal(OPERATOR),
                    serde_json::from_value(json!({
                        "space_id": SPACE,
                        "export_id": export_id,
                        "chunk_ordinal": chunk_count + 5,
                    }))
                    .expect("export get"),
                )
                .await
        ),
        SpokeRejectCode::InvalidInput
    );
}

#[tokio::test]
async fn exports_expire_and_quiescent_exports_need_a_drained_space() {
    let provider = transfer_space();
    seed_record(
        &provider,
        "rec_1",
        "The gate key is under the third barrel.",
    );

    // Active space ⇒ the drain barrier refuses a quiescent export.
    let quiescent = serde_json::from_value::<spoke_schemas::MemoryExportsCreateRequest>(json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "require_quiescent": true,
        "idempotency_key": "exp-q0",
    }))
    .expect("export request");
    assert_eq!(
        expect_reject(
            provider
                .create_export(&principal(OPERATOR), quiescent)
                .await
        ),
        SpokeRejectCode::InvalidInput
    );

    // Freeze, then a backup export leaves a pending receipt behind.
    let _ = expect_ok(
        provider
            .freeze_space(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "expected_revision": 1,
                    "idempotency_key": "fz-exp",
                }))
                .expect("freeze request"),
            )
            .await,
    );
    let backup = expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "require_quiescent": false,
                    "idempotency_key": "exp-backup",
                }))
                .expect("export request"),
            )
            .await,
    );
    let backup_receipt = wire(&backup)["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();
    let quiescent = serde_json::from_value::<spoke_schemas::MemoryExportsCreateRequest>(json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "require_quiescent": true,
        "idempotency_key": "exp-q1",
    }))
    .expect("export request");
    assert_eq!(
        expect_reject(
            provider
                .create_export(&principal(OPERATOR), quiescent)
                .await
        ),
        SpokeRejectCode::InvalidInput
    );

    // Draining the pending receipt admits the quiescent export.
    let completed = poll_receipt(&provider, SPACE, &backup_receipt).await;
    assert_eq!(completed["status"], "committed");
    let admitted = expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "require_quiescent": true,
                    "idempotency_key": "exp-q2",
                }))
                .expect("export request"),
            )
            .await,
    );
    assert_eq!(wire(&admitted)["status"], "pending");

    // Retention: the export is unreadable past export_retention_seconds.
    let admitted_receipt = wire(&admitted)["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();
    let completed = poll_receipt(&provider, SPACE, &admitted_receipt).await;
    let export_id = completed["result"]["export_id"]
        .as_str()
        .expect("export id")
        .to_string();
    provider.advance_clock(sqlite_memory::EXPORT_RETENTION_SECONDS + 1);
    assert_eq!(
        expect_reject(
            provider
                .get_export(
                    &principal(OPERATOR),
                    serde_json::from_value(json!({
                        "space_id": SPACE,
                        "export_id": export_id,
                        "chunk_ordinal": 0,
                    }))
                    .expect("export get"),
                )
                .await
        ),
        SpokeRejectCode::ExportExpired
    );
}

fn seed_scoped_record(provider: &SqliteMemoryProvider, record_id: &str, text: &str, scope: Value) {
    provider.provision_record(
        SPACE,
        json!({
            "schema_version": 1,
            "record_id": record_id,
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "kind": "fact",
            "content": { "text": text },
            "scope": scope,
            "provenance": {
                "source_context_ids": [CONTEXT_SELECTION],
                "anchors": [],
                "captured_by": OPERATOR,
                "captured_at": "2026-07-23T06:00:00Z",
            },
            "created_at": "2026-07-23T06:00:00Z",
            "updated_at": "2026-07-23T06:00:00Z",
            "revision": 1,
            "extensions": {},
        }),
    );
}

async fn completed_export(provider: &SqliteMemoryProvider, request: Value) -> (Value, Vec<Value>) {
    let created = expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(request).expect("export request"),
            )
            .await,
    );
    let completed = poll_receipt(
        provider,
        SPACE,
        wire(&created)["receipt_id"].as_str().expect("receipt id"),
    )
    .await;
    let export_id = completed["result"]["export_id"]
        .as_str()
        .expect("export id")
        .to_string();
    let chunks = provider
        .export_chunks(SPACE, &export_id)
        .expect("provider chunks");
    (completed, chunks)
}

fn bundle_record_ids(chunks: &[Value]) -> Vec<String> {
    let bundle = reassemble(chunks).expect("reassemble");
    bundle
        .get("records")
        .and_then(Value::as_array)
        .map(|records| {
            records
                .iter()
                .filter_map(|record| record["record_id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// PROVIDER-TAIL-1: the export selection is the grant's disclosure scope, so
/// records/candidates outside it must never reach the digest or the chunks.
#[tokio::test]
async fn export_selection_restricts_content_before_digest_and_partition() {
    let provider = transfer_space();
    seed_scoped_record(
        &provider,
        "rec_ctx_a",
        "The gate key is under the third barrel.",
        json!({ "visibility": "context", "context_id": CONTEXT_SELECTION }),
    );
    seed_scoped_record(
        &provider,
        "rec_ctx_b",
        "A market rumor from elsewhere.",
        json!({ "visibility": "context", "context_id": "ctx_elsewhere" }),
    );
    seed_scoped_record(
        &provider,
        "rec_shared",
        "A shared tally by the gate.",
        json!({ "visibility": "shared" }),
    );
    let captured = expect_ok(
        provider
            .capture(
                &principal(READER),
                serde_json::from_value(capture_request(
                    "Mira told Bo the gate key is hidden under the third barrel.",
                    "cap-selection",
                ))
                .expect("capture request"),
            )
            .await,
    );
    assert_eq!(
        wire(&captured)["result"]["candidate_ids"]
            .as_array()
            .expect("candidate ids")
            .len(),
        1
    );

    // The capture context: one record plus the pending candidate.
    let (narrow, narrow_chunks) = completed_export(
        &provider,
        json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": { "mode": "selected_contexts", "context_ids": [CONTEXT_SELECTION] },
            "idempotency_key": "exp-selection-narrow",
        }),
    )
    .await;
    assert_eq!(narrow["result"]["counts"]["records"], 1, "{narrow}");
    assert_eq!(narrow["result"]["counts"]["candidates"], 1, "{narrow}");
    assert_eq!(bundle_record_ids(&narrow_chunks), vec!["rec_ctx_a"]);
    assert!(verify_reassembly(&narrow_chunks).bundle_digest_ok);

    // Another context's selection: its record only — no candidate and no
    // shared-scope record, and no out-of-selection bytes anywhere.
    let (other, other_chunks) = completed_export(
        &provider,
        json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": { "mode": "selected_contexts", "context_ids": ["ctx_elsewhere"] },
            "idempotency_key": "exp-selection-other",
        }),
    )
    .await;
    assert_eq!(other["result"]["counts"]["records"], 1, "{other}");
    assert_eq!(other["result"]["counts"]["candidates"], 0, "{other}");
    assert_eq!(bundle_record_ids(&other_chunks), vec!["rec_ctx_b"]);
    let verification = verify_reassembly(&other_chunks);
    assert!(
        verification.bundle_digest_ok && verification.errors.is_empty(),
        "chunk verification failed: {:?}",
        verification.errors
    );
    let bundle = reassemble(&other_chunks).expect("reassemble");
    assert_eq!(
        bundle_digest(&bundle).expect("bundle digest"),
        other["result"]["bundle_digest"]
    );
    let raw = serde_json::to_string(&other_chunks).expect("chunks serialize");
    assert!(!raw.contains("gate key is under the third barrel"), "{raw}");
    assert!(!raw.contains("A shared tally"), "{raw}");
}

/// PROVIDER-TAIL-2: a tombstone is terminal. Neither update nor an
/// `include_tombstoned` export may carry content back into a scrubbed record.
#[tokio::test]
async fn tombstoned_records_cannot_regain_content_through_update_or_export() {
    let provider = transfer_space();
    seed_record(
        &provider,
        "rec_1",
        "The gate key is under the third barrel.",
    );
    let _ = expect_ok(
        provider
            .delete_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": "rec_1",
                    "expected_revision": 1,
                    "idempotency_key": "del-tombstone",
                }))
                .expect("delete request"),
            )
            .await,
    );

    let rejected = expect_reject(
        provider
            .update_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": "rec_1",
                    "expected_revision": 2,
                    "content": { "text": "restored cognition leak" },
                    "idempotency_key": "upd-tombstone",
                }))
                .expect("update request"),
            )
            .await,
    );
    assert_eq!(rejected, SpokeRejectCode::NotFound);

    let record = expect_ok(
        provider
            .get_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({ "space_id": SPACE, "record_id": "rec_1" }))
                    .expect("get request"),
            )
            .await,
    );
    let record = wire(&record)["record"].clone();
    assert_eq!(record["revision"], 2);
    assert_eq!(record["content"], json!({ "text": "" }));
    assert!(record["tombstone"].is_object(), "{record}");

    let (_completed, chunks) = completed_export(
        &provider,
        json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "include_tombstoned": true,
            "idempotency_key": "exp-tombstone",
        }),
    )
    .await;
    let bundle = reassemble(&chunks).expect("reassemble");
    let records = bundle["records"].as_array().expect("exported records");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["content"], json!({ "text": "" }));
    assert!(records[0]["tombstone"].is_object(), "{}", records[0]);
    let raw = serde_json::to_string(&chunks).expect("chunks serialize");
    assert!(!raw.contains("restored cognition leak"), "{raw}");
}

/// PROVIDER-TAIL-3: an evicted receipt body keeps its dedup tombstone, so the
/// same key/digest answers RECEIPT_EXPIRED and no mutation runs twice.
#[tokio::test]
async fn evicted_receipts_never_reexecute_create_update_promote_or_export() {
    let provider = transfer_space();
    let create = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "kind": "fact",
        "content": { "text": "Created exactly once." },
        "scope": { "visibility": "context", "context_id": CONTEXT },
        "idempotency_key": "rep-create",
    });
    let created = expect_ok(
        provider
            .create_record(
                &principal(OPERATOR),
                serde_json::from_value(create.clone()).expect("create request"),
            )
            .await,
    );
    let record_id = wire(&created)["result"]["record_id"]
        .as_str()
        .expect("record id")
        .to_string();
    let update = json!({
        "space_id": SPACE,
        "record_id": record_id,
        "expected_revision": 1,
        "content": { "text": "Updated exactly once." },
        "idempotency_key": "rep-update",
    });
    let updated = expect_ok(
        provider
            .update_record(
                &principal(OPERATOR),
                serde_json::from_value(update.clone()).expect("update request"),
            )
            .await,
    );
    assert_eq!(wire(&updated)["result"]["revision"], 2);
    let promote = json!({
        "space_id": SPACE,
        "record_id": record_id,
        "expected_revision": 2,
        "target_scope": { "visibility": "shared" },
        "idempotency_key": "rep-promote",
    });
    let _ = expect_ok(
        provider
            .promote_record(
                &principal(OPERATOR),
                serde_json::from_value(promote.clone()).expect("promote request"),
            )
            .await,
    );
    let export = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "idempotency_key": "rep-export",
    });
    let created_export = expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(export.clone()).expect("export request"),
            )
            .await,
    );
    let first_export = poll_receipt(
        &provider,
        SPACE,
        wire(&created_export)["receipt_id"]
            .as_str()
            .expect("receipt id"),
    )
    .await;
    assert_eq!(first_export["result"]["export_id"], "exp_sqlite_0001");

    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);

    expect_receipt_expired(
        provider
            .create_record(
                &principal(OPERATOR),
                serde_json::from_value(create.clone()).expect("create request"),
            )
            .await,
    );
    expect_receipt_expired(
        provider
            .update_record(
                &principal(OPERATOR),
                serde_json::from_value(update.clone()).expect("update request"),
            )
            .await,
    );
    expect_receipt_expired(
        provider
            .promote_record(
                &principal(OPERATOR),
                serde_json::from_value(promote.clone()).expect("promote request"),
            )
            .await,
    );
    expect_receipt_expired(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(export.clone()).expect("export request"),
            )
            .await,
    );

    // No re-execution: still exactly one record at revision 3, one export.
    let listed = expect_ok(
        provider
            .list_records(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": selection_all(),
                }))
                .expect("list request"),
            )
            .await,
    );
    assert_eq!(wire(&listed)["total_authorized"], 1);
    let record = expect_ok(
        provider
            .get_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": record_id,
                }))
                .expect("get request"),
            )
            .await,
    );
    assert_eq!(wire(&record)["record"]["revision"], 3);
    let (next, _chunks) = completed_export(
        &provider,
        json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "idempotency_key": "rep-export-next",
        }),
    )
    .await;
    assert_eq!(next["result"]["export_id"], "exp_sqlite_0002");
}

/// VT-1: one shared record-id allocation across fragment review, create and
/// promote — gaps and tombstones never let a later write replace retained
/// content.
#[tokio::test]
async fn record_ids_never_collide_across_fragments_creates_and_promotions() {
    let provider = provider();
    let captured = expect_ok(
        provider
            .capture(
                &principal(READER),
                serde_json::from_value(capture_request(
                    "Mira told Bo the gate key is hidden under the third barrel.",
                    "cap-ids-1",
                ))
                .expect("capture request"),
            )
            .await,
    );
    let candidate_id = wire(&captured)["result"]["candidate_ids"][0]
        .as_str()
        .expect("candidate id")
        .to_string();
    let reviewed = expect_ok(
        provider
            .review_candidate(
                &principal(READER),
                serde_json::from_value(review_request(
                    &candidate_id,
                    1,
                    "rev-ids-1",
                    json!({
                        "action": "fragment",
                        "fragments": [
                            { "kind": "fact", "content": { "text": "The gate key is hidden under the third barrel." } },
                            { "kind": "event", "content": { "text": "Mira revealed the key location to Bo." } },
                        ],
                    }),
                ))
                .expect("review request"),
            )
            .await,
    );
    let mut ids: Vec<String> = wire(&reviewed)["result"]["record_ids"]
        .as_array()
        .expect("record ids")
        .iter()
        .map(|id| id.as_str().expect("record id").to_string())
        .collect();
    assert_eq!(ids.len(), 2);

    let created = expect_ok(
        provider
            .create_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "kind": "fact",
                    "content": { "text": "A direct authored fact." },
                    "scope": { "visibility": "context", "context_id": CONTEXT },
                    "idempotency_key": "rec-ids-create",
                }))
                .expect("create request"),
            )
            .await,
    );
    let created_id = wire(&created)["result"]["record_id"]
        .as_str()
        .expect("record id")
        .to_string();
    assert!(
        !ids.contains(&created_id),
        "create reused a retained record id: {created_id}"
    );
    ids.push(created_id);

    let captured = expect_ok(
        provider
            .capture(
                &principal(READER),
                serde_json::from_value(capture_request(
                    "Bo confirmed the third barrel hides the key.",
                    "cap-ids-2",
                ))
                .expect("capture request"),
            )
            .await,
    );
    let candidate_id = wire(&captured)["result"]["candidate_ids"][0]
        .as_str()
        .expect("candidate id")
        .to_string();
    let reviewed = expect_ok(
        provider
            .review_candidate(
                &principal(READER),
                serde_json::from_value(review_request(
                    &candidate_id,
                    1,
                    "rev-ids-2",
                    json!({ "action": "promote" }),
                ))
                .expect("review request"),
            )
            .await,
    );
    let promoted_id = wire(&reviewed)["result"]["record_ids"][0]
        .as_str()
        .expect("record id")
        .to_string();
    assert!(
        !ids.contains(&promoted_id),
        "promote reused a retained record id: {promoted_id}"
    );
    ids.push(promoted_id);

    // The id of a tombstoned record is never reissued.
    let _ = expect_ok(
        provider
            .delete_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": ids[0],
                    "expected_revision": 1,
                    "idempotency_key": "del-ids",
                }))
                .expect("delete request"),
            )
            .await,
    );
    let created = expect_ok(
        provider
            .create_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "kind": "note",
                    "content": { "text": "Written after a tombstone." },
                    "scope": { "visibility": "context", "context_id": CONTEXT },
                    "idempotency_key": "rec-ids-after-delete",
                }))
                .expect("create request"),
            )
            .await,
    );
    let after_delete = wire(&created)["result"]["record_id"]
        .as_str()
        .expect("record id")
        .to_string();
    assert!(
        !ids.contains(&after_delete),
        "a later create reused a tombstoned record id: {after_delete}"
    );

    // Every surviving record still carries its own original content.
    let fragment_two = expect_ok(
        provider
            .get_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": ids[1],
                }))
                .expect("get request"),
            )
            .await,
    );
    assert_eq!(
        wire(&fragment_two)["record"]["content"]["text"],
        "Mira revealed the key location to Bo."
    );
    let listed = expect_ok(
        provider
            .list_records(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": selection_all(),
                }))
                .expect("list request"),
            )
            .await,
    );
    let listed = wire(&listed);
    let live = listed["records"].as_array().expect("records").len();
    assert_eq!(listed["total_authorized"], 4, "{listed}");
    assert_eq!(live, 4, "{listed}");
}

async fn export_chunks_of(provider: &SqliteMemoryProvider) -> (String, Vec<Value>) {
    export_chunks_with_key(provider, "exp-import").await
}

async fn export_chunks_with_key(
    provider: &SqliteMemoryProvider,
    key: &str,
) -> (String, Vec<Value>) {
    let created = expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "include_reflections": true,
                    "require_quiescent": false,
                    "idempotency_key": key,
                }))
                .expect("export request"),
            )
            .await,
    );
    let completed = poll_receipt(
        provider,
        SPACE,
        wire(&created)["receipt_id"].as_str().expect("receipt id"),
    )
    .await;
    let export_id = completed["result"]["export_id"]
        .as_str()
        .expect("export id")
        .to_string();
    let chunks = provider
        .export_chunks(SPACE, &export_id)
        .expect("provider chunks");
    (export_id, chunks)
}

/// Typed public `memory.imports.stage` request for one sealed provider chunk.
fn typed_stage_request(
    space: &str,
    chunk: &Value,
    key: String,
    conflict_policy: Option<&str>,
) -> MemoryImportsStageRequest {
    let mut request = json!({
        "space_id": space,
        "chunk": chunk,
        "idempotency_key": key,
    });
    if let Some(policy) = conflict_policy {
        request["conflict_policy"] = json!(policy);
    }
    serde_json::from_value(request).expect("typed stage request decodes")
}

async fn stage_chunks(
    provider: &SqliteMemoryProvider,
    space: &str,
    chunks: &[Value],
    adopt: Value,
) -> Value {
    let chunk_count = chunks.len();
    let mut final_receipt = Value::Null;
    for (ordinal, chunk) in chunks.iter().enumerate() {
        let mut request = json!({
            "space_id": space,
            "chunk": chunk,
            "idempotency_key": format!("stage-{space}-{ordinal}"),
        });
        if ordinal == 0 {
            request["conflict_policy"] = json!("fail");
            if adopt
                .as_array()
                .map(|items| !items.is_empty())
                .unwrap_or(false)
            {
                request["adopt_grants"] = adopt.clone();
            }
        }
        // Staged at value level so this helper returns the provider receipt JSON
        // directly; the typed public `imports.stage` boundary is covered by
        // `typed_staging_preserves_explicit_empty_chunk_arrays` and
        // `evicted_receipts_stop_grant_and_import_reexecution`.
        final_receipt = expect_ok(provider.stage_import_value(&principal(OPERATOR), request));
        assert_eq!(final_receipt["result"]["chunks_received"], ordinal + 1);
    }
    assert_eq!(
        final_receipt["result"]["chunks_received"],
        chunk_count as u64
    );
    final_receipt
}

fn empty_target_provider() -> SqliteMemoryProvider {
    let provider = provider();
    provider.provision_grant(
        OTHER_SPACE,
        grant_in(
            OTHER_SPACE,
            "grt_operator_admin_other",
            OPERATOR,
            &["admin"],
            None,
            None,
            None,
        ),
    );
    provider
}

#[tokio::test]
async fn staged_import_reports_counts_and_activates_atomically() {
    let provider = empty_target_provider();
    seed_record(
        &provider,
        "rec_1",
        "The gate key is under the third barrel.",
    );
    seed_record(&provider, "rec_2", "A second tally appears by the gate.");
    let _ = expect_ok(
        provider
            .update_profile(
                &principal(OPERATOR),
                serde_json::from_value(profile_update("personality", None, "prof-imp"))
                    .expect("profile update"),
            )
            .await,
    );
    let (_export_id, chunks) = export_chunks_of(&provider).await;

    let staged = stage_chunks(&provider, OTHER_SPACE, &chunks, json!([])).await;
    assert_eq!(staged["status"], "committed");
    let report = staged["result"]["staged_report"].clone();
    assert_eq!(report["counts"]["subjects"], 1);
    assert_eq!(report["counts"]["records"], 2);
    assert_eq!(report["counts"]["profiles"], 1);
    assert_eq!(report["grants_adopted"], 0);
    assert_eq!(report["grants_dropped"], 2);
    assert_eq!(report["verification"]["bundle_digest_ok"], true);
    let import_id = staged["result"]["import_id"]
        .as_str()
        .expect("import id")
        .to_string();

    // Staged data is invisible before activation.
    let before = expect_ok(
        provider
            .list_records(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": OTHER_SPACE,
                    "subject_id": SUBJECT,
                    "selection": selection_all(),
                }))
                .expect("list request"),
            )
            .await,
    );
    assert_eq!(wire(&before)["total_authorized"], 0);

    let activated = expect_ok(
        provider
            .activate_import(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": OTHER_SPACE,
                    "import_id": import_id,
                    "idempotency_key": "act-1",
                }))
                .expect("activate request"),
            )
            .await,
    );
    assert_eq!(wire(&activated)["status"], "pending");
    let completed = poll_receipt(
        &provider,
        OTHER_SPACE,
        wire(&activated)["receipt_id"].as_str().expect("receipt id"),
    )
    .await;
    assert_eq!(completed["status"], "committed");
    assert_eq!(completed["result"]["authority_generation"], 2);

    // Identity remap + revisions from 1.
    let after = expect_ok(
        provider
            .list_records(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": OTHER_SPACE,
                    "subject_id": SUBJECT,
                    "selection": selection_all(),
                }))
                .expect("list request"),
            )
            .await,
    );
    let after = wire(&after);
    assert_eq!(after["total_authorized"], 2);
    for record in after["records"].as_array().expect("records") {
        assert_eq!(record["revision"], 1);
        assert_eq!(record["space_id"], OTHER_SPACE);
    }
    let ids: Vec<&str> = after["records"]
        .as_array()
        .expect("records")
        .iter()
        .filter_map(|record| record["record_id"].as_str())
        .collect();
    assert_eq!(ids, vec!["rec_1", "rec_2"]);

    // No implicit adoption: the target keeps only its own admin grant.
    let grants = expect_ok(
        provider
            .list_grants(
                &principal(OPERATOR),
                serde_json::from_value(json!({ "space_id": OTHER_SPACE, "limit": 32 }))
                    .expect("list request"),
            )
            .await,
    );
    assert_eq!(wire(&grants)["grants"].as_array().expect("grants").len(), 1);
}

#[tokio::test]
async fn import_adoption_is_explicit_only_and_non_amplified() {
    let provider = empty_target_provider();
    seed_record(
        &provider,
        "rec_1",
        "The gate key is under the third barrel.",
    );
    let (_export_id, chunks) = export_chunks_of(&provider).await;

    let adopt = json!([{ "grant_id": "grt_reader_context_0002", "principal_id": OPERATOR }]);
    let staged = stage_chunks(&provider, OTHER_SPACE, &chunks, adopt).await;
    assert_eq!(staged["result"]["staged_report"]["grants_adopted"], 1);
    assert_eq!(staged["result"]["staged_report"]["grants_dropped"], 1);
    let import_id = staged["result"]["import_id"]
        .as_str()
        .expect("import id")
        .to_string();

    let activated = expect_ok(
        provider
            .activate_import(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": OTHER_SPACE,
                    "import_id": import_id,
                    "idempotency_key": "act-adopt",
                }))
                .expect("activate request"),
            )
            .await,
    );
    let _ = poll_receipt(
        &provider,
        OTHER_SPACE,
        wire(&activated)["receipt_id"].as_str().expect("receipt id"),
    )
    .await;

    let grants = expect_ok(
        provider
            .list_grants(
                &principal(OPERATOR),
                serde_json::from_value(json!({ "space_id": OTHER_SPACE, "limit": 32 }))
                    .expect("list request"),
            )
            .await,
    );
    let grants = wire(&grants)["grants"].clone();
    let grants = grants.as_array().expect("grants");
    assert_eq!(grants.len(), 2);
    assert!(grants
        .iter()
        .any(|grant| grant["grant_id"] == "grt_operator_admin_other"));
    let adopted = grants
        .iter()
        .find(|grant| grant["grant_id"] != "grt_operator_admin_other")
        .expect("adopted grant");
    assert!(adopted["capabilities"]
        .as_array()
        .expect("capabilities")
        .iter()
        .any(|capability| capability == "records_read"));
    // The unlisted source admin grant is absent (its id never appears).
    assert!(!grants
        .iter()
        .any(|grant| grant["grant_id"] == "grt_operator_admin_0001"));
}

#[tokio::test]
async fn discard_is_pre_activation_only() {
    let provider = empty_target_provider();
    seed_record(
        &provider,
        "rec_1",
        "The gate key is under the third barrel.",
    );
    let (_export_id, chunks) = export_chunks_of(&provider).await;
    let staged = stage_chunks(&provider, OTHER_SPACE, &chunks, json!([])).await;
    let import_id = staged["result"]["import_id"]
        .as_str()
        .expect("import id")
        .to_string();

    let discarded = expect_ok(
        provider
            .discard_import(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": OTHER_SPACE,
                    "import_id": import_id,
                    "idempotency_key": "disc-1",
                }))
                .expect("discard request"),
            )
            .await,
    );
    assert_eq!(wire(&discarded)["result"]["discarded"], true);

    assert_eq!(
        expect_reject(
            provider
                .activate_import(
                    &principal(OPERATOR),
                    serde_json::from_value(json!({
                        "space_id": OTHER_SPACE,
                        "import_id": import_id,
                        "idempotency_key": "act-after-discard",
                    }))
                    .expect("activate request"),
                )
                .await
        ),
        SpokeRejectCode::ImportConflict
    );
}

// ── C8 cognition lifecycle (P3/T1) ─────────────────────────────────────────

/// Valid recorded-at-time belief carrier (mirrors the shared lifecycle vector).
fn cognition_belief(record_id: &str, assessor: &str, assessed_at: &str, q: f64) -> Value {
    json!({
        "kind": "belief",
        "record_id": record_id,
        "holder": { "kind": "principal", "namespace": "system_a", "id": "alice" },
        "target": {
            "carrier": { "kind": "knowledge_entry", "namespace": "system_a", "id": "ke_alice" },
            "target_id": "t_prop_mem_1",
            "kind": "proposition"
        },
        "assessor": { "kind": "principal", "namespace": "system_a", "id": assessor },
        "assessor_role": "holder_self_report",
        "basis": { "kind": "evidence", "evidence": [
            { "kind": "knowledge_entry", "namespace": "system_a", "id": "ke_evt_mem_1" }
        ]},
        "assessment_context": { "context_id": "ctx_default" },
        "effective_at": "2026-07-23T06:00:00Z",
        "assessed_at": assessed_at,
        "view": "recorded_at",
        "order": 1,
        "q": {
            "value": q,
            "assertion": "belief_attribution",
            "method": "elicitation_v1",
            "calibration": "calibrated"
        },
        "extensions": {}
    })
}

fn cognition_capture_request(content: Value, key: &str) -> Value {
    json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "content": content,
        "capture_scope": { "visibility": "context", "context_id": CONTEXT },
        "source_context_ids": [CONTEXT],
        "kind_hint": "fact",
        "idempotency_key": key,
    })
}

async fn capture_value(provider: &SqliteMemoryProvider, content: Value, key: &str) -> Value {
    let receipt = expect_ok(
        provider
            .capture(
                &principal(READER),
                serde_json::from_value(cognition_capture_request(content, key))
                    .expect("capture request"),
            )
            .await,
    );
    wire(&receipt)
}

#[tokio::test]
async fn cognition_capture_pending_review_lifecycle() {
    let provider = provider();

    // A valid recorded-at-time carrier is admitted and stays on the candidate.
    let belief = cognition_belief("rec_belief_mem_1", "alice", "2026-07-23T06:05:00Z", 0.9);
    let captured = capture_value(
        &provider,
        json!({
            "text": "Mira believes the harbor guild fenced the spare key.",
            "cognition": [belief.clone()],
        }),
        "cap-cog-1",
    )
    .await;
    assert_eq!(captured["result"]["classified"], 1);
    let candidate_id = captured["result"]["candidate_ids"][0]
        .as_str()
        .expect("candidate id")
        .to_string();
    let candidate = expect_ok(
        provider
            .get_candidate(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "candidate_id": candidate_id,
                }))
                .expect("get request"),
            )
            .await,
    );
    let candidate = wire(&candidate)["candidate"].clone();
    assert_eq!(candidate["status"], "pending");
    assert_eq!(candidate["proposed"]["content"]["cognition"][0], belief);

    // A malformed carrier (current view) is rejected before any state change.
    assert_eq!(
        expect_reject(
            provider
                .capture(
                    &principal(READER),
                    serde_json::from_value(cognition_capture_request(
                        json!({
                            "text": "Mira believes the harbor guild fenced the spare key.",
                            "cognition": [{
                                "kind": "belief",
                                "record_id": "rec_belief_mem_1",
                                "holder": {
                                    "kind": "principal",
                                    "namespace": "system_a",
                                    "id": "alice"
                                },
                                "target": {
                                    "carrier": {
                                        "kind": "knowledge_entry",
                                        "namespace": "system_a",
                                        "id": "ke_alice"
                                    },
                                    "target_id": "t_prop_mem_1",
                                    "kind": "proposition"
                                },
                                "assessor": {
                                    "kind": "principal",
                                    "namespace": "system_a",
                                    "id": "alice"
                                },
                                "assessor_role": "holder_self_report",
                                "basis": { "kind": "evidence", "evidence": [] },
                                "assessment_context": { "context_id": "ctx_default" },
                                "effective_at": "2026-07-23T06:00:00Z",
                                "assessed_at": "2026-07-23T06:05:00Z",
                                "view": "current",
                                "order": 1,
                                "q": {
                                    "value": 0.9,
                                    "assertion": "belief_attribution",
                                    "method": "elicitation_v1",
                                    "calibration": "calibrated"
                                },
                                "extensions": {}
                            }],
                        }),
                        "cap-cog-bad",
                    ))
                    .expect("capture request"),
                )
                .await
        ),
        SpokeRejectCode::InvalidInput
    );

    // Cognition counts as content: a cognition-only candidate is not dropped.
    let cognition_only = capture_value(
        &provider,
        json!({ "text": "  ", "cognition": [belief.clone()] }),
        "cap-cog-only",
    )
    .await;
    assert_eq!(cognition_only["result"]["classified"], 1);

    // A review override carrying cognition is validated independently and
    // promoted with the cognition retained verbatim (recorded-at-time stays;
    // no KE is created or synchronized).
    let reviewed = expect_ok(
        provider
            .review_candidate(
                &principal(READER),
                serde_json::from_value(review_request(
                    &candidate_id,
                    1,
                    "rev-cog-1",
                    json!({
                        "action": "promote",
                        "content_override": {
                            "text": "Mira believes the harbor guild fenced the spare key.",
                            "cognition": [belief.clone()],
                        },
                    }),
                ))
                .expect("review request"),
            )
            .await,
    );
    let reviewed = wire(&reviewed);
    assert_eq!(reviewed["result"]["outcome"], "promoted");
    let record_id = reviewed["result"]["record_ids"][0]
        .as_str()
        .expect("record id")
        .to_string();
    let record = expect_ok(
        provider
            .get_record(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": record_id,
                }))
                .expect("get request"),
            )
            .await,
    );
    let record = wire(&record)["record"].clone();
    assert_eq!(record["content"]["cognition"][0], belief);
    assert_eq!(record["content"]["cognition"][0]["view"], "recorded_at");
    assert_eq!(record["scope"]["context_id"], CONTEXT);

    // The consumed shell scrubs the proposed content: no cognition remains.
    let consumed = expect_ok(
        provider
            .get_candidate(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "candidate_id": candidate_id,
                }))
                .expect("get request"),
            )
            .await,
    );
    let consumed = wire(&consumed)["candidate"].clone();
    assert_eq!(consumed["status"], "consumed");
    assert!(consumed["proposed"].get("content").is_none());
    assert!(consumed.get("content_digest").is_some());
}

fn assert_invalid_cognition(reject: &spoke_operations::SpokeReject, field: &str) {
    assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
    let details = reject.details.as_ref().expect("rejection details");
    assert_eq!(details["field"], json!(field));
    assert_eq!(details["reason"], json!("invalid_cognition_record"));
    assert!(
        details.get("limit").is_none(),
        "byte admission must not win over invalid cognition"
    );
}

#[tokio::test]
async fn cognition_classification_ignores_explicit_empty_array() {
    let provider = provider();
    let belief = cognition_belief("rec_belief_mem_1", "alice", "2026-07-23T06:05:00Z", 0.9);

    // Meaningful cognition with whitespace-only text is retainable content.
    let promoted = capture_value(
        &provider,
        json!({ "text": "   \n\t ", "cognition": [belief] }),
        "cap-cog-content",
    )
    .await;
    assert_eq!(promoted["result"]["classified"], 1);
    let candidate_id = promoted["result"]["candidate_ids"][0]
        .as_str()
        .expect("candidate id")
        .to_string();
    let candidate = expect_ok(
        provider
            .get_candidate(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "candidate_id": candidate_id,
                }))
                .expect("get request"),
            )
            .await,
    );
    assert!(!wire(&candidate)["candidate"]["proposed"]["content"]["cognition"]
        .as_array()
        .expect("cognition array")
        .is_empty());

    let pending = |provider_listed: Value| provider_listed["candidates"].clone();
    let listed_before = pending(wire(
        &expect_ok(
            provider
                .list_candidates(
                    &principal(READER),
                    serde_json::from_value::<spoke_schemas::MemoryCandidatesListRequest>(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                    }))
                    .expect("list request"),
                )
                .await,
        ),
    ));
    assert_eq!(
        listed_before.as_array().expect("candidates").len(),
        1,
        "the meaningful cognition capture stays pending"
    );

    // Whitespace text plus an explicitly present empty array records no
    // assessment: it must not create a candidate or alter pending state.
    let empty = capture_value(
        &provider,
        json!({ "text": "   \n\t ", "cognition": [] }),
        "cap-cog-empty",
    )
    .await;
    assert_eq!(empty["result"]["classified"], 0);
    assert_eq!(empty["result"]["candidate_ids"], json!([]));

    let listed_after = pending(wire(
        &expect_ok(
            provider
                .list_candidates(
                    &principal(READER),
                    serde_json::from_value::<spoke_schemas::MemoryCandidatesListRequest>(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                    }))
                    .expect("list request"),
                )
                .await,
        ),
    ));
    assert_eq!(listed_after, listed_before, "live state is unchanged");
}

#[tokio::test]
async fn cognition_invalid_wins_over_over_budget_at_provider_boundaries() {
    let provider = provider();
    let mut belief = cognition_belief("rec_belief_mem_1", "alice", "2026-07-23T06:05:00Z", 0.9);
    belief["view"] = json!("current");
    // Structurally complete except for the recorded-at-time view, and larger
    // than the provider's own admission limit, so both failures compete.
    let content = json!({
        "text": "x".repeat(70_000),
        "cognition": [belief],
    });
    assert!(
        content_serialized_bytes(&content) > sqlite_memory::MAX_CONTENT_BYTES as usize,
        "content must exceed the provider admission limit"
    );

    let capture = expect_reject_with(
        provider
            .capture(
                &principal(READER),
                serde_json::from_value(cognition_capture_request(
                    content.clone(),
                    "cap-cog-over-budget",
                ))
                .expect("capture request"),
            )
            .await,
    );
    assert_invalid_cognition(&capture, "content.cognition");

    let create = expect_reject_with(
        provider
            .create_record(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "kind": "fact",
                    "content": content.clone(),
                    "scope": { "visibility": "context", "context_id": CONTEXT },
                    "idempotency_key": "rec-cog-over-budget",
                }))
                .expect("create request"),
            )
            .await,
    );
    assert_invalid_cognition(&create, "content.cognition");

    let update = expect_reject_with(
        provider
            .update_record(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": "rec_cog_over_budget",
                    "expected_revision": 1,
                    "content": content,
                    "idempotency_key": "upd-cog-over-budget",
                }))
                .expect("update request"),
            )
            .await,
    );
    assert_invalid_cognition(&update, "content.cognition");
}

#[tokio::test]
async fn cognition_review_rejects_malformed_override_without_state_change() {
    let provider = provider();
    let captured = capture_value(
        &provider,
        json!({
            "text": "Mira believes the harbor guild fenced the spare key.",
            "cognition": [cognition_belief("rec_belief_mem_1", "alice", "2026-07-23T06:05:00Z", 0.9)],
        }),
        "cap-cog-2",
    )
    .await;
    let candidate_id = captured["result"]["candidate_ids"][0]
        .as_str()
        .expect("candidate id")
        .to_string();

    let malformed = json!({
        "action": "promote",
        "content_override": {
            "text": "Override.",
            "cognition": [{
                "kind": "belief",
                "record_id": "rec_belief_mem_1",
                "holder": {
                    "kind": "principal",
                    "namespace": "system_a",
                    "id": "alice"
                },
                "target": {
                    "carrier": {
                        "kind": "knowledge_entry",
                        "namespace": "system_a",
                        "id": "ke_alice"
                    },
                    "target_id": "t_prop_mem_1",
                    "kind": "proposition"
                },
                "assessor": {
                    "kind": "principal",
                    "namespace": "system_a",
                    "id": "alice"
                },
                "assessor_role": "holder_self_report",
                "basis": { "kind": "evidence", "evidence": [] },
                "assessment_context": { "context_id": "ctx_default" },
                "effective_at": "2026-07-23T06:00:00Z",
                "assessed_at": "2026-07-23T06:05:00Z",
                "view": "current",
                "order": 1,
                "q": {
                    "value": 0.9,
                    "assertion": "belief_attribution",
                    "method": "elicitation_v1",
                    "calibration": "calibrated"
                },
                "extensions": {}
            }],
        },
    });
    let reject = expect_reject_with(
        provider
            .review_candidate(
                &principal(READER),
                serde_json::from_value(review_request(
                    &candidate_id,
                    1,
                    "rev-cog-bad",
                    malformed.clone(),
                ))
                .expect("review request"),
            )
            .await,
    );
    assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
    let details = reject.details.expect("review rejection details");
    assert_eq!(details["field"], json!("decision.content_override"));
    assert_eq!(details["reason"], json!("invalid_cognition_record"));

    // The fragment path uses the same Content guard and the same identity,
    // independently of the override.
    let malformed_cognition = malformed["content_override"]["cognition"].clone();
    let fragment_decision = json!({
        "action": "fragment",
        "fragments": [{
            "kind": "note",
            "content": {
                "text": "Fragment.",
                "cognition": malformed_cognition,
            },
        }],
    });
    let reject = expect_reject_with(
        provider
            .review_candidate(
                &principal(READER),
                serde_json::from_value(review_request(
                    &candidate_id,
                    1,
                    "rev-cog-frag-bad",
                    fragment_decision,
                ))
                .expect("review request"),
            )
            .await,
    );
    let details = reject.details.expect("fragment rejection details");
    assert_eq!(details["field"], json!("decision.fragments.content"));
    assert_eq!(details["reason"], json!("invalid_cognition_record"));

    // The candidate is untouched: still pending at revision 1, no records.
    let candidate = expect_ok(
        provider
            .get_candidate(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "candidate_id": candidate_id,
                }))
                .expect("get request"),
            )
            .await,
    );
    let candidate = wire(&candidate)["candidate"].clone();
    assert_eq!(candidate["status"], "pending");
    assert_eq!(candidate["revision"], 1);
    let listed = expect_ok(
        provider
            .list_records(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": selection_all(),
                }))
                .expect("list request"),
            )
            .await,
    );
    assert_eq!(wire(&listed)["total_authorized"], 0);
}

#[tokio::test]
async fn cognition_create_update_retains_attribution_and_replays_idempotently() {
    let provider = provider();
    let belief = cognition_belief("rec_belief_mem_1", "alice", "2026-07-23T06:05:00Z", 0.9);
    let created = expect_ok(
        provider
            .create_record(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "kind": "fact",
                    "content": {
                        "text": "Mira believes the harbor guild fenced the spare key.",
                        "cognition": [belief.clone()],
                    },
                    "scope": { "visibility": "context", "context_id": CONTEXT },
                    "idempotency_key": "rec-cog-1",
                }))
                .expect("create request"),
            )
            .await,
    );
    let record_id = wire(&created)["result"]["record_id"]
        .as_str()
        .expect("record id")
        .to_string();

    // Malformed cognition on create/update rejects INVALID_INPUT.
    assert_eq!(
        expect_reject(
            provider
                .create_record(
                    &principal(READER),
                    serde_json::from_value(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                        "kind": "fact",
                        "content": {
                            "text": "x",
                            "cognition": [{
                                "kind": "belief",
                                "record_id": "rec_belief_mem_1",
                                "holder": {
                                    "kind": "principal",
                                    "namespace": "system_a",
                                    "id": "alice"
                                },
                                "target": {
                                    "carrier": {
                                        "kind": "knowledge_entry",
                                        "namespace": "system_a",
                                        "id": "ke_alice"
                                    },
                                    "target_id": "t_prop_mem_1",
                                    "kind": "proposition"
                                },
                                "assessor": {
                                    "kind": "principal",
                                    "namespace": "system_a",
                                    "id": "alice"
                                },
                                "assessor_role": "holder_self_report",
                                "basis": { "kind": "evidence", "evidence": [] },
                                "assessment_context": { "context_id": "ctx_default" },
                                "effective_at": "2026-07-23T06:00:00Z",
                                "assessed_at": "2026-07-23T06:05:00Z",
                                "view": "current",
                                "order": 1,
                                "q": {
                                    "value": 0.9,
                                    "assertion": "belief_attribution",
                                    "method": "elicitation_v1",
                                    "calibration": "calibrated"
                                },
                                "extensions": {}
                            }],
                        },
                        "scope": { "visibility": "context", "context_id": CONTEXT },
                        "idempotency_key": "rec-cog-bad",
                    }))
                    .expect("create request"),
                )
                .await
        ),
        SpokeRejectCode::InvalidInput
    );

    // The shared revision vector: the retained original entry keeps alice's
    // attribution and time; the reassessment is a separately attributed entry.
    let vector_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/cognition/memory-lifecycle-vectors.json"
    );
    let vectors: Value = serde_json::from_str(
        &std::fs::read_to_string(vector_path).expect("lifecycle vectors readable"),
    )
    .expect("lifecycle vectors parse");
    let revision_vector = vectors["vectors"]
        .as_array()
        .expect("vectors array")
        .iter()
        .find(|vector| vector["id"] == "lifecycle-revision-reassessment-content")
        .expect("revision vector present");
    let revised = revision_vector["content"].clone();

    let update_request = json!({
        "space_id": SPACE,
        "record_id": record_id,
        "expected_revision": 1,
        "content": revised,
        "idempotency_key": "rec-cog-upd",
    });
    let updated = expect_ok(
        provider
            .update_record(
                &principal(READER),
                serde_json::from_value(update_request.clone()).expect("update request"),
            )
            .await,
    );
    assert_eq!(wire(&updated)["result"]["revision"], 2);

    // An idempotent retry replays the stored receipt unchanged.
    let replayed = expect_ok(
        provider
            .update_record(
                &principal(READER),
                serde_json::from_value(update_request).expect("update request"),
            )
            .await,
    );
    assert_eq!(
        wire(&replayed)["receipt_id"],
        wire(&updated)["receipt_id"],
        "idempotent retry replays the same receipt"
    );

    // A stale write still rejects with the existing CAS error.
    assert_eq!(
        expect_reject(
            provider
                .update_record(
                    &principal(READER),
                    serde_json::from_value(json!({
                        "space_id": SPACE,
                        "record_id": record_id,
                        "expected_revision": 1,
                        "content": { "text": "stale" },
                        "idempotency_key": "rec-cog-stale",
                    }))
                    .expect("update request"),
                )
                .await
        ),
        SpokeRejectCode::RevisionConflict
    );

    let record = expect_ok(
        provider
            .get_record(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": record_id,
                }))
                .expect("get request"),
            )
            .await,
    );
    let record = wire(&record)["record"].clone();
    let cognition = record["content"]["cognition"]
        .as_array()
        .expect("cognition entries");
    assert_eq!(cognition.len(), 2);
    assert_eq!(cognition[0]["assessor"]["id"], "alice");
    assert_eq!(cognition[0]["assessed_at"], "2026-07-23T06:05:00Z");
    assert_eq!(cognition[1]["assessor"]["id"], "bob");
    assert_eq!(cognition[1]["assessed_at"], "2026-07-24T06:05:00Z");
    for entry in cognition {
        assert_eq!(entry["view"], "recorded_at");
    }

    // Delete scrubs the content: the tombstone shell carries no cognition.
    let revision = record["revision"].as_u64().expect("revision");
    let deleted = expect_ok(
        provider
            .delete_record(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": record_id,
                    "expected_revision": revision,
                    "idempotency_key": "rec-cog-del",
                }))
                .expect("delete request"),
            )
            .await,
    );
    assert_eq!(wire(&deleted)["result"]["tombstoned"], true);
    let tombstoned = expect_ok(
        provider
            .get_record(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": record_id,
                }))
                .expect("get request"),
            )
            .await,
    );
    let tombstoned = wire(&tombstoned)["record"].clone();
    assert_eq!(tombstoned["content"], json!({ "text": "" }));
    assert!(tombstoned["content"].get("cognition").is_none());
    assert!(tombstoned.get("tombstone").is_some());
}

/// Memory-wire boundary only: scrub/content_digest/export digest across the
/// typed provider. KnowledgeEntry state is proven by the separate
/// `feature/cognition-c8-no-ke-boundary` track (`memory_loopback.rs`).
#[tokio::test]
async fn cognition_scrub_privacy_digest_and_memory_wire_boundary() {
    let provider = provider();
    let belief = cognition_belief("rec_belief_mem_1", "alice", "2026-07-23T06:05:00Z", 0.9);
    let original_content = json!({
        "text": "Mira believes the harbor guild fenced the spare key.",
        "cognition": [belief],
    });

    // Capture and review-promote: the consumed shell keeps no proposed content
    // and its content_digest commits to the exact cognition-bearing content.
    let captured = capture_value(&provider, original_content.clone(), "cap-priv-1").await;
    let candidate_id = captured["result"]["candidate_ids"][0]
        .as_str()
        .expect("candidate id")
        .to_string();
    let reviewed = expect_ok(
        provider
            .review_candidate(
                &principal(READER),
                serde_json::from_value(review_request(
                    &candidate_id,
                    1,
                    "rev-priv-1",
                    json!({ "action": "promote", "kind": "fact" }),
                ))
                .expect("review request"),
            )
            .await,
    );
    let reviewed = wire(&reviewed);
    let record_id = reviewed["result"]["record_ids"][0]
        .as_str()
        .expect("record id")
        .to_string();
    let consumed = expect_ok(
        provider
            .get_candidate(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "candidate_id": candidate_id,
                }))
                .expect("get request"),
            )
            .await,
    );
    let consumed = wire(&consumed)["candidate"].clone();
    assert!(consumed["proposed"].get("content").is_none());
    assert_eq!(
        consumed["content_digest"],
        json!(expect_ok(jcs_sha256_hex(&original_content))),
        "the consumed digest commits to the original cognition-bearing content"
    );

    // Promote/retention moves placement only: the retained cognition and its
    // attribution are unchanged and no KnowledgeEntry surface is emitted.
    let promoted = expect_ok(
        provider
            .promote_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": record_id,
                    "expected_revision": 1,
                    "target_scope": { "visibility": "shared" },
                    "idempotency_key": "promote-priv-1",
                }))
                .expect("promote request"),
            )
            .await,
    );
    let promoted = wire(&promoted);
    let promote_result = promoted["result"].as_object().expect("promote result");
    assert!(promote_result.get("knowledge_entry").is_none());
    assert!(promote_result.get("knowledge_entry_id").is_none());
    assert!(!promoted.to_string().contains("knowledge_entry"));

    // The promoted placement is shared; read it with the space-wide operator.
    let record = expect_ok(
        provider
            .get_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": record_id,
                }))
                .expect("get request"),
            )
            .await,
    );
    let record = wire(&record)["record"].clone();
    assert_eq!(
        record["content"], original_content,
        "promotion mutates placement, never the retained cognition"
    );
    let revision = record["revision"].as_u64().expect("revision");

    // Full retirement scrubs the content; the tombstone carries no cognition.
    let deleted = expect_ok(
        provider
            .delete_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": record_id,
                    "expected_revision": revision,
                    "idempotency_key": "del-priv-1",
                }))
                .expect("delete request"),
            )
            .await,
    );
    assert_eq!(wire(&deleted)["result"]["tombstoned"], true);

    // A post-scrub export (including tombstones) exposes no residual cognition,
    // and its chunk/bundle digests still verify against the scrubbed bundle.
    let created = expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "include_tombstoned": true,
                    "require_quiescent": false,
                    "idempotency_key": "exp-priv-1",
                }))
                .expect("export request"),
            )
            .await,
    );
    let created = wire(&created);
    let completed = poll_receipt(
        &provider,
        SPACE,
        created["receipt_id"].as_str().expect("receipt id"),
    )
    .await;
    assert_eq!(completed["status"], "committed");
    let export_id = completed["result"]["export_id"]
        .as_str()
        .expect("export id")
        .to_string();
    let advertised = completed["result"]["bundle_digest"]
        .as_str()
        .expect("bundle digest")
        .to_string();
    let raw_chunks = provider
        .export_chunks(SPACE, &export_id)
        .expect("provider chunks");
    for (ordinal, chunk) in raw_chunks.iter().enumerate() {
        assert_eq!(
            chunk_digest(chunk).expect("chunk digest"),
            chunk["chunk_digest"],
            "chunk {ordinal} digest"
        );
        assert_eq!(chunk["bundle_digest"], json!(advertised));
    }
    let verification = verify_reassembly(&raw_chunks);
    assert!(
        verification.bundle_digest_ok && verification.errors.is_empty(),
        "chunk verification failed: {:?}",
        verification.errors
    );
    let bundle = reassemble(&raw_chunks).expect("reassemble");
    assert_eq!(bundle_digest(&bundle).expect("bundle digest"), advertised);
    assert_eq!(bundle["digest"], json!(advertised));
    assert!(
        !serde_json::to_string(&bundle)
            .expect("bundle serializes")
            .contains("\"cognition\""),
        "the post-scrub export carries no residual cognition"
    );
    let exported_record = bundle["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|record| record["record_id"] == json!(record_id))
        .expect("the tombstone is exported");
    assert_eq!(exported_record["content"], json!({ "text": "" }));
    assert!(exported_record.get("tombstone").is_some());
    assert!(
        bundle.get("knowledge_entries").is_none(),
        "the Memory export has no KnowledgeEntry surface"
    );
}

#[tokio::test]
async fn cognition_transfer_remaps_references() {
    let provider = provider();
    // A cognition record referencing the bundled subject and an external
    // cross-system identity, plus opaque members that merely look like
    // references and must survive verbatim.
    let mut belief = cognition_belief("rec_belief_mem_1", "alice", "2026-07-23T06:05:00Z", 0.9);
    belief["holder"] = json!({ "kind": "memory_subject", "namespace": "mem", "id": SUBJECT });
    belief["assessor"] =
        json!({ "kind": "external", "namespace": "system_a", "id": "alice" });
    belief["order"] = json!(2);
    belief["target"]["text"] = json!("untrusted text naming alice");
    belief["target"]["about"] = json!({
        "carrier": { "kind": "external", "namespace": "system_a", "id": "alice" },
        "target_id": "t_prop_nested",
        "kind": "proposition"
    });
    belief["extensions"] = json!({
        "product": { "kind": "external", "namespace": "system_a", "id": "alice" }
    });
    provider.provision_record(
        SPACE,
        json!({
            "schema_version": 1,
            "record_id": "rec_cog",
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "kind": "fact",
            "content": {
                "text": "Mira believes the harbor guild fenced the spare key.",
                "structured": {
                    "kind": "external",
                    "namespace": "system_a",
                    "id": "alice",
                    "extra": 1
                },
                "cognition": [belief],
            },
            "scope": { "visibility": "context", "context_id": CONTEXT },
            "provenance": {
                "source_context_ids": [CONTEXT],
                "anchors": [],
                "captured_by": OPERATOR,
                "captured_at": "2026-07-23T06:00:00Z",
            },
            "created_at": "2026-07-23T06:00:00Z",
            "updated_at": "2026-07-23T06:00:00Z",
            "revision": 1,
            "extensions": {},
        }),
    );
    let (_export_id, chunks) = export_chunks_of(&provider).await;

    // The target already owns the subject, so the rename policy applies and
    // the subject reference must follow; the explicit pair translates the
    // external reference across systems.
    let provider = empty_target_provider();
    provider.provision_subject(OTHER_SPACE, SUBJECT, Some(("toy", "character")));
    let chunk_count = chunks.len();
    let mut staged = Value::Null;
    for (ordinal, chunk) in chunks.iter().enumerate() {
        let mut request = json!({
            "space_id": OTHER_SPACE,
            "chunk": chunk,
            "idempotency_key": format!("stage-cog-{ordinal}"),
        });
        if ordinal == 0 {
            request["conflict_policy"] = json!("rename_subject");
            // The typed transport carries the exact pairs; the generated
            // request variant must preserve them verbatim.
            request["reference_translations"] = json!([
                {
                    "from": { "kind": "external", "namespace": "system_a", "id": "alice" },
                    "to": { "kind": "external", "namespace": "system_c", "id": "alice" }
                }
            ]);
        }
        let typed: MemoryImportsStageRequest =
            serde_json::from_value(request).expect("typed stage request");
        let response = expect_ok(provider.stage_import(&principal(OPERATOR), typed).await);
        staged = wire(&response);
        assert_eq!(staged["result"]["chunks_received"], ordinal + 1);
    }
    assert_eq!(staged["result"]["chunks_received"], chunk_count as u64);
    let import_id = staged["result"]["import_id"]
        .as_str()
        .expect("import id")
        .to_string();
    let activated = expect_ok(
        provider
            .activate_import(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": OTHER_SPACE,
                    "import_id": import_id,
                    "idempotency_key": "act-cog",
                }))
                .expect("activate request"),
            )
            .await,
    );
    let completed = poll_receipt(
        &provider,
        OTHER_SPACE,
        wire(&activated)["receipt_id"].as_str().expect("receipt id"),
    )
    .await;
    assert_eq!(completed["status"], "committed");

    let imported = expect_ok(
        provider
            .get_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": OTHER_SPACE,
                    "record_id": "rec_cog",
                }))
                .expect("get request"),
            )
            .await,
    );
    let imported = wire(&imported)["record"].clone();
    let content = &imported["content"];
    let cognition = &content["cognition"][0];
    let holder = &cognition["holder"];
    assert_eq!(holder["kind"], "memory_subject");
    assert_ne!(holder["id"], SUBJECT, "no source subject id is retained");
    let cross_system = json!({ "kind": "external", "namespace": "system_c", "id": "alice" });
    assert_eq!(&cognition["assessor"], &cross_system);
    // §6.3 translates the complete typed target chain, not bare ids.
    assert_eq!(&cognition["target"]["about"]["carrier"], &cross_system);
    // Opaque lookalikes are never treated as references.
    assert_eq!(
        &content["structured"],
        &json!({
            "kind": "external",
            "namespace": "system_a",
            "id": "alice",
            "extra": 1
        })
    );
    assert_eq!(
        &cognition["extensions"],
        &json!({ "product": { "kind": "external", "namespace": "system_a", "id": "alice" } })
    );
    assert_eq!(
        cognition["target"]["text"],
        json!("untrusted text naming alice")
    );
}

#[tokio::test]
async fn typed_staging_preserves_explicit_empty_chunk_arrays() {
    // §6.13 seals the chunk's exact JSON minus `chunk_digest`. A source chunk
    // with explicitly empty bundle arrays (here every array but `subjects`)
    // must reach activation through the typed `MemoryPort::stage_import`
    // boundary: the generated carriers keep `[]` distinct from absence, so the
    // recomputed chunk digest still matches the sealed bytes.
    use spoke_fixture_memory_engine::transfer::{bundle_digest, partition_bundle};

    let provider = empty_target_provider();
    let mut bundle = json!({
        "schema_version": 1,
        "bundle_format_version": 1,
        "memory_protocol_version": 1,
        "export_id": "exp_empty_arrays_0001",
        "space_id": SPACE,
        "created_at": "2026-07-23T06:00:00Z",
        "source": {
            "authority_host_id": "host_tw_primary",
            "authority_generation": 1,
            "engine_policy_version": ENGINE_POLICY_VERSION,
        },
        "subjects": [{
            "schema_version": 1,
            "subject_id": "toy:character:empty-arrays",
            "space_id": SPACE,
            "status": "active",
            "fenced_contexts": [],
            "created_at": "2026-07-23T06:00:00Z",
            "revision": 1,
            "extensions": {},
        }],
        "records": [],
        "profiles": [],
        "reflections": [],
        "candidates": [],
        "receipt_summaries": [],
        "grants": [],
        "extensions": {},
    });
    let digest = bundle_digest(&bundle).expect("bundle digest computes");
    bundle["digest"] = json!(digest);

    let chunks = partition_bundle(&bundle, 1 << 20).expect("bundle partitions");
    assert_eq!(chunks.len(), 1, "the fixture keeps one header chunk");
    assert_eq!(chunks[0]["profiles"], json!([]), "explicit empty arrays are sealed");

    let request = json!({
        "space_id": OTHER_SPACE,
        "chunk": chunks[0],
        "conflict_policy": "fail",
        "idempotency_key": "stage-empty-arrays",
    });
    let typed: MemoryImportsStageRequest =
        serde_json::from_value(request).expect("typed stage request decodes");
    let response = expect_ok(provider.stage_import(&principal(OPERATOR), typed).await);
    assert_eq!(wire(&response)["result"]["chunks_received"], 1);
}

#[tokio::test]
async fn cognition_transfer_invalid_translated_content_fails_closed() {
    let provider = provider();
    // A digest-valid bundle whose cognition record violates the Content guard
    // (a completed record in the current view) must not be staged: §6.2
    // validates translated Content before persistence, not only at activation.
    let mut belief = cognition_belief("rec_belief_mem_1", "alice", "2026-07-23T06:05:00Z", 0.9);
    belief["view"] = json!("current");
    provider.provision_record(
        SPACE,
        json!({
            "schema_version": 1,
            "record_id": "rec_cog",
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "kind": "fact",
            "content": {
                "text": "Mira believes the harbor guild fenced the spare key.",
                "cognition": [belief],
            },
            "scope": { "visibility": "context", "context_id": CONTEXT },
            "provenance": {
                "source_context_ids": [CONTEXT],
                "anchors": [],
                "captured_by": OPERATOR,
                "captured_at": "2026-07-23T06:00:00Z",
            },
            "created_at": "2026-07-23T06:00:00Z",
            "updated_at": "2026-07-23T06:00:00Z",
            "revision": 1,
            "extensions": {},
        }),
    );
    let (_export_id, chunks) = export_chunks_of(&provider).await;
    let provider = empty_target_provider();
    let chunk_count = chunks.len();

    for (ordinal, chunk) in chunks.iter().enumerate() {
        let mut request = json!({
            "space_id": OTHER_SPACE,
            "chunk": chunk,
            "idempotency_key": format!("stage-cog-invalid-{ordinal}"),
        });
        if ordinal == 0 {
            request["conflict_policy"] = json!("fail");
        }
        let typed: MemoryImportsStageRequest =
            serde_json::from_value(request).expect("typed stage request");
        let result = provider.stage_import(&principal(OPERATOR), typed).await;
        if ordinal + 1 == chunk_count {
            let reject = match result {
                SpokeResult::Reject(reject) => reject,
                SpokeResult::Ok(value) => {
                    panic!("invalid translated cognition must reject, got {value:?}")
                }
            };
            assert_eq!(reject.code, SpokeRejectCode::ImportConflict);
            let errors = reject
                .details
                .as_ref()
                .and_then(|details| details.get("errors"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            assert!(
                errors.iter().any(|error| error
                    .as_str()
                    .is_some_and(|text| text.contains("record rec_cog")
                        && text.contains("recorded-at-time"))),
                "content validation error expected, got {errors:?}"
            );
        } else {
            expect_ok(result);
        }
    }
    // A rejected final chunk stages nothing: the import id sequence never
    // advanced, so activation reports the import as absent.
    let activated = provider
        .activate_import(
            &principal(OPERATOR),
            serde_json::from_value(json!({
                "space_id": OTHER_SPACE,
                "import_id": "imp_sqlite_0001",
                "idempotency_key": "act-cog-invalid",
            }))
            .expect("activate request"),
        )
        .await;
    assert_eq!(expect_reject(activated), SpokeRejectCode::NotFound);
}

#[tokio::test]
async fn cognition_transfer_missing_reference_fails_closed() {
    let provider = provider();
    let mut belief = cognition_belief("rec_belief_mem_1", "alice", "2026-07-23T06:05:00Z", 0.9);
    // `subj_unmapped` is not a bundled subject: no assignment can exist.
    belief["holder"] = json!({ "kind": "memory_subject", "namespace": "mem", "id": "subj_unmapped" });
    provider.provision_record(
        SPACE,
        json!({
            "schema_version": 1,
            "record_id": "rec_cog",
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "kind": "fact",
            "content": { "text": "Mira believes the harbor guild fenced the spare key.", "cognition": [belief] },
            "scope": { "visibility": "context", "context_id": CONTEXT },
            "provenance": {
                "source_context_ids": [CONTEXT],
                "anchors": [],
                "captured_by": OPERATOR,
                "captured_at": "2026-07-23T06:00:00Z",
            },
            "created_at": "2026-07-23T06:00:00Z",
            "updated_at": "2026-07-23T06:00:00Z",
            "revision": 1,
            "extensions": {},
        }),
    );
    let (_export_id, chunks) = export_chunks_of(&provider).await;

    let provider = empty_target_provider();
    let chunk_count = chunks.len();
    for (ordinal, chunk) in chunks.iter().enumerate() {
        let mut request = json!({
            "space_id": OTHER_SPACE,
            "chunk": chunk,
            "idempotency_key": format!("stage-cog-miss-{ordinal}"),
        });
        if ordinal == 0 {
            request["conflict_policy"] = json!("fail");
        }
        let result = provider.stage_import_value(&principal(OPERATOR), request);
        if ordinal + 1 == chunk_count {
            assert_eq!(
                expect_reject(result),
                SpokeRejectCode::ImportConflict,
                "unresolved cognition reference must fail closed at staging"
            );
        } else {
            expect_ok(result);
        }
    }
}

// ── Provider authority / replay closing wave ──────────────────────────────

/// A same-key replay after the receipt body was evicted must answer
/// `RECEIPT_EXPIRED` (MEM-R021) and carry the tombstone's last known status.
fn expect_receipt_expired<T: std::fmt::Debug>(
    result: SpokeResult<T>,
) -> spoke_operations::SpokeReject {
    expect_receipt_expired_as(result, "committed")
}

fn expect_receipt_expired_as<T: std::fmt::Debug>(
    result: SpokeResult<T>,
    status: &str,
) -> spoke_operations::SpokeReject {
    let reject = expect_reject_with(result);
    assert_eq!(reject.code, SpokeRejectCode::ReceiptExpired, "{reject:?}");
    assert_eq!(
        reject
            .details
            .as_ref()
            .and_then(|details| details.get("last_known_status")),
        Some(&json!(status)),
        "replay error details: {reject:?}"
    );
    reject
}

const TRANSFER: &str = "prn_transfer";
const TRANSFER_SHARED: &str = "prn_transfer_shared";
const TRANSFER_PEER: &str = "prn_transfer_peer";
const SPACE_TRANSFER: &str = "prn_space_transfer";
const CONTEXT_ELSEWHERE: &str = "ctx_elsewhere";
/// F1 fixture: a legal opaque requester whose id contains the U+001F dedup
/// delimiter, plus a principal that is merely its prefix.
const OPAQUE_REQUESTER: &str = "alice\u{1f}bob";
const PREFIX_IMPOSTOR: &str = "alice";
/// F1 fixture: a principal/key pair that shared one namespace under the old
/// `principal\u{1f}op\u{1f}key` encoding.
const ALIAS_KEY_PRINCIPAL: &str = "alice";
const ALIAS_KEY: &str = "a\u{1f}memory.exports.create\u{1f}b";
const ALIAS_PRINCIPAL: &str = "alice\u{1f}memory.exports.create\u{1f}a";

/// Transfers restricted to one placement context, with and without shared
/// access — the readers PSF-1/2/3 describe. `TRANSFER_PEER` holds the same
/// coverage as `TRANSFER` under a different principal.
fn restricted_transfer_space() -> SqliteMemoryProvider {
    restricted_transfer_on(transfer_space())
}

/// The restricted-transfer grants over any provider instance (in-memory or
/// file-backed).
fn restricted_transfer_on(provider: SqliteMemoryProvider) -> SqliteMemoryProvider {
    provider.provision_grant(
        SPACE,
        grant(
            "grt_transfer_narrow_0003",
            TRANSFER,
            &["transfer"],
            Some(SUBJECT),
            Some(&[CONTEXT_SELECTION]),
            Some(false),
        ),
    );
    provider.provision_grant(
        SPACE,
        grant(
            "grt_transfer_peer_0005",
            TRANSFER_PEER,
            &["transfer"],
            Some(SUBJECT),
            Some(&[CONTEXT_SELECTION]),
            Some(false),
        ),
    );
    provider.provision_grant(
        SPACE,
        grant(
            "grt_transfer_shared_0004",
            TRANSFER_SHARED,
            &["transfer"],
            Some(SUBJECT),
            Some(&[CONTEXT_SELECTION]),
            Some(true),
        ),
    );
    provider
}

/// PSF-1/PSF-2/PSF-3: an omitted selection means the whole requested scope,
/// requested reflections obey their pinned disclosure basis, and retrieval
/// re-checks the stored export coverage under the current principal and grant.
#[tokio::test]
async fn restricted_transfer_only_reaches_its_covered_selection() {
    let provider = restricted_transfer_space();
    seed_scoped_record(
        &provider,
        "rec_ctx_a",
        "The gate key is under the third barrel.",
        json!({ "visibility": "context", "context_id": CONTEXT_SELECTION }),
    );
    seed_scoped_record(
        &provider,
        "rec_ctx_b",
        "A market rumor from elsewhere.",
        json!({ "visibility": "context", "context_id": CONTEXT_ELSEWHERE }),
    );
    seed_scoped_record(
        &provider,
        "rec_shared_only",
        "A shared tally by the gate.",
        json!({ "visibility": "shared" }),
    );
    let _ = expect_ok(
        provider
            .update_profile(
                &principal(OPERATOR),
                serde_json::from_value(profile_update("personality", None, "prof-transfer"))
                    .expect("profile update"),
            )
            .await,
    );
    // Pinned to the whole subject: disclosing this derived body needs
    // unrestricted coverage, so a context-limited export cannot carry it.
    provider.provision_reflection(
        SPACE,
        json!({
            "schema_version": 1,
            "reflection_id": "refl_all_scope",
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": { "mode": "subject_all" },
            "basis": {
                "source_set_digest": "0".repeat(64),
                "record_count": 3,
                "profile_revisions": { "personality": 1 },
                "state_digest": "1".repeat(64),
            },
            "body": { "format": "markdown", "body": "Derived from every placement." },
            "stale": false,
            "generator": { "engine_policy_version": ENGINE_POLICY_VERSION },
            "created_at": "2026-07-23T06:00:00Z",
            "revision": 1,
            "extensions": {},
        }),
    );

    let request = |selection: Option<Value>, include_reflections: bool, key: &str| {
        let mut value = json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "require_quiescent": false,
            "idempotency_key": key,
        });
        if let Some(selection) = selection {
            value["selection"] = selection;
        }
        if include_reflections {
            value["include_reflections"] = json!(true);
        }
        value
    };
    let covered_selection =
        json!({ "mode": "selected_contexts", "context_ids": [CONTEXT_SELECTION] });

    // 1. The covered selection succeeds and carries only its own record.
    let created = expect_ok(
        provider
            .create_export(
                &principal(TRANSFER),
                serde_json::from_value(request(Some(covered_selection.clone()), false, "exp-narrow"))
                    .expect("export request"),
            )
            .await,
    );
    let covered = poll_receipt(
        &provider,
        SPACE,
        wire(&created)["receipt_id"].as_str().expect("receipt id"),
    )
    .await;
    assert_eq!(covered["status"], "committed", "{covered}");
    assert_eq!(covered["result"]["counts"]["records"], 1, "{covered}");
    assert_eq!(covered["result"]["counts"]["profiles"], 0, "{covered}");
    assert_eq!(covered["result"]["counts"]["reflections"], 0, "{covered}");
    let export_id = covered["result"]["export_id"]
        .as_str()
        .expect("export id")
        .to_string();
    // The restricted public payload is what gets inspected: every chunk
    // fetched through `memory.exports.get`, reassembled and digest-verified
    // (TA-1). The support accessor is not evidence of the served bytes.
    let chunk_count = covered["result"]["chunk_count"]
        .as_u64()
        .expect("chunk count");
    let mut public_chunks = Vec::new();
    for ordinal in 0..chunk_count {
        let response = expect_ok(
            provider
                .get_export(
                    &principal(TRANSFER),
                    serde_json::from_value(json!({
                        "space_id": SPACE,
                        "export_id": export_id,
                        "chunk_ordinal": ordinal,
                    }))
                    .expect("export get"),
                )
                .await,
        );
        let chunk = wire(&response)["chunk"].clone();
        assert_eq!(chunk["export_id"], json!(export_id));
        assert_eq!(chunk["chunk_ordinal"], json!(ordinal));
        public_chunks.push(chunk);
    }
    assert_eq!(bundle_record_ids(&public_chunks), vec!["rec_ctx_a"]);
    let verification = verify_reassembly(&public_chunks);
    assert!(
        verification.bundle_digest_ok,
        "{:?}",
        verification.errors
    );
    assert!(verification.errors.is_empty(), "{:?}", verification.errors);
    let text = serde_json::to_string(&public_chunks).expect("chunks serialize");
    assert!(!text.contains("market rumor"), "{text}");
    assert!(!text.contains("shared tally"), "{text}");
    assert!(!text.contains("Guarded, loyal"), "profile leaked: {text}");

    // 2. Omitted selection is the whole subject: a context-limited grant
    //    cannot obtain it (PSF-1).
    assert_eq!(
        expect_reject(
            provider
                .create_export(
                    &principal(TRANSFER),
                    serde_json::from_value(request(None, false, "exp-whole"))
                        .expect("export request"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );
    // 3. `include_shared` is not implied by a context-only grant.
    assert_eq!(
        expect_reject(
            provider
                .create_export(
                    &principal(TRANSFER),
                    serde_json::from_value(request(
                        Some(json!({
                            "mode": "selected_contexts",
                            "context_ids": [CONTEXT_SELECTION],
                            "include_shared": true,
                        })),
                        false,
                        "exp-shared",
                    ))
                    .expect("export request"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );
    // 4. Another context is outside the grant.
    assert_eq!(
        expect_reject(
            provider
                .create_export(
                    &principal(TRANSFER),
                    serde_json::from_value(request(
                        Some(json!({
                            "mode": "selected_contexts",
                            "context_ids": [CONTEXT_ELSEWHERE],
                        })),
                        false,
                        "exp-other",
                    ))
                    .expect("export request"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );
    // 5. A requested reflection pinned to the whole subject denies the whole
    //    request instead of being silently dropped (PSF-2).
    assert_eq!(
        expect_reject(
            provider
                .create_export(
                    &principal(TRANSFER),
                    serde_json::from_value(request(
                        Some(covered_selection.clone()),
                        true,
                        "exp-refl",
                    ))
                    .expect("export request"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );

    // 6. An export created by a fully authorized principal keeps its original
    //    coverage: a narrower principal cannot read it (PSF-3).
    let created = expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(request(
                    Some(json!({
                        "mode": "selected_contexts",
                        "context_ids": [CONTEXT_ELSEWHERE],
                    })),
                    false,
                    "exp-wide",
                ))
                .expect("export request"),
            )
            .await,
    );
    let wide = poll_receipt(
        &provider,
        SPACE,
        wire(&created)["receipt_id"].as_str().expect("receipt id"),
    )
    .await;
    let wide_id = wide["result"]["export_id"]
        .as_str()
        .expect("export id")
        .to_string();
    assert_eq!(
        expect_reject(
            provider
                .get_export(
                    &principal(TRANSFER_SHARED),
                    serde_json::from_value(json!({
                        "space_id": SPACE,
                        "export_id": wide_id,
                        "chunk_ordinal": 0,
                    }))
                    .expect("export get"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );
    let _ = expect_ok(
        provider
            .get_export(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "export_id": wide_id,
                    "chunk_ordinal": 0,
                }))
                .expect("export get"),
            )
            .await,
    );

    // 7. A genuinely different principal with creator-equivalent coverage
    //    reads the same sealed chunk (TA-1); this is not a second creator read.
    let peer_chunk = expect_ok(
        provider
            .get_export(
                &principal(TRANSFER_PEER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "export_id": export_id,
                    "chunk_ordinal": 0,
                }))
                .expect("export get"),
            )
            .await,
    );
    assert_eq!(wire(&peer_chunk)["chunk"], public_chunks[0]);

    // 8. Narrowing the grant is re-checked at retrieval, not just creation.
    let _ = expect_ok(
        provider
            .revoke_grant(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "grant_id": "grt_transfer_narrow_0003",
                    "expected_revision": 1,
                    "idempotency_key": "rv-transfer",
                }))
                .expect("revoke request"),
            )
            .await,
    );
    assert_eq!(
        expect_reject(
            provider
                .get_export(
                    &principal(TRANSFER),
                    serde_json::from_value(json!({
                        "space_id": SPACE,
                        "export_id": export_id,
                        "chunk_ordinal": 0,
                    }))
                    .expect("export get"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );
}

/// TA-2/PSF-1/PSF-3: a space-wide transfer grant limited to one context can
/// neither export the whole space nor keep reading an export after its
/// coverage is narrowed, while the capability itself stays granted.
#[tokio::test]
async fn space_wide_transfer_grants_bound_export_and_retrieval_coverage() {
    let provider = restricted_transfer_space();
    provider.provision_grant(
        SPACE,
        grant(
            "grt_space_transfer_0006",
            SPACE_TRANSFER,
            &["transfer"],
            None,
            Some(&[CONTEXT_SELECTION]),
            Some(false),
        ),
    );
    seed_scoped_record(
        &provider,
        "rec_ctx_a",
        "The gate key is under the third barrel.",
        json!({ "visibility": "context", "context_id": CONTEXT_SELECTION }),
    );
    seed_scoped_record(
        &provider,
        "rec_ctx_b",
        "A market rumor from elsewhere.",
        json!({ "visibility": "context", "context_id": CONTEXT_ELSEWHERE }),
    );

    let request = |subject: Option<&str>, selection: Option<Value>, key: &str| {
        let mut value = json!({ "space_id": SPACE, "idempotency_key": key });
        if let Some(subject) = subject {
            value["subject_id"] = json!(subject);
        }
        if let Some(selection) = selection {
            value["selection"] = selection;
        }
        value
    };

    // 1. An omitted selection is the whole space: a context-limited transfer
    //    grant cannot obtain that export.
    assert_eq!(
        expect_reject(
            provider
                .create_export(
                    &principal(SPACE_TRANSFER),
                    serde_json::from_value(request(None, None, "space-whole"))
                        .expect("export request"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );

    // 2. A covered selected-context export succeeds and carries only its own
    //    record through the restricted principal's read.
    let created = expect_ok(
        provider
            .create_export(
                &principal(SPACE_TRANSFER),
                serde_json::from_value(request(
                    Some(SUBJECT),
                    Some(json!({
                        "mode": "selected_contexts",
                        "context_ids": [CONTEXT_SELECTION],
                    })),
                    "space-covered",
                ))
                .expect("export request"),
            )
            .await,
    );
    let covered = poll_receipt(
        &provider,
        SPACE,
        wire(&created)["receipt_id"].as_str().expect("receipt id"),
    )
    .await;
    assert_eq!(covered["result"]["counts"]["records"], 1, "{covered}");
    let export_id = covered["result"]["export_id"]
        .as_str()
        .expect("export id")
        .to_string();
    let chunk = expect_ok(
        provider
            .get_export(
                &principal(SPACE_TRANSFER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "export_id": export_id,
                    "chunk_ordinal": 0,
                }))
                .expect("export get"),
            )
            .await,
    );
    assert_eq!(
        bundle_record_ids(&[wire(&chunk)["chunk"].clone()]),
        vec!["rec_ctx_a"]
    );

    // 3. Narrow the grant to another context: transfer stays granted, the
    //    stored export coverage no longer matches.
    let _ = expect_ok(
        provider
            .revoke_grant(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "grant_id": "grt_space_transfer_0006",
                    "expected_revision": 1,
                    "idempotency_key": "space-narrow-revoke",
                }))
                .expect("revoke request"),
            )
            .await,
    );
    let _ = expect_ok(
        provider
            .grant(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "principal_id": SPACE_TRANSFER,
                    "capabilities": ["transfer"],
                    "context_ids": [CONTEXT_ELSEWHERE],
                    "shared_access": false,
                    "idempotency_key": "space-narrow-grant",
                }))
                .expect("grant request"),
            )
            .await,
    );
    assert_eq!(
        expect_reject(
            provider
                .get_export(
                    &principal(SPACE_TRANSFER),
                    serde_json::from_value(json!({
                        "space_id": SPACE,
                        "export_id": export_id,
                        "chunk_ordinal": 0,
                    }))
                    .expect("export get"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );

    // 4. The still-granted capability keeps working inside the new coverage.
    let created = expect_ok(
        provider
            .create_export(
                &principal(SPACE_TRANSFER),
                serde_json::from_value(request(
                    Some(SUBJECT),
                    Some(json!({
                        "mode": "selected_contexts",
                        "context_ids": [CONTEXT_ELSEWHERE],
                    })),
                    "space-elsewhere",
                ))
                .expect("export request"),
            )
            .await,
    );
    let elsewhere = poll_receipt(
        &provider,
        SPACE,
        wire(&created)["receipt_id"].as_str().expect("receipt id"),
    )
    .await;
    assert_eq!(elsewhere["result"]["counts"]["records"], 1, "{elsewhere}");
    let elsewhere_id = elsewhere["result"]["export_id"]
        .as_str()
        .expect("export id")
        .to_string();
    let chunk = expect_ok(
        provider
            .get_export(
                &principal(SPACE_TRANSFER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "export_id": elsewhere_id,
                    "chunk_ordinal": 0,
                }))
                .expect("export get"),
            )
            .await,
    );
    assert_eq!(
        bundle_record_ids(&[wire(&chunk)["chunk"].clone()]),
        vec!["rec_ctx_b"]
    );
}

/// ASF-1: `memory.grants.grant` resolves the granter's current admin authority
/// before conflict, replay or expiry, without re-applying the mutation-only
/// issuance expiry bound.
#[tokio::test]
async fn grant_replay_requires_current_operator_authority() {
    let provider = provider();
    let grant_request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "principal_id": READER,
        "capabilities": ["recall"],
        "idempotency_key": "authz-grant",
    });
    let issued = expect_ok(
        provider
            .grant(
                &principal(OPERATOR),
                serde_json::from_value(grant_request.clone()).expect("grant request"),
            )
            .await,
    );
    let issued = wire(&issued);
    let receipt_id = issued["receipt_id"].clone();
    let issued_grant_id = issued["result"]["grant_id"].clone();

    // While the admin coverage holds, the same-key replay is the receipt.
    let replayed = expect_ok(
        provider
            .grant(
                &principal(OPERATOR),
                serde_json::from_value(grant_request.clone()).expect("grant request"),
            )
            .await,
    );
    assert_eq!(wire(&replayed)["receipt_id"], receipt_id);

    // Lose the unrestricted admin grant, then regain only a narrowed one.
    let _ = expect_ok(
        provider
            .revoke_grant(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "grant_id": "grt_operator_admin_0001",
                    "expected_revision": 1,
                    "idempotency_key": "authz-admin-revoke",
                }))
                .expect("revoke request"),
            )
            .await,
    );
    provider.provision_grant(
        SPACE,
        grant(
            "grt_operator_admin_narrowed",
            OPERATOR,
            &["admin"],
            None,
            Some(&[CONTEXT]),
            None,
        ),
    );

    // Within that narrowed coverage a matching issuance and its retained
    // replay still succeed, and the issued grant carries the requested
    // restriction (TSF-2): narrowing must not categorically deny grant
    // management.
    let narrowed_request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "principal_id": READER,
        "capabilities": ["recall"],
        "context_ids": [CONTEXT],
        "idempotency_key": "authz-grant-narrowed",
    });
    let narrowed = expect_ok(
        provider
            .grant(
                &principal(OPERATOR),
                serde_json::from_value(narrowed_request.clone()).expect("grant request"),
            )
            .await,
    );
    let narrowed = wire(&narrowed);
    assert_eq!(narrowed["status"], "committed", "{narrowed}");
    let narrowed_receipt = narrowed["receipt_id"].clone();
    let narrowed_grant_id = narrowed["result"]["grant_id"].clone();
    let retained = expect_ok(
        provider
            .grant(
                &principal(OPERATOR),
                serde_json::from_value(narrowed_request).expect("grant request"),
            )
            .await,
    );
    assert_eq!(
        wire(&retained)["receipt_id"],
        narrowed_receipt,
        "{retained:?}"
    );
    let narrowed_list = expect_ok(
        provider
            .list_grants(
                &principal(OPERATOR),
                serde_json::from_value(json!({ "space_id": SPACE, "limit": 10 }))
                    .expect("list request"),
            )
            .await,
    );
    let narrowed_list = wire(&narrowed_list);
    let narrowed_grant = narrowed_list["grants"]
        .as_array()
        .expect("grants")
        .iter()
        .find(|grant| grant["grant_id"] == narrowed_grant_id)
        .expect("narrowed issuance");
    assert_eq!(
        narrowed_grant["principal_id"],
        json!(READER),
        "{narrowed_grant}"
    );
    assert_eq!(
        narrowed_grant["context_ids"],
        json!([CONTEXT]),
        "{narrowed_grant}"
    );
    assert!(
        narrowed_grant.get("shared_access").is_none(),
        "{narrowed_grant}"
    );
    assert_eq!(narrowed_grant["revision"], 1, "{narrowed_grant}");

    // The stored receipt no longer authorizes a replay.
    assert_eq!(
        expect_reject(
            provider
                .grant(
                    &principal(OPERATOR),
                    serde_json::from_value(grant_request.clone()).expect("grant request"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );
    // A conflicting payload cannot bypass admission either.
    let conflicting = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "principal_id": READER,
        "capabilities": ["recall", "capture"],
        "idempotency_key": "authz-grant",
    });
    assert_eq!(
        expect_reject(
            provider
                .grant(
                    &principal(OPERATOR),
                    serde_json::from_value(conflicting.clone()).expect("grant request"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );

    // Expiry is not reported to an unauthorized principal: authority first.
    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);
    assert_eq!(
        expect_reject(
            provider
                .grant(
                    &principal(OPERATOR),
                    serde_json::from_value(grant_request.clone()).expect("grant request"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );

    // Restoring the authority restores the authorized outcomes: the receipt
    // was evicted, the conflict is a conflict, and no grant ran twice.
    provider.provision_grant(
        SPACE,
        grant(
            "grt_operator_admin_restored",
            OPERATOR,
            &["admin"],
            None,
            None,
            None,
        ),
    );
    expect_receipt_expired(
        provider
            .grant(
                &principal(OPERATOR),
                serde_json::from_value(grant_request).expect("grant request"),
            )
            .await,
    );
    assert_eq!(
        expect_reject(
            provider
                .grant(
                    &principal(OPERATOR),
                    serde_json::from_value(conflicting).expect("grant request"),
                )
                .await
        ),
        SpokeRejectCode::IdempotencyConflict
    );
    let listed = expect_ok(
        provider
            .list_grants(
                &principal(OPERATOR),
                serde_json::from_value(json!({ "space_id": SPACE, "limit": 10 }))
                    .expect("list request"),
            )
            .await,
    );
    let listed = wire(&listed);
    let issued_grant = listed["grants"]
        .as_array()
        .expect("grants")
        .iter()
        .find(|grant| grant["grant_id"] == issued_grant_id)
        .expect("issued grant");
    assert_eq!(issued_grant["revision"], 1, "{issued_grant}");
}

/// ASF-2: an evicted receipt body keeps the status it actually held, and the
/// late worker still settles the owed work without a second effect.
#[tokio::test]
async fn evicted_pending_receipts_keep_their_actual_status() {
    let provider = transfer_space();
    seed_record(
        &provider,
        "rec_1",
        "The gate key is under the third barrel.",
    );
    let _ = expect_ok(
        provider
            .update_profile(
                &principal(OPERATOR),
                serde_json::from_value(profile_update("personality", None, "pending-prof"))
                    .expect("profile update"),
            )
            .await,
    );
    let digest = state_of(&provider).await["state_digest"]
        .as_str()
        .expect("digest")
        .to_string();

    let export_request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "require_quiescent": false,
        "idempotency_key": "pending-export",
    });
    let created = expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(export_request.clone()).expect("export request"),
            )
            .await,
    );
    let created = wire(&created);
    assert_eq!(created["status"], "pending", "{created}");
    let export_receipt = created["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();

    let generated = expect_ok(
        provider
            .generate_reflection(
                &principal(OPERATOR),
                serde_json::from_value(generate_request(&digest, "pending-reflection"))
                    .expect("generate"),
            )
            .await,
    );
    let generated = wire(&generated);
    assert_eq!(generated["status"], "pending", "{generated}");
    let reflection_receipt = generated["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();

    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);

    // The public receipt lookup answers from the same tombstone with the same
    // actual status, before the worker settles the owed work (TSF-1).
    let receipt_request = |receipt_id: &str| {
        serde_json::from_value::<MemoryReceiptsGetRequest>(json!({
            "space_id": SPACE,
            "receipt_id": receipt_id,
        }))
        .expect("receipt request")
    };
    for receipt_id in [&export_receipt, &reflection_receipt] {
        expect_receipt_expired_as(
            provider
                .get_receipt(&principal(OPERATOR), receipt_request(receipt_id))
                .await,
            "pending",
        );
    }

    // Evicted while unfinished: the tombstone reports `pending`, never a
    // committed outcome that has not happened.
    expect_receipt_expired_as(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(export_request.clone()).expect("export request"),
            )
            .await,
        "pending",
    );
    expect_receipt_expired_as(
        provider
            .generate_reflection(
                &principal(OPERATOR),
                serde_json::from_value(generate_request(&digest, "pending-reflection"))
                    .expect("generate"),
            )
            .await,
        "pending",
    );

    // The owed work stays observable after the receipt body is gone.
    let space = space_row(&provider).await;
    assert_eq!(space["pending_receipts"], 2, "{space}");

    // The late worker settles both and updates the tombstones.
    provider.run_worker();
    expect_receipt_expired(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(export_request).expect("export request"),
            )
            .await,
    );
    expect_receipt_expired(
        provider
            .generate_reflection(
                &principal(OPERATOR),
                serde_json::from_value(generate_request(&digest, "pending-reflection"))
                    .expect("generate"),
            )
            .await,
    );
    // Both tombstones now report the settled outcome, through the public
    // receipt lookup as well, and no owed work remains (TSF-1).
    for receipt_id in [&export_receipt, &reflection_receipt] {
        expect_receipt_expired_as(
            provider
                .get_receipt(&principal(OPERATOR), receipt_request(receipt_id))
                .await,
            "committed",
        );
    }
    let space = space_row(&provider).await;
    assert_eq!(space["pending_receipts"], 0, "{space}");

    // Exactly the original effects: one prior export, one published reflection.
    let (next, _chunks) = completed_export(
        &provider,
        json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "idempotency_key": "pending-export-next",
        }),
    )
    .await;
    assert_eq!(next["result"]["export_id"], "exp_sqlite_0002");
    let listed = expect_ok(
        provider
            .list_reflections(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "limit": 10,
                }))
                .expect("list request"),
            )
            .await,
    );
    let listed = wire(&listed);
    assert_eq!(
        listed["reflections"].as_array().expect("reflections").len(),
        1,
        "{listed}"
    );
}

/// SR-1: an evicted receipt's retained status is disclosed only to its
/// requester — while that principal still holds current authority — or to a
/// current space admin; an unrelated or revoked principal learns nothing.
#[tokio::test]
async fn evicted_receipt_status_requires_current_receipt_authority() {
    let provider = restricted_transfer_space();
    seed_scoped_record(
        &provider,
        "rec_ctx_a",
        "The gate key is under the third barrel.",
        json!({ "visibility": "context", "context_id": CONTEXT_SELECTION }),
    );
    let request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "selection": { "mode": "selected_contexts", "context_ids": [CONTEXT_SELECTION] },
        "idempotency_key": "receipt-authority-export",
    });
    let created = expect_ok(
        provider
            .create_export(
                &principal(TRANSFER),
                serde_json::from_value(request).expect("export request"),
            )
            .await,
    );
    let created = wire(&created);
    assert_eq!(created["status"], "pending", "{created}");
    let receipt_id = created["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();
    let receipt_request = |receipt_id: &str| {
        serde_json::from_value::<MemoryReceiptsGetRequest>(json!({
            "space_id": SPACE,
            "receipt_id": receipt_id,
        }))
        .expect("receipt request")
    };

    // Body present: the requester and a space admin read it; a principal that
    // merely knows the id is denied.
    let own = expect_ok(
        provider
            .get_receipt(&principal(TRANSFER), receipt_request(&receipt_id))
            .await,
    );
    assert_eq!(wire(&own)["receipt"]["status"], "pending");

    // The negative assertions use this traversal: it must see the real receipt
    // body and a nested retained status, and ignore a harmless diagnostic that
    // merely contains "status".
    assert_eq!(
        receipt_detail_paths(wire(&own).as_object()),
        vec!["details.receipt".to_string()]
    );
    let nested = json!({
        "request_status_check": "ok",
        "result": { "last_known_status": "committed" },
    });
    assert_eq!(
        receipt_detail_paths(nested.as_object()),
        vec!["details.result.last_known_status".to_string()]
    );
    let admin = expect_ok(
        provider
            .get_receipt(&principal(OPERATOR), receipt_request(&receipt_id))
            .await,
    );
    assert_eq!(wire(&admin)["receipt"]["status"], "pending");
    assert_eq!(
        expect_reject(
            provider
                .get_receipt(&principal(TRANSFER_PEER), receipt_request(&receipt_id))
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );

    // Body evicted: the same authority rules gate the retained status.
    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);
    expect_receipt_expired_as(
        provider
            .get_receipt(&principal(TRANSFER), receipt_request(&receipt_id))
            .await,
        "pending",
    );
    expect_receipt_expired_as(
        provider
            .get_receipt(&principal(OPERATOR), receipt_request(&receipt_id))
            .await,
        "pending",
    );
    assert_eq!(
        expect_reject(
            provider
                .get_receipt(&principal(TRANSFER_PEER), receipt_request(&receipt_id))
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );

    // A revoked requester must not read receipts (MEM-R018).
    let _ = expect_ok(
        provider
            .revoke_grant(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "grant_id": "grt_transfer_narrow_0003",
                    "expected_revision": 1,
                    "idempotency_key": "receipt-authority-revoke",
                }))
                .expect("revoke request"),
            )
            .await,
    );
    assert_eq!(
        expect_reject(
            provider
                .get_receipt(&principal(TRANSFER), receipt_request(&receipt_id))
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );

    // The worker settles the owed work; the admin observes the real outcome.
    provider.run_worker();
    expect_receipt_expired_as(
        provider
            .get_receipt(&principal(OPERATOR), receipt_request(&receipt_id))
            .await,
        "committed",
    );

    // Restoring current authority restores the requester's read.
    provider.provision_grant(
        SPACE,
        grant(
            "grt_transfer_restored_0007",
            TRANSFER,
            &["transfer"],
            Some(SUBJECT),
            Some(&[CONTEXT_SELECTION]),
            Some(false),
        ),
    );
    expect_receipt_expired_as(
        provider
            .get_receipt(&principal(TRANSFER), receipt_request(&receipt_id))
            .await,
        "committed",
    );
}

/// SR-2: lifecycle evidence whose retained status was cleared must never make
/// work that is still owed look committed.
#[tokio::test]
async fn legacy_dedup_status_never_invents_a_commit() {
    let provider = transfer_space();
    seed_record(&provider, "rec_1", "The gate key is under the third barrel.");
    let request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "idempotency_key": "legacy-status-export",
    });
    let created = expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(request.clone()).expect("export request"),
            )
            .await,
    );
    let created = wire(&created);
    assert_eq!(created["status"], "pending", "{created}");
    let receipt_id = created["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();

    // Evict the stale body first: eviction retains the real `pending` status,
    // which is then cleared so only the still-owed work can explain the
    // receipt.
    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);
    expect_ok(
        provider
            .list_grants(
                &principal(OPERATOR),
                serde_json::from_value(json!({ "space_id": SPACE, "limit": 10 }))
                    .expect("list request"),
            )
            .await,
    );
    provider.forget_receipt_evidence_statuses();

    // The effect is still owed, so neither the replay nor the public receipt
    // lookup may claim it committed.
    expect_receipt_expired_as(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(request.clone()).expect("export request"),
            )
            .await,
        "pending",
    );
    expect_receipt_expired_as(
        provider
            .get_receipt(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryReceiptsGetRequest>(json!({
                    "space_id": SPACE,
                    "receipt_id": receipt_id,
                }))
                .expect("receipt request"),
            )
            .await,
        "pending",
    );

    // Work written by the current code still reports its own status.
    let current = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "idempotency_key": "legacy-status-current",
    });
    let fresh = expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(current.clone()).expect("export request"),
            )
            .await,
    );
    let fresh = wire(&fresh);
    assert_eq!(fresh["status"], "pending", "{fresh}");
    let replayed = expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(current).expect("export request"),
            )
            .await,
    );
    assert_eq!(wire(&replayed)["receipt_id"], fresh["receipt_id"]);

    // The late worker settles the legacy work; the tombstone now says so.
    provider.run_worker();
    expect_receipt_expired_as(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(request).expect("export request"),
            )
            .await,
        "committed",
    );
}

/// Read one receipt through the typed public port.
async fn read_receipt(
    provider: &SqliteMemoryProvider,
    reader: &str,
    receipt_id: &str,
) -> SpokeResult<MemoryReceiptsGetResponse> {
    provider
        .get_receipt(
            &principal(reader),
            serde_json::from_value::<MemoryReceiptsGetRequest>(json!({
                "space_id": SPACE,
                "receipt_id": receipt_id,
            }))
            .expect("receipt request"),
        )
        .await
}

/// F1: principal ids are opaque and may legally contain U+001F. The real
/// requester keeps its receipt and its retained status; a principal that is
/// merely a prefix of that id is a stranger, not the requester.
#[tokio::test]
async fn opaque_requester_delimiter_never_matches_a_prefix_impostor() {
    let provider = transfer_space();
    seed_record(
        &provider,
        "rec_1",
        "The gate key is under the third barrel.",
    );
    for (grant_id, grantee) in [
        ("grt_opaque_requester_0020", OPAQUE_REQUESTER),
        ("grt_prefix_impostor_0021", PREFIX_IMPOSTOR),
    ] {
        provider.provision_grant(
            SPACE,
            grant(grant_id, grantee, &["transfer"], None, None, None),
        );
    }
    let created = expect_ok(
        provider
            .create_export(
                &principal(OPAQUE_REQUESTER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "idempotency_key": "opaque-requester-export",
                }))
                .expect("export request"),
            )
            .await,
    );
    let receipt_id = wire(&created)["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();
    provider.run_worker();

    // The body names the full opaque requester, so the impostor learns nothing.
    let own = expect_ok(read_receipt(&provider, OPAQUE_REQUESTER, &receipt_id).await);
    assert_eq!(
        wire(&own)["receipt"]["requester_principal_id"],
        json!(OPAQUE_REQUESTER)
    );
    let denial = expect_reject_with(read_receipt(&provider, PREFIX_IMPOSTOR, &receipt_id).await);
    assert_eq!(denial.code, SpokeRejectCode::NotAuthorized, "{denial:?}");
    assert!(
        receipt_detail_paths(denial.details.as_ref()).is_empty(),
        "denial leaked receipt/status detail: {denial:?}"
    );

    // The tombstone keeps the same identity: the requester and the space admin
    // still read the retained status; the prefix impostor still does not.
    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);
    expect_receipt_expired_as(
        read_receipt(&provider, OPAQUE_REQUESTER, &receipt_id).await,
        "committed",
    );
    expect_receipt_expired_as(
        read_receipt(&provider, OPERATOR, &receipt_id).await,
        "committed",
    );
    let denial = expect_reject_with(read_receipt(&provider, PREFIX_IMPOSTOR, &receipt_id).await);
    assert_eq!(denial.code, SpokeRejectCode::NotAuthorized, "{denial:?}");
    assert!(
        receipt_detail_paths(denial.details.as_ref()).is_empty(),
        "denial leaked receipt/status detail: {denial:?}"
    );
}

/// F1: the dedup namespace must encode `(principal, op, key)` injectively. A
/// key that itself carries U+001F must not alias another principal's request:
/// each principal keeps its own receipt, replay and retained status.
#[tokio::test]
async fn opaque_namespace_never_aliases_another_principals_request() {
    let provider = transfer_space();
    seed_record(
        &provider,
        "rec_1",
        "The gate key is under the third barrel.",
    );
    for (grant_id, grantee) in [
        ("grt_alias_key_0022", ALIAS_KEY_PRINCIPAL),
        ("grt_alias_principal_0023", ALIAS_PRINCIPAL),
    ] {
        provider.provision_grant(
            SPACE,
            grant(grant_id, grantee, &["transfer"], None, None, None),
        );
    }
    let export = |key: &str| {
        serde_json::from_value::<MemoryExportsCreateRequest>(json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "idempotency_key": key,
        }))
        .expect("export request")
    };
    let keyed = wire(&expect_ok(
        provider
            .create_export(&principal(ALIAS_KEY_PRINCIPAL), export(ALIAS_KEY))
            .await,
    ));
    // Same payload except the key: the old `principal\u{1f}op\u{1f}key`
    // namespace made this principal collide with the one above.
    let aliased = wire(&expect_ok(
        provider
            .create_export(&principal(ALIAS_PRINCIPAL), export("b"))
            .await,
    ));
    let keyed_receipt = keyed["receipt_id"].as_str().expect("receipt id").to_string();
    let aliased_receipt = aliased["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();
    assert_ne!(keyed_receipt, aliased_receipt);
    assert_eq!(keyed["status"], "pending", "{keyed}");
    assert_eq!(aliased["status"], "pending", "{aliased}");
    assert_eq!(keyed["requester_principal_id"], json!(ALIAS_KEY_PRINCIPAL));
    assert_eq!(aliased["requester_principal_id"], json!(ALIAS_PRINCIPAL));

    // Each principal replays its own key to its own receipt.
    let replayed = wire(&expect_ok(
        provider
            .create_export(&principal(ALIAS_PRINCIPAL), export("b"))
            .await,
    ));
    assert_eq!(replayed["receipt_id"], json!(aliased_receipt));
    let replay_keyed = wire(&expect_ok(
        provider
            .create_export(&principal(ALIAS_KEY_PRINCIPAL), export(ALIAS_KEY))
            .await,
    ));
    assert_eq!(replay_keyed["receipt_id"], json!(keyed_receipt));
    // A genuine same-namespace conflict still answers as one.
    let conflicting: MemoryExportsCreateRequest = serde_json::from_value(json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "include_reflections": true,
        "idempotency_key": ALIAS_KEY,
    }))
    .expect("export request");
    assert_eq!(
        expect_reject(
            provider
                .create_export(&principal(ALIAS_KEY_PRINCIPAL), conflicting)
                .await
        ),
        SpokeRejectCode::IdempotencyConflict
    );

    provider.run_worker();
    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);
    // Retained statuses stay with their own requester.
    expect_receipt_expired_as(
        read_receipt(&provider, ALIAS_KEY_PRINCIPAL, &keyed_receipt).await,
        "committed",
    );
    expect_receipt_expired_as(
        read_receipt(&provider, ALIAS_PRINCIPAL, &aliased_receipt).await,
        "committed",
    );
    let foreign =
        expect_reject_with(read_receipt(&provider, ALIAS_PRINCIPAL, &keyed_receipt).await);
    assert_eq!(foreign.code, SpokeRejectCode::NotAuthorized, "{foreign:?}");
    assert!(
        receipt_detail_paths(foreign.details.as_ref()).is_empty(),
        "denial leaked receipt/status detail: {foreign:?}"
    );
    let foreign =
        expect_reject_with(read_receipt(&provider, ALIAS_KEY_PRINCIPAL, &aliased_receipt).await);
    assert_eq!(foreign.code, SpokeRejectCode::NotAuthorized, "{foreign:?}");
    assert!(
        receipt_detail_paths(foreign.details.as_ref()).is_empty(),
        "denial leaked receipt/status detail: {foreign:?}"
    );
}

/// F2 control: owed work proves `pending` even without a retained status, and
/// the worker's settlement then records the proven `committed` outcome.
#[tokio::test]
async fn evidenced_pending_and_settled_statuses_survive_eviction() {
    let provider = transfer_space();
    seed_record(
        &provider,
        "rec_1",
        "The gate key is under the third barrel.",
    );
    let request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "idempotency_key": "f2-evidenced",
    });
    let created = wire(&expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(request.clone()).expect("export request"),
            )
            .await,
    ));
    let receipt_id = created["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();
    assert_eq!(created["status"], "pending", "{created}");

    provider.forget_receipt_evidence_statuses();
    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);
    expect_receipt_expired_as(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(request.clone()).expect("export request"),
            )
            .await,
        "pending",
    );
    expect_receipt_expired_as(
        read_receipt(&provider, OPERATOR, &receipt_id).await,
        "pending",
    );

    // Settlement is the affirmative completion evidence.
    provider.run_worker();
    expect_receipt_expired_as(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(request).expect("export request"),
            )
            .await,
        "committed",
    );
    expect_receipt_expired_as(
        read_receipt(&provider, OPERATOR, &receipt_id).await,
        "committed",
    );
}

/// F2: with no retained outcome status and no owed work there is no evidence
/// of the outcome; the provider fails closed instead of guessing `committed`.
/// The proof compares the whole durable space (effects and id sequences
/// included): a read is `INTERNAL_ERROR` / `retryable`, a mutating replay is
/// `OUTCOME_UNCERTAIN` / `uncertain`, current authorization and the changed
/// digest conflict keep their precedence, and an overtaken CAS or fence is
/// never re-evaluated over lost evidence.
#[tokio::test]
async fn absent_outcome_evidence_fails_closed() {
    let path = temp_state_path("absent-outcome");
    let provider = restricted_transfer_on(seeded(
        SqliteMemoryProvider::open(&path).expect("fresh store initializes"),
    ));
    seed_scoped_record(
        &provider,
        "rec_ctx_a",
        "The gate key is under the third barrel.",
        json!({ "visibility": "context", "context_id": CONTEXT_SELECTION }),
    );
    let export_request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "selection": { "mode": "selected_contexts", "context_ids": [CONTEXT_SELECTION] },
        "idempotency_key": "f2-unproven",
    });
    let created = wire(&expect_ok(
        provider
            .create_export(
                &principal(TRANSFER),
                serde_json::from_value(export_request.clone()).expect("export request"),
            )
            .await,
    ));
    let receipt_id = created["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();
    let update_request = json!({
        "space_id": SPACE,
        "record_id": "rec_ctx_a",
        "expected_revision": 1,
        "content": { "text": "A tally the CAS guard already moved past." },
        "idempotency_key": "f2-cas",
    });
    let update_receipt = wire(&expect_ok(
        provider
            .update_record(
                &principal(OPERATOR),
                serde_json::from_value(update_request.clone()).expect("update request"),
            )
            .await,
    ))["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();
    let fence_request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "target": { "type": "context", "context_id": CONTEXT_SELECTION },
        "action": "fence",
        "expected_revision": 1,
        "idempotency_key": "f2-fence",
    });
    let fence_receipt = wire(&expect_ok(
        provider
            .fence_subject(
                &principal(OPERATOR),
                serde_json::from_value(fence_request.clone()).expect("fence request"),
            )
            .await,
    ))["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();
    provider.run_worker();
    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);
    // The bodies are evicted on the next public calls, keeping the status they
    // actually held: ordinary valid evidence records.
    expect_receipt_expired_as(
        read_receipt(&provider, TRANSFER, &receipt_id).await,
        "committed",
    );
    for receipt in [&update_receipt, &fence_receipt] {
        expect_receipt_expired_as(
            read_receipt(&provider, OPERATOR, receipt.as_str()).await,
            "committed",
        );
    }
    let space = space_row(&provider).await;
    assert_eq!(space["pending_receipts"], 0, "{space}");

    // Exact provenance: a valid committed state whose only change is the
    // deliberately cleared outcome status, never a guessed commit.
    let valid_document: Value =
        serde_json::from_str(&stored_document(&path)).expect("stored document is json");
    for receipt in [&receipt_id, &update_receipt, &fence_receipt] {
        assert_eq!(
            valid_document["spaces"][SPACE]["receipt_evidence"]
                [receipt.as_str()]["status"],
            "committed",
            "{receipt}"
        );
    }
    provider.forget_receipt_evidence_statuses();
    let corrupt_bytes = stored_document(&path);
    let mut expected = valid_document.clone();
    for receipt in [&receipt_id, &update_receipt, &fence_receipt] {
        expected["spaces"][SPACE]["receipt_evidence"]
            [receipt.as_str()]
            .as_object_mut()
            .expect("evidence object")
            .remove("status");
    }
    let corrupt_document: Value = serde_json::from_str(&corrupt_bytes).expect("corrupt state");
    assert_eq!(corrupt_document, expected);

    // A read committed no effect, so the existing retryable
    // `INTERNAL_ERROR` is truthful — and no status is invented.
    let unproven = expect_reject_with(read_receipt(&provider, TRANSFER, &receipt_id).await);
    assert_eq!(
        unproven.code,
        SpokeRejectCode::InternalError,
        "{unproven:?}"
    );
    assert_eq!(
        unproven
            .details
            .as_ref()
            .and_then(|details| details.get("retry_class")),
        Some(&json!("retryable")),
        "{unproven:?}"
    );
    assert!(
        receipt_detail_paths(unproven.details.as_ref()).is_empty(),
        "unproven outcome leaked receipt/status detail: {unproven:?}"
    );

    // Current authorization precedes any disclosure: a differently covered
    // principal and an unknown one learn nothing, even knowing the id.
    for reader in [TRANSFER_PEER, "prn_stranger"] {
        let denial = expect_reject_with(read_receipt(&provider, reader, &receipt_id).await);
        assert_eq!(denial.code, SpokeRejectCode::NotAuthorized, "{denial:?}");
        assert!(
            receipt_detail_paths(denial.details.as_ref()).is_empty(),
            "denial leaked receipt/status detail: {denial:?}"
        );
    }

    // Revoke the ORIGINAL requester's actual grant: being the stored owner and
    // knowing the id is still not authority. The same owner's typed read and
    // matching mutating replay are both `NOT_AUTHORIZED` with no status/body
    // disclosure, and neither writes durable state, effects or sequences.
    expect_ok(
        provider
            .revoke_grant(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryGrantsRevokeRequest>(json!({
                    "space_id": SPACE,
                    "grant_id": "grt_transfer_narrow_0003",
                    "expected_revision": 1,
                    "idempotency_key": "f2-revoke-transfer",
                }))
                .expect("revoke request"),
            )
            .await,
    );
    let revoked_bytes = stored_document(&path);
    let revoked_document: Value =
        serde_json::from_str(&revoked_bytes).expect("revoked state is json");
    let revoked_baseline = revoked_document["spaces"][SPACE].clone();
    for (label, denial) in [
        (
            "read",
            expect_reject_with(read_receipt(&provider, TRANSFER, &receipt_id).await),
        ),
        (
            "replay",
            expect_reject_with(
                provider
                    .create_export(
                        &principal(TRANSFER),
                        serde_json::from_value(export_request.clone()).expect("export request"),
                    )
                    .await,
            ),
        ),
    ] {
        assert_eq!(
            denial.code,
            SpokeRejectCode::NotAuthorized,
            "revoked owner {label}: {denial:?}"
        );
        assert!(
            receipt_detail_paths(denial.details.as_ref()).is_empty(),
            "revoked owner {label} leaked receipt/status detail: {denial:?}"
        );
    }
    assert_eq!(
        stored_document(&path),
        revoked_bytes,
        "revoked-owner controls wrote state"
    );
    let denied_state: Value =
        serde_json::from_str(&stored_document(&path)).expect("denied state is json");
    assert_eq!(
        denied_state["spaces"][SPACE], revoked_baseline,
        "revoked-owner controls changed durable state"
    );

    // Restore valid current authority for the original requester; the missing
    // outcome evidence then keeps the read/mutation split.
    provider.provision_grant(
        SPACE,
        grant(
            "grt_transfer_restored_0099",
            TRANSFER,
            &["transfer"],
            Some(SUBJECT),
            Some(&[CONTEXT_SELECTION]),
            Some(false),
        ),
    );
    let restored_bytes = stored_document(&path);
    let restored_document: Value =
        serde_json::from_str(&restored_bytes).expect("restored state is json");
    let restored_baseline = restored_document["spaces"][SPACE].clone();

    // Restored authority sees the same missing evidence through the read
    // channel: retryable and detail-free, not `RECEIPT_EXPIRED`.
    let unproven = expect_reject_with(read_receipt(&provider, TRANSFER, &receipt_id).await);
    assert_eq!(
        unproven.code,
        SpokeRejectCode::InternalError,
        "{unproven:?}"
    );
    assert_eq!(
        unproven
            .details
            .as_ref()
            .and_then(|details| details.get("retry_class")),
        Some(&json!("retryable")),
        "{unproven:?}"
    );
    assert!(
        receipt_detail_paths(unproven.details.as_ref()).is_empty(),
        "unproven outcome leaked receipt/status detail: {unproven:?}"
    );

    // The same-key replay may follow a mutation that did commit, so it is
    // `uncertain` — never a clean no-effect claim and never a re-execution.
    let unproven = expect_reject_with(
        provider
            .create_export(
                &principal(TRANSFER),
                serde_json::from_value(export_request.clone()).expect("export request"),
            )
            .await,
    );
    assert_eq!(
        unproven.code,
        SpokeRejectCode::OutcomeUncertain,
        "{unproven:?}"
    );
    assert_eq!(
        unproven
            .details
            .as_ref()
            .and_then(|details| details.get("retry_class")),
        Some(&json!("uncertain")),
        "{unproven:?}"
    );
    assert!(
        receipt_detail_paths(unproven.details.as_ref()).is_empty(),
        "unproven outcome leaked receipt/status detail: {unproven:?}"
    );

    // A changed payload reusing the key is still the dedup conflict: the
    // digest check precedes the missing outcome evidence, and nothing runs.
    let mut changed = export_request.clone();
    changed["include_reflections"] = json!(true);
    assert_eq!(
        expect_reject(
            provider
                .create_export(
                    &principal(TRANSFER),
                    serde_json::from_value(changed).expect("export request"),
                )
                .await
        ),
        SpokeRejectCode::IdempotencyConflict
    );

    // The update's CAS and the fence's revision were overtaken by their own
    // commit: a naive re-execution would fail with a revision conflict. The
    // lost evidence is answered first (`uncertain`) and no effect runs.
    for (label, reject) in [
        (
            "cas",
            expect_reject_with(
                provider
                    .update_record(
                        &principal(OPERATOR),
                        serde_json::from_value(update_request.clone()).expect("update request"),
                    )
                    .await,
            ),
        ),
        (
            "fence",
            expect_reject_with(
                provider
                    .fence_subject(
                        &principal(OPERATOR),
                        serde_json::from_value(fence_request.clone()).expect("fence request"),
                    )
                    .await,
            ),
        ),
    ] {
        assert_eq!(
            reject.code,
            SpokeRejectCode::OutcomeUncertain,
            "{label}: {reject:?}"
        );
        assert_eq!(
            reject
                .details
                .as_ref()
                .and_then(|details| details.get("retry_class")),
            Some(&json!("uncertain")),
            "{label}: {reject:?}"
        );
        assert!(
            receipt_detail_paths(reject.details.as_ref()).is_empty(),
            "{label}: {reject:?}"
        );
    }

    // Full durable/effect/sequence comparison against the restored-authority
    // state: no receipt, effect, owed work or id counter moved, and the stored
    // bytes are untouched.
    let after: Value = serde_json::from_str(&stored_document(&path)).expect("stored document");
    assert_eq!(stored_document(&path), restored_bytes, "guards wrote state");
    assert_eq!(
        after["spaces"][SPACE], restored_baseline,
        "guards changed state"
    );
    assert_eq!(
        after["spaces"][SPACE]["receipt_sequence"],
        restored_baseline["receipt_sequence"]
    );
    assert_eq!(
        after["spaces"][SPACE]["record_sequence"],
        restored_baseline["record_sequence"]
    );
    assert_eq!(
        after["spaces"][SPACE]["records"]["rec_ctx_a"]["revision"],
        2,
        "{after}"
    );
    assert_eq!(
        after["spaces"][SPACE]["subjects"][SUBJECT]["revision"],
        2,
        "{after}"
    );
    assert_eq!(
        after["spaces"][SPACE]["subjects"][SUBJECT]["fenced_contexts"],
        json!([CONTEXT_SELECTION]),
        "{after}"
    );
    assert_eq!(after["spaces"][SPACE]["pending"], json!({}), "{after}");
    for receipt in [&receipt_id, &update_receipt, &fence_receipt] {
        assert!(
            after["spaces"][SPACE]["receipts"]
                .as_object()
                .expect("receipts object")
                .get(receipt.as_str())
                .is_none(),
            "{receipt}: no re-executed body may appear"
        );
    }

    // Ordinary current state is untouched: new work keeps its own evidence.
    let current = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "idempotency_key": "f2-current",
    });
    let fresh = wire(&expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(current.clone()).expect("export request"),
            )
            .await,
    ));
    assert_eq!(fresh["status"], "pending", "{fresh}");
    let replayed = wire(&expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(current.clone()).expect("export request"),
            )
            .await,
    ));
    assert_eq!(replayed["receipt_id"], fresh["receipt_id"]);
    provider.run_worker();
    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);
    expect_receipt_expired_as(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(current).expect("export request"),
            )
            .await,
        "committed",
    );

    std::fs::remove_file(&path).expect("remove state file");
}

/// Critical (checked identity): a stale body whose own status is absent and
/// whose individual lifecycle entry was removed cannot fall back to owed work
/// alone. Both public eviction and worker settlement must refuse before any
/// write, keeping the body, the work and every byte; restoring the real
/// evidence then lets the same work settle truthfully without a new id.
#[tokio::test]
async fn pending_body_without_identity_evidence_fails_closed_until_restored() {
    let path = temp_state_path("pending-missing-evidence");
    let provider = seeded(SqliteMemoryProvider::open(&path).expect("fresh store initializes"));
    seed_record(
        &provider,
        "rec_missing_evidence",
        "The tally waits for the bearer of the missing identity.",
    );
    let request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "idempotency_key": "missing-identity",
    });
    let created = wire(&expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(request.clone()).expect("export request"),
            )
            .await,
    ));
    assert_eq!(created["status"], "pending", "{created}");
    let receipt_id = created["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();
    // The real body is the valid state restored below.
    let valid_body = created.clone();
    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);

    // Deliberate private corruption of exactly what the guards must check: the
    // body's own status (so eviction would fall back to owed work) and the
    // receipt's individual identity entry (the map itself stays present).
    let valid_document: Value =
        serde_json::from_str(&stored_document(&path)).expect("stored document is json");
    assert_eq!(
        valid_document["spaces"][SPACE]["receipts"][&receipt_id]["status"],
        "pending",
        "{valid_document}"
    );
    assert!(
        valid_document["spaces"][SPACE]["receipt_evidence"]
            .as_object()
            .expect("evidence map")
            .contains_key(&receipt_id),
        "{valid_document}"
    );
    let mut corrupt = valid_document;
    corrupt["spaces"][SPACE]["receipts"][&receipt_id]
        .as_object_mut()
        .expect("receipt body")
        .remove("status");
    corrupt["spaces"][SPACE]["receipt_evidence"]
        .as_object_mut()
        .expect("evidence map")
        .remove(&receipt_id);
    let corrupt_bytes = serde_json::to_string(&corrupt).expect("corrupt state serializes");
    write_stored_document(&path, &corrupt_bytes);
    let sequence = corrupt["spaces"][SPACE]["receipt_sequence"].clone();
    let record_sequence = corrupt["spaces"][SPACE]["record_sequence"].clone();

    // Public read triggers checked eviction: refuse with no write.
    let eviction = expect_reject_with(read_receipt(&provider, OPERATOR, &receipt_id).await);
    assert_eq!(eviction.code, SpokeRejectCode::InternalError, "{eviction:?}");
    assert_eq!(
        eviction
            .details
            .as_ref()
            .and_then(|details| details.get("retry_class")),
        Some(&json!("retryable")),
        "{eviction:?}"
    );
    assert!(
        receipt_detail_paths(eviction.details.as_ref()).is_empty(),
        "{eviction:?}"
    );
    assert_eq!(stored_document(&path), corrupt_bytes, "eviction wrote state");

    // The worker cannot consume owed work whose identity record is gone.
    let refusal = expect_reject_with(provider.try_run_worker());
    assert_eq!(refusal.code, SpokeRejectCode::InternalError, "{refusal:?}");
    assert_eq!(
        refusal
            .details
            .as_ref()
            .and_then(|details| details.get("retry_class")),
        Some(&json!("retryable")),
        "{refusal:?}"
    );
    assert_eq!(stored_document(&path), corrupt_bytes, "worker wrote state");

    // Body, owed work and both id sequences survived intact.
    let kept: Value = serde_json::from_str(&corrupt_bytes).expect("kept state");
    assert_eq!(
        kept["spaces"][SPACE]["pending"]
            .as_object()
            .expect("pending map")
            .len(),
        1,
        "{kept}"
    );
    assert!(
        kept["spaces"][SPACE]["receipts"][&receipt_id].is_object(),
        "the stale body must survive: {kept}"
    );
    assert_eq!(kept["spaces"][SPACE]["receipt_sequence"], sequence);
    assert_eq!(kept["spaces"][SPACE]["record_sequence"], record_sequence);

    // Valid control: restoring the real body identity/clock lets the same owed
    // work settle; only then does the evicted receipt report `committed`.
    provider.provision_receipt(SPACE, valid_body);
    expect_ok(provider.try_run_worker());
    expect_receipt_expired_as(
        read_receipt(&provider, OPERATOR, &receipt_id).await,
        "committed",
    );
    let after: Value = serde_json::from_str(&stored_document(&path)).expect("settled state");
    assert!(
        after["spaces"][SPACE]["receipts"]
            .as_object()
            .expect("receipts object")
            .get(&receipt_id)
            .is_none(),
        "{after}"
    );
    assert_eq!(after["spaces"][SPACE]["pending"], json!({}), "{after}");
    assert_eq!(
        after["spaces"][SPACE]["receipt_evidence"][&receipt_id]["status"],
        "committed",
        "{after}"
    );
    assert_eq!(after["spaces"][SPACE]["receipt_sequence"], sequence);
    assert_eq!(after["spaces"][SPACE]["record_sequence"], record_sequence);

    std::fs::remove_file(&path).expect("remove state file");
}

/// F3: `include_tombstoned` is an admin-only part of the CURRENT request's
/// authority. It is checked before any replay/conflict answer, so a
/// transfer-only caller cannot flip it and a revoked admin cannot replay its
/// own export.
#[tokio::test]
async fn export_include_tombstoned_requires_current_admin_authority() {
    let provider = restricted_transfer_space();
    seed_scoped_record(
        &provider,
        "rec_ctx_a",
        "The gate key is under the third barrel.",
        json!({ "visibility": "context", "context_id": CONTEXT_SELECTION }),
    );
    let export = |key: &str, tombstones: bool| {
        let mut value = json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": { "mode": "selected_contexts", "context_ids": [CONTEXT_SELECTION] },
            "idempotency_key": key,
        });
        if tombstones {
            value["include_tombstoned"] = json!(true);
        }
        serde_json::from_value::<MemoryExportsCreateRequest>(value).expect("export request")
    };

    // A transfer-only caller's covered export is admitted without tombstones.
    let first = wire(&expect_ok(
        provider
            .create_export(&principal(TRANSFER), export("f3-tombstone-flag", false))
            .await,
    ));
    assert_eq!(first["status"], "pending", "{first}");
    provider.run_worker();

    // Reusing the key with the admin-only bit is denied before the conflict.
    assert_eq!(
        expect_reject(
            provider
                .create_export(&principal(TRANSFER), export("f3-tombstone-flag", true))
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );

    // A current admin keeps the positive path.
    let admin = wire(&expect_ok(
        provider
            .create_export(&principal(OPERATOR), export("f3-admin-tombstone", true))
            .await,
    ));
    let admin_receipt = admin["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();
    provider.run_worker();
    let replayed = wire(&expect_ok(
        provider
            .create_export(&principal(OPERATOR), export("f3-admin-tombstone", true))
            .await,
    ));
    assert_eq!(replayed["receipt_id"], json!(admin_receipt));
    assert_eq!(replayed["status"], "committed", "{replayed}");

    // Transfer retained, admin revoked: the identical replay is denied, and a
    // changed payload is denied rather than answered with a conflict.
    provider.provision_grant(
        SPACE,
        grant("grt_operator_transfer_0024", OPERATOR, &["transfer"], None, None, None),
    );
    expect_ok(
        provider
            .revoke_grant(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "grant_id": "grt_operator_admin_0001",
                    "expected_revision": 1,
                    "idempotency_key": "f3-admin-revoked",
                }))
                .expect("revoke request"),
            )
            .await,
    );
    assert_eq!(
        expect_reject(
            provider
                .create_export(&principal(OPERATOR), export("f3-admin-tombstone", true))
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );
    let changed: MemoryExportsCreateRequest = serde_json::from_value(json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "selection": { "mode": "selected_contexts", "context_ids": [CONTEXT_SELECTION] },
        "include_reflections": true,
        "include_tombstoned": true,
        "idempotency_key": "f3-admin-tombstone",
    }))
    .expect("export request");
    assert_eq!(
        expect_reject(
            provider
                .create_export(&principal(OPERATOR), changed)
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );

    // Authority precedes the expired-status answer; restoring admin restores
    // the stored-coverage replay.
    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);
    let expired = expect_reject_with(
        provider
            .create_export(&principal(OPERATOR), export("f3-admin-tombstone", true))
            .await,
    );
    assert_eq!(expired.code, SpokeRejectCode::NotAuthorized, "{expired:?}");
    assert!(
        receipt_detail_paths(expired.details.as_ref()).is_empty(),
        "denial leaked receipt/status detail: {expired:?}"
    );
    provider.provision_grant(
        SPACE,
        grant("grt_operator_admin_restored_0025", OPERATOR, &["admin"], None, None, None),
    );
    expect_receipt_expired_as(
        provider
            .create_export(&principal(OPERATOR), export("f3-admin-tombstone", true))
            .await,
        "committed",
    );
}

/// F3: `include_tombstoned` administration covers the export's own facets —
/// subject, selected contexts, shared visibility and `subject_all` (§6.5,
/// §7.29) — not an empty space-wide requirement. Every facet negative holds an
/// independently covering TRANSFER grant: the identical scope succeeds as a
/// no-tombstone export, and only the restricted admin gate denies the tombstone
/// request.
#[tokio::test]
async fn export_include_tombstoned_admin_covers_the_export_scope() {
    const ADMIN_OTHER_SUBJECT: &str = "prn_admin_other_subject";
    const ADMIN_OTHER_CONTEXT: &str = "prn_admin_other_context";
    const ADMIN_NO_SHARED: &str = "prn_admin_no_shared";
    const ADMIN_CONTEXT_SHARED: &str = "prn_admin_context_shared";
    const ADMIN_SUBJECT_ONLY: &str = "prn_admin_subject_only";
    const ADMIN_REVOKED: &str = "prn_admin_revoked";
    const OTHER_SUBJECT: &str = "toy:character:otom";

    fn export_request(
        subject: Option<&str>,
        selection: Value,
        include_tombstoned: bool,
        key: &str,
    ) -> MemoryExportsCreateRequest {
        let mut value = json!({
            "space_id": SPACE,
            "include_tombstoned": include_tombstoned,
            "idempotency_key": key,
        });
        if let Some(subject) = subject {
            value["subject_id"] = json!(subject);
        }
        if !selection.is_null() {
            value["selection"] = selection;
        }
        serde_json::from_value(value).expect("export request")
    }

    let provider = transfer_space();
    provider.provision_subject(SPACE, OTHER_SUBJECT, Some(("toy", "character")));
    // Subject facet: admin for SUBJECT only, plus space-wide transfer.
    provider.provision_grant(
        SPACE,
        grant(
            "grt_admin_facet_subject_0030",
            ADMIN_OTHER_SUBJECT,
            &["admin"],
            Some(SUBJECT),
            None,
            None,
        ),
    );
    provider.provision_grant(
        SPACE,
        grant(
            "grt_transfer_facet_subject_0031",
            ADMIN_OTHER_SUBJECT,
            &["transfer"],
            None,
            None,
            None,
        ),
    );
    // Context facet: admin limited to C, plus space-wide transfer.
    provider.provision_grant(
        SPACE,
        grant(
            "grt_admin_facet_context_0032",
            ADMIN_OTHER_CONTEXT,
            &["admin"],
            Some(SUBJECT),
            Some(&[CONTEXT_SELECTION]),
            Some(false),
        ),
    );
    provider.provision_grant(
        SPACE,
        grant(
            "grt_transfer_facet_context_0033",
            ADMIN_OTHER_CONTEXT,
            &["transfer"],
            None,
            None,
            None,
        ),
    );
    // Shared facet: admin without shared access, transfer with it.
    provider.provision_grant(
        SPACE,
        grant(
            "grt_admin_facet_shared_0034",
            ADMIN_NO_SHARED,
            &["admin"],
            Some(SUBJECT),
            Some(&[CONTEXT_SELECTION]),
            Some(false),
        ),
    );
    provider.provision_grant(
        SPACE,
        grant(
            "grt_transfer_facet_shared_0035",
            ADMIN_NO_SHARED,
            &["transfer"],
            Some(SUBJECT),
            Some(&[CONTEXT_SELECTION]),
            Some(true),
        ),
    );
    // `subject_all` facet: context-limited admin WITH shared access, plus an
    // unrestricted space-wide transfer.
    provider.provision_grant(
        SPACE,
        grant(
            "grt_admin_facet_subject_all_0036",
            ADMIN_CONTEXT_SHARED,
            &["admin"],
            Some(SUBJECT),
            Some(&[CONTEXT_SELECTION]),
            Some(true),
        ),
    );
    provider.provision_grant(
        SPACE,
        grant(
            "grt_transfer_facet_subject_all_0037",
            ADMIN_CONTEXT_SHARED,
            &["transfer"],
            None,
            None,
            None,
        ),
    );
    // Whole-space facet: subject-scoped admin, plus space-wide transfer.
    provider.provision_grant(
        SPACE,
        grant(
            "grt_admin_facet_whole_space_0038",
            ADMIN_SUBJECT_ONLY,
            &["admin"],
            Some(SUBJECT),
            None,
            None,
        ),
    );
    provider.provision_grant(
        SPACE,
        grant(
            "grt_transfer_facet_whole_space_0039",
            ADMIN_SUBJECT_ONLY,
            &["transfer"],
            None,
            None,
            None,
        ),
    );
    // Revoked facet: the admin grant is revoked; an independent transfer grant
    // with the same coverage remains.
    provider.provision_grant(
        SPACE,
        grant(
            "grt_admin_facet_revoked_0040",
            ADMIN_REVOKED,
            &["admin"],
            Some(SUBJECT),
            Some(&[CONTEXT_SELECTION]),
            Some(true),
        ),
    );
    provider.provision_grant(
        SPACE,
        grant(
            "grt_transfer_facet_revoked_0041",
            ADMIN_REVOKED,
            &["transfer"],
            Some(SUBJECT),
            Some(&[CONTEXT_SELECTION]),
            Some(true),
        ),
    );

    let selected_c = || json!({ "mode": "selected_contexts", "context_ids": [CONTEXT_SELECTION] });
    let selected_c_shared = || {
        json!({
            "mode": "selected_contexts",
            "context_ids": [CONTEXT_SELECTION],
            "include_shared": true,
        })
    };
    let selected_d = || json!({ "mode": "selected_contexts", "context_ids": [CONTEXT_ELSEWHERE] });

    // Covered admins stay admitted: a bounded scoped admin, an unrestricted
    // subject admin, and a space-wide admin.
    expect_ok(
        provider
            .create_export(
                &principal(ADMIN_CONTEXT_SHARED),
                export_request(
                    Some(SUBJECT),
                    selected_c_shared(),
                    true,
                    "f3-facet-covered-scoped",
                ),
            )
            .await,
    );
    expect_ok(
        provider
            .create_export(
                &principal(ADMIN_SUBJECT_ONLY),
                export_request(Some(SUBJECT), selection_all(), true, "f3-facet-covered-subject-all"),
            )
            .await,
    );
    expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                export_request(None, Value::Null, true, "f3-facet-covered-whole-space"),
            )
            .await,
    );

    // Each facet negative first proves the identical requested scope is a
    // successful TRANSFER-only export (explicit no-tombstone positive
    // control), then that only the tombstone request needs the admin gate.
    for (requester, subject, selection, key) in [
        (
            ADMIN_OTHER_SUBJECT,
            Some(OTHER_SUBJECT),
            selected_c(),
            "f3-facet-subject",
        ),
        (
            ADMIN_OTHER_CONTEXT,
            Some(SUBJECT),
            selected_d(),
            "f3-facet-context",
        ),
        (
            ADMIN_NO_SHARED,
            Some(SUBJECT),
            selected_c_shared(),
            "f3-facet-shared",
        ),
        (
            ADMIN_CONTEXT_SHARED,
            Some(SUBJECT),
            selection_all(),
            "f3-facet-subject-all",
        ),
        (
            ADMIN_SUBJECT_ONLY,
            None,
            Value::Null,
            "f3-facet-whole-space",
        ),
    ] {
        expect_ok(
            provider
                .create_export(
                    &principal(requester),
                    export_request(
                        subject,
                        selection.clone(),
                        false,
                        &format!("{key}-transfer"),
                    ),
                )
                .await,
        );
        assert_eq!(
            expect_reject(
                provider
                    .create_export(
                        &principal(requester),
                        export_request(subject, selection, true, key),
                    )
                    .await
            ),
            SpokeRejectCode::NotAuthorized,
            "{key}: the restricted admin grant must not cover the facet"
        );
    }

    // A changed payload reusing a committed key is authorized for the scope it
    // actually requests BEFORE any conflict answer. With broad transfer but a
    // context-limited admin, the admin gate denies it; with the admin-covered
    // scope the different digest is the conflict.
    let committed_key = "f3-facet-changed";
    expect_ok(
        provider
            .create_export(
                &principal(ADMIN_OTHER_CONTEXT),
                export_request(Some(SUBJECT), selected_c(), false, committed_key),
            )
            .await,
    );
    provider.run_worker();
    assert_eq!(
        expect_reject(
            provider
                .create_export(
                    &principal(ADMIN_OTHER_CONTEXT),
                    export_request(Some(SUBJECT), selected_d(), true, committed_key),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );
    assert_eq!(
        expect_reject(
            provider
                .create_export(
                    &principal(ADMIN_OTHER_CONTEXT),
                    export_request(Some(SUBJECT), selection_all(), true, committed_key),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );
    assert_eq!(
        expect_reject(
            provider
                .create_export(
                    &principal(ADMIN_OTHER_CONTEXT),
                    export_request(Some(SUBJECT), selected_c(), true, committed_key),
                )
                .await
        ),
        SpokeRejectCode::IdempotencyConflict
    );

    // The identical matching replay of an admin export is denied once the
    // admin grant is revoked; the retained transfer grant still serves a
    // non-tombstone export.
    expect_ok(
        provider
            .create_export(
                &principal(ADMIN_REVOKED),
                export_request(Some(SUBJECT), selected_c_shared(), true, "f3-facet-revoked"),
            )
            .await,
    );
    provider.run_worker();
    expect_ok(
        provider
            .revoke_grant(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "grant_id": "grt_admin_facet_revoked_0040",
                    "expected_revision": 1,
                    "idempotency_key": "f3-facet-revoke",
                }))
                .expect("revoke request"),
            )
            .await,
    );
    assert_eq!(
        expect_reject(
            provider
                .create_export(
                    &principal(ADMIN_REVOKED),
                    export_request(Some(SUBJECT), selected_c_shared(), true, "f3-facet-revoked"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );
    expect_ok(
        provider
            .create_export(
                &principal(ADMIN_REVOKED),
                export_request(
                    Some(SUBJECT),
                    selected_c_shared(),
                    false,
                    "f3-facet-revoked-transfer",
                ),
            )
            .await,
    );
}

/// SR-3: a changed payload reusing an export key is authorized for the scope
/// it actually requests before any conflict or replay answer.
#[tokio::test]
async fn export_replay_authorizes_the_changed_payload_scope() {
    let provider = restricted_transfer_space();
    seed_scoped_record(
        &provider,
        "rec_ctx_a",
        "The gate key is under the third barrel.",
        json!({ "visibility": "context", "context_id": CONTEXT_SELECTION }),
    );
    let request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "selection": { "mode": "selected_contexts", "context_ids": [CONTEXT_SELECTION] },
        "idempotency_key": "export-scope-authority",
    });
    let created = expect_ok(
        provider
            .create_export(
                &principal(TRANSFER),
                serde_json::from_value(request.clone()).expect("export request"),
            )
            .await,
    );
    let receipt_id = wire(&created)["receipt_id"].clone();
    provider.run_worker();
    let replayed = expect_ok(
        provider
            .create_export(
                &principal(TRANSFER),
                serde_json::from_value(request.clone()).expect("export request"),
            )
            .await,
    );
    let replayed = wire(&replayed);
    assert_eq!(replayed["receipt_id"], receipt_id);
    assert_eq!(replayed["status"], "committed", "{replayed}");

    // Same key, different payload, uncovered context: the requested scope is
    // denied instead of answering with a conflict borrowed from the stored
    // scope.
    let mut uncovered = request.clone();
    uncovered["selection"] =
        json!({ "mode": "selected_contexts", "context_ids": [CONTEXT_ELSEWHERE] });
    assert_eq!(
        expect_reject(
            provider
                .create_export(
                    &principal(TRANSFER),
                    serde_json::from_value(uncovered).expect("export request"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );

    // Same key, different payload, covered scope: the conflict is the answer.
    let mut changed = request.clone();
    changed["include_reflections"] = json!(true);
    assert_eq!(
        expect_reject(
            provider
                .create_export(
                    &principal(TRANSFER),
                    serde_json::from_value(changed).expect("export request"),
                )
                .await
        ),
        SpokeRejectCode::IdempotencyConflict
    );
}

/// ASF-3: an existing export replay is authorized by its original stored
/// coverage under current grants; a reflection created later must neither
/// change that replay nor be silently dropped from a genuinely new request.
#[tokio::test]
async fn export_replay_keeps_its_original_coverage_after_new_reflections() {
    let provider = restricted_transfer_space();
    seed_scoped_record(
        &provider,
        "rec_ctx_a",
        "The gate key is under the third barrel.",
        json!({ "visibility": "context", "context_id": CONTEXT_SELECTION }),
    );
    const LATE_ADMIN: &str = "prn_late_reflection_admin";
    provider.provision_grant(
        SPACE,
        grant(
            "grt_late_reflection_admin_0042",
            LATE_ADMIN,
            &["admin"],
            Some(SUBJECT),
            Some(&[CONTEXT_SELECTION]),
            Some(false),
        ),
    );
    let admin_request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "selection": { "mode": "selected_contexts", "context_ids": [CONTEXT_SELECTION] },
        "include_reflections": true,
        "include_tombstoned": true,
        "idempotency_key": "export-replay-reflection-admin",
    });
    let admin_receipt = wire(&expect_ok(
        provider
            .create_export(
                &principal(LATE_ADMIN),
                serde_json::from_value(admin_request.clone()).expect("export request"),
            )
            .await,
    ))["receipt_id"]
        .clone();
    provider.run_worker();
    let request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "selection": { "mode": "selected_contexts", "context_ids": [CONTEXT_SELECTION] },
        "include_reflections": true,
        "idempotency_key": "export-replay-reflection",
    });
    let created = expect_ok(
        provider
            .create_export(
                &principal(TRANSFER),
                serde_json::from_value(request.clone()).expect("export request"),
            )
            .await,
    );
    let receipt_id = wire(&created)["receipt_id"].clone();
    let covered = poll_receipt(
        &provider,
        SPACE,
        receipt_id.as_str().expect("receipt id"),
    )
    .await;
    assert_eq!(covered["status"], "committed", "{covered}");
    let export_id = covered["result"]["export_id"].clone();

    // Another authorized actor pins a reflection outside the restricted
    // principal's coverage after the export was sealed.
    provider.provision_reflection(
        SPACE,
        json!({
            "schema_version": 1,
            "reflection_id": "refl_late_all_scope",
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": { "mode": "subject_all" },
            "basis": {
                "source_set_digest": "0".repeat(64),
                "record_count": 1,
                "profile_revisions": {},
                "state_digest": "1".repeat(64),
            },
            "body": { "format": "markdown", "body": "Derived from every placement." },
            "stale": false,
            "generator": { "engine_policy_version": ENGINE_POLICY_VERSION },
            "created_at": "2026-07-23T07:00:00Z",
            "revision": 1,
            "extensions": {},
        }),
    );

    // The identical retry answers the original receipt: the stored coverage,
    // not the new reflection snapshot, authorizes it.
    let replayed = expect_ok(
        provider
            .create_export(
                &principal(TRANSFER),
                serde_json::from_value(request.clone()).expect("export request"),
            )
            .await,
    );
    let replayed = wire(&replayed);
    assert_eq!(replayed["receipt_id"], receipt_id);
    assert_eq!(replayed["status"], "committed", "{replayed}");
    assert_eq!(replayed["result"]["export_id"], export_id);

    // Admin-specific: the same stored original coverage still authorizes the
    // tombstone replay, so the later unrestricted reflection neither widens
    // nor breaks it.
    let admin_replayed = wire(&expect_ok(
        provider
            .create_export(
                &principal(LATE_ADMIN),
                serde_json::from_value(admin_request.clone()).expect("export request"),
            )
            .await,
    ));
    assert_eq!(admin_replayed["receipt_id"], admin_receipt);
    assert_eq!(admin_replayed["status"], "committed", "{admin_replayed}");

    // Recomputing coverage on that matching replay would change the answer: a
    // fresh admin request that selects the late unrestricted reflection is
    // denied, so the replay above is sensitive to a recompute-on-replay bug.
    let mut fresh_admin = admin_request.clone();
    fresh_admin["idempotency_key"] = json!("export-replay-reflection-admin-new");
    assert_eq!(
        expect_reject(
            provider
                .create_export(
                    &principal(LATE_ADMIN),
                    serde_json::from_value(fresh_admin).expect("export request"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );

    // A genuinely new request still obeys the current pins: deny the whole
    // request instead of dropping the requested reflection.
    let mut fresh_request = request.clone();
    fresh_request["idempotency_key"] = json!("export-replay-reflection-new");
    assert_eq!(
        expect_reject(
            provider
                .create_export(
                    &principal(TRANSFER),
                    serde_json::from_value(fresh_request).expect("export request"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );

    // The evicted-receipt case keeps enough stored coverage to authorize the
    // replay before answering RECEIPT_EXPIRED.
    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);
    expect_receipt_expired(
        provider
            .create_export(
                &principal(TRANSFER),
                serde_json::from_value(request).expect("export request"),
            )
            .await,
    );
    expect_receipt_expired_as(
        provider
            .create_export(
                &principal(LATE_ADMIN),
                serde_json::from_value(admin_request).expect("export request"),
            )
            .await,
        "committed",
    );
}

/// TEST-2: the promoted id is allocated from the shared monotonic sequence,
/// not from the record count. A provisioned id occupied at the count-derived
/// next value must survive the promotion untouched.
#[tokio::test]
async fn review_promotion_skips_an_occupied_count_derived_id() {
    let provider = provider();
    seed_scoped_record(
        &provider,
        "rec_sqlite_0001",
        "Retained record one.",
        json!({ "visibility": "context", "context_id": CONTEXT }),
    );
    seed_scoped_record(
        &provider,
        "rec_sqlite_0003",
        "Retained record three.",
        json!({ "visibility": "context", "context_id": CONTEXT }),
    );
    // Two retained records: a count-based allocator would pick
    // `rec_sqlite_0003`, which is already occupied.
    let captured = expect_ok(
        provider
            .capture(
                &principal(READER),
                serde_json::from_value(capture_request(
                    "Mira told Bo the gate key is hidden under the third barrel.",
                    "cap-gap-1",
                ))
                .expect("capture request"),
            )
            .await,
    );
    let candidate_id = wire(&captured)["result"]["candidate_ids"][0]
        .as_str()
        .expect("candidate id")
        .to_string();
    let reviewed = expect_ok(
        provider
            .review_candidate(
                &principal(READER),
                serde_json::from_value(review_request(
                    &candidate_id,
                    1,
                    "rev-gap-1",
                    json!({ "action": "promote" }),
                ))
                .expect("review request"),
            )
            .await,
    );
    let promoted = wire(&reviewed)["result"]["record_ids"][0]
        .as_str()
        .expect("record id")
        .to_string();
    assert_eq!(promoted, "rec_sqlite_0002");
    let retained = expect_ok(
        provider
            .get_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": "rec_sqlite_0003",
                }))
                .expect("get request"),
            )
            .await,
    );
    assert_eq!(
        wire(&retained)["record"]["content"]["text"],
        "Retained record three."
    );
    let listed = expect_ok(
        provider
            .list_records(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": selection_all(),
                }))
                .expect("list request"),
            )
            .await,
    );
    assert_eq!(wire(&listed)["total_authorized"], 3, "{:?}", wire(&listed));
}

/// PSF-4/TEST-3: record, candidate, fence, capture and profile replays answer
/// `RECEIPT_EXPIRED` (with the tombstone's last known status) instead of a
/// consumed/CAS error, and no effect runs twice.
#[tokio::test]
async fn evicted_receipts_stop_record_candidate_fence_and_capture_reexecution() {
    let provider = provider();
    let create_request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "kind": "fact",
        "content": { "text": "Created exactly once." },
        "scope": { "visibility": "context", "context_id": CONTEXT },
        "idempotency_key": "rep2-del-create",
    });
    let created = expect_ok(
        provider
            .create_record(
                &principal(OPERATOR),
                serde_json::from_value(create_request.clone()).expect("create request"),
            )
            .await,
    );
    let create_receipt_id = wire(&created)["receipt_id"].clone();
    let record_id = wire(&created)["result"]["record_id"]
        .as_str()
        .expect("record id")
        .to_string();
    let delete_request = json!({
        "space_id": SPACE,
        "record_id": record_id,
        "expected_revision": 1,
        "idempotency_key": "rep2-del",
    });
    let _ = expect_ok(
        provider
            .delete_record(
                &principal(OPERATOR),
                serde_json::from_value(delete_request.clone()).expect("delete request"),
            )
            .await,
    );

    let captured = expect_ok(
        provider
            .capture(
                &principal(READER),
                serde_json::from_value(capture_request(
                    "A dismissable claim about the gate key.",
                    "rep2-dismiss-cap",
                ))
                .expect("capture request"),
            )
            .await,
    );
    let dismiss_candidate = wire(&captured)["result"]["candidate_ids"][0]
        .as_str()
        .expect("candidate id")
        .to_string();
    let dismiss_request = json!({
        "space_id": SPACE,
        "candidate_id": dismiss_candidate,
        "expected_revision": 1,
        "idempotency_key": "rep2-dismiss",
    });
    let _ = expect_ok(
        provider
            .dismiss_candidate(
                &principal(READER),
                serde_json::from_value(dismiss_request.clone()).expect("dismiss request"),
            )
            .await,
    );

    let captured = expect_ok(
        provider
            .capture(
                &principal(READER),
                serde_json::from_value(capture_request(
                    "A reviewable claim about the gate key.",
                    "rep2-review-cap",
                ))
                .expect("capture request"),
            )
            .await,
    );
    let review_candidate_id = wire(&captured)["result"]["candidate_ids"][0]
        .as_str()
        .expect("candidate id")
        .to_string();
    let review_request_value = review_request(
        &review_candidate_id,
        1,
        "rep2-review",
        json!({ "action": "drop" }),
    );
    let reviewed = expect_ok(
        provider
            .review_candidate(
                &principal(READER),
                serde_json::from_value(review_request_value.clone()).expect("review request"),
            )
            .await,
    );
    let review_receipt_id = wire(&reviewed)["receipt_id"].clone();

    let capture_request_value = capture_request(
        "A captured claim that stays pending at the gate.",
        "rep2-capture",
    );
    let captured = expect_ok(
        provider
            .capture(
                &principal(READER),
                serde_json::from_value(capture_request_value.clone()).expect("capture request"),
            )
            .await,
    );
    let capture_receipt_id = wire(&captured)["receipt_id"].clone();

    let profile_request = profile_update("personality", None, "rep2-profile");
    let _ = expect_ok(
        provider
            .update_profile(
                &principal(OPERATOR),
                serde_json::from_value(profile_request.clone()).expect("profile update"),
            )
            .await,
    );

    let fence_request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "target": { "type": "context", "context_id": CONTEXT },
        "action": "fence",
        "expected_revision": 1,
        "idempotency_key": "rep2-fence",
    });
    let _ = expect_ok(
        provider
            .fence_subject(
                &principal(OPERATOR),
                serde_json::from_value(fence_request.clone()).expect("fence request"),
            )
            .await,
    );

    // TA-4: an already-committed request keeps answering its receipt after the
    // fence — the fence gates new effects, not retained replays.
    let replayed_create = expect_ok(
        provider
            .create_record(
                &principal(OPERATOR),
                serde_json::from_value(create_request.clone()).expect("create request"),
            )
            .await,
    );
    assert_eq!(wire(&replayed_create)["receipt_id"], create_receipt_id);
    assert_eq!(
        wire(&replayed_create)["result"]["record_id"],
        json!(record_id.as_str())
    );
    let replayed_review = expect_ok(
        provider
            .review_candidate(
                &principal(READER),
                serde_json::from_value(review_request_value.clone()).expect("review request"),
            )
            .await,
    );
    assert_eq!(wire(&replayed_review)["receipt_id"], review_receipt_id);
    let replayed_capture = expect_ok(
        provider
            .capture(
                &principal(READER),
                serde_json::from_value(capture_request_value.clone()).expect("capture request"),
            )
            .await,
    );
    assert_eq!(wire(&replayed_capture)["receipt_id"], capture_receipt_id);

    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);

    expect_receipt_expired(
        provider
            .create_record(
                &principal(OPERATOR),
                serde_json::from_value(create_request).expect("create request"),
            )
            .await,
    );
    expect_receipt_expired(
        provider
            .delete_record(
                &principal(OPERATOR),
                serde_json::from_value(delete_request).expect("delete request"),
            )
            .await,
    );
    expect_receipt_expired(
        provider
            .dismiss_candidate(
                &principal(READER),
                serde_json::from_value(dismiss_request).expect("dismiss request"),
            )
            .await,
    );
    expect_receipt_expired(
        provider
            .review_candidate(
                &principal(READER),
                serde_json::from_value(review_request_value).expect("review request"),
            )
            .await,
    );
    expect_receipt_expired(
        provider
            .capture(
                &principal(READER),
                serde_json::from_value(capture_request_value).expect("capture request"),
            )
            .await,
    );
    expect_receipt_expired(
        provider
            .update_profile(
                &principal(OPERATOR),
                serde_json::from_value(profile_request).expect("profile update"),
            )
            .await,
    );
    expect_receipt_expired(
        provider
            .fence_subject(
                &principal(OPERATOR),
                serde_json::from_value(fence_request).expect("fence request"),
            )
            .await,
    );

    // No re-execution: the tombstone, the consumed candidates, the pending
    // candidate, the profile revision and the fence revision are unchanged.
    let record = expect_ok(
        provider
            .get_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": record_id,
                }))
                .expect("get request"),
            )
            .await,
    );
    let record = wire(&record);
    assert_eq!(record["record"]["revision"], 2, "{record}");
    assert_eq!(record["record"]["content"]["text"], "", "{record}");
    for candidate_id in [&dismiss_candidate, &review_candidate_id] {
        let candidate = expect_ok(
            provider
                .get_candidate(
                    &principal(READER),
                    serde_json::from_value(json!({
                        "space_id": SPACE,
                        "candidate_id": candidate_id,
                    }))
                    .expect("get request"),
                )
                .await,
        );
        assert_eq!(wire(&candidate)["candidate"]["status"], "consumed");
        assert_eq!(wire(&candidate)["candidate"]["revision"], 2);
    }
    let pending = expect_ok(
        provider
            .list_candidates(
                &principal(READER),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "limit": 10,
                }))
                .expect("list request"),
            )
            .await,
    );
    let pending = wire(&pending);
    assert_eq!(
        pending["candidates"].as_array().expect("candidates").len(),
        3,
        "a re-executed capture would add a candidate: {pending}"
    );
    let profile = expect_ok(
        provider
            .get_profile(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                }))
                .expect("get profile"),
            )
            .await,
    );
    assert_eq!(wire(&profile)["profiles"][0]["revision"], 1);
    let subject = expect_ok(
        provider
            .get_subject(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                }))
                .expect("get subject"),
            )
            .await,
    );
    assert_eq!(wire(&subject)["subject"]["revision"], 2);
    assert_eq!(
        wire(&subject)["subject"]["fenced_contexts"],
        json!([CONTEXT])
    );
}

/// PSF-4/TEST-3: grant issuance, revocation and the import stage/activate/
/// discard replays answer `RECEIPT_EXPIRED` and never run their effect twice.
#[tokio::test]
async fn evicted_receipts_stop_grant_and_import_reexecution() {
    let provider = empty_target_provider();
    seed_record(
        &provider,
        "rec_replay_grant",
        "The gate key is under the third barrel.",
    );

    let grant_request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "principal_id": READER,
        "capabilities": ["recall"],
        "context_ids": [CONTEXT],
        "shared_access": true,
        "idempotency_key": "rep2-grant",
    });
    let issued = expect_ok(
        provider
            .grant(
                &principal(OPERATOR),
                serde_json::from_value(grant_request.clone()).expect("grant request"),
            )
            .await,
    );
    let issued_grant_id = wire(&issued)["result"]["grant_id"]
        .as_str()
        .expect("grant id")
        .to_string();
    let revoke_request = json!({
        "space_id": SPACE,
        "grant_id": issued_grant_id,
        "expected_revision": 1,
        "idempotency_key": "rep2-revoke",
    });
    let _ = expect_ok(
        provider
            .revoke_grant(
                &principal(OPERATOR),
                serde_json::from_value(revoke_request.clone()).expect("revoke request"),
            )
            .await,
    );

    let (_export_a, chunks_a) = export_chunks_with_key(&provider, "rep2-export-a").await;
    let (_export_b, chunks_b) = export_chunks_with_key(&provider, "rep2-export-b").await;
    let (_export_c, chunks_c) = export_chunks_with_key(&provider, "rep2-export-c").await;

    // Import A: fully staged through the typed public `imports.stage` port;
    // activation follows once every import is staged (an activated import
    // creates the subject in the target space).
    let mut import_a = String::new();
    for (ordinal, chunk) in chunks_a.iter().enumerate() {
        let typed = typed_stage_request(
            OTHER_SPACE,
            chunk,
            format!("rep2-stage-a-{ordinal}"),
            (ordinal == 0).then_some("fail"),
        );
        let receipt = expect_ok(provider.stage_import(&principal(OPERATOR), typed).await);
        let receipt = wire(&receipt);
        assert_eq!(receipt["result"]["chunks_received"], ordinal + 1);
        import_a = receipt["result"]["import_id"]
            .as_str()
            .expect("import id")
            .to_string();
    }
    let activate_request = json!({
        "space_id": OTHER_SPACE,
        "import_id": import_a,
        "idempotency_key": "rep2-activate",
    });

    // Import B: fully staged through the typed port, then discarded.
    let mut import_b = String::new();
    for (ordinal, chunk) in chunks_b.iter().enumerate() {
        let typed = typed_stage_request(
            OTHER_SPACE,
            chunk,
            format!("rep2-stage-b-{ordinal}"),
            (ordinal == 0).then_some("fail"),
        );
        let receipt = expect_ok(provider.stage_import(&principal(OPERATOR), typed).await);
        import_b = wire(&receipt)["result"]["import_id"]
            .as_str()
            .expect("import id")
            .to_string();
    }
    let discard_request = json!({
        "space_id": OTHER_SPACE,
        "import_id": import_b,
        "idempotency_key": "rep2-discard",
    });
    let _ = expect_ok(
        provider
            .discard_import(
                &principal(OPERATOR),
                serde_json::from_value(discard_request.clone()).expect("discard request"),
            )
            .await,
    );

    // Import C: only the first chunk is staged, through the typed port.
    let stage_first_c = typed_stage_request(
        OTHER_SPACE,
        &chunks_c[0],
        "rep2-stage-c-0".to_string(),
        Some("fail"),
    );
    let staged = expect_ok(
        provider
            .stage_import(&principal(OPERATOR), stage_first_c.clone())
            .await,
    );
    assert_eq!(wire(&staged)["result"]["chunks_received"], 1);

    let activated = expect_ok(
        provider
            .activate_import(
                &principal(OPERATOR),
                serde_json::from_value(activate_request.clone()).expect("activate request"),
            )
            .await,
    );
    let _ = poll_receipt(
        &provider,
        OTHER_SPACE,
        wire(&activated)["receipt_id"]
            .as_str()
            .expect("receipt id"),
    )
    .await;

    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);

    expect_receipt_expired(
        provider
            .grant(
                &principal(OPERATOR),
                serde_json::from_value(grant_request).expect("grant request"),
            )
            .await,
    );
    expect_receipt_expired(
        provider
            .revoke_grant(
                &principal(OPERATOR),
                serde_json::from_value(revoke_request).expect("revoke request"),
            )
            .await,
    );
    expect_receipt_expired(
        provider
            .stage_import(&principal(OPERATOR), stage_first_c)
            .await,
    );
    expect_receipt_expired(
        provider
            .activate_import(
                &principal(OPERATOR),
                serde_json::from_value(activate_request).expect("activate request"),
            )
            .await,
    );
    expect_receipt_expired(
        provider
            .discard_import(
                &principal(OPERATOR),
                serde_json::from_value(discard_request).expect("discard request"),
            )
            .await,
    );

    // The new grant was issued once (revision 2 after its single revoke).
    let listed = expect_ok(
        provider
            .list_grants(
                &principal(OPERATOR),
                serde_json::from_value(json!({ "space_id": SPACE, "limit": 10 }))
                    .expect("list request"),
            )
            .await,
    );
    let listed = wire(&listed);
    let grants = listed["grants"].as_array().expect("grants");
    assert_eq!(grants.len(), 3, "{listed}");
    let issued_grant = grants
        .iter()
        .find(|grant| grant["grant_id"] == json!(issued_grant_id))
        .expect("issued grant");
    assert_eq!(issued_grant["revision"], 2, "{issued_grant}");

    // Import A stayed activated: discard is pre-activation only.
    assert_eq!(
        expect_reject(
            provider
                .discard_import(
                    &principal(OPERATOR),
                    serde_json::from_value(json!({
                        "space_id": OTHER_SPACE,
                        "import_id": import_a,
                        "idempotency_key": "rep2-check-a",
                    }))
                    .expect("discard request"),
                )
                .await
        ),
        SpokeRejectCode::ImportConflict
    );
    // Import B stays discarded, and its committed stage effect is bound to
    // its own key namespace (same key + different digest is a conflict).
    assert_eq!(
        expect_reject(
            provider
                .stage_import(
                    &principal(OPERATOR),
                    typed_stage_request(
                        OTHER_SPACE,
                        &chunks_b[0],
                        "rep2-stage-b-0".to_string(),
                        Some("rename_subject"),
                    ),
                )
                .await
        ),
        SpokeRejectCode::IdempotencyConflict
    );
    assert_eq!(
        expect_reject(
            provider
                .activate_import(
                    &principal(OPERATOR),
                    serde_json::from_value(json!({
                        "space_id": OTHER_SPACE,
                        "import_id": import_b,
                        "idempotency_key": "rep2-check-b",
                    }))
                    .expect("activate request"),
                )
                .await
        ),
        SpokeRejectCode::ImportConflict
    );
    // Import C still holds exactly one chunk.
    assert_eq!(
        expect_reject(
            provider
                .stage_import(
                    &principal(OPERATOR),
                    typed_stage_request(
                        OTHER_SPACE,
                        &chunks_c[0],
                        "rep2-check-c".to_string(),
                        Some("fail"),
                    ),
                )
                .await
        ),
        SpokeRejectCode::InvalidInput
    );
}

/// Storage: a document persisted without the required lifecycle-evidence map,
/// or with an evidence record missing its identity, must fail closed through
/// the existing loader error, never be replaced by an empty state or a
/// defaulted map, and keep every stored byte for the operator.
#[tokio::test]
async fn unsupported_stored_document_fails_closed_and_keeps_its_bytes() {
    for label in ["missing-evidence-map", "missing-evidence-requester"] {
        let path = temp_state_path(label);
        let provider = seeded(SqliteMemoryProvider::open(&path).expect("fresh store initializes"));
        let ensured = expect_ok(
            provider
                .ensure_subject(
                    &principal(READER),
                    serde_json::from_value::<MemorySubjectsEnsureRequest>(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                    }))
                    .expect("ensure request"),
                )
                .await,
        );
        let receipt_id = wire(&ensured)["receipt_id"]
            .as_str()
            .expect("receipt id")
            .to_string();
        drop(provider);

        let mut document: Value =
            serde_json::from_str(&stored_document(&path)).expect("stored document is json");
        match label {
            "missing-evidence-map" => {
                document["spaces"][SPACE]
                    .as_object_mut()
                    .expect("space object")
                    .remove("receipt_evidence");
            }
            _ => {
                document["spaces"][SPACE]["receipt_evidence"][&receipt_id]
                    .as_object_mut()
                    .expect("evidence object")
                    .remove("requester_id");
            }
        }
        write_stored_document(
            &path,
            &serde_json::to_string(&document).expect("document serializes"),
        );
        let legacy_bytes = stored_document(&path);

        // Every attempted read fails closed and changes nothing.
        for attempt in 0..2 {
            assert!(
                SqliteMemoryProvider::open(&path).is_err(),
                "{label} attempt {attempt} must not accept an unsupported document"
            );
            assert_eq!(stored_document(&path), legacy_bytes, "{label}");
        }

        std::fs::remove_file(&path).expect("remove state file");
    }
}

/// Storage: a genuinely new store initializes, while a valid current store
/// reopens with its real record intact and its bytes untouched by the reopen.
#[tokio::test]
async fn new_store_initializes_and_current_store_reopens_intact() {
    let path = temp_state_path("current");
    let provider = seeded(SqliteMemoryProvider::open(&path).expect("new store initializes"));
    seed_record(
        &provider,
        "rec_current",
        "The guard remembers the second tally.",
    );
    let current_bytes = stored_document(&path);
    assert!(current_bytes.contains("rec_current"), "{current_bytes}");
    drop(provider);

    let reopened = SqliteMemoryProvider::open(&path).expect("current store reopens");
    assert_eq!(stored_document(&path), current_bytes);
    let record = expect_ok(
        reopened
            .get_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({ "space_id": SPACE, "record_id": "rec_current" }))
                    .expect("get request"),
            )
            .await,
    );
    assert_eq!(
        wire(&record)["record"]["content"]["text"],
        "The guard remembers the second tally."
    );

    std::fs::remove_file(&path).expect("remove state file");
}

/// Storage: an undecodable snapshot is never written over live state; the
/// restore control stops on its existing error path with the state intact.
#[test]
fn undecodable_snapshot_never_replaces_live_state() {
    let path = temp_state_path("snapshot");
    let provider = seeded(SqliteMemoryProvider::open(&path).expect("fresh store initializes"));
    seed_record(
        &provider,
        "rec_live",
        "The watchword survives a bad snapshot.",
    );
    provider.save_snapshot();
    let live_bytes = stored_document(&path);
    assert!(live_bytes.contains("rec_live"), "{live_bytes}");

    let connection = rusqlite::Connection::open(&path).expect("open state db");
    let snapshot: String = connection
        .query_row(
            "SELECT document FROM memory_snapshot ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("stored snapshot");
    let mut document: Value = serde_json::from_str(&snapshot).expect("snapshot is json");
    // Unsupported shape for the current model: an evidence record without its
    // mandatory requester identity must not be defaulted.
    document["spaces"][SPACE]["receipt_evidence"]["rcp_snapshot_0001"] = json!({
        "created_at": "2026-07-23T06:00:00Z",
    });
    connection
        .execute(
            "UPDATE memory_snapshot SET document = ?1 WHERE id = (SELECT MAX(id) FROM memory_snapshot)",
            rusqlite::params![serde_json::to_string(&document).expect("document serializes")],
        )
        .expect("write unsupported snapshot");

    assert!(std::panic::catch_unwind(|| provider.restore_snapshot()).is_err());
    assert_eq!(stored_document(&path), live_bytes);

    std::fs::remove_file(&path).expect("remove state file");
}

/// Storage: after a successful open, a document that becomes undecodable must
/// not panic a public operation or poison the lock. The read and mutation
/// paths answer the existing `INTERNAL_ERROR` / `retryable` (no effect
/// committed), keep the stored bytes, and work again once state is valid.
#[tokio::test]
async fn undecodable_state_after_open_fails_public_ops_without_panicking() {
    let path = temp_state_path("public-corrupt");
    let provider = seeded(SqliteMemoryProvider::open(&path).expect("fresh store initializes"));
    seed_record(
        &provider,
        "rec_live",
        "The watchword survives a broken document.",
    );
    let live_bytes = stored_document(&path);
    assert!(live_bytes.contains("rec_live"), "{live_bytes}");

    // An external writer makes the stored row undecodable after the open.
    write_stored_document(&path, "{\"spaces\": \"not a provider document\"}");
    let corrupt_bytes = stored_document(&path);

    let assert_storage_error = |reject: spoke_operations::SpokeReject| {
        assert_eq!(
            reject.code,
            SpokeRejectCode::InternalError,
            "{reject:?}"
        );
        assert_eq!(
            reject
                .details
                .as_ref()
                .and_then(|details| details.get("retry_class")),
            Some(&json!("retryable")),
            "{reject:?}"
        );
    };

    // Read path (`inspect`): no panic, an error envelope, no write.
    assert_storage_error(expect_reject_with(
        provider
            .list_grants(
                &principal(OPERATOR),
                serde_json::from_value(json!({ "space_id": SPACE, "limit": 10 }))
                    .expect("list request"),
            )
            .await,
    ));
    assert_eq!(stored_document(&path), corrupt_bytes);

    // Mutation path (`mutate_value`): the stored document cannot be decoded
    // before any effect, so `retryable` is the truthful no-effect class. (A
    // failed *write* after the effect is `OUTCOME_UNCERTAIN`; covered below.)
    assert_storage_error(expect_reject_with(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "idempotency_key": "exp-corrupt-state",
                }))
                .expect("export request"),
            )
            .await,
    ));
    assert_eq!(stored_document(&path), corrupt_bytes);

    // Restoring valid state resumes normal service: the lock is usable and the
    // seeded record is intact.
    write_stored_document(&path, &live_bytes);
    let record = expect_ok(
        provider
            .get_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({ "space_id": SPACE, "record_id": "rec_live" }))
                    .expect("get request"),
            )
            .await,
    );
    assert_eq!(
        wire(&record)["record"]["content"]["text"],
        "The watchword survives a broken document."
    );

    std::fs::remove_file(&path).expect("remove state file");
}

/// MEM-R022/§6.10/§7.0.5: `memory.subjects.ensure` is exempt from keyed dedup,
/// not from receipt persistence. Its real unkeyed receipt is readable
/// immediately and within retention by its requester and a current admin, never
/// by a foreign or revoked principal; past retention the body is gone and only
/// content-free lifecycle evidence survives, across a real reopen. No synthetic
/// idempotency key or dedup binding is invented.
#[tokio::test]
async fn unkeyed_ensure_receipt_survives_with_body_and_evidence() {
    let path = temp_state_path("ensure-receipt");
    let provider = seeded(SqliteMemoryProvider::open(&path).expect("fresh store initializes"));
    let ensured = wire(&expect_ok(
        provider
            .ensure_subject(
                &principal(READER),
                serde_json::from_value::<MemorySubjectsEnsureRequest>(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                }))
                .expect("ensure request"),
            )
            .await,
    ));
    assert_eq!(ensured["op"], "memory.subjects.ensure", "{ensured}");
    assert_eq!(ensured["status"], "committed", "{ensured}");
    assert!(ensured.get("idempotency_key").is_none(), "{ensured}");
    let receipt_id = ensured["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();
    let created_at = ensured["created_at"].clone();

    // Immediate typed read: the body is the status authority, and the whole
    // original receipt must come back — not just its id, status and clock.
    let own = wire(&expect_ok(
        read_receipt(&provider, READER, &receipt_id).await,
    ));
    assert_eq!(own["receipt"], ensured, "{own}");

    // Within retention the same receipt is served, unchanged.
    provider.advance_clock(120);
    let within = wire(&expect_ok(
        read_receipt(&provider, READER, &receipt_id).await,
    ));
    assert_eq!(within["receipt"], ensured, "{within}");

    // A current admin reads the same complete receipt; a foreign principal
    // with a live grant does not.
    let admin = wire(&expect_ok(
        read_receipt(&provider, OPERATOR, &receipt_id).await,
    ));
    assert_eq!(admin["receipt"], ensured, "{admin}");
    const FOREIGN: &str = "prn_foreign_live";
    provider.provision_grant(
        SPACE,
        grant(
            "grt_foreign_live_0044",
            FOREIGN,
            &["recall"],
            Some(SUBJECT),
            None,
            None,
        ),
    );
    let denial = expect_reject_with(read_receipt(&provider, FOREIGN, &receipt_id).await);
    assert_eq!(denial.code, SpokeRejectCode::NotAuthorized, "{denial:?}");
    assert!(
        receipt_detail_paths(denial.details.as_ref()).is_empty(),
        "denial leaked receipt/status detail: {denial:?}"
    );

    // Past retention the body is evicted in the same write that retains the
    // truthful `committed` status.
    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);
    expect_receipt_expired_as(
        read_receipt(&provider, READER, &receipt_id).await,
        "committed",
    );
    expect_receipt_expired_as(
        read_receipt(&provider, OPERATOR, &receipt_id).await,
        "committed",
    );
    assert!(provider.poll_receipt_value(SPACE, &receipt_id).is_none());

    // A revoked requester learns nothing; restoring authority restores the read.
    expect_ok(
        provider
            .revoke_grant(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryGrantsRevokeRequest>(json!({
                    "space_id": SPACE,
                    "grant_id": "grt_reader_context_0002",
                    "expected_revision": 1,
                    "idempotency_key": "ensure-receipt-revoke",
                }))
                .expect("revoke request"),
            )
            .await,
    );
    let denial = expect_reject_with(read_receipt(&provider, READER, &receipt_id).await);
    assert_eq!(denial.code, SpokeRejectCode::NotAuthorized, "{denial:?}");
    assert!(
        receipt_detail_paths(denial.details.as_ref()).is_empty(),
        "denial leaked receipt/status detail: {denial:?}"
    );
    provider.provision_grant(
        SPACE,
        grant(
            "grt_reader_restored_0045",
            READER,
            &["capture"],
            Some(SUBJECT),
            Some(&[CONTEXT]),
            Some(false),
        ),
    );
    expect_receipt_expired_as(
        read_receipt(&provider, READER, &receipt_id).await,
        "committed",
    );

    // Durable state: body gone; the remaining evidence is content-free and no
    // keyed binding was invented for the unkeyed ensure.
    let stored: Value =
        serde_json::from_str(&stored_document(&path)).expect("stored document is json");
    let space = &stored["spaces"][SPACE];
    assert!(
        space["receipts"]
            .as_object()
            .expect("receipts object")
            .get(&receipt_id)
            .is_none(),
        "evicted body must be gone: {space}"
    );
    let evidence = &space["receipt_evidence"][receipt_id.as_str()];
    assert_eq!(evidence["status"], "committed", "{evidence}");
    assert_eq!(evidence["requester_id"], READER, "{evidence}");
    assert_eq!(evidence["created_at"], created_at, "{evidence}");
    let mut members: Vec<String> = evidence
        .as_object()
        .expect("evidence object")
        .keys()
        .cloned()
        .collect();
    members.sort();
    assert_eq!(members, vec!["created_at", "requester_id", "status"]);
    assert!(
        space["dedup"]
            .as_object()
            .expect("dedup object")
            .values()
            .all(|entry| entry["receipt_id"] != json!(receipt_id)),
        "an unkeyed ensure must not invent a dedup binding: {space}"
    );
    let sequence = space["receipt_sequence"].clone();
    let mut evidence_ids: Vec<String> = space["receipt_evidence"]
        .as_object()
        .expect("evidence map")
        .keys()
        .cloned()
        .collect();
    evidence_ids.sort();

    // A real reopen serves the same expired `committed` status with unchanged
    // ids, and the read writes nothing.
    let before_reopen = stored_document(&path);
    drop(provider);
    let reopened = SqliteMemoryProvider::open(&path).expect("store reopens");
    expect_receipt_expired_as(
        read_receipt(&reopened, READER, &receipt_id).await,
        "committed",
    );
    assert_eq!(stored_document(&path), before_reopen);
    let after: Value =
        serde_json::from_str(&before_reopen).expect("stored document is json");
    assert_eq!(after["spaces"][SPACE]["receipt_sequence"], sequence);
    let mut evidence_after: Vec<String> = after["spaces"][SPACE]["receipt_evidence"]
        .as_object()
        .expect("evidence map")
        .keys()
        .cloned()
        .collect();
    evidence_after.sort();
    assert_eq!(evidence_after, evidence_ids);

    std::fs::remove_file(&path).expect("remove state file");
}

/// §7.0.5/MEM-R021: a committed tombstone survives body eviction and a store
/// reopen — typed read and same-key replay both answer `RECEIPT_EXPIRED` with
/// the truthful status, and the replay re-executes nothing.
#[tokio::test]
async fn committed_tombstone_survives_reopen_for_read_and_replay() {
    let path = temp_state_path("committed-tombstone");
    let provider = seeded(SqliteMemoryProvider::open(&path).expect("fresh store initializes"));
    seed_record(
        &provider,
        "rec_reopen",
        "The gate key is under the third barrel.",
    );
    let request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "idempotency_key": "reopen-committed",
    });
    let created = wire(&expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(request.clone()).expect("export request"),
            )
            .await,
    ));
    let receipt_id = created["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();
    assert_eq!(created["status"], "pending", "{created}");
    provider.run_worker();
    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);

    // Eviction happens on the next provider call and is durable.
    expect_receipt_expired_as(
        read_receipt(&provider, OPERATOR, &receipt_id).await,
        "committed",
    );
    assert!(provider.poll_receipt_value(SPACE, &receipt_id).is_none());
    drop(provider);

    let reopened = SqliteMemoryProvider::open(&path).expect("store reopens");
    let after_reopen = stored_document(&path);
    expect_receipt_expired_as(
        read_receipt(&reopened, OPERATOR, &receipt_id).await,
        "committed",
    );
    expect_receipt_expired_as(
        reopened
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(request).expect("export request"),
            )
            .await,
        "committed",
    );
    // Neither path re-executed: no new receipt body and no state change.
    assert!(reopened
        .poll_receipt_value(SPACE, &receipt_id)
        .is_none());
    assert_eq!(stored_document(&path), after_reopen);

    std::fs::remove_file(&path).expect("remove state file");
}

/// MEM-R022/§6.10: a `failed` terminal status is retained through body
/// eviction and a reopen, and neither the typed read nor the same-key replay
/// invents a commit or re-executes. Private valid-state fixture: no public op
/// produces a `failed` receipt. The real admission first fixes the binding and
/// its concrete original scope; a reflection admitted later widens only a
/// freshly computed scope, so a matching replay answering the original failed
/// lifecycle proves it consumed the stored scope instead of recomputing it.
#[tokio::test]
async fn failed_tombstone_survives_reopen_for_read_and_replay() {
    let path = temp_state_path("failed-tombstone");
    let provider = restricted_transfer_on(seeded(
        SqliteMemoryProvider::open(&path).expect("fresh store initializes"),
    ));
    let request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "selection": { "mode": "selected_contexts", "context_ids": [CONTEXT_SELECTION] },
        "include_reflections": true,
        "idempotency_key": "reopen-failed",
    });
    seed_scoped_record(
        &provider,
        "rec_failed_export",
        "A tally the failed export could not deliver.",
        json!({ "visibility": "context", "context_id": CONTEXT_SELECTION }),
    );
    // A real public admission of this exact keyed export first: the receipt id,
    // payload digest, clock, owner and dedup binding (including the original
    // export scope) all come from the provider, not from the fixture.
    let created = wire(&expect_ok(
        provider
            .create_export(
                &principal(TRANSFER),
                serde_json::from_value(request.clone()).expect("export request"),
            )
            .await,
    ));
    assert_eq!(created["status"], "pending", "{created}");
    provider.run_worker();
    let receipt_id = created["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();
    let digest = created["payload_digest"]
        .as_str()
        .expect("payload digest")
        .to_string();
    // Snapshot the complete binding the real admission wrote, before the
    // terminal-body fixture exists, and pin its concrete scope values: the
    // selection alone (no reflection was included yet) demands exactly this
    // coverage of `TRANSFER`'s narrow grant.
    let admitted: Value = serde_json::from_str(&stored_document(&path)).expect("admitted state");
    let binding_before = admitted["spaces"][SPACE]["dedup"]
        .as_object()
        .expect("dedup map")
        .values()
        .find(|entry| entry["receipt_id"] == json!(receipt_id))
        .expect("the real export binding exists")
        .clone();
    assert_eq!(binding_before["receipt_id"], json!(receipt_id), "{binding_before}");
    assert_eq!(binding_before["digest"], json!(digest), "{binding_before}");
    assert_eq!(
        binding_before["export_scope"],
        json!({
            "context_ids": [CONTEXT_SELECTION],
            "reads_shared": false,
            "subject_all": false,
        }),
        "the admitted export keeps its actual original scope: {binding_before}"
    );
    // The private fixture only replaces the terminal body of that same real
    // lifecycle state; the dedup binding stays exactly as admitted.
    let fixture = json!({
        "schema_version": 1,
        "receipt_id": receipt_id.clone(),
        "op": "memory.exports.create",
        "space_id": SPACE,
        "requester_principal_id": TRANSFER,
        "idempotency_key": "reopen-failed",
        "payload_digest": digest.clone(),
        "status": "failed",
        "created_at": created["created_at"].clone(),
        "completed_at": sqlite_memory::CLOCK_ORIGIN,
        "error": {
            "code": "INTERNAL_ERROR",
            "message": "private failed-state fixture",
            "details": { "retry_class": "retryable" },
            "extensions": {},
        },
        "extensions": {},
    });
    // The private fixture must be a legal generated receipt before any
    // lifecycle control relies on it. Keep its typed form: it is the whole
    // expected value of the replay below, not just an id/status pair.
    let expected_receipt = serde_json::from_value::<spoke_schemas::MemoryReceipt>(fixture.clone())
        .expect("fixture is a legal generated receipt");
    match &expected_receipt {
        spoke_schemas::MemoryReceipt::FailedReceipt(_) => {}
        other => panic!("fixture must decode as a failed receipt, got {other:?}"),
    }
    provider.provision_receipt(SPACE, fixture);
    // The terminal-body fixture must not substitute a synthetic scope: the
    // complete binding (id, digest, scope) is byte-for-byte the admitted one.
    let after_fixture: Value =
        serde_json::from_str(&stored_document(&path)).expect("state after fixture");
    let binding_after_fixture = after_fixture["spaces"][SPACE]["dedup"]
        .as_object()
        .expect("dedup map")
        .values()
        .find(|entry| entry["receipt_id"] == json!(receipt_id))
        .expect("the real export binding survives the fixture");
    assert_eq!(*binding_after_fixture, binding_before);

    // A reflection admitted after the export pins an unrestricted (`subject_all`)
    // basis outside `TRANSFER`'s narrow grant. A recomputed scope would absorb
    // it and deny the whole request; the stored original scope must not.
    provider.provision_reflection(
        SPACE,
        json!({
            "schema_version": 1,
            "reflection_id": "refl_failed_late_all_scope",
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": { "mode": "subject_all" },
            "basis": {
                "source_set_digest": "0".repeat(64),
                "record_count": 1,
                "profile_revisions": {},
                "state_digest": "1".repeat(64),
            },
            "body": { "format": "markdown", "body": "Derived from every placement." },
            "stale": false,
            "generator": { "engine_policy_version": ENGINE_POLICY_VERSION },
            "created_at": "2026-07-23T07:00:00Z",
            "revision": 1,
            "extensions": {},
        }),
    );
    // The matching replay answers the real `failed` receipt body. It passed the
    // stored scope's requirement while the fresh control below is denied, so a
    // recompute-on-replay bug would fail here with `NotAuthorized`. The whole
    // response is compared, not just id and status: the replay's owner, key,
    // digest, timestamps and error body must all match the provisioned failed
    // fixture. Both sides are read through the same public typed receipt, so
    // the comparison is not a raw wire-shape artefact.
    let replayed = expect_ok(
        provider
            .create_export(
                &principal(TRANSFER),
                serde_json::from_value(request.clone()).expect("export request"),
            )
            .await,
    );
    let replayed_receipt: spoke_schemas::MemoryReceipt =
        serde_json::from_value(wire(&replayed)).expect("replay answers a typed receipt");
    assert_eq!(
        wire(&replayed_receipt),
        wire(&expected_receipt),
        "the replay answers the complete provisioned failed receipt: {replayed:?}"
    );

    // The late coverage really widens a newly computed scope: the equivalent
    // fresh request is denied under the current narrow grant and creates no
    // second binding.
    let mut fresh = request.clone();
    fresh["idempotency_key"] = json!("reopen-failed-new");
    assert_eq!(
        expect_reject(
            provider
                .create_export(
                    &principal(TRANSFER),
                    serde_json::from_value(fresh.clone()).expect("export request"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );

    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);

    // Eviction copies the body's truthful `failed` status to the tombstone.
    expect_receipt_expired_as(read_receipt(&provider, TRANSFER, &receipt_id).await, "failed");
    assert!(provider.poll_receipt_value(SPACE, &receipt_id).is_none());
    drop(provider);

    let reopened = SqliteMemoryProvider::open(&path).expect("store reopens");
    let after_reopen = stored_document(&path);
    let reopened_state: Value = serde_json::from_str(&after_reopen).expect("reopened state");
    let binding_after_reopen = reopened_state["spaces"][SPACE]["dedup"]
        .as_object()
        .expect("dedup map")
        .values()
        .find(|entry| entry["receipt_id"] == json!(receipt_id))
        .expect("the admitted binding survives the reopen");
    assert_eq!(*binding_after_reopen, binding_before);
    expect_receipt_expired_as(read_receipt(&reopened, TRANSFER, &receipt_id).await, "failed");
    expect_receipt_expired_as(
        reopened
            .create_export(
                &principal(TRANSFER),
                serde_json::from_value(request).expect("export request"),
            )
            .await,
        "failed",
    );
    assert_eq!(
        expect_reject(
            reopened
                .create_export(
                    &principal(TRANSFER),
                    serde_json::from_value(fresh).expect("export request"),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized
    );
    // Neither path re-executed: no new receipt body and no state change.
    assert!(reopened.poll_receipt_value(SPACE, &receipt_id).is_none());
    assert_eq!(stored_document(&path), after_reopen);

    std::fs::remove_file(&path).expect("remove state file");
}

/// F2/R022: owed work alone proves `pending` after eviction and a reopen. The
/// body is evicted first, then the retained status is dropped, so only the
/// durable owed work can explain the receipt — never an invented commit.
#[tokio::test]
async fn owed_work_pending_survives_reopen_without_retained_status() {
    let path = temp_state_path("pending-tombstone");
    let provider = seeded(SqliteMemoryProvider::open(&path).expect("fresh store initializes"));
    seed_record(
        &provider,
        "rec_pending",
        "A tally the worker settles later.",
    );
    let request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "idempotency_key": "reopen-pending",
    });
    let created = wire(&expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(request.clone()).expect("export request"),
            )
            .await,
    ));
    let receipt_id = created["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();
    assert_eq!(created["status"], "pending", "{created}");
    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);

    // Eviction copies `pending` from the body; drop that retained status so
    // only the owed work remains as evidence. (`poll_receipt_value` would run
    // the worker and settle it, so it is deliberately not used here.)
    expect_ok(
        provider
            .list_grants(
                &principal(OPERATOR),
                serde_json::from_value(json!({ "space_id": SPACE, "limit": 10 }))
                    .expect("list request"),
            )
            .await,
    );
    provider.forget_receipt_evidence_statuses();
    drop(provider);

    let reopened = SqliteMemoryProvider::open(&path).expect("store reopens");
    expect_receipt_expired_as(
        read_receipt(&reopened, OPERATOR, &receipt_id).await,
        "pending",
    );
    expect_receipt_expired_as(
        reopened
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(request.clone()).expect("export request"),
            )
            .await,
        "pending",
    );

    // Settlement records the proven outcome and drops the owed work.
    reopened.run_worker();
    expect_receipt_expired_as(
        read_receipt(&reopened, OPERATOR, &receipt_id).await,
        "committed",
    );
    expect_receipt_expired_as(
        reopened
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(request).expect("export request"),
            )
            .await,
        "committed",
    );

    std::fs::remove_file(&path).expect("remove state file");
}

/// MEM-R022/§6.10: eviction must not delete a stale body whose truthful
/// outcome cannot be retained. Both a missing and a wrong-typed body status
/// fail closed with the existing read error before any write, keeping the
/// stored document — body included — byte for byte.
#[tokio::test]
async fn eviction_without_retainable_evidence_fails_closed_and_keeps_bytes() {
    for (label, status) in [
        ("missing-status", None::<Value>),
        ("wrong-type-status", Some(json!(7))),
    ] {
        let path = temp_state_path(label);
        let provider = seeded(SqliteMemoryProvider::open(&path).expect("fresh store initializes"));
        let receipt_id = format!("rcpt_corrupt_{label}");
        let mut receipt = json!({
            "schema_version": 1,
            "receipt_id": receipt_id.clone(),
            "op": "memory.exports.create",
            "space_id": SPACE,
            "requester_principal_id": OPERATOR,
            "payload_digest": "sha256:corrupt",
            "created_at": sqlite_memory::CLOCK_ORIGIN,
            "extensions": {},
        });
        if let Some(status) = status {
            receipt["status"] = status;
        }
        provider.provision_receipt(SPACE, receipt);
        provider.provision_dedup_entry(
            SPACE,
            OPERATOR,
            "memory.exports.create",
            "corrupt-key",
            &receipt_id,
            "sha256:corrupt",
        );
        provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);
        let corrupt_bytes = stored_document(&path);

        let reject = expect_reject_with(
            provider
                .list_grants(
                    &principal(OPERATOR),
                    serde_json::from_value(json!({ "space_id": SPACE, "limit": 10 }))
                        .expect("list request"),
                )
                .await,
        );
        assert_eq!(
            reject.code,
            SpokeRejectCode::InternalError,
            "{label}: {reject:?}"
        );
        assert_eq!(
            reject
                .details
                .as_ref()
                .and_then(|details| details.get("retry_class")),
            Some(&json!("retryable")),
            "{label}: {reject:?}"
        );
        assert!(
            receipt_detail_paths(reject.details.as_ref()).is_empty(),
            "{label}: {reject:?}"
        );
        // Nothing was evicted or written: the exact bytes and the body remain.
        assert_eq!(stored_document(&path), corrupt_bytes, "{label}");
        assert!(
            provider.poll_receipt_value(SPACE, &receipt_id).is_some(),
            "{label}: last body must survive a refused eviction"
        );

        std::fs::remove_file(&path).expect("remove state file");
    }
}

/// MEM-R022: a stale body without its own status is removable when the
/// lifecycle evidence already retains a truthful status, or when owed work
/// proves `pending` — and the answer still reports that evidence.
#[tokio::test]
async fn eviction_accepts_retained_status_or_owed_work_without_body_status() {
    // (1) The evidence already retained the truthful terminal status. A body
    // without its own status is only reachable through deliberate private-state
    // corruption, so the fixture writes that exact shape.
    let path = temp_state_path("retained-evidence");
    let provider = seeded(SqliteMemoryProvider::open(&path).expect("fresh store initializes"));
    provider.provision_receipt(
        SPACE,
        json!({
            "schema_version": 1,
            "receipt_id": "rcpt_retained_status",
            "op": "memory.exports.create",
            "space_id": SPACE,
            "requester_principal_id": OPERATOR,
            "payload_digest": "sha256:retained",
            "created_at": sqlite_memory::CLOCK_ORIGIN,
            "extensions": {},
        }),
    );
    let mut document: Value =
        serde_json::from_str(&stored_document(&path)).expect("stored document is json");
    document["spaces"][SPACE]["receipt_evidence"]["rcpt_retained_status"]["status"] =
        json!("committed");
    write_stored_document(
        &path,
        &serde_json::to_string(&document).expect("document serializes"),
    );
    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);
    expect_receipt_expired_as(
        read_receipt(&provider, OPERATOR, "rcpt_retained_status").await,
        "committed",
    );
    assert!(provider
        .poll_receipt_value(SPACE, "rcpt_retained_status")
        .is_none());
    std::fs::remove_file(&path).expect("remove state file");

    // (2) Owed work alone proves `pending` for a wrong-typed body status.
    let provider = transfer_space();
    seed_record(&provider, "rec_owed", "A tally the worker still owes.");
    let created = wire(&expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "idempotency_key": "owed-key",
                }))
                .expect("export request"),
            )
            .await,
    ));
    let receipt_id = created["receipt_id"]
        .as_str()
        .expect("receipt id")
        .to_string();
    provider.provision_receipt(
        SPACE,
        json!({
            "schema_version": 1,
            "receipt_id": receipt_id.clone(),
            "op": "memory.exports.create",
            "space_id": SPACE,
            "requester_principal_id": OPERATOR,
            "payload_digest": "sha256:owed",
            "status": 9,
            "created_at": sqlite_memory::CLOCK_ORIGIN,
            "extensions": {},
        }),
    );
    provider.forget_receipt_evidence_statuses();
    provider.advance_clock(sqlite_memory::RECEIPT_RETENTION_SECONDS + 1);
    expect_receipt_expired_as(
        read_receipt(&provider, OPERATOR, &receipt_id).await,
        "pending",
    );
    assert!(provider.poll_receipt_value(SPACE, &receipt_id).is_none());
}

/// MEM-R022 write invariant: a receipt with no status is rejected at the
/// persistence boundary before any write, so no partial receipt, dedup entry
/// or owed work is persisted.
#[tokio::test]
async fn missing_status_remember_writes_no_partial_state() {
    let path = temp_state_path("statusless-remember");
    let provider = seeded(SqliteMemoryProvider::open(&path).expect("fresh store initializes"));
    let before = stored_document(&path);

    let reject = provider
        .attempt_statusless_remember(SPACE)
        .expect("statusless receipt must be rejected");
    assert_eq!(reject.code, SpokeRejectCode::InternalError, "{reject:?}");
    assert_eq!(
        stored_document(&path),
        before,
        "rejected mutation wrote state"
    );
    assert!(provider
        .poll_receipt_value(SPACE, "rcpt_statusless")
        .is_none());
    let space = space_row(&provider).await;
    assert_eq!(space["pending_receipts"], 0, "{space}");

    std::fs::remove_file(&path).expect("remove state file");
}

/// §14.2/§14.3: a public mutation whose store write fails cannot claim a clean
/// no-effect outcome — the commit state is not established. The fault is a
/// plain SQLite trigger (no provider fault hook); the aborted write leaves the
/// stored bytes intact and service resumes once it is removed.
#[tokio::test]
async fn mutation_write_failure_reports_uncertain_outcome() {
    let path = temp_state_path("write-uncertain");
    let provider = seeded(SqliteMemoryProvider::open(&path).expect("fresh store initializes"));
    seed_record(
        &provider,
        "rec_write_fault",
        "A tally that must survive a failed write.",
    );

    let request = json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "idempotency_key": "exp-write-uncertain",
    });
    let fault = rusqlite::Connection::open(&path).expect("open state db");
    fault
        .execute_batch(
            "CREATE TRIGGER memory_state_insert_fault BEFORE INSERT ON memory_state
             BEGIN SELECT RAISE(ABORT, 'injected write failure'); END;
             CREATE TRIGGER memory_state_update_fault BEFORE UPDATE ON memory_state
             BEGIN SELECT RAISE(ABORT, 'injected write failure'); END;",
        )
        .expect("install write fault");
    let before = stored_document(&path);

    let reject = expect_reject_with(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(request.clone()).expect("export request"),
            )
            .await,
    );
    assert_eq!(
        reject.code,
        SpokeRejectCode::OutcomeUncertain,
        "{reject:?}"
    );
    assert_eq!(
        reject
            .details
            .as_ref()
            .and_then(|details| details.get("retry_class")),
        Some(&json!("uncertain")),
        "{reject:?}"
    );
    // The aborting statement committed nothing.
    assert_eq!(stored_document(&path), before);

    fault
        .execute_batch(
            "DROP TRIGGER memory_state_insert_fault;
             DROP TRIGGER memory_state_update_fault;",
        )
        .expect("remove write fault");
    drop(fault);

    // The same public call now succeeds: the store and lock stay usable.
    let created = wire(&expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(request).expect("export request"),
            )
            .await,
    ));
    assert_eq!(created["status"], "pending", "{created}");

    std::fs::remove_file(&path).expect("remove state file");
}

/// A unique on-disk state path; each test removes its own file.
fn temp_state_path(label: &str) -> PathBuf {
    let unique = STATE_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "spoke-memory-{label}-{}-{unique}.sqlite3",
        std::process::id()
    ))
}

static STATE_FILE_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

fn stored_document(path: &Path) -> String {
    rusqlite::Connection::open(path)
        .expect("open state db")
        .query_row("SELECT document FROM memory_state WHERE id = 1", [], |row| {
            row.get(0)
        })
        .expect("stored document")
}

fn write_stored_document(path: &Path, document: &str) {
    rusqlite::Connection::open(path)
        .expect("open state db")
        .execute(
            "UPDATE memory_state SET document = ?1 WHERE id = 1",
            rusqlite::params![document],
        )
        .expect("write stored document");
}

/// Shared C9 query vectors on the typed SQLite provider: the same file the
/// reference engine runner consumes. The provider must produce the exact
/// eligible ids/counts, page ids, cursor continuations, overflow first record,
/// cognition-bearing byte boundary and reflection bases pinned by the vector.
#[tokio::test]
async fn cognition_query_shared_vectors_provider() {
    use spoke_schemas::data::retained_record::Content;
    use spoke_schemas::{
        MemoryRecallRequest, MemoryRecordsListRequest, MemoryReflectionsGenerateRequest,
        MemoryReflectionsListRequest, MemoryReflectionsStateRequest,
    };

    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/cognition/memory-query-vectors.json"
    );
    let file: Value = serde_json::from_str(
        &std::fs::read_to_string(path).expect("memory-query-vectors.json readable"),
    )
    .expect("memory-query-vectors.json parses");
    let provider = provider();
    provider.provision_profile(
        SPACE,
        json!({
            "schema_version": 1,
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "profile_kind": "personality",
            "document": {},
            "revision": 1,
        }),
    );
    let mut provisioned: std::collections::BTreeMap<String, Value> =
        std::collections::BTreeMap::new();
    for record in file["records"].as_array().expect("records") {
        let mut stored = record.clone();
        stored["schema_version"] = json!(1);
        stored["space_id"] = json!(SPACE);
        stored["subject_id"] = json!(SUBJECT);
        stored["scope"] = json!({ "visibility": "shared" });
        stored["provenance"] = json!({
            "source_context_ids": ["ctx_vector"],
            "anchors": [],
            "captured_by": OPERATOR,
            "captured_at": "2026-07-23T06:00:00Z",
        });
        stored["updated_at"] = stored["created_at"].clone();
        stored["extensions"] = json!({});
        provider.provision_record(SPACE, stored.clone());
        provisioned.insert(
            stored["record_id"].as_str().expect("record_id").to_string(),
            stored,
        );
    }

    let recall_request = |filter: Option<&Value>,
                          cursor: Option<&str>,
                          max_items: u64,
                          max_bytes: Option<u64>| {
        let mut request = json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": { "mode": "shared_only" },
            "budget": { "max_items": max_items },
        });
        if let Some(filter) = filter {
            request["cognition_filter"] = filter.clone();
        }
        if let Some(cursor) = cursor {
            request["cursor"] = json!(cursor);
        }
        if let Some(bytes) = max_bytes {
            request["budget"]["max_bytes"] = json!(bytes);
        }
        serde_json::from_value::<MemoryRecallRequest>(request).expect("recall request")
    };
    let list_request = |filter: &Value, cursor: Option<&str>, limit: u64, kinds: Option<Value>| {
        let mut request = json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": { "mode": "shared_only" },
            "cognition_filter": filter.clone(),
            "limit": limit,
        });
        if let Some(cursor) = cursor {
            request["cursor"] = json!(cursor);
        }
        if let Some(kinds) = kinds {
            request["kinds"] = kinds;
        }
        serde_json::from_value::<MemoryRecordsListRequest>(request).expect("list request")
    };

    let queries = file["queries"].as_array().expect("queries");
    let mut cursors: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for query in queries {
        let id = query["id"].as_str().expect("query id").to_string();
        let expect = &query["expect"];
        let filter = query.get("filter").cloned();
        let max_items = query["budget"]["max_items"]
            .as_u64()
            .expect("max_items");
        let max_bytes = query["budget"]["max_bytes"].as_u64();
        let cursor = query["after"]
            .as_str()
            .and_then(|previous| cursors.get(previous))
            .map(String::as_str);
        let recall = expect_ok(
            provider
                .recall(
                    &principal(OPERATOR),
                    recall_request(filter.as_ref(), cursor, max_items, max_bytes),
                )
                .await,
        );
        let recall = wire(&recall);
        let page_ids: Vec<&str> = recall["records"]
            .as_array()
            .expect("records")
            .iter()
            .map(|record| record["record_id"].as_str().expect("record_id"))
            .collect();
        assert_eq!(json!(page_ids), expect["page_ids"], "{id}: recall page");
        assert_eq!(
            recall["total_authorized"], expect["total_authorized"],
            "{id}: total_authorized"
        );
        assert_eq!(
            recall["truncated"], expect["truncated"],
            "{id}: truncated"
        );
        assert_eq!(
            recall.get("overflow_record_id").and_then(Value::as_str),
            expect.get("overflow_record_id").and_then(Value::as_str),
            "{id}: overflow_record_id"
        );
        if let Some(next) = recall["next_cursor"].as_str() {
            cursors.insert(id.clone(), next.to_string());
        }

        if let Some(filter) = &filter {
            let list = expect_ok(
                provider
                    .list_records(
                        &principal(OPERATOR),
                        list_request(filter, None, 100, None),
                    )
                    .await,
            );
            let list = wire(&list);
            let list_ids: Vec<&str> = list["records"]
                .as_array()
                .expect("records")
                .iter()
                .map(|record| record["record_id"].as_str().expect("record_id"))
                .collect();
            assert_eq!(
                json!(list_ids), expect["eligible_ids"],
                "{id}: list eligibility"
            );
            assert_eq!(
                list["total_authorized"], expect["total_authorized"],
                "{id}: list total_authorized"
            );
        }
    }

    // A changed selection-affecting member invalidates cursor reuse (MEM-R026).
    let page_filter = queries
        .iter()
        .find(|query| query["id"] == "page-first")
        .and_then(|query| query.get("filter"))
        .cloned()
        .expect("page-first filter");
    let other_filter = queries
        .iter()
        .find(|query| query["id"] == "eligible-conjunction")
        .and_then(|query| query.get("filter"))
        .cloned()
        .expect("eligible-conjunction filter");
    let recall_cursor = cursors["page-first"].clone();
    assert_eq!(
        expect_reject(
            provider
                .recall(
                    &principal(OPERATOR),
                    recall_request(Some(&other_filter), Some(&recall_cursor), 2, None),
                )
                .await
        ),
        SpokeRejectCode::CursorInvalid,
        "a changed cognition filter must invalidate a recall cursor"
    );

    let first_list = expect_ok(
        provider
            .list_records(
                &principal(OPERATOR),
                list_request(&page_filter, None, 2, None),
            )
            .await,
    );
    let first_list = wire(&first_list);
    // List eligibility is id-sorted, so page 1 is the exact known prefix of the
    // vector's eligible list — not merely the right length.
    let page_eligible: Vec<Value> = queries
        .iter()
        .find(|query| query["id"] == "page-first")
        .and_then(|query| query["expect"]["eligible_ids"].as_array().cloned())
        .expect("page-first eligible ids");
    assert_eq!(
        json!(first_list["records"]
            .as_array()
            .expect("records")
            .iter()
            .map(|record| record["record_id"].clone())
            .collect::<Vec<Value>>()),
        json!(page_eligible[..2].to_vec()),
        "list page 1 is the exact id-sorted prefix"
    );
    assert_eq!(first_list["total_authorized"], 3);
    let list_cursor = first_list["next_cursor"]
        .as_str()
        .expect("list page 1 carries a cursor")
        .to_string();
    let second_list = expect_ok(
        provider
            .list_records(
                &principal(OPERATOR),
                list_request(&page_filter, Some(&list_cursor), 2, None),
            )
            .await,
    );
    let second_list = wire(&second_list);
    let second_list_ids: Vec<&str> = second_list["records"]
        .as_array()
        .expect("records")
        .iter()
        .map(|record| record["record_id"].as_str().expect("record_id"))
        .collect();
    assert_eq!(json!(second_list_ids), json!(["rec_cog_full"]));
    assert!(
        second_list.get("next_cursor").is_none(),
        "the last page carries no cursor"
    );
    assert_eq!(
        expect_reject(
            provider
                .list_records(
                    &principal(OPERATOR),
                    list_request(&other_filter, Some(&list_cursor), 2, None),
                )
                .await
        ),
        SpokeRejectCode::CursorInvalid,
        "a changed cognition filter must invalidate a list cursor"
    );
    assert_eq!(
        expect_reject(
            provider
                .list_records(
                    &principal(OPERATOR),
                    list_request(&page_filter, Some(&list_cursor), 2, Some(json!(["note"]))),
                )
                .await
        ),
        SpokeRejectCode::CursorInvalid,
        "a changed kind filter must invalidate a list cursor"
    );

    // The emitted byte measure includes cognition: sizing the page budget to
    // the same content WITHOUT cognition must still overflow on the first,
    // cognition-bearing record.
    let first_record = file["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|record| record["record_id"] == "rec_cog_alice_high")
        .expect("first ranked record");
    let with_cognition: Content = serde_json::from_value(first_record["content"].clone())
        .expect("cognition-bearing content");
    let mut without = first_record["content"].clone();
    without
        .as_object_mut()
        .expect("content object")
        .remove("cognition");
    let without_cognition: Content =
        serde_json::from_value(without).expect("text-only content");
    let with_bytes = serde_json::to_vec(&with_cognition).expect("serialize").len();
    let without_bytes = serde_json::to_vec(&without_cognition).expect("serialize").len();
    assert!(
        with_bytes > without_bytes,
        "the cognition member contributes emitted bytes"
    );
    let bounded = expect_ok(
        provider
            .recall(
                &principal(OPERATOR),
                recall_request(Some(&page_filter), None, 10, Some(without_bytes as u64)),
            )
            .await,
    );
    let bounded = wire(&bounded);
    assert_eq!(
        bounded["overflow_record_id"],
        json!("rec_cog_alice_high"),
        "the oversized first cognition-bearing record is surfaced whole"
    );
    assert_eq!(
        bounded["records"].as_array().expect("records").len(),
        1,
        "the overflow page is the first record only"
    );
    assert_eq!(bounded["truncated"], json!(true));
    // The oversized first record is returned WHOLE: its identity, revision,
    // attribution and cognition are the stored values, never an id or a length.
    let whole = &bounded["records"][0];
    assert_eq!(
        whole, &provisioned["rec_cog_alice_high"],
        "the oversized record is the complete stored record"
    );
    let whole_content: Content = serde_json::from_value(whole["content"].clone())
        .expect("the whole content is the typed Content");
    assert_eq!(
        whole_content.text, "Alice expects the gate to be shut.",
        "the emitted content keeps its text"
    );
    let entries: &[spoke_schemas::retained_record::CognitionRecord] = whole_content
        .cognition
        .as_deref()
        .expect("recorded-at-time cognition is present");
    assert_eq!(entries.len(), 1, "every recorded entry survives whole");
    let entry = &whole["content"]["cognition"][0];
    assert_eq!(entry["order"], json!(1));
    assert_eq!(entry["target"]["target_id"], json!("t_prop_1"));
    assert_eq!(entry["assessor"]["id"], json!("alice"));
    assert_eq!(whole["provenance"]["captured_by"], json!(OPERATOR));

    // Reflection basis: pinned filter identity + the same eligible id+revision
    // set; reflections list only those generated with the supplied filter.
    let reflection_vectors = &file["reflection"];
    let filters = &reflection_vectors["filters"];
    let state_request = |filter: &Value| {
        serde_json::from_value::<MemoryReflectionsStateRequest>(json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": { "mode": "subject_all" },
            "cognition_filter": filter.clone(),
        }))
        .expect("state request")
    };
    let narrow_state = wire(
        &expect_ok(
            provider
                .get_reflection_state(&principal(OPERATOR), state_request(&filters["narrow"]))
                .await,
        ),
    );
    let same_state = wire(
        &expect_ok(
            provider
                .get_reflection_state(&principal(OPERATOR), state_request(&filters["same_sources"]))
                .await,
        ),
    );
    let disjoint_state = wire(
        &expect_ok(
            provider
                .get_reflection_state(&principal(OPERATOR), state_request(&filters["disjoint"]))
                .await,
        ),
    );
    assert_eq!(narrow_state["basis"]["record_count"], 3);
    assert_eq!(same_state["basis"]["record_count"], 3);
    assert_eq!(disjoint_state["basis"]["record_count"], 1);
    assert_ne!(
        narrow_state["state_digest"], same_state["state_digest"],
        "the filter identity is pinned even for an equal source set"
    );
    assert_ne!(narrow_state["state_digest"], disjoint_state["state_digest"]);

    // Independently known basis: the vector's declared eligible ids plus each
    // record's declared revision fed to the canonical digest helper — never the
    // provider response echoed back at itself.
    let revision_of = |record_id: &str| -> u64 {
        file["records"]
            .as_array()
            .expect("records")
            .iter()
            .find(|record| record["record_id"] == record_id)
            .and_then(|record| record["revision"].as_u64())
            .expect("declared record revision")
    };
    let declared_ids = |value: &Value| -> Vec<String> {
        value
            .as_array()
            .expect("declared ids")
            .iter()
            .map(|id| id.as_str().expect("record id").to_string())
            .collect()
    };
    let declared_profiles: Vec<(&str, u64)> = reflection_vectors["profile_revisions"]
        .as_object()
        .expect("profile revisions")
        .iter()
        .map(|(kind, revision)| (kind.as_str(), revision.as_u64().expect("revision")))
        .collect();
    let expected_reflection = |ids: Vec<String>, filter: &Value| {
        let sources: Vec<(&str, u64)> = ids
            .iter()
            .map(|id| (id.as_str(), revision_of(id)))
            .collect();
        expected_basis(
            reflection_vectors["selection"].clone(),
            None,
            Some(filter.clone()),
            &sources,
            &declared_profiles,
        )
    };
    let narrow_expected = expected_reflection(
        declared_ids(&reflection_vectors["expect"]["narrow_eligible_ids"]),
        &filters["narrow"],
    );
    let same_expected = expected_reflection(
        declared_ids(&reflection_vectors["expect"]["same_sources_eligible_ids"]),
        &filters["same_sources"],
    );
    let disjoint_expected = expected_reflection(
        declared_ids(&reflection_vectors["expect"]["disjoint_eligible_ids"]),
        &filters["disjoint"],
    );
    assert_eq!(
        narrow_state["state_digest"],
        json!(narrow_expected.0),
        "independent basis for the narrow filter"
    );
    assert_eq!(same_state["state_digest"], json!(same_expected.0));
    assert_eq!(disjoint_state["state_digest"], json!(disjoint_expected.0));
    assert_eq!(
        narrow_expected.1, same_expected.1,
        "equal eligible id+revision sets keep one source-set digest"
    );
    assert_ne!(
        narrow_expected.1, disjoint_expected.1,
        "a disjoint source set moves the source-set digest"
    );

    let generate = |filter: &Value, digest: &str, key: &str| {
        serde_json::from_value::<MemoryReflectionsGenerateRequest>(json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": { "mode": "subject_all" },
            "cognition_filter": filter.clone(),
            "expected_state_digest": digest,
            "idempotency_key": key,
        }))
        .expect("generate request")
    };
    for (filter, digest, key) in [
        (
            &filters["narrow"],
            narrow_state["state_digest"]
                .as_str()
                .expect("state digest")
                .to_string(),
            "vec-gen-narrow",
        ),
        (
            &filters["same_sources"],
            same_state["state_digest"]
                .as_str()
                .expect("state digest")
                .to_string(),
            "vec-gen-same-sources",
        ),
    ] {
        let generated = expect_ok(
            provider
                .generate_reflection(&principal(OPERATOR), generate(filter, &digest, key))
                .await,
        );
        assert_eq!(wire(&generated)["status"], json!("pending"));
    }
    provider.run_worker();

    let all = expect_ok(
        provider
            .list_reflections(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryReflectionsListRequest>(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "limit": 10,
                }))
                .expect("list reflections"),
            )
            .await,
    );
    let all = wire(&all);
    let reflections = all["reflections"].as_array().expect("reflections");
    assert_eq!(reflections.len(), 2, "{all}");
    let narrow_reflection = reflections
        .iter()
        .find(|reflection| reflection["cognition_filter"] == filters["narrow"])
        .expect("narrow reflection pins its filter");
    let same_reflection = reflections
        .iter()
        .find(|reflection| reflection["cognition_filter"] == filters["same_sources"])
        .expect("same-sources reflection pins its filter");
    assert_eq!(
        narrow_reflection["basis"]["state_digest"], narrow_state["state_digest"],
        "the stored basis is the state digest the request pinned"
    );
    assert_eq!(
        same_reflection["basis"]["state_digest"], same_state["state_digest"]
    );
    assert_eq!(narrow_reflection["basis"]["record_count"], 3);
    // The PUBLISHED source-set digest, not two provider responses judged
    // against each other: each equals its independently declared id+revision
    // set fed to the canonical digest helper.
    assert_eq!(
        narrow_reflection["basis"]["source_set_digest"],
        json!(&narrow_expected.1),
        "the published narrow source-set digest is the independently expected one"
    );
    assert_eq!(
        same_reflection["basis"]["source_set_digest"],
        json!(&same_expected.1),
        "the published same-sources source-set digest is the independently expected one"
    );

    let narrow_only = expect_ok(
        provider
            .list_reflections(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryReflectionsListRequest>(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "cognition_filter": filters["narrow"].clone(),
                    "limit": 10,
                }))
                .expect("list reflections"),
            )
            .await,
    );
    let narrow_only = wire(&narrow_only);
    assert_eq!(
        narrow_only["reflections"].as_array().expect("reflections").len(),
        1,
        "a supplied filter lists only reflections generated with it"
    );
    let disjoint_only = expect_ok(
        provider
            .list_reflections(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryReflectionsListRequest>(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "cognition_filter": filters["disjoint"].clone(),
                    "limit": 10,
                }))
                .expect("list reflections"),
            )
            .await,
    );
    assert_eq!(
        wire(&disjoint_only)["reflections"]
            .as_array()
            .expect("reflections")
            .len(),
        0
    );

    // Authorization precedes source output, and an unimplemented predicate
    // rejects through the canonical C5 grammar instead of being stripped.
    assert_eq!(
        expect_reject(
            provider
                .list_records(
                    &principal(READER),
                    list_request(&page_filter, None, 10, None),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized,
        "the context-limited reader cannot read shared records"
    );
    let invalid_filter = json!({ "q": { "min": 1.5 } });
    assert_eq!(
        expect_reject(
            provider
                .recall(
                    &principal(OPERATOR),
                    recall_request(Some(&invalid_filter), None, 2, None),
                )
                .await
        ),
        SpokeRejectCode::InvalidInput,
        "an out-of-range q predicate is unsupported, never silently stripped"
    );
}

// ---------------------------------------------------------------------------
// C9 source-root regressions (§7 items 1–7): one eligible set, op-bound
// cursors, provenance-carrying reflection basis, and reflective membership.
// ---------------------------------------------------------------------------

/// A minimal stored retained record for eligibility-level proof.
fn stored_record(
    record_id: &str,
    kind: &str,
    created_at: &str,
    scope: Value,
    source_context_ids: &[&str],
    content: Value,
) -> Value {
    json!({
        "schema_version": 1,
        "record_id": record_id,
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "kind": kind,
        "content": content,
        "scope": scope,
        "provenance": {
            "source_context_ids": source_context_ids,
            "anchors": [],
            "captured_by": OPERATOR,
            "captured_at": "2026-07-23T06:00:00Z",
        },
        "created_at": created_at,
        "updated_at": created_at,
        "revision": 1,
        "extensions": {},
    })
}

fn shared_scope() -> Value {
    json!({ "visibility": "shared" })
}

fn context_scope(context_id: &str) -> Value {
    json!({ "visibility": "context", "context_id": context_id })
}

/// Independently known basis digests: the caller supplies the exact sorted
/// `(record_id, revision)` set and profile revisions, so the assertion never
/// echoes a provider response back at itself.
fn expected_basis(
    selection: Value,
    filter: Option<Value>,
    cognition_filter: Option<Value>,
    sources: &[(&str, u64)],
    profiles: &[(&str, u64)],
) -> (String, String) {
    let selection: ReadSelection = serde_json::from_value(selection).expect("selection parses");
    let filter: Option<ProvenanceFilter> =
        filter.map(|value| serde_json::from_value(value).expect("provenance filter parses"));
    let cognition_filter: Option<CognitionFilter> = cognition_filter
        .map(|value| serde_json::from_value(value).expect("cognition filter parses"));
    let source_refs: Vec<SourceRef<'_>> = sources
        .iter()
        .map(|(record_id, revision)| SourceRef {
            record_id,
            revision: *revision,
        })
        .collect();
    let profile_refs: Vec<(&str, u64)> = profiles.to_vec();
    let basis = ReflectionBasis {
        selection: &selection,
        filter: filter.as_ref(),
        cognition_filter: cognition_filter.as_ref(),
        sources: &source_refs,
        profile_revisions: &profile_refs,
    };
    let digests = basis_digests(&basis);
    (digests.state_digest, digests.source_set_digest)
}

fn belief_at(id: &str, holder: &str, q: f64) -> Value {
    let mut belief = cognition_belief(id, holder, "2026-07-23T06:00:00Z", q);
    belief["holder"]["id"] = json!(holder);
    belief
}

fn holder_filter(holder: &str, min: f64) -> Value {
    json!({
        "holder": { "kind": "principal", "namespace": "system_a", "id": holder },
        "q": { "min": min }
    })
}

/// §7 items 1–2: tombstoned records leave the one eligible set before counts,
/// ranking, budgets and the reflection basis — with or without a cognition
/// filter — while an explicitly authorized admin display still works and an
/// ordinary covered reader never sees tombstone metadata.
#[tokio::test]
async fn cognition_tombstones_leave_the_eligible_set_and_stay_admin_only() {
    let provider = provider();
    let alice = holder_filter("alice", 0.5);
    provider.provision_profile(
        SPACE,
        json!({
            "schema_version": 1,
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "profile_kind": "personality",
            "document": {},
            "revision": 1,
        }),
    );
    provider.provision_record(
        SPACE,
        stored_record(
            "rec_live",
            "fact",
            "2026-07-23T06:00:00Z",
            shared_scope(),
            &["ctx_alpha"],
            json!({ "text": "Alice expects the gate shut.", "cognition": [belief_at("bel_live", "alice", 0.9)] }),
        ),
    );
    provider.provision_record(
        SPACE,
        stored_record(
            "rec_dead",
            "fact",
            "2026-07-23T06:00:00Z",
            shared_scope(),
            &["ctx_alpha"],
            json!({ "text": "Alice expects the gate open.", "cognition": [belief_at("bel_dead", "alice", 0.9)] }),
        ),
    );
    provider.provision_record(
        SPACE,
        stored_record(
            "rec_ctx_dead",
            "fact",
            "2026-07-23T06:00:00Z",
            context_scope(CONTEXT),
            &[CONTEXT],
            json!({ "text": "The gate key is under the barrel.", "cognition": [belief_at("bel_ctx_dead", "alice", 0.9)] }),
        ),
    );
    for (record_id, expected_revision, key) in [
        ("rec_dead", 1, "del-rec-dead"),
        ("rec_ctx_dead", 1, "del-rec-ctx-dead"),
    ] {
        let deleted = expect_ok(
            provider
                .delete_record(
                    &principal(OPERATOR),
                    serde_json::from_value(json!({
                        "space_id": SPACE,
                        "record_id": record_id,
                        "expected_revision": expected_revision,
                        "idempotency_key": key,
                    }))
                    .expect("delete request"),
                )
                .await,
        );
        assert_eq!(wire(&deleted)["result"]["tombstoned"], true, "{record_id}");
    }

    let recall_of = |request: Value| {
        serde_json::from_value::<MemoryRecallRequest>(request).expect("recall request")
    };
    let list_of = |request: Value| {
        serde_json::from_value::<MemoryRecordsListRequest>(request).expect("list request")
    };
    let ids_of = |value: &Value| -> Vec<String> {
        value["records"]
            .as_array()
            .expect("records")
            .iter()
            .map(|record| record["record_id"].as_str().expect("record_id").to_string())
            .collect()
    };

    // Unfiltered recall: the tombstone shell is neither counted nor paged.
    let recalled = wire(
        &expect_ok(
            provider
                .recall(
                    &principal(OPERATOR),
                    recall_of(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                        "selection": selection_all(),
                        "budget": { "max_items": 10 },
                    })),
                )
                .await,
        ),
    );
    assert_eq!(ids_of(&recalled), vec!["rec_live".to_string()]);
    assert_eq!(recalled["total_authorized"], json!(1));
    assert_eq!(recalled["truncated"], json!(false));

    // A matching cognition filter never resurrects it either.
    let recalled = wire(
        &expect_ok(
            provider
                .recall(
                    &principal(OPERATOR),
                    recall_of(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                        "selection": selection_all(),
                        "cognition_filter": alice.clone(),
                        "budget": { "max_items": 10 },
                    })),
                )
                .await,
        ),
    );
    assert_eq!(ids_of(&recalled), vec!["rec_live".to_string()]);
    assert_eq!(recalled["total_authorized"], json!(1));

    let listed = wire(
        &expect_ok(
            provider
                .list_records(
                    &principal(OPERATOR),
                    list_of(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                        "selection": selection_all(),
                    })),
                )
                .await,
        ),
    );
    assert_eq!(ids_of(&listed), vec!["rec_live".to_string()]);
    assert_eq!(listed["total_authorized"], json!(1));

    // The reflection basis counts the same eligible set.
    let state = wire(
        &expect_ok(
            provider
                .get_reflection_state(
                    &principal(OPERATOR),
                    serde_json::from_value::<MemoryReflectionsStateRequest>(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                        "selection": selection_all(),
                    }))
                    .expect("state request"),
                )
                .await,
        ),
    );
    assert_eq!(state["basis"]["record_count"], json!(1));
    assert_eq!(state["data_sufficiency"], json!("sufficient"));
    let (expected_digest, _) = expected_basis(
        selection_all(),
        None,
        None,
        &[("rec_live", 1)],
        &[("personality", 1)],
    );
    assert_eq!(state["state_digest"], json!(expected_digest));
    let filtered = wire(
        &expect_ok(
            provider
                .get_reflection_state(
                    &principal(OPERATOR),
                    serde_json::from_value::<MemoryReflectionsStateRequest>(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                        "selection": selection_all(),
                        "cognition_filter": alice.clone(),
                    }))
                    .expect("state request"),
                )
                .await,
        ),
    );
    assert_eq!(filtered["basis"]["record_count"], json!(1));

    // A subject whose only record is tombstoned is insufficient, never a
    // reflection over deleted content.
    const OTHER_SUBJECT_Q: &str = "toy:character:bo";
    provider.provision_subject(SPACE, OTHER_SUBJECT_Q, None);
    provider.provision_profile(
        SPACE,
        json!({
            "schema_version": 1,
            "space_id": SPACE,
            "subject_id": OTHER_SUBJECT_Q,
            "profile_kind": "personality",
            "document": {},
            "revision": 1,
        }),
    );
    let mut bo_record = stored_record(
        "rec_bo",
        "fact",
        "2026-07-23T06:00:00Z",
        shared_scope(),
        &["ctx_alpha"],
        json!({ "text": "Bo expects nothing.", "cognition": [belief_at("bel_bo", "bo", 0.9)] }),
    );
    bo_record["subject_id"] = json!(OTHER_SUBJECT_Q);
    provider.provision_record(SPACE, bo_record);
    expect_ok(
        provider
            .delete_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": "rec_bo",
                    "expected_revision": 1,
                    "idempotency_key": "del-rec-bo",
                }))
                .expect("delete request"),
            )
            .await,
    );
    let bo_state = wire(
        &expect_ok(
            provider
                .get_reflection_state(
                    &principal(OPERATOR),
                    serde_json::from_value::<MemoryReflectionsStateRequest>(json!({
                        "space_id": SPACE,
                        "subject_id": OTHER_SUBJECT_Q,
                        "selection": selection_all(),
                    }))
                    .expect("state request"),
                )
                .await,
        ),
    );
    assert_eq!(bo_state["basis"]["record_count"], json!(0));
    assert_eq!(bo_state["data_sufficiency"], json!("insufficient"));
    assert_eq!(
        expect_reject(
            provider
                .generate_reflection(
                    &principal(OPERATOR),
                    serde_json::from_value::<MemoryReflectionsGenerateRequest>(json!({
                        "space_id": SPACE,
                        "subject_id": OTHER_SUBJECT_Q,
                        "selection": selection_all(),
                        "expected_state_digest": bo_state["state_digest"],
                        "idempotency_key": "gen-bo-tombstoned",
                    }))
                    .expect("generate request"),
                )
                .await
        ),
        SpokeRejectCode::InsufficientData
    );

    // §7 item 2: the admin display still shows the shells, for an admin only.
    let admin_view = wire(
        &expect_ok(
            provider
                .list_records(
                    &principal(OPERATOR),
                    list_of(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                        "selection": selection_all(),
                        "include_tombstoned": true,
                    })),
                )
                .await,
        ),
    );
    assert_eq!(
        ids_of(&admin_view),
        vec!["rec_ctx_dead".to_string(), "rec_dead".to_string(), "rec_live".to_string()]
    );
    assert_eq!(admin_view["total_authorized"], json!(3));
    let shell = admin_view["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|record| record["record_id"] == "rec_dead")
        .expect("the admin display includes the tombstone shell");
    assert!(shell.get("tombstone").is_some(), "{shell}");

    // The covered ordinary reader: no tombstone metadata, no admin flag.
    let reader_view = wire(
        &expect_ok(
            provider
                .list_records(
                    &principal(READER),
                    list_of(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                        "selection": { "mode": "selected_contexts", "context_ids": [CONTEXT] },
                    })),
                )
                .await,
        ),
    );
    assert_eq!(reader_view["total_authorized"], json!(0), "{reader_view}");
    assert_eq!(
        expect_reject(
            provider
                .list_records(
                    &principal(READER),
                    list_of(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                        "selection": { "mode": "selected_contexts", "context_ids": [CONTEXT] },
                        "include_tombstoned": true,
                    })),
                )
                .await
        ),
        SpokeRejectCode::NotAuthorized,
        "a covered ordinary reader never obtains the admin tombstone view"
    );
    let reader_recall = wire(
        &expect_ok(
            provider
                .recall(
                    &principal(READER),
                    recall_of(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                        "selection": { "mode": "selected_contexts", "context_ids": [CONTEXT] },
                        "budget": { "max_items": 10 },
                    })),
                )
                .await,
        ),
    );
    assert_eq!(reader_recall["total_authorized"], json!(0), "{reader_recall}");
}

/// §7 item 5: a cursor carries its operation identity, so one op's page never
/// resumes another's ordering; continuations stay exact and stable while the
/// pinned set is unchanged, and a real grant or source-revision move
/// invalidates reuse.
#[tokio::test]
async fn cognition_cursors_bind_operation_grant_and_record_revision() {
    let provider = provider();
    provider.provision_profile(
        SPACE,
        json!({
            "schema_version": 1,
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "profile_kind": "personality",
            "document": {},
            "revision": 1,
        }),
    );
    provider.provision_record(
        SPACE,
        stored_record(
            "rec_alpha",
            "note",
            "2026-07-23T06:00:00Z",
            shared_scope(),
            &["ctx_alpha", "ctx_beta"],
            json!({ "text": "alpha" }),
        ),
    );
    provider.provision_record(
        SPACE,
        stored_record(
            "rec_beta",
            "fact",
            "2026-07-23T06:00:00Z",
            shared_scope(),
            &["ctx_alpha", "ctx_beta"],
            json!({ "text": "beta" }),
        ),
    );

    let recall_of = |cursor: Option<&str>| {
        let mut request = json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": selection_all(),
            "budget": { "max_items": 1 },
        });
        if let Some(cursor) = cursor {
            request["cursor"] = json!(cursor);
        }
        serde_json::from_value::<MemoryRecallRequest>(request).expect("recall request")
    };
    let list_of = |cursor: Option<&str>| {
        let mut request = json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": selection_all(),
            "limit": 1,
        });
        if let Some(cursor) = cursor {
            request["cursor"] = json!(cursor);
        }
        serde_json::from_value::<MemoryRecordsListRequest>(request).expect("list request")
    };

    // Recall ranks the fact first; list pages by record id.
    let recall_page = wire(&expect_ok(provider.recall(&principal(OPERATOR), recall_of(None)).await));
    assert_eq!(recall_page["records"][0]["record_id"], json!("rec_beta"));
    let recall_cursor = recall_page["next_cursor"]
        .as_str()
        .expect("recall page 1 cursor")
        .to_string();
    let list_page = wire(&expect_ok(provider.list_records(&principal(OPERATOR), list_of(None)).await));
    assert_eq!(list_page["records"][0]["record_id"], json!("rec_alpha"));
    let list_cursor = list_page["next_cursor"]
        .as_str()
        .expect("list page 1 cursor")
        .to_string();

    // Cross-operation reuse skips or duplicates: reject both directions.
    assert_eq!(
        expect_reject(provider.list_records(&principal(OPERATOR), list_of(Some(&recall_cursor))).await),
        SpokeRejectCode::CursorInvalid,
        "a recall cursor must not resume a list page"
    );
    assert_eq!(
        expect_reject(provider.recall(&principal(OPERATOR), recall_of(Some(&list_cursor))).await),
        SpokeRejectCode::CursorInvalid,
        "a list cursor must not resume a recall page"
    );

    // Each own continuation is exact, unique and repeatable.
    let recall_tail = wire(&expect_ok(
        provider.recall(&principal(OPERATOR), recall_of(Some(&recall_cursor))).await,
    ));
    assert_eq!(recall_tail["records"][0]["record_id"], json!("rec_alpha"));
    assert!(recall_tail.get("next_cursor").is_none());
    let recall_tail_again = wire(&expect_ok(
        provider.recall(&principal(OPERATOR), recall_of(Some(&recall_cursor))).await,
    ));
    assert_eq!(recall_tail_again, recall_tail, "an unchanged pin repeats exactly");
    let list_tail = wire(&expect_ok(
        provider.list_records(&principal(OPERATOR), list_of(Some(&list_cursor))).await,
    ));
    assert_eq!(list_tail["records"][0]["record_id"], json!("rec_beta"));
    assert!(list_tail.get("next_cursor").is_none());
    assert_ne!(
        list_tail["records"][0]["record_id"], list_page["records"][0]["record_id"],
        "the continuation never repeats page 1"
    );

    // The reflection listing is a third ordering, so its cursor is also op-bound.
    for (index, filter) in [
        json!({ "source_context_ids": ["ctx_alpha"] }),
        json!({ "source_context_ids": ["ctx_beta"] }),
    ]
    .iter()
    .enumerate()
    {
        let state = wire(&expect_ok(
            provider
                .get_reflection_state(
                    &principal(OPERATOR),
                    serde_json::from_value::<MemoryReflectionsStateRequest>(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                        "selection": selection_all(),
                        "filter": filter,
                    }))
                    .expect("state request"),
                )
                .await,
        ));
        assert_eq!(state["basis"]["record_count"], json!(2), "{state}");
        expect_ok(
            provider
                .generate_reflection(
                    &principal(OPERATOR),
                    serde_json::from_value::<MemoryReflectionsGenerateRequest>(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                        "selection": selection_all(),
                        "filter": filter,
                        "expected_state_digest": state["state_digest"],
                        "idempotency_key": format!("gen-op-bound-{index}"),
                    }))
                    .expect("generate request"),
                )
                .await,
        );
        provider.run_worker();
    }
    // Same principal/space/subject/selection and no other bound member: the
    // only thing separating this cursor from the recall one is the operation
    // identity, so a shared binding would silently accept the swap.
    let reflections = wire(&expect_ok(
        provider
            .list_reflections(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryReflectionsListRequest>(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": selection_all(),
                    "limit": 1,
                }))
                .expect("list reflections"),
            )
            .await,
    ));
    assert_eq!(
        expect_reject(
            provider
                .list_reflections(
                    &principal(OPERATOR),
                    serde_json::from_value::<MemoryReflectionsListRequest>(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                        "selection": selection_all(),
                        "limit": 1,
                        "cursor": recall_cursor,
                    }))
                    .expect("list reflections"),
                )
                .await
        ),
        SpokeRejectCode::CursorInvalid,
        "a recall (rank-ordered) cursor must not resume the newest-first reflection list"
    );
    let reflection_page = reflections["reflections"].as_array().expect("reflections");
    assert_eq!(reflection_page.len(), 1, "{reflections}");
    let reflection_cursor = reflections["next_cursor"]
        .as_str()
        .expect("reflection page 1 cursor")
        .to_string();
    assert_eq!(
        expect_reject(provider.list_records(&principal(OPERATOR), list_of(Some(&reflection_cursor))).await),
        SpokeRejectCode::CursorInvalid,
        "a reflection-list cursor must not resume a record list"
    );
    assert_eq!(
        expect_reject(provider.recall(&principal(OPERATOR), recall_of(Some(&reflection_cursor))).await),
        SpokeRejectCode::CursorInvalid,
        "a reflection-list cursor must not resume a recall"
    );
    assert_eq!(
        expect_reject(
            provider
                .list_reflections(
                    &principal(OPERATOR),
                    serde_json::from_value::<MemoryReflectionsListRequest>(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                        "selection": selection_all(),
                        "limit": 1,
                        "cursor": list_cursor,
                    }))
                    .expect("list reflections"),
                )
                .await
        ),
        SpokeRejectCode::CursorInvalid,
        "a record-list cursor must not resume the reflection list"
    );
    let reflection_tail = wire(&expect_ok(
        provider
            .list_reflections(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryReflectionsListRequest>(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": selection_all(),
                    "limit": 1,
                    "cursor": reflection_cursor,
                }))
                .expect("list reflections"),
            )
            .await,
    ));
    assert_eq!(
        reflection_tail["reflections"].as_array().expect("reflections").len(),
        1
    );
    assert_ne!(
        reflection_tail["reflections"][0]["reflection_id"],
        reflection_page[0]["reflection_id"],
        "the reflection continuation is the other member"
    );

    // A real grant mutation moves the snapshot marker: the old cursor is dead.
    provider.provision_grant(SPACE, grant("grt_temp_admin", "prn_temp", &["admin"], None, None, None));
    let before = wire(&expect_ok(provider.list_records(&principal(OPERATOR), list_of(None)).await));
    let before_cursor = before["next_cursor"]
        .as_str()
        .expect("list page 1 cursor")
        .to_string();
    expect_ok(
        provider
            .revoke_grant(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryGrantsRevokeRequest>(json!({
                    "space_id": SPACE,
                    "grant_id": "grt_temp_admin",
                    "expected_revision": 1,
                    "idempotency_key": "rv-temp-admin",
                }))
                .expect("revoke request"),
            )
            .await,
    );
    assert_eq!(
        expect_reject(provider.list_records(&principal(OPERATOR), list_of(Some(&before_cursor))).await),
        SpokeRejectCode::CursorInvalid,
        "a grant revoke invalidates a cursor over the previous snapshot"
    );

    // A source revision move invalidates reuse too; a fresh cursor is stable.
    let fresh = wire(&expect_ok(provider.list_records(&principal(OPERATOR), list_of(None)).await));
    let fresh_cursor = fresh["next_cursor"]
        .as_str()
        .expect("list page 1 cursor")
        .to_string();
    expect_ok(
        provider
            .update_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": "rec_beta",
                    "expected_revision": 1,
                    "content": { "text": "beta" },
                    "idempotency_key": "upd-rec-beta-revision-only",
                }))
                .expect("update request"),
            )
            .await,
    );
    assert_eq!(
        expect_reject(provider.list_records(&principal(OPERATOR), list_of(Some(&fresh_cursor))).await),
        SpokeRejectCode::CursorInvalid,
        "a moved record revision invalidates the pinned page"
    );
    let after = wire(&expect_ok(provider.list_records(&principal(OPERATOR), list_of(None)).await));
    let after_cursor = after["next_cursor"]
        .as_str()
        .expect("list page 1 cursor")
        .to_string();
    for _ in 0..2 {
        let tail = wire(&expect_ok(
            provider.list_records(&principal(OPERATOR), list_of(Some(&after_cursor))).await,
        ));
        assert_eq!(tail["records"][0]["record_id"], json!("rec_beta"));
        assert!(tail.get("next_cursor").is_none());
    }
}

/// §7 item 6: provenance is part of the pinned basis, so two filters over the
/// same id+revision set share `source_set_digest` and differ in `state_digest`;
/// the pending work carries it into publication, and a revision-only move
/// reads stale.
#[tokio::test]
async fn cognition_provenance_basis_pin_publishes_and_reads_stale() {
    let provider = provider();
    provider.provision_profile(
        SPACE,
        json!({
            "schema_version": 1,
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "profile_kind": "personality",
            "document": {},
            "revision": 1,
        }),
    );
    for record_id in ["rec_one", "rec_two"] {
        provider.provision_record(
            SPACE,
            stored_record(
                record_id,
                "fact",
                "2026-07-23T06:00:00Z",
                shared_scope(),
                &["ctx_alpha", "ctx_beta"],
                json!({ "text": format!("source {record_id}") }),
            ),
        );
    }
    let alpha = json!({ "source_context_ids": ["ctx_alpha"] });
    let beta = json!({ "source_context_ids": ["ctx_beta"] });

    let operator = principal(OPERATOR);
    let state_of = |filter: &Value| {
        provider.get_reflection_state(
            &operator,
            serde_json::from_value::<MemoryReflectionsStateRequest>(json!({
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "selection": selection_all(),
                "filter": filter,
            }))
            .expect("state request"),
        )
    };
    let alpha_state = wire(&expect_ok(state_of(&alpha).await));
    let beta_state = wire(&expect_ok(state_of(&beta).await));
    assert_eq!(alpha_state["basis"]["record_count"], json!(2));
    assert_eq!(beta_state["basis"]["record_count"], json!(2));
    assert_ne!(
        alpha_state["state_digest"], beta_state["state_digest"],
        "same eligible sources, different provenance filter, different state identity"
    );
    let (alpha_expected, alpha_sources_expected) = expected_basis(
        selection_all(),
        Some(alpha.clone()),
        None,
        &[("rec_one", 1), ("rec_two", 1)],
        &[("personality", 1)],
    );
    let (beta_expected, _) = expected_basis(
        selection_all(),
        Some(beta.clone()),
        None,
        &[("rec_one", 1), ("rec_two", 1)],
        &[("personality", 1)],
    );
    assert_eq!(alpha_state["state_digest"], json!(alpha_expected));
    assert_eq!(beta_state["state_digest"], json!(beta_expected));

    let alpha_digest = alpha_expected.clone();
    let beta_digest = beta_expected.clone();
    for (filter, digest, key) in [
        (alpha.clone(), alpha_digest.clone(), "gen-prov-alpha"),
        (beta.clone(), beta_digest.clone(), "gen-prov-beta"),
    ] {
        let pending = expect_ok(
            provider
                .generate_reflection(
                    &principal(OPERATOR),
                    serde_json::from_value::<MemoryReflectionsGenerateRequest>(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                        "selection": selection_all(),
                        "filter": filter,
                        "expected_state_digest": digest,
                        "idempotency_key": key,
                    }))
                    .expect("generate request"),
                )
                .await,
        );
        assert_eq!(wire(&pending)["status"], json!("pending"));
    }

    // A revision-only public update (identical text) moves the basis.
    expect_ok(
        provider
            .update_record(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "record_id": "rec_one",
                    "expected_revision": 1,
                    "content": { "text": "source rec_one" },
                    "idempotency_key": "upd-rec-one-revision-only",
                }))
                .expect("update request"),
            )
            .await,
    );
    let moved_alpha = wire(&expect_ok(state_of(&alpha).await));
    assert_eq!(moved_alpha["basis"]["record_count"], json!(2));
    assert_ne!(
        moved_alpha["state_digest"], json!(alpha_digest),
        "a revision-only change moves the state digest"
    );
    let (moved_expected, moved_sources_expected) = expected_basis(
        selection_all(),
        Some(alpha.clone()),
        None,
        &[("rec_one", 2), ("rec_two", 1)],
        &[("personality", 1)],
    );
    assert_eq!(moved_alpha["state_digest"], json!(moved_expected));

    provider.run_worker();
    let listed = wire(&expect_ok(
        provider
            .list_reflections(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryReflectionsListRequest>(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "limit": 10,
                }))
                .expect("list reflections"),
            )
            .await,
    ));
    let reflections = listed["reflections"].as_array().expect("reflections");
    assert_eq!(reflections.len(), 2, "{listed}");
    let alpha_reflection = reflections
        .iter()
        .find(|reflection| reflection.get("filter") == Some(&alpha))
        .expect("the provenance filter is pinned on the reflection");
    let beta_reflection = reflections
        .iter()
        .find(|reflection| reflection.get("filter") == Some(&beta))
        .expect("the provenance filter is pinned on the reflection");
    assert_eq!(alpha_reflection["basis"]["state_digest"], json!(alpha_digest));
    assert_eq!(beta_reflection["basis"]["state_digest"], json!(beta_digest));
    assert_eq!(
        alpha_reflection["basis"]["source_set_digest"],
        json!(alpha_sources_expected)
    );
    assert_eq!(
        alpha_reflection["basis"]["source_set_digest"], beta_reflection["basis"]["source_set_digest"],
        "an equal eligible id+revision set keeps one source-set digest"
    );
    assert_eq!(alpha_reflection["basis"]["record_count"], json!(2));
    assert_eq!(alpha_reflection["stale"], json!(true), "the moved basis reads stale");
    assert_eq!(beta_reflection["stale"], json!(true));

    // Provenance membership: each supplied filter lists only its own pin.
    for (filter, expected_id) in [
        (&alpha, alpha_reflection["reflection_id"].clone()),
        (&beta, beta_reflection["reflection_id"].clone()),
    ] {
        let only = wire(&expect_ok(
            provider
                .list_reflections(
                    &principal(OPERATOR),
                    serde_json::from_value::<MemoryReflectionsListRequest>(json!({
                        "space_id": SPACE,
                        "subject_id": SUBJECT,
                        "filter": filter,
                        "limit": 10,
                    }))
                    .expect("list reflections"),
                )
                .await,
        ));
        let ids: Vec<Value> = only["reflections"]
            .as_array()
            .expect("reflections")
            .iter()
            .map(|reflection| reflection["reflection_id"].clone())
            .collect();
        assert_eq!(ids, vec![expected_id], "provenance pins select members");
    }

    // A reflection generated after the move is current, not stale.
    let current = wire(&expect_ok(state_of(&alpha).await));
    let generated = expect_ok(
        provider
            .generate_reflection(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryReflectionsGenerateRequest>(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": selection_all(),
                    "filter": alpha,
                    "expected_state_digest": current["state_digest"],
                    "idempotency_key": "gen-prov-alpha-current",
                }))
                .expect("generate request"),
            )
            .await,
    );
    assert_eq!(wire(&generated)["status"], json!("pending"));
    provider.run_worker();
    let current_list = wire(&expect_ok(
        provider
            .list_reflections(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryReflectionsListRequest>(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "filter": {"source_context_ids": ["ctx_alpha"]},
                    "limit": 10,
                }))
                .expect("list reflections"),
            )
            .await,
    ));
    let current_list = current_list["reflections"].as_array().expect("reflections");
    assert_eq!(current_list.len(), 2, "{current_list:?}");
    let fresh = current_list
        .iter()
        .find(|reflection| reflection["basis"]["state_digest"] == current["state_digest"])
        .expect("the current generation is listed");
    assert_eq!(fresh["stale"], json!(false));
    // The fresh publication carries the MOVED id+revision source set: its
    // published digest equals the independently expected post-move digest and
    // differs from the pre-move expectation — not another actual value.
    assert_eq!(
        fresh["basis"]["source_set_digest"],
        json!(&moved_sources_expected),
        "the fresh reflection publishes the moved id+revision source set"
    );
    assert_ne!(
        fresh["basis"]["source_set_digest"],
        json!(&alpha_sources_expected),
        "the revision-only move changed the published source-set digest"
    );
}

/// §7 item 5 (reflection listing): requested selection, provenance and pinned
/// cognition compose by AND before pages, each returning exact members, and the
/// grant must cover the returned reflections' pinned scopes.
#[tokio::test]
async fn cognition_reflection_membership_composes_selection_provenance_and_cognition() {
    let provider = provider();
    provider.provision_profile(
        SPACE,
        json!({
            "schema_version": 1,
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "profile_kind": "personality",
            "document": {},
            "revision": 1,
        }),
    );
    provider.provision_record(
        SPACE,
        stored_record(
            "rec_shared",
            "fact",
            "2026-07-23T06:00:00Z",
            shared_scope(),
            &["ctx_alpha"],
            json!({ "text": "shared source", "cognition": [belief_at("bel_shared", "alice", 0.9)] }),
        ),
    );
    provider.provision_record(
        SPACE,
        stored_record(
            "rec_ctxp",
            "fact",
            "2026-07-23T06:00:00Z",
            context_scope("ctx_p"),
            &["ctx_p"],
            json!({ "text": "context source", "cognition": [belief_at("bel_ctxp", "bo", 0.9)] }),
        ),
    );
    let alice = holder_filter("alice", 0.5);
    let bo = holder_filter("bo", 0.5);
    let ctx_p_pin = json!({ "mode": "selected_contexts", "context_ids": ["ctx_p"] });
    let shared_pin = json!({ "mode": "shared_only" });

    // R1: subject_all + provenance ctx_alpha + alice cognition.
    let r1_state = wire(&expect_ok(
        provider
            .get_reflection_state(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryReflectionsStateRequest>(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": selection_all(),
                    "filter": { "source_context_ids": ["ctx_alpha"] },
                    "cognition_filter": alice.clone(),
                }))
                .expect("state request"),
            )
            .await,
    ));
    assert_eq!(r1_state["basis"]["record_count"], json!(1), "{r1_state}");
    expect_ok(
        provider
            .generate_reflection(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryReflectionsGenerateRequest>(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": selection_all(),
                    "filter": { "source_context_ids": ["ctx_alpha"] },
                    "cognition_filter": alice.clone(),
                    "expected_state_digest": r1_state["state_digest"],
                    "idempotency_key": "gen-r1",
                }))
                .expect("generate request"),
            )
            .await,
    );
    provider.run_worker();

    // R2: the ctx_p pin + bo cognition, no provenance filter.
    let r2_state = wire(&expect_ok(
        provider
            .get_reflection_state(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryReflectionsStateRequest>(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": ctx_p_pin.clone(),
                    "cognition_filter": bo.clone(),
                }))
                .expect("state request"),
            )
            .await,
    ));
    assert_eq!(r2_state["basis"]["record_count"], json!(1), "{r2_state}");
    expect_ok(
        provider
            .generate_reflection(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryReflectionsGenerateRequest>(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": ctx_p_pin.clone(),
                    "cognition_filter": bo.clone(),
                    "expected_state_digest": r2_state["state_digest"],
                    "idempotency_key": "gen-r2",
                }))
                .expect("generate request"),
            )
            .await,
    );
    provider.run_worker();

    // R3: shared_only, no filters at all.
    let r3_state = wire(&expect_ok(
        provider
            .get_reflection_state(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryReflectionsStateRequest>(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": shared_pin.clone(),
                }))
                .expect("state request"),
            )
            .await,
    ));
    assert_eq!(r3_state["basis"]["record_count"], json!(1), "{r3_state}");
    expect_ok(
        provider
            .generate_reflection(
                &principal(OPERATOR),
                serde_json::from_value::<MemoryReflectionsGenerateRequest>(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": shared_pin.clone(),
                    "expected_state_digest": r3_state["state_digest"],
                    "idempotency_key": "gen-r3",
                }))
                .expect("generate request"),
            )
            .await,
    );
    provider.run_worker();

    let list_ids = |value: &Value| -> Vec<String> {
        value["reflections"]
            .as_array()
            .expect("reflections")
            .iter()
            .map(|reflection| {
                reflection["reflection_id"]
                    .as_str()
                    .expect("reflection_id")
                    .to_string()
            })
            .collect()
    };
    let list_of = |request: Value| {
        serde_json::from_value::<MemoryReflectionsListRequest>(request).expect("list reflections")
    };

    let all = wire(&expect_ok(
        provider
            .list_reflections(&principal(OPERATOR), list_of(json!({
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "limit": 10,
            })))
            .await,
    ));
    let all_ids = list_ids(&all);
    assert_eq!(
        all_ids,
        vec![
            "refl_sqlite_0001".to_string(),
            "refl_sqlite_0002".to_string(),
            "refl_sqlite_0003".to_string()
        ],
        "{all}"
    );

    // Request selection restricts membership to wholly covered pins.
    let subj_all = wire(&expect_ok(
        provider
            .list_reflections(&principal(OPERATOR), list_of(json!({
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "selection": selection_all(),
                "limit": 10,
            })))
            .await,
    ));
    assert_eq!(list_ids(&subj_all), all_ids, "subject_all covers every pin");

    let ctx_p_inclusive = wire(&expect_ok(
        provider
            .list_reflections(&principal(OPERATOR), list_of(json!({
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "selection": { "mode": "selected_contexts", "context_ids": ["ctx_p"], "include_shared": true },
                "limit": 10,
            })))
            .await,
    ));
    assert_eq!(
        list_ids(&ctx_p_inclusive),
        vec!["refl_sqlite_0002".to_string(), "refl_sqlite_0003".to_string()],
        "the subject_all pin is broader than the requested view"
    );

    let ctx_p_narrow = wire(&expect_ok(
        provider
            .list_reflections(&principal(OPERATOR), list_of(json!({
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "selection": { "mode": "selected_contexts", "context_ids": ["ctx_p"] },
                "limit": 10,
            })))
            .await,
    ));
    assert_eq!(
        list_ids(&ctx_p_narrow),
        vec!["refl_sqlite_0002".to_string()],
        "a shared pin is not covered by a context-only view"
    );

    // Provenance and cognition filters, alone and combined.
    let alpha_only = wire(&expect_ok(
        provider
            .list_reflections(&principal(OPERATOR), list_of(json!({
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "filter": { "source_context_ids": ["ctx_alpha"] },
                "limit": 10,
            })))
            .await,
    ));
    assert_eq!(list_ids(&alpha_only), vec!["refl_sqlite_0001".to_string()]);
    let alice_only = wire(&expect_ok(
        provider
            .list_reflections(&principal(OPERATOR), list_of(json!({
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "cognition_filter": alice.clone(),
                "limit": 10,
            })))
            .await,
    ));
    assert_eq!(list_ids(&alice_only), vec!["refl_sqlite_0001".to_string()]);
    let combined_empty = wire(&expect_ok(
        provider
            .list_reflections(&principal(OPERATOR), list_of(json!({
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "filter": { "source_context_ids": ["ctx_alpha"] },
                "cognition_filter": bo.clone(),
                "limit": 10,
            })))
            .await,
    ));
    assert!(
        list_ids(&combined_empty).is_empty(),
        "AND across provenance and cognition: {combined_empty}"
    );

    // Membership happens before pages: the only matching member is page 1.
    let paged = wire(&expect_ok(
        provider
            .list_reflections(&principal(OPERATOR), list_of(json!({
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "cognition_filter": bo.clone(),
                "limit": 1,
            })))
            .await,
    ));
    assert_eq!(list_ids(&paged), vec!["refl_sqlite_0002".to_string()]);
    assert!(
        paged.get("next_cursor").is_none(),
        "a single selected member has no continuation"
    );

    // Page continuation stays exact and non-overlapping.
    let page_one = wire(&expect_ok(
        provider
            .list_reflections(&principal(OPERATOR), list_of(json!({
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "limit": 1,
            })))
            .await,
    ));
    let cursor = page_one["next_cursor"]
        .as_str()
        .expect("page 1 cursor")
        .to_string();
    let page_two = wire(&expect_ok(
        provider
            .list_reflections(&principal(OPERATOR), list_of(json!({
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "limit": 1,
                "cursor": cursor,
            })))
            .await,
    ));
    assert_eq!(
        list_ids(&page_two),
        vec!["refl_sqlite_0002".to_string()],
        "the continuation is the next member, never a repeat"
    );

    // A narrow reflect grant reads only the reflections its selection covers.
    // The uncovered subject_all pin (the newest member) is a non-member: it
    // never turns the request into a denial, and the covered member still
    // fills page 1 at limit 1.
    provider.provision_grant(
        SPACE,
        grant(
            "grt_narrow_reflect",
            "prn_narrow",
            &["reflect"],
            Some(SUBJECT),
            Some(&["ctx_p"]),
            Some(false),
        ),
    );
    let narrow_all = wire(&expect_ok(
        provider
            .list_reflections(&principal("prn_narrow"), list_of(json!({
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "limit": 1,
            })))
            .await,
    ));
    assert_eq!(
        list_ids(&narrow_all),
        vec!["refl_sqlite_0002".to_string()],
        "the ctx_p-covered member is returned while the newer all-subject and shared pins are not: {narrow_all}"
    );
    assert!(
        narrow_all.get("next_cursor").is_none(),
        "the single covered member has no continuation"
    );
    let narrow_view = wire(&expect_ok(
        provider
            .list_reflections(&principal("prn_narrow"), list_of(json!({
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "selection": { "mode": "selected_contexts", "context_ids": ["ctx_p"] },
                "limit": 10,
            })))
            .await,
    ));
    assert_eq!(
        list_ids(&narrow_view),
        vec!["refl_sqlite_0002".to_string()],
        "the covered context pin is exactly what the narrow grant reads"
    );

    // Two separately covered pins need no single union grant: the ctx_p-only
    // grant covers R2 and the shared-capable grant covers R3, one grant each.
    provider.provision_grant(
        SPACE,
        grant(
            "grt_pair_ctx_p",
            "prn_pair",
            &["reflect"],
            Some(SUBJECT),
            Some(&["ctx_p"]),
            Some(false),
        ),
    );
    provider.provision_grant(
        SPACE,
        grant(
            "grt_pair_shared",
            "prn_pair",
            &["reflect"],
            Some(SUBJECT),
            Some(&["ctx_alpha"]),
            Some(true),
        ),
    );
    let pair_page = wire(&expect_ok(
        provider
            .list_reflections(&principal("prn_pair"), list_of(json!({
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "limit": 1,
            })))
            .await,
    ));
    assert_eq!(
        list_ids(&pair_page),
        vec!["refl_sqlite_0002".to_string()],
        "the inaccessible newer all-subject pin neither denies nor consumes the covered page: {pair_page}"
    );
    let pair_cursor = pair_page["next_cursor"]
        .as_str()
        .expect("the second covered member continues")
        .to_string();
    let pair_tail = wire(&expect_ok(
        provider
            .list_reflections(&principal("prn_pair"), list_of(json!({
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "limit": 1,
                "cursor": pair_cursor,
            })))
            .await,
    ));
    assert_eq!(
        list_ids(&pair_tail),
        vec!["refl_sqlite_0003".to_string()],
        "separately covered pins both list without a union grant: {pair_tail}"
    );
}

/// §7 item 1: placement, provenance and candidate distractors that match the
/// cognition filter never reach the count, rank or budget.
#[tokio::test]
async fn cognition_distractors_leave_the_eligible_set_before_counts() {
    let provider = provider();
    let alice = holder_filter("alice", 0.5);
    provider.provision_record(
        SPACE,
        stored_record(
            "rec_ctx_live",
            "fact",
            "2026-07-23T06:00:00Z",
            context_scope(CONTEXT),
            &["ctx_alpha"],
            json!({ "text": "context live", "cognition": [belief_at("bel_ctx_live", "alice", 0.9)] }),
        ),
    );
    provider.provision_record(
        SPACE,
        stored_record(
            "rec_other_ctx",
            "fact",
            "2026-07-23T06:00:00Z",
            context_scope("ctx_other"),
            &["ctx_alpha"],
            json!({ "text": "other context", "cognition": [belief_at("bel_other_ctx", "alice", 0.9)] }),
        ),
    );
    provider.provision_record(
        SPACE,
        stored_record(
            "rec_other_prov",
            "fact",
            "2026-07-23T06:00:00Z",
            shared_scope(),
            &["ctx_secret"],
            json!({ "text": "other provenance", "cognition": [belief_at("bel_other_prov", "alice", 0.9)] }),
        ),
    );
    // A pending candidate carries the same cognition but is not a record yet.
    capture_value(
        &provider,
        json!({
            "text": "candidate text",
            "cognition": [belief_at("bel_candidate", "alice", 0.9)],
        }),
        "cap-distractor",
    )
    .await;

    let reader_recall = wire(&expect_ok(
        provider
            .recall(
                &principal(READER),
                serde_json::from_value::<MemoryRecallRequest>(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "selection": { "mode": "selected_contexts", "context_ids": [CONTEXT] },
                    "cognition_filter": alice.clone(),
                    "budget": { "max_items": 10 },
                }))
                .expect("recall request"),
            )
            .await,
    ));
    let reader_ids: Vec<&str> = reader_recall["records"]
        .as_array()
        .expect("records")
        .iter()
        .map(|record| record["record_id"].as_str().expect("record_id"))
        .collect();
    assert_eq!(reader_ids, vec!["rec_ctx_live"]);
    assert_eq!(
        reader_recall["total_authorized"], json!(1),
        "placement-ineligible and candidate material never inflates the count"
    );

    let operator_recall = |max_items: u64| {
        serde_json::from_value::<MemoryRecallRequest>(json!({
            "space_id": SPACE,
            "subject_id": SUBJECT,
            "selection": selection_all(),
            "filter": { "source_context_ids": ["ctx_alpha"] },
            "cognition_filter": alice.clone(),
            "budget": { "max_items": max_items },
        }))
        .expect("recall request")
    };
    let full = wire(&expect_ok(provider.recall(&principal(OPERATOR), operator_recall(10)).await));
    let ids: Vec<&str> = full["records"]
        .as_array()
        .expect("records")
        .iter()
        .map(|record| record["record_id"].as_str().expect("record_id"))
        .collect();
    assert_eq!(ids, vec!["rec_ctx_live", "rec_other_ctx"]);
    assert_eq!(full["total_authorized"], json!(2));

    let bounded = wire(&expect_ok(provider.recall(&principal(OPERATOR), operator_recall(1)).await));
    assert_eq!(bounded["records"].as_array().expect("records").len(), 1);
    assert_eq!(bounded["total_authorized"], json!(2));
    assert_eq!(bounded["truncated"], json!(true));
}
