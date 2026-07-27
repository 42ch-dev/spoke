//! Reference ToyWorldAdapter for the toy-world fixture graph.
//!
//! Baseline ports use an in-memory OCC store; computable / fork return
//! wire-valid stubs seeded from sibling JSON under `fixtures/toy-world/`.

mod memory_store;
mod toy_world_adapter;

pub use memory_store::{toy_world_fixtures_root, MemoryStore, MemoryStoreSeed};
pub use toy_world_adapter::{as_baseline_only, BaselineOnlyAdapter, ToyWorldAdapter};
