//! Parity tests for ToyWorldAdapter baseline + Full stub orchestration.

use serde_json::{json, Value};
use spoke_fixture_toy_world::{
    as_baseline_only, toy_world_fixtures_root, MemoryStoreSeed, ToyWorldAdapter,
};
use spoke_operations::{
    orchestrate_assemble, orchestrate_check, orchestrate_compute, orchestrate_fork_check,
    orchestrate_project, orchestrate_promote, orchestrate_relate, orchestrate_upsert, spoke_ok,
    CheckRunInput, ForkTimelineQueryPort, FullAdapter, KnowledgeEntryPort, SpokeRejectCode,
    SpokeResult,
};
use spoke_schemas::{
    AssembleRequest, CheckRequest, ComputeRequest, ComputeResponse, Finding, KnowledgeEntry,
    ProjectRequest, ProjectResponse, PromoteRequest, RelateRequest, Relation, Rule, Scope,
    TimelineEvent, UpsertRequest,
};

fn load_fixture<T: serde::de::DeserializeOwned>(filename: &str) -> T {
    let path = toy_world_fixtures_root().join(filename);
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", path.display());
    });
    serde_json::from_str(&raw).unwrap_or_else(|error| {
        panic!("failed to parse {}: {error}", path.display());
    })
}

fn provisional_mira(overrides: Value) -> KnowledgeEntry {
    let mira = load_fixture::<Value>("kb_tw_mira.json");
    let mut base = mira;
    if let Value::Object(map) = &mut base {
        map.insert("status".into(), json!("provisional"));
        map.insert("revision".into(), json!(0));
        if let Value::Object(over) = overrides {
            for (key, value) in over {
                map.insert(key, value);
            }
        }
    }
    serde_json::from_value(base).expect("provisional mira KnowledgeEntry")
}

#[test]
fn orchestrate_upsert_creates_knowledge_entry_through_occ_put() {
    let adapter = ToyWorldAdapter::default();
    let candidate: KnowledgeEntry = serde_json::from_value(json!({
        "schema_version": 1,
        "entry_id": "kb_tw_new_cartographer",
        "entry_type": "character",
        "canonical_name": "New Cartographer",
        "status": "provisional",
        "body": { "summary": "Fresh provisional entry" },
        "extensions": {}
    }))
    .expect("candidate");
    let request: UpsertRequest = serde_json::from_value(json!({
        "knowledge_entries": [candidate]
    }))
    .expect("UpsertRequest");

    let result = orchestrate_upsert(&adapter, request);
    assert!(result.is_ok(), "{result:?}");
    if let SpokeResult::Ok(spoke_schemas::UpsertResponse::Variant0 {
        knowledge_entries, ..
    }) = result
    {
        assert_eq!(knowledge_entries.len(), 1);
        assert_eq!(knowledge_entries[0].entry_id, "kb_tw_new_cartographer");
    } else {
        panic!("expected upsert success");
    }

    adapter.with_store(|store| {
        assert!(store.entries.contains_key("kb_tw_new_cartographer"));
    });
}

#[test]
fn orchestrate_promote_persists_confirmed_knowledge_entry() {
    let candidate = provisional_mira(json!({
        "entry_id": "kb_tw_promote",
        "canonical_name": "Promote Candidate",
    }));
    let adapter = ToyWorldAdapter::default();
    let request: PromoteRequest =
        serde_json::from_value(json!({ "candidate": candidate })).expect("PromoteRequest");

    let result = orchestrate_promote(&adapter, request);
    assert!(result.is_ok(), "{result:?}");
    if let SpokeResult::Ok(spoke_schemas::PromoteResponse::Variant0 {
        knowledge_entry, ..
    }) = result
    {
        assert_eq!(knowledge_entry.status, "confirmed");
        assert_eq!(knowledge_entry.revision, Some(1));
    } else {
        panic!("expected promote success");
    }

    adapter.with_store(|store| {
        assert_eq!(
            store.entries.get("kb_tw_promote").map(|e| e.status.as_str()),
            Some("confirmed")
        );
    });
}

#[test]
fn orchestrate_relate_persists_relation() {
    let adapter = ToyWorldAdapter::default();
    let mut relation: Value = serde_json::to_value(load_fixture::<Relation>(
        "rel_tw_mira_harbor.json",
    ))
    .expect("relation value");
    relation["relation_id"] = json!("rel_tw_adapter_demo");
    let request: RelateRequest =
        serde_json::from_value(json!({ "relation": relation })).expect("RelateRequest");

    let result = orchestrate_relate(&adapter, request);
    assert!(result.is_ok(), "{result:?}");
    if let SpokeResult::Ok(spoke_schemas::RelateResponse::Variant0 { relation: got, .. }) = result {
        assert_eq!(got.relation_id, "rel_tw_adapter_demo");
    } else {
        panic!("expected relate success");
    }

    adapter.with_store(|store| {
        assert!(store.relations.contains_key("rel_tw_adapter_demo"));
    });
}

#[test]
fn orchestrate_check_runs_trivial_checker_and_puts_findings() {
    let mira = load_fixture::<KnowledgeEntry>("kb_tw_mira.json");
    let harbor = load_fixture::<KnowledgeEntry>("kb_tw_harbor.json");
    let rule = load_fixture::<Rule>("rule_tw_consistency.json");
    let adapter = ToyWorldAdapter::with_committed_fixtures();
    let request: CheckRequest = serde_json::from_value(json!({
        "scope": {
            "scope_id": "toy-scope-001",
            "entry_ids": ["kb_tw_mira", "kb_tw_harbor"]
        },
        "rule_refs": ["rule_tw_consistency"]
    }))
    .expect("CheckRequest");
    let finding = load_fixture::<Finding>("fnd_tw_open.json");
    let mut finding_value = serde_json::to_value(finding).expect("finding value");
    finding_value["finding_id"] = json!("fnd_tw_adapter_check");
    let check_finding: Finding =
        serde_json::from_value(finding_value).expect("check finding");

    let result = orchestrate_check(&adapter, request, |input: CheckRunInput| {
        let mut ids: Vec<_> = input.entries.iter().map(|e| e.entry_id.clone()).collect();
        ids.sort();
        assert_eq!(ids, vec!["kb_tw_harbor".to_string(), "kb_tw_mira".to_string()]);
        assert_eq!(input.rules.len(), 1);
        assert_eq!(input.rules[0].rule_id, rule.rule_id);
        assert!(input.entries.iter().any(|e| e.entry_id == mira.entry_id));
        assert!(input.entries.iter().any(|e| e.entry_id == harbor.entry_id));
        spoke_ok(vec![check_finding.clone()])
    });

    assert!(result.is_ok(), "{result:?}");
    if let SpokeResult::Ok(spoke_schemas::CheckResponse::Variant0 { findings, .. }) = result {
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].finding_id, "fnd_tw_adapter_check");
    } else {
        panic!("expected check success");
    }

    adapter.with_store(|store| {
        assert!(store
            .findings
            .iter()
            .any(|f| f.finding_id == "fnd_tw_adapter_check"));
    });
}

#[test]
fn orchestrate_assemble_builds_packet_from_scoped_knowledge_entries() {
    let adapter = ToyWorldAdapter::with_committed_fixtures();
    let request: AssembleRequest = serde_json::from_value(json!({
        "scope": {
            "scope_id": "toy-scope-001",
            "entry_ids": ["kb_tw_mira", "kb_tw_harbor"]
        }
    }))
    .expect("AssembleRequest");

    let result = orchestrate_assemble(&adapter, request);
    assert!(result.is_ok(), "{result:?}");
    if let SpokeResult::Ok(spoke_schemas::AssembleResponse::Variant0 { packet, .. }) = result {
        assert_eq!(packet.packet_id, "assemble:toy-scope-001");
        assert_eq!(packet.entries.len(), 2);
        let mut ids: Vec<_> = packet.entries.iter().map(|e| e.entry_id.clone()).collect();
        ids.sort();
        assert_eq!(ids, vec!["kb_tw_harbor".to_string(), "kb_tw_mira".to_string()]);
    } else {
        panic!("expected assemble success");
    }
}

#[test]
fn put_knowledge_entry_rejects_occ_mismatch() {
    let stored = provisional_mira(json!({
        "entry_id": "kb_tw_occ",
        "revision": 2,
    }));
    let adapter = ToyWorldAdapter::new(Some(MemoryStoreSeed {
        entries: vec![stored.clone()],
        ..Default::default()
    }));
    let mut updated = stored;
    updated.canonical_name = "Stale write".parse().expect("canonical_name");
    updated.revision = Some(3);

    let result = adapter.put_knowledge_entry(updated, Some(1));
    assert!(result.is_reject());
    if let SpokeResult::Reject(reject) = result {
        assert_eq!(reject.code, SpokeRejectCode::StoredRevisionStale);
    }
}

#[test]
fn returns_capability_port_missing_for_baseline_only_adapter() {
    let full = ToyWorldAdapter::default();
    let baseline = as_baseline_only(full);
    let request: ProjectRequest = load_fixture("op_tw_project_request.json");

    let result = orchestrate_project(&baseline, request);
    assert!(result.is_reject());
    if let SpokeResult::Reject(reject) = result {
        assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
    }
}

#[test]
fn full_adapter_orchestrate_project_returns_fixture_shaped_success() {
    let adapter = ToyWorldAdapter::with_committed_fixtures();
    let _: &dyn FullAdapter = &adapter;
    let request: ProjectRequest = load_fixture("op_tw_project_request.json");
    let fixture_response: ProjectResponse = load_fixture("op_tw_project_response.json");

    let result = orchestrate_project(&adapter, request.clone());
    assert!(result.is_ok(), "{result:?}");
    if let (
        SpokeResult::Ok(ProjectResponse::Variant0 {
            session_id,
            entry_id,
            computable,
            ..
        }),
        ProjectResponse::Variant0 {
            computable: fixture_computable,
            ..
        },
    ) = (result, fixture_response)
    {
        assert_eq!(session_id, request.session_id);
        assert_eq!(entry_id, request.entry_id);
        assert_eq!(computable, fixture_computable);
    } else {
        panic!("expected project success");
    }
}

#[test]
fn orchestrate_compute_settle_returns_wire_valid_success_from_fixture() {
    let adapter = ToyWorldAdapter::with_committed_fixtures();
    let request: ComputeRequest = load_fixture("op_tw_compute_settle_request.json");
    let fixture_response: ComputeResponse = load_fixture("op_tw_compute_settle_response.json");

    let result = orchestrate_compute(&adapter, request.clone());
    assert!(result.is_ok(), "{result:?}");
    if let (
        SpokeResult::Ok(ComputeResponse::Variant0 {
            session_id,
            entry_id,
            computable,
            state,
            ..
        }),
        ComputeResponse::Variant0 {
            state: fixture_state,
            ..
        },
    ) = (result, fixture_response)
    {
        assert_eq!(session_id, request.session_id);
        assert_eq!(entry_id, request.entry_id);
        assert_eq!(computable, request.computable);
        assert_eq!(state, fixture_state);
    } else {
        panic!("expected compute success");
    }
}

#[test]
fn list_fork_timeline_events_returns_seeded_events_for_storm_branch() {
    let adapter = ToyWorldAdapter::with_committed_fixtures();
    let storm = load_fixture::<TimelineEvent>("evt_tw_harbor_storm_delay.json");
    let scope: Scope = serde_json::from_value(json!({
        "scope_id": "toy-scope-001",
        "fork_id": "fork_tw_storm_branch"
    }))
    .expect("Scope");

    let result = adapter.list_fork_timeline_events(&scope);
    assert!(result.is_ok(), "{result:?}");
    if let SpokeResult::Ok(events) = result {
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].timeline_event_id,
            storm.timeline_event_id
        );
        assert_eq!(
            events[0].fork_id.as_ref().map(|v| v.as_str()),
            Some("fork_tw_storm_branch")
        );
    }
}

#[test]
fn full_adapter_orchestrate_fork_check_uses_fork_timeline_stub() {
    let adapter = ToyWorldAdapter::with_committed_fixtures();
    let _: &dyn FullAdapter = &adapter;
    let request: CheckRequest = serde_json::from_value(json!({
        "scope": {
            "scope_id": "toy-scope-001",
            "entry_ids": ["kb_tw_mira"],
            "fork_id": "fork_tw_storm_branch"
        }
    }))
    .expect("CheckRequest");
    let mut finding = load_fixture::<Finding>("fnd_tw_open.json");
    finding.finding_id = "fnd_tw_fork_check".into();

    let result = orchestrate_fork_check(&adapter, request, |input: CheckRunInput| {
        assert_eq!(input.events.len(), 1);
        assert_eq!(
            input.events[0].timeline_event_id,
            "evt_tw_harbor_storm_delay"
        );
        spoke_ok(vec![finding.clone()])
    });

    assert!(result.is_ok(), "{result:?}");
    if let SpokeResult::Ok(spoke_schemas::CheckResponse::Variant0 { findings, .. }) = result {
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].finding_id, "fnd_tw_fork_check");
    } else {
        panic!("expected fork check success");
    }
}
