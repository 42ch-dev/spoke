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
            &["capture", "review", "records_write", "recall"],
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
