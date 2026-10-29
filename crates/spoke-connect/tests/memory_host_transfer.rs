//! Typed transfer tests for the shared loopback memory host
//! (`support/memory_host.rs`).
//!
//! The host is driven through the typed `MemoryPort` boundary with generated
//! request/response types (no transport — the serving face is covered by
//! `memory_loopback.rs`). Coverage: export chunking with verifiable §6.13
//! digests, the §7.29 drain barrier, the §8.2 freeze allowlist, §7.31 staging
//! with identity remap + staged report, §7.32 atomic activation with
//! `authority_generation++`, explicit-only non-amplified grant adoption, and
//! §7.33 pre-activation-only discard.

#![cfg(feature = "remote-adapter")]

#[path = "support/memory_host.rs"]
mod memory_host;

use std::sync::Arc;

use memory_host::{MemoryHost, Store, DEFAULT_SPACE as SPACE, DEFAULT_SUBJECT as SUBJECT};
use serde::Serialize;
use serde_json::{json, Value};
use spoke_memory::transfer::{bundle_digest, chunk_digest, reassemble, verify_reassembly};
use spoke_memory::ENGINE_POLICY_VERSION;
use spoke_operations::{MemoryPort, SpokeReject, SpokeRejectCode, SpokeResult};
use spoke_schemas::Principal;

const OTHER_SPACE: &str = "spc_import_target";
const CONTEXT: &str = "bnd_mira_harbor_run_7";
const OPERATOR: &str = "prn_operator";
const READER: &str = "prn_reader";

fn principal(id: &str) -> Principal {
    serde_json::from_value(json!({ "principal_id": id, "extensions": {} }))
        .expect("valid principal")
}

fn expect_ok<T>(result: SpokeResult<T>) -> T {
    match result {
        SpokeResult::Ok(value) => value,
        SpokeResult::Reject(reject) => panic!("expected success, got {reject:?}"),
    }
}

fn expect_reject<T: std::fmt::Debug>(result: SpokeResult<T>) -> SpokeReject {
    match result {
        SpokeResult::Ok(value) => panic!("expected a rejection, got {value:?}"),
        SpokeResult::Reject(reject) => reject,
    }
}

fn wire<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("wire value serializes")
}

fn typed<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("typed request decodes")
}

/// One store with a seeded source space: the operator holds capture-family
/// plus transfer grants on the source (unrestricted, so exports.get's
/// shared-read coverage passes), and transfer + admin on the target.
fn transfer_host() -> Arc<MemoryHost> {
    let store = Store::default();
    store.add_grant(
        OPERATOR,
        SPACE,
        Some(SUBJECT),
        &["capture", "review", "records_write", "recall", "transfer"],
        &[],
        false,
    );
    store.add_grant(OPERATOR, OTHER_SPACE, None, &["transfer", "admin"], &[], false);
    store.seed_record(SPACE, CONTEXT, "fact", "The gate key is under the third barrel.");
    store.seed_record(SPACE, "", "note", "A shared tally by the gate.");
    Arc::new(MemoryHost {
        store: Arc::new(store),
    })
}

fn export_request(key: &str) -> Value {
    json!({
        "space_id": SPACE,
        "subject_id": SUBJECT,
        "idempotency_key": key,
    })
}

/// Create an export and return `(export_id, raw_chunks)` from provider state.
async fn export_chunks(host: &Arc<MemoryHost>, key: &str) -> (String, Vec<Value>) {
    let created = expect_ok(
        host.create_export(&principal(OPERATOR), typed(export_request(key)))
            .await,
    );
    let export_id = wire(&created)["result"]["export_id"]
        .as_str()
        .expect("export id")
        .to_string();
    let chunks = host
        .store
        .export_chunks(&export_id)
        .expect("provider chunks");
    (export_id, chunks)
}

/// Stage every chunk into `space`; returns the final committed receipt.
/// Staging runs at value level: the typed chunk round-trip is not
/// byte-faithful (generated types normalize optional fields), so the §6.13
/// digests are verified against the raw wire chunks — the same entry point
/// the SQLite reference provider exposes.
async fn stage_chunks(host: &Arc<MemoryHost>, space: &str, chunks: &[Value]) -> Value {
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
        }
        let receipt = expect_ok(
            host.store
                .stage_import_value(&principal(OPERATOR), request),
        );
        final_receipt = receipt;
        assert_eq!(
            final_receipt["result"]["chunks_received"],
            ordinal + 1,
            "{final_receipt}"
        );
    }
    assert_eq!(
        final_receipt["result"]["chunks_received"],
        chunk_count as u64
    );
    final_receipt
}

fn recall_request(space: &str) -> Value {
    json!({
        "space_id": space,
        "subject_id": SUBJECT,
        "selection": { "mode": "subject_all" },
        "budget": { "max_items": 16 },
    })
}

#[tokio::test]
async fn export_chunks_round_trip_with_verifiable_digests() {
    let host = transfer_host();
    let (export_id, raw_chunks) = export_chunks(&host, "exp-1").await;

    let created = expect_ok(
        host.create_export(&principal(OPERATOR), typed(export_request("exp-1")))
            .await,
    );
    let created = wire(&created);
    assert_eq!(created["status"], "committed", "{created}");
    assert_eq!(created["result"]["counts"]["records"], 2, "{created}");
    let chunk_count = created["result"]["chunk_count"].as_u64().expect("chunk count");
    let advertised = created["result"]["bundle_digest"]
        .as_str()
        .expect("bundle digest")
        .to_string();
    // Same key + same payload replays the original receipt without a second
    // export being created.
    assert_eq!(created["receipt_id"], wire(&expect_ok(
        host.create_export(&principal(OPERATOR), typed(export_request("exp-1")))
            .await
    ))["receipt_id"]);

    let mut chunks = Vec::new();
    for ordinal in 0..chunk_count {
        let chunk = expect_ok(
            host.get_export(
                &principal(OPERATOR),
                typed(json!({
                    "space_id": SPACE,
                    "export_id": export_id,
                    "chunk_ordinal": ordinal,
                })),
            )
            .await,
        );
        chunks.push(wire(&chunk)["chunk"].clone());
    }
    // Every advertised chunk digest and the reassembled bundle must verify
    // against the provider's own chunk JSON.
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
    // The bundle reports the source engine policy version (§7.31 declared
    // policy check on activation).
    assert_eq!(bundle["source"]["engine_policy_version"], ENGINE_POLICY_VERSION);

    assert_eq!(
        expect_reject(
            host.get_export(
                &principal(OPERATOR),
                typed(json!({
                    "space_id": SPACE,
                    "export_id": export_id,
                    "chunk_ordinal": chunk_count + 5,
                })),
            )
            .await
        )
        .code,
        SpokeRejectCode::InvalidInput
    );
    // A principal without a transfer grant cannot fetch chunks.
    assert_eq!(
        expect_reject(
            host.get_export(
                &principal(READER),
                typed(json!({
                    "space_id": SPACE,
                    "export_id": export_id,
                    "chunk_ordinal": 0,
                })),
            )
            .await
        )
        .code,
        SpokeRejectCode::NotAuthorized
    );
}

#[tokio::test]
async fn quiescent_exports_need_a_frozen_space_and_transfer_stays_allowlisted() {
    let host = transfer_host();

    // Active space ⇒ the drain barrier refuses a quiescent export.
    let denied = expect_reject(
        host.create_export(
            &principal(OPERATOR),
            typed(json!({
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "require_quiescent": true,
                "idempotency_key": "exp-q0",
            })),
        )
        .await,
    );
    assert_eq!(denied.code, SpokeRejectCode::InvalidInput, "{denied:?}");
    assert_eq!(
        denied.details.as_ref().and_then(|d| d.get("reason")),
        Some(&json!("space_not_drained")),
        "{denied:?}"
    );

    // Freeze: capture (outside the §8.2 allowlist) now rejects, while the
    // transfer ops still run.
    host.store.set_frozen(true);
    let frozen_capture = expect_reject(
        host.capture(
            &principal(OPERATOR),
            typed(json!({
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "content": { "text": "Mira told Bo the gate key is hidden." },
                "capture_scope": { "visibility": "context", "context_id": CONTEXT },
                "idempotency_key": "cap-frozen",
            })),
        )
        .await,
    );
    assert_eq!(
        frozen_capture.code,
        SpokeRejectCode::SpaceFrozen,
        "{frozen_capture:?}"
    );

    let quiescent = expect_ok(
        host.create_export(
            &principal(OPERATOR),
            typed(json!({
                "space_id": SPACE,
                "subject_id": SUBJECT,
                "require_quiescent": true,
                "idempotency_key": "exp-q1",
            })),
        )
        .await,
    );
    assert_eq!(wire(&quiescent)["status"], "committed");

    // Imports stage and activate while the store is frozen.
    let (_export_id, chunks) = export_chunks(&host, "exp-frozen").await;
    let staged = stage_chunks(&host, OTHER_SPACE, &chunks).await;
    let import_id = staged["result"]["import_id"]
        .as_str()
        .expect("import id")
        .to_string();
    let activated = expect_ok(
        host.activate_import(
            &principal(OPERATOR),
            typed(json!({
                "space_id": OTHER_SPACE,
                "import_id": import_id,
                "idempotency_key": "act-frozen",
            })),
        )
        .await,
    );
    assert_eq!(wire(&activated)["status"], "committed");
}

#[tokio::test]
async fn staged_import_reports_counts_and_activates_atomically_into_an_empty_target() {
    let host = transfer_host();
    let (_export_id, chunks) = export_chunks(&host, "exp-stage").await;

    let staged = stage_chunks(&host, OTHER_SPACE, &chunks).await;
    assert_eq!(staged["status"], "committed", "{staged}");
    let report = &staged["result"]["staged_report"];
    assert_eq!(report["counts"]["subjects"], 1, "{report}");
    assert_eq!(report["counts"]["records"], 2, "{report}");
    assert_eq!(report["counts"]["candidates"], 0, "{report}");
    assert_eq!(report["grants_adopted"], 0, "{report}");
    assert_eq!(report["grants_dropped"], 1, "{report}");
    assert_eq!(report["verification"]["bundle_digest_ok"], true, "{report}");
    assert!(
        report["verification"]["errors"]
            .as_array()
            .expect("errors")
            .is_empty(),
        "{report}"
    );
    // Identity remap on an empty target: ids pass through unchanged.
    assert_eq!(
        report["subject_id_map"][SUBJECT],
        json!(SUBJECT),
        "{report}"
    );
    for record_id in ["rec_seed_0001", "rec_seed_0002"] {
        assert_eq!(
            report["record_id_map"][record_id],
            json!(record_id),
            "{report}"
        );
    }
    let import_id = staged["result"]["import_id"]
        .as_str()
        .expect("import id")
        .to_string();

    // Staged data is invisible before activation (admin implies recall).
    let before = expect_ok(
        host.recall(&principal(OPERATOR), typed(recall_request(OTHER_SPACE)))
            .await,
    );
    assert_eq!(wire(&before)["total_authorized"], 0);

    let activated = expect_ok(
        host.activate_import(
            &principal(OPERATOR),
            typed(json!({
                "space_id": OTHER_SPACE,
                "import_id": import_id,
                "idempotency_key": "act-1",
            })),
        )
        .await,
    );
    let activated = wire(&activated);
    assert_eq!(activated["status"], "committed", "{activated}");
    assert_eq!(
        activated["result"]["authority_generation"], 2,
        "activation bumps the space authority generation: {activated}"
    );
    assert_eq!(activated["result"]["activated"]["records"], 2, "{activated}");
    assert_eq!(
        host.store.import_status(&import_id).as_deref(),
        Some("activated")
    );

    // Identity remap + revisions from 1: the target serves the imported
    // records under unchanged ids and the target space.
    let after = expect_ok(
        host.recall(&principal(OPERATOR), typed(recall_request(OTHER_SPACE)))
            .await,
    );
    let after = wire(&after);
    assert_eq!(after["total_authorized"], 2, "{after}");
    let ids: Vec<&str> = after["records"]
        .as_array()
        .expect("records")
        .iter()
        .map(|record| record["record_id"].as_str().expect("record id"))
        .collect();
    assert_eq!(ids, vec!["rec_seed_0001", "rec_seed_0002"], "{after}");
    for record in after["records"].as_array().expect("records") {
        assert_eq!(record["space_id"], OTHER_SPACE, "{record}");
        assert_eq!(record["revision"], 1, "{record}");
    }

    // Re-activating the same import is a conflict, not a double-apply.
    assert_eq!(
        expect_reject(
            host.activate_import(
                &principal(OPERATOR),
                typed(json!({
                    "space_id": OTHER_SPACE,
                    "import_id": import_id,
                    "idempotency_key": "act-2",
                })),
            )
            .await
        )
        .code,
        SpokeRejectCode::ImportConflict
    );
}

#[tokio::test]
async fn import_adoption_is_explicit_only_and_non_amplified() {
    let host = transfer_host();
    // A reader grant in the source bundle: recall, context-restricted.
    host.store.add_grant(
        READER,
        SPACE,
        Some(SUBJECT),
        &["recall"],
        &[CONTEXT],
        false,
    );
    let (_export_id, chunks) = export_chunks(&host, "exp-adopt").await;

    // Only the listed grant is adopted; the operator's own source grant is
    // dropped (never implicit).
    let staged = stage_chunks_with_adopt(&host, OTHER_SPACE, &chunks, &import_adopt()).await;
    let report = &staged["result"]["staged_report"];
    assert_eq!(report["grants_adopted"], 1, "{report}");
    assert_eq!(report["grants_dropped"], 1, "{report}");
    let import_id = staged["result"]["import_id"]
        .as_str()
        .expect("import id")
        .to_string();

    let activated = expect_ok(
        host.activate_import(
            &principal(OPERATOR),
            typed(json!({
                "space_id": OTHER_SPACE,
                "import_id": import_id,
                "idempotency_key": "act-adopt",
            })),
        )
        .await,
    );
    assert_eq!(wire(&activated)["status"], "committed");

    // The adopted grant lands on the requested principal in the target
    // space; the unlisted source admin/operator grants never appear.
    let state = host.store.state.lock().expect("state lock");
    let adopted: Vec<Value> = state
        .grants
        .iter()
        .map(wire)
        .filter(|grant| grant["space_id"] == OTHER_SPACE)
        .collect();
    assert_eq!(adopted.len(), 2, "{adopted:?}");
    let adopted = adopted
        .iter()
        .find(|grant| grant["principal_id"] == READER)
        .expect("adopted reader grant");
    assert!(
        adopted["capabilities"]
            .as_array()
            .expect("capabilities")
            .iter()
            .any(|capability| capability == "recall"),
        "{adopted}"
    );
    assert_eq!(adopted["context_ids"], json!([CONTEXT]), "{adopted}");
}

fn import_adopt() -> Value {
    // Grant order: grt_01 operator/source, grt_02 operator/target, grt_03
    // reader/source — only the listed reader grant is adopted.
    json!([{ "grant_id": "grt_03", "principal_id": READER }])
}

async fn stage_chunks_with_adopt(
    host: &Arc<MemoryHost>,
    space: &str,
    chunks: &[Value],
    adopt: &Value,
) -> Value {
    let chunk_count = chunks.len();
    let mut final_receipt = Value::Null;
    for (ordinal, chunk) in chunks.iter().enumerate() {
        let mut request = json!({
            "space_id": space,
            "chunk": chunk,
            "idempotency_key": format!("stage-adopt-{space}-{ordinal}"),
        });
        if ordinal == 0 {
            request["conflict_policy"] = json!("fail");
            request["adopt_grants"] = adopt.clone();
        }
        let receipt = expect_ok(
            host.store
                .stage_import_value(&principal(OPERATOR), request),
        );
        final_receipt = receipt;
        assert_eq!(
            final_receipt["result"]["chunks_received"],
            ordinal + 1,
            "{final_receipt}"
        );
    }
    assert_eq!(
        final_receipt["result"]["chunks_received"],
        chunk_count as u64
    );
    final_receipt
}

#[tokio::test]
async fn adoption_rejects_amplification_against_the_stager() {
    // The operator's target admin grant is context-restricted; the source
    // bundle carries an unrestricted reader grant, and adopting it would
    // amplify the stager's coverage.
    let store = Store::default();
    store.add_grant(
        OPERATOR,
        SPACE,
        Some(SUBJECT),
        &["capture", "review", "records_write", "recall", "transfer"],
        &[CONTEXT],
        false,
    );
    store.add_grant(
        OPERATOR,
        OTHER_SPACE,
        None,
        &["transfer", "admin"],
        &[CONTEXT],
        false,
    );
    store.add_grant(READER, SPACE, Some(SUBJECT), &["recall"], &[], false);
    store.seed_record(SPACE, CONTEXT, "fact", "The gate key is under the third barrel.");
    let host = Arc::new(MemoryHost {
        store: Arc::new(store),
    });

    let (_export_id, chunks) = export_chunks(&host, "exp-amplify").await;
    // Grant order: grt_03 is the reader's unrestricted source grant.
    let staged = stage_chunks_with_adopt(
        &host,
        OTHER_SPACE,
        &chunks,
        &json!([{ "grant_id": "grt_03", "principal_id": READER }]),
    )
    .await;
    let import_id = staged["result"]["import_id"]
        .as_str()
        .expect("import id")
        .to_string();
    let amplified = expect_reject(
        host.activate_import(
            &principal(OPERATOR),
            typed(json!({
                "space_id": OTHER_SPACE,
                "import_id": import_id,
                "idempotency_key": "act-amplify",
            })),
        )
        .await,
    );
    assert_eq!(
        amplified.code,
        SpokeRejectCode::ImportConflict,
        "adoption beyond the stager's coverage must reject: {amplified:?}"
    );
    // The failed activation applied nothing.
    assert!(
        host.store
            .state
            .lock()
            .expect("state lock")
            .records
            .iter()
            .all(|record| record.space_id != OTHER_SPACE),
        "a rejected activation writes no records"
    );
}

#[tokio::test]
async fn discard_is_pre_activation_only() {
    let host = transfer_host();
    let (_export_id, chunks) = export_chunks(&host, "exp-discard").await;
    let staged = stage_chunks(&host, OTHER_SPACE, &chunks).await;
    let import_id = staged["result"]["import_id"]
        .as_str()
        .expect("import id")
        .to_string();

    let discarded = expect_ok(
        host.discard_import(
            &principal(OPERATOR),
            typed(json!({
                "space_id": OTHER_SPACE,
                "import_id": import_id,
                "idempotency_key": "disc-1",
            })),
        )
        .await,
    );
    let discarded = wire(&discarded);
    assert_eq!(discarded["result"]["discarded"], true, "{discarded}");
    assert_eq!(
        host.store.import_status(&import_id).as_deref(),
        Some("discarded")
    );

    assert_eq!(
        expect_reject(
            host.activate_import(
                &principal(OPERATOR),
                typed(json!({
                    "space_id": OTHER_SPACE,
                    "import_id": import_id,
                    "idempotency_key": "act-after-discard",
                })),
            )
            .await
        )
        .code,
        SpokeRejectCode::ImportConflict
    );
}

#[tokio::test]
async fn subject_collision_rejects_or_renames_per_conflict_policy() {
    let host = transfer_host();
    let (_export_id, chunks) = export_chunks(&host, "exp-collision").await;

    // The source subject already exists in the source space: `fail` rejects
    // on the final chunk and leaves nothing staged.
    let rejected = expect_reject(
        host.store
            .stage_import_value(&principal(OPERATOR), stage_request(SPACE, &chunks[0], "fail", None)),
    );
    assert_eq!(rejected.code, SpokeRejectCode::NotAuthorized);

    // With transfer+admin on the source space, `fail` rejects on the
    // subject collision itself.
    host.store
        .add_grant(OPERATOR, SPACE, None, &["transfer", "admin"], &[], false);
    let collided = expect_reject(
        host.store
            .stage_import_value(&principal(OPERATOR), stage_request(SPACE, &chunks[0], "fail", None)),
    );
    assert_eq!(
        collided.code,
        SpokeRejectCode::ImportConflict,
        "an existing subject with conflict_policy=fail rejects: {collided:?}"
    );

    // `rename_subject` remaps the colliding subject and activates under the
    // renamed identity.
    let staged = expect_ok(
        host.store.stage_import_value(
            &principal(OPERATOR),
            stage_request(SPACE, &chunks[0], "rename_subject", None),
        ),
    );
    let staged = wire(&staged);
    let report = &staged["result"]["staged_report"];
    let renamed = format!("{SUBJECT}_imported_1");
    assert_eq!(
        report["subject_id_map"][SUBJECT],
        json!(renamed),
        "{report}"
    );
    let import_id = staged["result"]["import_id"]
        .as_str()
        .expect("import id")
        .to_string();
    let activated = expect_ok(
        host.activate_import(
            &principal(OPERATOR),
            typed(json!({
                "space_id": SPACE,
                "import_id": import_id,
                "idempotency_key": "act-rename",
            })),
        )
        .await,
    );
    assert_eq!(wire(&activated)["status"], "committed");
    let state = host.store.state.lock().expect("state lock");
    assert!(
        state
            .records
            .iter()
            .any(|record| record.space_id == SPACE && record.subject_id == renamed),
        "the renamed subject holds the imported records"
    );
}

fn stage_request(space: &str, chunk: &Value, conflict_policy: &str, adopt: Option<&Value>) -> Value {
    let mut request = json!({
        "space_id": space,
        "chunk": chunk,
        "conflict_policy": conflict_policy,
        "idempotency_key": format!("stage-collision-{space}"),
    });
    if let Some(adopt) = adopt {
        request["adopt_grants"] = adopt.clone();
    }
    request
}
