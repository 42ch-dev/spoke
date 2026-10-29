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

fn grant(
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
        "space_id": SPACE,
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
