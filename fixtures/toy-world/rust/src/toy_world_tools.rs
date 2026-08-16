//! Deterministic toy-world tool handlers — the copyable provider reference
//! (mirror of `src/adapter/toy-world-tools.ts`).
//!
//! Both handlers are pure functions of their arguments (no I/O, no global
//! state): `roll_dice` derives a seed from the arguments, `lore_lookup`
//! reads the store without mutating it. Same arguments always produce the
//! same result, so e2e assertions are stable.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use serde_json::{json, Value};
use spoke_operations::{spoke_ok, spoke_reject, SpokeRejectCode, SpokeResult};

use crate::memory_store::MemoryStore;

/// Frozen tool capability ids (docs/snippet byte-parity).
pub const TOY_WORLD_ROLL_DICE_ID: &str = "tools.toy_world.roll_dice";
pub const TOY_WORLD_LORE_LOOKUP_ID: &str = "tools.toy_world.lore_lookup";

/// Registered tool handler — shape mirrors `spoke-connect`'s `ToolHandler`
/// (the plan-3 serving surface): receives the tool arguments JSON value and
/// resolves with the tool result as a `SpokeResult`. Kept local so the
/// fixture stays dependency-free beyond operations/schemas.
pub type ToolHandler =
    Arc<dyn Fn(Value) -> BoxFuture<'static, SpokeResult<Value>> + Send + Sync>;

/// 64-bit FNV-1a hash — stable across runs and platforms.
fn fnv1a(input: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

/// SplitMix64 PRNG — deterministic for a given 64-bit seed.
fn splitmix64(seed: u64) -> impl FnMut() -> u64 {
    let mut state = seed;
    move || {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }
}

/// Deterministic dice handler: `count` rolls of `sides`-sided dice.
/// The seed is derived from `count` and `sides`, so the same arguments
/// always produce the same rolls. `sides` defaults to 6.
pub fn roll_dice(args: &Value) -> SpokeResult<Value> {
    let count = args.get("count").and_then(Value::as_u64);
    let sides = args
        .get("sides")
        .and_then(Value::as_u64)
        .unwrap_or(6);
    let Some(count) = count else {
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            "roll_dice count must be a positive integer",
            None,
        );
    };
    if count == 0 {
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            "roll_dice count must be a positive integer",
            None,
        );
    }
    if sides < 2 {
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            "roll_dice sides must be an integer >= 2",
            None,
        );
    }

    let mut next = splitmix64(fnv1a(&format!("{count}:{sides}")));
    let rolls: Vec<Value> = (0..count)
        .map(|_| json!(1 + (next() % sides)))
        .collect();
    let total: u64 = rolls.iter().filter_map(Value::as_u64).sum();
    spoke_ok(json!({ "rolls": rolls, "total": total }))
}

/// Read-only lore handler: look up a toy-world KnowledgeEntry by id from
/// the adapter store. The store's own reject (e.g.
/// `KNOWLEDGE_ENTRY_NOT_FOUND`) passes through unchanged.
pub fn lore_lookup(store: &MemoryStore, args: &Value) -> SpokeResult<Value> {
    let Some(entry_id) = args.get("entry_id").and_then(Value::as_str) else {
        return spoke_reject(
            SpokeRejectCode::InvalidInput,
            "lore_lookup entry_id must be a non-empty string",
            None,
        );
    };
    match store.get_knowledge_entry(entry_id) {
        SpokeResult::Ok(entry) => spoke_ok(json!({ "entry": entry })),
        SpokeResult::Reject(reject) => SpokeResult::Reject(reject),
    }
}

/// Default handler registry for a toy-world adapter: both frozen tools are
/// servable out of the box. `lore_lookup` is bound to the adapter store.
pub fn default_tool_handlers(
    store: Arc<Mutex<MemoryStore>>,
) -> HashMap<String, ToolHandler> {
    let mut handlers: HashMap<String, ToolHandler> = HashMap::new();
    handlers.insert(
        TOY_WORLD_ROLL_DICE_ID.to_owned(),
        Arc::new(|args: Value| Box::pin(async move { roll_dice(&args) })),
    );
    let lore_store = store.clone();
    handlers.insert(
        TOY_WORLD_LORE_LOOKUP_ID.to_owned(),
        Arc::new(move |args: Value| {
            let store = lore_store.clone();
            Box::pin(async move {
                let store = store.lock().expect("toy-world store lock");
                lore_lookup(&store, &args)
            })
        }),
    );
    handlers
}
