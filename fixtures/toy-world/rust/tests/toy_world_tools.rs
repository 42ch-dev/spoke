//! Tool-surface parity tests for the toy-world Rust reference provider:
//! manifest consistency (plan-2 `validate_manifest_tools`) + deterministic
//! handlers (plan-3 serving surface).

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use spoke_fixture_toy_world::toy_world_tools::roll_dice;
use spoke_fixture_toy_world::{toy_world_fixtures_root, MemoryStore, ToyWorldAdapter};
use spoke_operations::{
    find_tool, list_tools, validate_manifest_tools, SpokeRejectCode, SpokeResult,
};
use spoke_schemas::HostCapabilityManifest;

/// Frozen tool ids (docs/snippet byte-parity).
const TOY_WORLD_ROLL_DICE_ID: &str = "tools.toy_world.roll_dice";
const TOY_WORLD_LORE_LOOKUP_ID: &str = "tools.toy_world.lore_lookup";

fn self_manifest() -> HostCapabilityManifest {
    let path = toy_world_fixtures_root().join("host_tw_primary.json");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", path.display());
    });
    serde_json::from_str(&raw).unwrap_or_else(|error| {
        panic!("failed to parse {}: {error}", path.display());
    })
}

fn roll_value(value: &Value) -> (Vec<i64>, i64) {
    let rolls = value["rolls"]
        .as_array()
        .expect("rolls array")
        .iter()
        .map(|roll| roll.as_i64().expect("integer roll"))
        .collect::<Vec<_>>();
    let total = value["total"].as_i64().expect("integer total");
    (rolls, total)
}

#[test]
fn manifest_declares_both_frozen_tools_and_passes_validate_manifest_tools() {
    let manifest = self_manifest();

    assert!(validate_manifest_tools(&manifest).is_ok());

    let descriptors = list_tools(&manifest);
    let ids: Vec<&str> = descriptors
        .iter()
        .map(|descriptor| descriptor.capability_id.as_str())
        .collect();
    assert_eq!(ids, vec![TOY_WORLD_ROLL_DICE_ID, TOY_WORLD_LORE_LOOKUP_ID]);

    for capability_id in [TOY_WORLD_ROLL_DICE_ID, TOY_WORLD_LORE_LOOKUP_ID] {
        assert!(
            manifest
                .capabilities
                .iter()
                .any(|capability| capability.as_str() == capability_id),
            "capabilities[] must list {capability_id}"
        );
        let namespace = capability_id.split('.').nth(1).expect("three segments");
        assert!(
            manifest
                .namespaces
                .iter()
                .any(|owned| owned.as_str() == namespace),
            "namespaces[] must own {namespace}"
        );
        let descriptor = find_tool(&manifest, capability_id).expect("declared tool");
        assert_eq!(descriptor.op.as_str(), capability_id);
    }
}

#[test]
fn adapter_tool_descriptors_matches_manifest() {
    let adapter = ToyWorldAdapter::with_committed_fixtures();
    let ids: Vec<String> = adapter
        .tool_descriptors()
        .iter()
        .map(|descriptor| descriptor.capability_id.to_string())
        .collect();
    assert_eq!(
        ids,
        vec![TOY_WORLD_ROLL_DICE_ID.to_string(), TOY_WORLD_LORE_LOOKUP_ID.to_string()]
    );
}

#[test]
fn roll_dice_is_deterministic_same_args_same_rolls() {
    let adapter = ToyWorldAdapter::with_committed_fixtures();
    let args = json!({ "count": 3, "sides": 6 });

    let first = pollster::block_on(adapter.invoke_tool(TOY_WORLD_ROLL_DICE_ID, args.clone()));
    let second = pollster::block_on(adapter.invoke_tool(TOY_WORLD_ROLL_DICE_ID, args));

    assert_eq!(first, second, "same arguments must produce identical rolls");
    let (rolls, total) = match first {
        SpokeResult::Ok(value) => roll_value(&value),
        SpokeResult::Reject(reject) => panic!("roll_dice rejected: {reject:?}"),
    };
    assert_eq!(rolls.len(), 3);
    for roll in &rolls {
        assert!((1..=6).contains(roll), "roll {roll} out of range 1..=6");
    }
    assert_eq!(total, rolls.iter().sum::<i64>());
}

#[test]
fn roll_dice_defaults_sides_to_6() {
    let adapter = ToyWorldAdapter::with_committed_fixtures();
    let result = pollster::block_on(adapter.invoke_tool(TOY_WORLD_ROLL_DICE_ID, json!({ "count": 1 })));

    let (rolls, _) = match result {
        SpokeResult::Ok(value) => roll_value(&value),
        SpokeResult::Reject(reject) => panic!("roll_dice rejected: {reject:?}"),
    };
    assert_eq!(rolls.len(), 1);
    assert!((1..=6).contains(&rolls[0]), "default-sided roll out of range");
}

#[test]
fn roll_dice_rejects_missing_count_and_invalid_sides() {
    let adapter = ToyWorldAdapter::with_committed_fixtures();

    let missing = pollster::block_on(adapter.invoke_tool(TOY_WORLD_ROLL_DICE_ID, json!({})));
    assert_eq!(reject_code(missing), SpokeRejectCode::InvalidInput);

    let bad_sides = pollster::block_on(adapter.invoke_tool(
        TOY_WORLD_ROLL_DICE_ID,
        json!({ "count": 1, "sides": 1 }),
    ));
    assert_eq!(reject_code(bad_sides), SpokeRejectCode::InvalidInput);
}

#[test]
fn roll_dice_rejects_invalid_sides_variants_with_field_details() {
    let adapter = ToyWorldAdapter::with_committed_fixtures();

    // TS mirror parity: a present-but-invalid `sides` (`-1`, `1.5`, `"x"`,
    // `0`, `1`) is rejected with INVALID_INPUT and details `{ field: "sides" }`
    // instead of silently defaulting to 6.
    let variants: [Value; 5] = [json!(-1), json!(1.5), json!("x"), json!(0), json!(1)];
    for invalid in variants {
        let result = pollster::block_on(adapter.invoke_tool(
            TOY_WORLD_ROLL_DICE_ID,
            json!({ "count": 1, "sides": invalid }),
        ));
        let reject = match result {
            SpokeResult::Ok(value) => panic!("roll_dice accepted invalid sides {invalid}: {value}"),
            SpokeResult::Reject(reject) => reject,
        };
        assert_eq!(reject.code, SpokeRejectCode::InvalidInput);
        assert_eq!(
            reject.details,
            Some(
                json!({ "field": "sides" })
                    .as_object()
                    .expect("details object")
                    .clone()
            ),
            "invalid sides {invalid} must carry field details"
        );
    }
}

#[test]
fn lore_lookup_returns_seeded_entry_read_only() {
    let adapter = ToyWorldAdapter::with_committed_fixtures();
    let result = pollster::block_on(adapter.invoke_tool(
        TOY_WORLD_LORE_LOOKUP_ID,
        json!({ "entry_id": "kb_tw_mira" }),
    ));

    match result {
        SpokeResult::Ok(value) => {
            assert_eq!(value["entry"]["entry_id"].as_str(), Some("kb_tw_mira"));
        }
        SpokeResult::Reject(reject) => panic!("lore_lookup rejected: {reject:?}"),
    }
}

#[test]
fn lore_lookup_rejects_unknown_entries_and_missing_entry_id() {
    let adapter = ToyWorldAdapter::with_committed_fixtures();

    let unknown = pollster::block_on(adapter.invoke_tool(
        TOY_WORLD_LORE_LOOKUP_ID,
        json!({ "entry_id": "kb_tw_nope" }),
    ));
    assert_eq!(reject_code(unknown), SpokeRejectCode::KnowledgeEntryNotFound);

    let missing = pollster::block_on(adapter.invoke_tool(TOY_WORLD_LORE_LOOKUP_ID, json!({})));
    assert_eq!(reject_code(missing), SpokeRejectCode::InvalidInput);
}

#[test]
fn invoke_tool_rejects_unlisted_tool_with_capability_port_missing() {
    let adapter = ToyWorldAdapter::with_committed_fixtures();
    let result = pollster::block_on(adapter.invoke_tool("tools.toy_world.snipe", json!({})));
    let reject = match result {
        SpokeResult::Ok(_) => panic!("expected reject, got Ok"),
        SpokeResult::Reject(reject) => reject,
    };
    // M2: structured details parity with the TS adapter —
    // `{ capability: capability_id }` on the unlisted-tool reject.
    assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
    assert_eq!(
        reject.details,
        Some(json!({ "capability": "tools.toy_world.snipe" }).as_object().expect("details object").clone())
    );
}

#[test]
fn invoke_tool_rejects_declared_tool_without_registered_handler() {
    // M1: explicit empty handler registry — the manifest still declares
    // both frozen tools, but the adapter serves none (provider-bug state).
    let adapter = ToyWorldAdapter::from_store_with_handlers(
        MemoryStore::from_committed_fixtures(),
        HashMap::new(),
    );
    let result = pollster::block_on(adapter.invoke_tool(
        TOY_WORLD_ROLL_DICE_ID,
        json!({ "count": 1 }),
    ));
    let reject = match result {
        SpokeResult::Ok(_) => panic!("expected reject, got Ok"),
        SpokeResult::Reject(reject) => reject,
    };
    assert_eq!(reject.code, SpokeRejectCode::CapabilityPortMissing);
    assert!(
        reject.message.contains("declared but has no registered handler"),
        "message must name the declared-but-unregistered state: {}",
        reject.message
    );
    assert_eq!(
        reject.details,
        Some(json!({ "capability": TOY_WORLD_ROLL_DICE_ID }).as_object().expect("details object").clone())
    );
}

#[test]
fn invoke_tool_rejects_non_tool_capability_id() {
    let adapter = ToyWorldAdapter::with_committed_fixtures();
    let result = pollster::block_on(adapter.invoke_tool("spoke-baseline", json!({})));
    assert_eq!(reject_code(result), SpokeRejectCode::InvalidInput);
}

#[test]
fn register_tool_handler_overwrites_last_wins_and_panics_on_non_tool_id() {
    let adapter = ToyWorldAdapter::with_committed_fixtures();

    let outcome = std::panic::catch_unwind(|| {
        adapter.register_tool_handler("spoke-baseline", Arc::new(|_| Box::pin(async { SpokeResult::Ok(json!(null)) })));
    });
    assert!(outcome.is_err(), "non-tool capability id must panic");

    adapter.register_tool_handler(
        TOY_WORLD_ROLL_DICE_ID,
        Arc::new(|args| Box::pin(async move { SpokeResult::Ok(json!({ "custom": args })) })),
    );
    let result = pollster::block_on(adapter.invoke_tool(TOY_WORLD_ROLL_DICE_ID, json!({ "count": 1 })));
    match result {
        SpokeResult::Ok(value) => assert_eq!(value, json!({ "custom": { "count": 1 } })),
        SpokeResult::Reject(reject) => panic!("custom handler rejected: {reject:?}"),
    }
}

#[test]
fn roll_dice_handler_is_pure_and_deterministic() {
    let args = json!({ "count": 4, "sides": 8 });
    let first = roll_dice(&args);
    let second = roll_dice(&args);
    assert_eq!(first, second, "pure handler: same args, same rolls");
    let (rolls, total) = match first {
        SpokeResult::Ok(value) => roll_value(&value),
        SpokeResult::Reject(reject) => panic!("roll_dice rejected: {reject:?}"),
    };
    assert_eq!(rolls.len(), 4);
    for roll in &rolls {
        assert!((1..=8).contains(roll), "roll {roll} out of range 1..=8");
    }
    assert_eq!(total, rolls.iter().sum::<i64>());
}

fn reject_code<T>(result: SpokeResult<T>) -> SpokeRejectCode {
    match result {
        SpokeResult::Ok(_) => panic!("expected reject, got Ok"),
        SpokeResult::Reject(reject) => reject.code,
    }
}
