//! Builder + wire smoke tests for the canonical re-exports.
//!
//! Locks in the contracts that are easy to misread from generated code:
//!  - the typify `Builder` IS exposed on the canonical re-export (`Scope::builder()`),
//!    so downstream can construct without exhaustive struct literals;
//!  - the full builder migration compiles end-to-end and produces a valid `Scope`;
//!  - the additive `Scope.extensions` field round-trips through JSON on the canonical type.

#[test]
fn scope_builder_full_chain_compiles_on_canonical_reexport() {
    // The documented 0.x Rust migration for the `extensions` source break:
    // exhaustive literal -> typify Builder, terminated via `TryFrom`/`try_into()` (not `.build()`).
    let scope: spoke_schemas::Scope = spoke_schemas::Scope::builder()
        .scope_id("world_1")
        .try_into()
        .expect("builder -> Scope via try_into");
    assert_eq!(scope.scope_id, "world_1");
}

#[test]
fn scope_extensions_round_trip_on_canonical_type() {
    // A Scope carrying product-scoped extensions (the additive wire field) round-trips via JSON.
    let json = r#"{"scope_id":"world_1","extensions":{"nexus":{"branch_id":"br_a"}}}"#;
    let scope: spoke_schemas::Scope = serde_json::from_str(json).expect("deserialize Scope");
    assert_eq!(scope.scope_id, "world_1");
    assert!(!scope.extensions.is_empty(), "extensions carried on canonical Scope");

    let back = serde_json::to_string(&scope).expect("serialize Scope");
    assert!(back.contains("nexus"), "extensions survive round-trip");
}
