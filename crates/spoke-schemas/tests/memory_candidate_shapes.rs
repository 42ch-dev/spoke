//! Regression: the pending candidate's `proposed` carries `content` while the
//! consumed shell keeps the kind-only shape (§6.7). The two inline shapes used
//! to share one JSON `title`, so typify merged them and the pending branch lost
//! `content`.

use serde_json::{json, Value};
use spoke_schemas::data::memory_candidate::MemoryCandidate;

const PENDING_WITH_CONTENT: &str = r#"{
  "schema_version": 1,
  "candidate_id": "cand_pending",
  "space_id": "spc_local",
  "subject_id": "subj_mira",
  "capture_scope": {"visibility": "shared"},
  "proposed": {
    "kind": "fact",
    "content": {"text": "Mira promised the harbor master she would return before dawn."}
  },
  "classification": {"disposition": "promote", "rationale_codes": []},
  "status": "pending",
  "provenance": {
    "source_context_ids": [],
    "anchors": [],
    "captured_by": "prn_host",
    "captured_at": "2026-10-28T09:00:00Z"
  },
  "created_at": "2026-10-28T09:00:00Z",
  "revision": 1,
  "extensions": {}
}"#;

const CONSUMED_SHELL: &str = r#"{
  "schema_version": 1,
  "candidate_id": "cand_consumed",
  "space_id": "spc_local",
  "subject_id": "subj_mira",
  "capture_scope": {"visibility": "context", "context_id": "bnd_mira_harbor_run_7"},
  "proposed": {"kind": "fact"},
  "classification": {"disposition": "promote", "rationale_codes": ["core_kind"]},
  "status": "consumed",
  "consumed_by": {
    "op": "memory.candidates.review",
    "receipt_id": "rcp_1",
    "outcome": "promoted",
    "record_ids": ["rec_a"]
  },
  "content_digest": "0000000000000000000000000000000000000000000000000000000000000000",
  "provenance": {
    "source_context_ids": ["bnd_mira_harbor_run_7"],
    "anchors": [],
    "captured_by": "prn_host",
    "captured_at": "2026-10-28T09:00:00Z"
  },
  "created_at": "2026-10-28T09:00:00Z",
  "revision": 2,
  "extensions": {}
}"#;

#[test]
fn pending_candidate_with_proposed_content_deserializes() {
    let candidate: MemoryCandidate = serde_json::from_str(PENDING_WITH_CONTENT)
        .expect("a pending candidate with content parses");

    let wire: Value = serde_json::to_value(&candidate).expect("candidate serializes");
    assert_eq!(
        wire["proposed"],
        json!({
            "kind": "fact",
            "content": {"text": "Mira promised the harbor master she would return before dawn."},
        })
    );
    assert_eq!(wire["status"], json!("pending"));
    assert!(
        matches!(candidate, MemoryCandidate::PendingCandidate(_)),
        "the pending variant is selected"
    );
}

#[test]
fn consumed_shell_still_deserializes() {
    let direct: spoke_schemas::data::memory_candidate::ConsumedCandidate =
        serde_json::from_str(CONSUMED_SHELL).expect("the consumed shell parses directly");
    assert!(
        direct.proposed.is_some(),
        "the consumed shell keeps its proposed kind shell"
    );
    let candidate: MemoryCandidate =
        serde_json::from_str(CONSUMED_SHELL).expect("the consumed shell parses");

    let wire: Value = serde_json::to_value(&candidate).expect("candidate serializes");
    assert_eq!(wire["proposed"], json!({"kind": "fact"}));
    assert_eq!(wire["status"], json!("consumed"));
    assert_eq!(wire["consumed_by"]["record_ids"], json!(["rec_a"]));
    assert!(
        matches!(candidate, MemoryCandidate::ConsumedCandidate(_)),
        "the consumed variant is selected"
    );
}

#[test]
fn consumed_candidate_with_proposed_content_is_rejected() {
    // §6.7: consumption scrubs `proposed.content`; the shell retains the kind only.
    let mut wire: Value = serde_json::from_str(CONSUMED_SHELL).expect("fixture parses");
    wire["proposed"] = json!({"kind": "fact", "content": {"text": "leaked content"}});

    let result = serde_json::from_value::<MemoryCandidate>(wire);
    assert!(
        result.is_err(),
        "a consumed candidate must not carry proposed content"
    );
}
