//! Generated-type pins for the shared wire scalars (spoke-data-model
//! §Shared wire scalars): the `SchemaVersion` ceiling, the inclusive revision
//! ceiling, and the strict timestamp grammar, asserted on the public Rust
//! types rather than on the schema or an operation-level gate.

use std::str::FromStr;

use serde_json::{json, Value};
use spoke_schemas::data::adoption_anchor::AdoptionAnchor;
use spoke_schemas::data::judgment::JudgmentEvidenceRef;
use spoke_schemas::data::relation::Relation;
use spoke_schemas::{ConnectSession, SchemaVersion};

const VERSION_MAX: u64 = 9_007_199_254_740_991; // 2^53 - 1
const REVISION_MAX: u64 = 9_007_199_254_740_992; // 2^53

fn relation(revision: Option<u64>, created_at: Option<&str>) -> Value {
    let mut value = json!({
        "schema_version": 1,
        "relation_id": "rel_example",
        "relation_type": "related_to",
        "from_id": "ke_a",
        "to_id": "ke_b",
        "extensions": {},
    });
    if let Some(revision) = revision {
        value["revision"] = json!(revision);
    }
    if let Some(created_at) = created_at {
        value["created_at"] = json!(created_at);
    }
    value
}

fn session(opened_at: &str) -> Value {
    json!({
        "session_id": "session-1",
        "initiator_peer_id": "peer_a",
        "responder_peer_id": "peer_b",
        "opened_at": opened_at,
        "negotiated_capabilities": ["spoke-connect"],
        "initial_sequence": 0,
        "signature": "A".repeat(86),
        "extensions": {},
    })
}

#[test]
fn schema_version_ceiling_holds_on_the_wrapped_and_inlined_types() {
    assert_eq!(
        SchemaVersion::from_str(&VERSION_MAX.to_string())
            .expect("2^53-1 is in domain")
            .get(),
        VERSION_MAX
    );
    assert!(
        SchemaVersion::from_str("9007199254740992").is_err(),
        "2^53 is refused"
    );
    assert!(serde_json::from_value::<SchemaVersion>(json!(VERSION_MAX)).is_ok());
    assert!(
        serde_json::from_value::<SchemaVersion>(json!(REVISION_MAX)).is_err(),
        "wrapped decode refuses 2^53"
    );

    let mut value = relation(None, None);
    value["schema_version"] = json!(VERSION_MAX);
    let decoded: Relation = serde_json::from_value(value).expect("2^53-1 is in domain");
    assert_eq!(decoded.schema_version.get(), VERSION_MAX);

    let mut value = relation(None, None);
    value["schema_version"] = json!(REVISION_MAX);
    assert!(
        serde_json::from_value::<Relation>(value).is_err(),
        "inlined schema_version refuses 2^53"
    );

    // `KnowledgeEntry.adoption_anchor` reuses a shared copy of the version
    // newtype; its decode must enforce the same ceiling.
    let anchor = |version: u64| {
        json!({
            "schema_version": version,
            "source_scope_id": "spc_shared",
            "source_entry_id": "ke_source",
            "source_revision": 3,
            "extensions": {},
        })
    };
    assert!(serde_json::from_value::<AdoptionAnchor>(anchor(VERSION_MAX)).is_ok());
    assert!(
        serde_json::from_value::<AdoptionAnchor>(anchor(REVISION_MAX)).is_err(),
        "shared version newtype refuses 2^53"
    );
}

#[test]
fn revision_ceiling_is_inclusive() {
    let decoded: Relation =
        serde_json::from_value(relation(Some(REVISION_MAX), None)).expect("2^53 is in domain");
    assert_eq!(decoded.revision, Some(REVISION_MAX));

    let refusal = REVISION_MAX + 1;
    assert!(
        serde_json::from_value::<Relation>(relation(Some(refusal), None)).is_err(),
        "2^53+1 is refused"
    );

    assert!(
        serde_json::from_value::<Relation>(relation(Some(0), None)).is_ok(),
        "revision 0 is in domain"
    );
}

/// The bound is asserted on the parsed value: an integral-float spelling of
/// an in-domain revision decodes (`9007199254740992.0` ≡ `2^53`), while an
/// exact integer past the endpoint refuses without an `f64` round-trip.
#[test]
fn integral_float_spellings_decode_by_parsed_value() {
    let mut value = relation(None, None);
    value["revision"] = json!(9_007_199_254_740_992.0);
    let decoded: Relation =
        serde_json::from_value(value).expect("the endpoint as an integral float");
    assert_eq!(decoded.revision, Some(REVISION_MAX));

    let anchor = |revision: Value| {
        json!({
            "schema_version": 1,
            "source_scope_id": "spc_shared",
            "source_entry_id": "ke_source",
            "source_revision": revision,
            "extensions": {},
        })
    };
    let decoded: AdoptionAnchor = serde_json::from_value(anchor(json!(9_007_199_254_740_992.0)))
        .expect("the anchor endpoint as an integral float");
    assert_eq!(decoded.source_revision, REVISION_MAX);
    assert!(
        serde_json::from_value::<AdoptionAnchor>(anchor(json!(9_007_199_254_740_993_u64))).is_err(),
        "an exact integer past the endpoint refuses"
    );

    let mut value = relation(None, None);
    value["revision"] = json!(9_007_199_254_740_993_u64);
    assert!(
        serde_json::from_value::<Relation>(value).is_err(),
        "an exact 2^53+1 is never rounded into the domain"
    );
    let mut value = relation(None, None);
    value["revision"] = json!(1.5);
    assert!(
        serde_json::from_value::<Relation>(value).is_err(),
        "a non-integral number is not a revision"
    );

    // The narrowed Judgment ceiling applies to a float spelling too.
    let evidence = |revision: Value| {
        json!({
            "id": "ke_a",
            "kind": "knowledge_entry",
            "revision": revision,
        })
    };
    let decoded: JudgmentEvidenceRef =
        serde_json::from_value(evidence(json!(9007199254740991.0)))
            .expect("2^53-1 as an integral float");
    assert_eq!(decoded.revision.get(), VERSION_MAX);
    assert!(
        serde_json::from_value::<JudgmentEvidenceRef>(evidence(json!(9_007_199_254_740_992.0)))
            .is_err(),
        "Judgment refuses the 2^53 endpoint in either spelling"
    );
}

#[test]
fn judgment_revision_stays_one_below_the_revision_ceiling() {
    let evidence = |revision: u64| {
        json!({
            "id": "ke_a",
            "kind": "knowledge_entry",
            "revision": revision,
        })
    };
    let decoded: JudgmentEvidenceRef =
        serde_json::from_value(evidence(VERSION_MAX)).expect("2^53-1 is in domain");
    assert_eq!(decoded.revision.get(), VERSION_MAX);
    assert!(
        serde_json::from_value::<JudgmentEvidenceRef>(evidence(REVISION_MAX)).is_err(),
        "Judgment refuses 2^53"
    );
}

#[test]
fn direct_wire_timestamps_use_the_strict_grammar() {
    for accepted in [
        "2026-07-23T09:00:00Z",
        "2026-07-23t09:00:00z",
        "2026-07-23T09:00:00.123+08:00",
        "2026-07-23T09:00:60Z",
    ] {
        let decoded: ConnectSession =
            serde_json::from_value(session(accepted)).expect("grammar-admitted timestamp");
        assert_eq!(
            decoded.opened_at,
            spoke_schemas::timestamp::parse_strict(accepted).expect("same grammar")
        );
    }
    for refused in [
        "2026-11-23 10:00:00Z",
        "2026-11-23T10:00:00 UTC",
        "2026-11-23T10:00:00",
        "2026-11-23T10:00:5Z",
        "2026-11-23T10:00:00+24:00",
    ] {
        assert!(
            serde_json::from_value::<ConnectSession>(session(refused)).is_err(),
            "opened_at accepted {refused:?}"
        );
        assert!(
            serde_json::from_value::<Relation>(relation(None, Some(refused))).is_err(),
            "created_at accepted {refused:?}"
        );
    }
}
