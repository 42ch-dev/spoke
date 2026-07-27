# Adapters

Capability-sliced **adapter port contracts**, **`*Adapter` composed-port aliases**, and **injection orchestration** live in the operations packages:

- TypeScript: [`@42ch/spoke-operations`](../packages/spoke-operations/) (`KnowledgeEntryPort`, `BaselineAdapter` / `FullAdapter`, `orchestrateUpsert`, …)
- Rust: [`spoke-operations`](../crates/spoke-operations/) (`KnowledgeEntryPort`, `BaselineAdapter` / `FullAdapter`, `orchestrate_upsert`, …)

Normative matrix and sequences: [`.mstar/specs/spoke-operations.md`](../.mstar/specs/spoke-operations.md) § Adapter Interfaces / Injection Orchestration.

**Reference implementations** — `ToyWorldAdapter` under [`fixtures/toy-world/`](../fixtures/toy-world/) (TypeScript `src/adapter/`; Rust crate `spoke-fixture-toy-world` in `rust/`, `publish = false`). FullAdapter stubs seed from committed fixture JSON; baseline-only negative tests demonstrate `CAPABILITY_PORT_MISSING`.

**Product bindings** — packages that implement ports for a concrete product store ship in **consumer repositories** when scheduled. Each binding:

1. Implements the operations port traits/interfaces for that product store
2. Maps product DTOs to and from SPOKE wire types (`@42ch/spoke-schemas` / `spoke-schemas`)
3. Preserves `extensions.<namespace>` on round-trip
4. Calls operations orchestration entrypoints for shared lifecycle sequences
5. Owns transport, persistence, retries, and **transaction boundaries** (including multi-entry upsert)

Active-uniqueness helpers in operations take **caller-supplied peer sets**. Orchestration passes batch-local peers; adapters that need store-wide uniqueness load a wider peer snapshot before calling those helpers.

This root directory is a transitional placeholder (README only) until implement plan removes it. Schemas in [`schemas/`](../schemas/) remain the protocol SSOT. See [`AGENTS.md`](../AGENTS.md) and [`.mstar/specs/spoke-protocol.md`](../.mstar/specs/spoke-protocol.md).
