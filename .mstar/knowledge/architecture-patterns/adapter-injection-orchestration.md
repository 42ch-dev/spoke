# Adapter injection orchestration

> **Category:** architecture-patterns  
> **Source:** compound 2026-07-26 (adapter interface protocol)  
> **Packages:** `@42ch/spoke-operations` (npm), `spoke-operations` (crates.io)

## Problem

Pure helpers keep lifecycle invariants consistent, but every product still invents load → validate → save glue and drifts from the shared sequences. Adopters need a clear **implementation protocol**: which methods to implement so operations can drive schema entity lifecycles without the library performing I/O.

## Pattern

1. **Ports live in operations** — TypeScript interfaces and Rust traits (`KnowledgeEntryPort`, `RelationPort`, `ScopeQueryPort`, `FindingPort`, `RuleQueryPort`, `HostManifestPort`, plus optional `ComputablePort` / `ForkTimelineQueryPort`) are exported from the operations packages alongside pure helpers.
2. **Capability composition** — `BaselinePorts` requires six families (the five OCC/query ports plus baseline-required `HostManifestPort`); `ComputablePorts`, `ForkPorts`, `FullPorts` inherit via `BaselinePorts`. Ergonomic aliases `BaselineAdapter`, `ComputableAdapter`, `ForkAdapter`, `FullAdapter` name the same compositions (TS: `export type *Adapter = *Ports`; Rust: marker trait `*Adapter: *Ports` + blanket impl). Claim `spoke-baseline` / `l2-computable` / `l5-fork` and implement the required `*Port` families on one adapter type.
3. **Injection orchestration** — per-op entrypoints (`orchestrateUpsert`, …, `orchestrateForkAssemble`) take composed ports (+ `runChecker` on check paths), call pure helpers, and perform all reads/writes through ports. The library stays I/O-free.
4. **Async `SpokeResult` ports** — normative port methods return `Promise<SpokeResult<T>>` (TS) / `#[async_trait] async fn … -> SpokeResult<T>` with `Send` futures (Rust); all `orchestrate*` are async entrypoints that await every injected port call. The library stays I/O-free; pure helpers and checker callbacks remain sync. The sync surface was removed (pre-1.0 breaking): no sync variants, no `T | Promise<T>` unions, no shims. Migration method: [`async-surface-migration-frozen-contract.md`](../conventions/async-surface-migration-frozen-contract.md).
5. **Missing optional ports** — dynamic boundaries return `CAPABILITY_PORT_MISSING` (not throw/panic).
6. **Check injectee** — checkers remain product-owned via `CheckRunInput` + `runChecker` callback after ports load scoped data.
7. **Persisted-entity OCC** — both `KnowledgeEntryPort` and `RelationPort` require compare-and-put on their `put*` methods. `putKnowledgeEntry(entry, expectedBaseRevision)` and `putRelation(relation, expectedBaseRevision)` treat `null`/`None` as create (entity must be absent) and a non-null value as the store’s required current revision before accepting the write (`STORED_REVISION_STALE` or `REVISION_CONFLICT` on mismatch; `RELATION_ALREADY_EXISTS` / `KNOWLEDGE_ENTRY_ALREADY_EXISTS` when a create finds the id present). Orchestrators pass the loaded stored revision (or `null`/`None` on create). The two ports are full parity: the same load → validate → OCC → put sequence drives both entity lifecycles. See [`relation-occ-parity.md`](relation-occ-parity.md) for the focused Relation OCC pattern.

## Gotchas

- **Transactions:** multi-entry `orchestrateUpsert` is not atomic; adapters own transaction boundaries around port `put*` calls.
- **Uniqueness peers:** active-uniqueness helpers evaluate **caller-supplied peer sets**. Orchestration supplies batch-local peers; store-wide uniqueness requires the adapter to load a wider peer snapshot into that helper.
- **Fork queries:** fork check/assemble load knowledge entries via `ScopeQueryPort.listKnowledgeEntries` and timeline events via `ForkTimelineQueryPort.listForkTimelineEvents` (not baseline `listTimelineEvents`).
- Product DTO mapping in **consumer product repositories** implements these ports; reference examples live under `fixtures/toy-world/`. They are not a second copy of lifecycle rules.

## See also

- `.mstar/specs/spoke-operations.md` — Adapter Interfaces + Injection Orchestration + Host collaboration
- `architecture-patterns/host-capability-manifest-collaboration.md` — `HostCapabilityManifest` roles, namespace exclusivity, peer-list rules
- `architecture-patterns/spoke-operations-pure-actions.md` — pure helper families
- `architecture-patterns/rust-spoke-operations-parity.md` — Rust crate layout and reject-code parity
- `architecture-patterns/encapsulated-remote-adapter-bridge.md` — a `RemoteAdapter` that implements the same ports by proxying over a connect session (drop-in remote adapter)
- `fixtures/toy-world/` — reference `ToyWorldAdapter` (TS: `src/adapter/`; Rust: `spoke-fixture-toy-world` under `rust/`)

## Adopter path

1. Choose capability flags (`spoke-baseline`, optional `l2-computable`, optional `l5-fork`).
2. Implement **one** adapter type satisfying the matching `*Port` families (type as `BaselineAdapter` / `FullAdapter` aliases).
3. Call `orchestrate*` / `orchestrate_*` with that adapter; own persistence, transactions, and DTO mapping in the product runtime.
4. Copy patterns from `fixtures/toy-world/` (`ToyWorldAdapter` TS + `spoke-fixture-toy-world` Rust).
