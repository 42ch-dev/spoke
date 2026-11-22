//! Regression pins for the revision-snapshot wire-presence rule (fix-g).
//!
//! The history contract requires explicitly present-and-empty stored members
//! — `modules: {}`, `body.tags: []`, `body.state: {}`, `body.attributes: []`,
//! map-shaped `body.computable` — to survive replay and RFC 8785
//! canonicalization byte-identically. The generated `RevisionSnapshot` and
//! `HistoryRevisionsGetResponse` carriers keep absence and explicit-empty
//! distinct (`Option<Vec>`/`Option<Map>` + presence-preserving decoders), so
//! a typed decode/encode round-trip must reproduce the canonical input bytes
//! exactly, and an explicit `null` must stay invalid.

use serde_json::{json, Value};
use spoke_schemas::{HistoryRevisionsGetResponse, RevisionSnapshot};

/// Minimal valid knowledge-entry snapshot with every presence-sensitive
/// member supplied explicitly empty.
fn presence_snapshot() -> Value {
    json!({
        "entity_id": "ke_mira_harbor",
        "entity_kind": "knowledge_entry",
        "extensions": {},
        "revision": 3,
        "schema_version": 1,
        "snapshot": {
            "body": {
                "attributes": [],
                "computable": {},
                "state": {},
                "tags": []
            },
            "canonical_name": "Mira",
            "entry_id": "ke_mira_harbor",
            "entry_type": "character",
            "extensions": {},
            "modules": {},
            "schema_version": 1,
            "status": "confirmed"
        }
    })
}

#[test]
fn explicit_empty_members_round_trip_byte_identically() {
    // serde_json without `preserve_order` sorts map keys, matching both the
    // alphabetical field order typify emits and RFC 8785 canonical form.
    let input = serde_json::to_string(&presence_snapshot()).expect("canonical input");
    let decoded: RevisionSnapshot =
        serde_json::from_str(&input).expect("snapshot with explicit-empty members decodes");
    let encoded = serde_json::to_string(&decoded).expect("snapshot encodes");

    assert_eq!(encoded, input, "typed round-trip must be byte-identical");

    // Presence is visible on the value level too: every member is still
    // there, still empty.
    let value = serde_json::to_value(&decoded).expect("snapshot to value");
    let entity = &value["snapshot"];
    assert_eq!(entity["modules"], json!({}));
    assert_eq!(entity["body"]["tags"], json!([]));
    assert_eq!(entity["body"]["state"], json!({}));
    assert_eq!(entity["body"]["attributes"], json!([]));
    assert_eq!(entity["body"]["computable"], json!({}));
}

#[test]
fn absent_members_stay_absent_through_the_typed_round_trip() {
    let mut value = presence_snapshot();
    let entity = value["snapshot"].as_object_mut().expect("entity object");
    entity.remove("modules");
    entity["body"].as_object_mut().expect("body object").remove("state");
    entity["body"].as_object_mut().expect("body object").remove("tags");

    let input = serde_json::to_string(&value).expect("canonical input");
    let decoded: RevisionSnapshot = serde_json::from_str(&input).expect("snapshot decodes");
    let encoded = serde_json::to_string(&decoded).expect("snapshot encodes");

    assert_eq!(encoded, input, "absent members must not reappear");
    let round_tripped = serde_json::to_value(&decoded).expect("snapshot to value");
    let entity = &round_tripped["snapshot"];
    assert!(entity.get("modules").is_none(), "modules must stay absent");
    assert!(
        entity["body"].get("state").is_none(),
        "body.state must stay absent"
    );
    assert!(
        entity["body"].get("tags").is_none(),
        "body.tags must stay absent"
    );
}

#[test]
fn explicit_null_for_a_presence_sensitive_member_stays_invalid() {
    let mut value = presence_snapshot();
    value["snapshot"]["modules"] = Value::Null;

    let input = serde_json::to_string(&value).expect("canonical input");
    assert!(
        serde_json::from_str::<RevisionSnapshot>(&input).is_err(),
        "modules:null must not decode"
    );
}

#[test]
fn response_carrier_preserves_explicit_empty_members() {
    let response = json!({ "snapshot": presence_snapshot(), "extensions": {} });

    let input = serde_json::to_string(&response).expect("canonical input");
    let decoded: HistoryRevisionsGetResponse =
        serde_json::from_str(&input).expect("response decodes");
    let encoded = serde_json::to_string(&decoded).expect("response encodes");

    assert_eq!(encoded, input, "response round-trip must be byte-identical");
    let round_tripped = serde_json::to_value(&decoded).expect("response to value");
    assert_eq!(
        round_tripped["snapshot"]["snapshot"]["modules"],
        json!({}),
        "nested snapshot keeps modules:{{}}"
    );
    assert_eq!(
        round_tripped["snapshot"]["snapshot"]["body"]["attributes"],
        json!([]),
        "nested snapshot keeps body.attributes:[]"
    );
}
