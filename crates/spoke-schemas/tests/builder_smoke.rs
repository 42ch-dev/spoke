//! Builder + wire smoke tests for the canonical re-exports.
//!
//! Locks in two contracts that are easy to misread from generated code:
//!  - the typify `Builder` IS exposed on the canonical re-export (`Scope::builder()`),
//!    so downstream can construct without exhaustive struct literals;
//!  - the additive `Scope.extensions` field round-trips through JSON on the canonical type.

#[test]
fn scope_builder_exists_on_canonical_reexport() {
    // `rust-gen` sets typify `with_struct_builder(true)`; the builder travels with the type
    // through the `pub use ...::*` re-export. This guards against regressions and clarifies
    // the supported non-breaking construction path for downstream Rust consumers.
    let _builder = spoke_schemas::Scope::builder();
}

#[test]
fn scope_extensions_round_trip_on_canonical_type() {
    // A Scope carrying product-scoped extensions (the additive wire field) round-trips via JSON.
    let json = r#"{"scope_id":"world_1","extensions":{"nexus":{"branch_id":"br_a"}}}"#;
    let scope: spoke_schemas::Scope = serde_json::from_str(json).expect("deserialize Scope");
    assert_eq!(scope.scope_id, "world_1");
    assert!(!scope.extensions.is_empty(), "extensions carried on canonical Scope");

    // Re-serialize; extensions preserved (empty maps are skipped, non-empty are emitted).
    let back = serde_json::to_string(&scope).expect("serialize Scope");
    assert!(back.contains("nexus"), "extensions survive round-trip");
}
