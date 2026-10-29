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
use spoke_memory::ENGINE_POLICY_VERSION;
use spoke_operations::{MemoryPort, SpokeRejectCode, SpokeResult};
use spoke_schemas::{
    MemoryDescribeRequest, MemoryGrantsGrantRequest, MemoryGrantsListRequest,
    MemoryGrantsRevokeRequest, MemoryReceiptsGetRequest, MemorySpacesFreezeRequest,
    MemorySpacesListRequest, MemorySpacesUnfreezeRequest, MemorySubjectsEnsureRequest,
    MemorySubjectsFenceRequest, MemorySubjectsGetRequest, Principal,
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
    let provider = SqliteMemoryProvider::in_memory();
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

use spoke_memory::transfer::{bundle_digest, chunk_digest, reassemble, verify_reassembly};

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

async fn export_chunks_of(provider: &SqliteMemoryProvider) -> (String, Vec<Value>) {
    let created = expect_ok(
        provider
            .create_export(
                &principal(OPERATOR),
                serde_json::from_value(json!({
                    "space_id": SPACE,
                    "subject_id": SUBJECT,
                    "include_reflections": true,
                    "require_quiescent": false,
                    "idempotency_key": "exp-import",
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
        // Staged at value level: the generated chunk types decode the provider
        // chunk but re-serialize it with normalized optional fields
        // (`structured: null`, …), so a typed round-trip is not byte-faithful and
        // the chunk digest no longer matches. See the codegen-repair report.
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
