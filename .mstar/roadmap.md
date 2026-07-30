# SPOKE Roadmap

> Living **project** roadmap (tracked result). Strategy and architecture live in [`STRATEGY.md`](../STRATEGY.md) and [`.mstar/specs/`](specs/). Per-slice execution detail stays in local `delivery-compass.md` (process; gitignored).

**Updated:** 2026-07-30  
**North star:** Cross-product KnowledgeEntry dialect for check + assemble I/O across independent product runtimes.

---

## Now (in progress)

**Integrator notes (durable):** Adapters own transaction boundaries for multi-entry upsert. Active-uniqueness helpers evaluate **caller-supplied peer sets** — orchestration supplies batch-local peers; store-wide uniqueness is available when the adapter loads a wider peer snapshot into that helper. Composed-port aliases (`BaselineAdapter`, `ComputableAdapter`, `ForkAdapter`, `FullAdapter`) export from `@42ch/spoke-operations` and `spoke-operations`; reference examples and conformance harness live under `fixtures/toy-world/`. Baseline composition includes required `HostManifestPort` for host self-description and product-supplied peer manifests.

**Persisted-entity OCC parity (durable invariant):** every entity with a write `*Port` and a create-or-update op carries structural `revision` and is persisted through `put*(entity, expectedBaseRevision)` (null/None on create, stored revision on update). `KnowledgeEntry` (upsert/promote) and `Relation` (relate) are the carriers; `Finding`, `Rule`, `HostCapabilityManifest`, `TimelineEvent`, `SourceAnchor`, and `AssemblePacket` are exempt by design (see `spoke-data-model.md` §Persisted-entity OCC parity). `orchestrateRelate` / `orchestrate_relate` deep-integrate load → validate(create/update) → OCC → put, symmetric with `orchestrateUpsert`. Relation error codes: `RELATION_NOT_FOUND`, `RELATION_ALREADY_EXISTS` (plus reused `STORED_REVISION_STALE` / `REVISION_CONFLICT`).

**Extension bags — core / modules / extensions triad (durable):** SPOKE carries three placement homes. **Core fields** serve all baseline hosts. **`extensions.<product>`** is a product/adapter-owned namespace bag on durable data objects and on `Scope` (product-scoped query metadata, e.g. branch / search filters); adapters round-trip unknown namespaces verbatim. **`modules.*`** is the cross-product **functional**-dialect bag — **shipped** as optional `ModuleMap` (object or array namespace values) on `KnowledgeEntry` + `AssemblePacket`, behind the **`narrative-modules`** capability flag (opt-in, same pattern as `l2-computable` / `l5-fork`); inner dialect shapes (activation / pack / placement / activation_trace) stay handbook-defined. Functional dialects never live under `extensions.*` (see [`spoke-extension-modules.md`](specs/spoke-extension-modules.md)).

**Assemble recipes + `modules` wire shipped:** the narrative-dialect foundation for multi-host lore interchange is complete (Phase A exit met).

---

## Up next (planned)

| Slice | What ships | Notes |
|-------|------------|-------|
| **Integrator docs site** | VitePress `docs/` + GitHub Pages publishing Domain Profile handbooks and CONCEPTS-aligned guides | Consumer-facing home for profile handbooks; repo-local SSOT remains `.mstar/specs/` until promoted |
| **Registry release** (when convenient) | SemVer cut when maintainers choose after additive wire lands on `main` | Optional via **New release** — not gated on docs site |
| **Consumer pack I/O + activation engines** | Narrative hosts implement pack import/export, keyword/hop activation, assemble inspectors against handbooks + optional `modules` wire | Product-local engines and UX — **outside** this protocol repository; protocol stays wire + pure helpers only |

Consumer-repo multi-adapter composition using manifests for in-process discovery remains product-side work outside this protocol repository.

---

## Done (delivered)

Newest first. Dates are delivery dates on `main`.

| When | Slice | What landed |
|------|-------|-------------|
| 2026-07-30 | Phase A close: `modules` shipped + assemble recipes | `ModuleMap` (object\|array namespace values) shipped as **optional, capability-flagged (`narrative-modules`)** on `KnowledgeEntry` + `AssemblePacket`; codegen (count 24); `mergeModuleMaps`/`preserveModuleMaps` pure ops helpers (generalized namespace-merge core; extension helpers unchanged); ADR/CONCEPTS/handbooks flipped proposed → shipped; AssemblePacket `placement[]`/`activation_trace[]` recipes handbook ([assemble-module-recipes.md](specs/assemble-module-recipes.md)); **Phase A exit met — Phase B (Nexus) unlocked**; knowledge `capability-flagged-optional-bag` |
| 2026-07-30 | Naming triad + Scope.extensions + W1 handbooks | Core / **proposed** `modules.*` / `extensions.<product>` triad ADR ([spoke-extension-modules.md](specs/spoke-extension-modules.md)) + CONCEPTS; optional `Scope.extensions` (`ExtensionMap`) wire + codegen TS/Rust + matcher-ignore preserve tests + fixture (unblocks consumer product query metadata); lore-activation Domain Profile ([domain-profile-lore-activation.md](specs/domain-profile-lore-activation.md), proposed `modules.activation`); Narrative Knowledge Pack handbook ([domain-profile-narrative-knowledge-pack.md](specs/domain-profile-narrative-knowledge-pack.md)) + Seed/Pool pattern + toy-world companion fixture; knowledge `proposed-wire-shape-companion-fixture` |
| 2026-07-29 | Relation OCC parity | `Relation.revision` wire (optional, integer ≥ 0); `RelationPort` OCC parity (`getRelation` + `putRelation(relation, expectedBaseRevision)`, TS+Rust); `orchestrateRelate` deep-integrated load→validate(create/update)→OCC→put; relate gate create/update rules; `RELATION_NOT_FOUND`/`RELATION_ALREADY_EXISTS`; toy-world OCC-aware adapter; dual-language tests; normative persisted-entity OCC-parity guardrail; knowledge `relation-occ-parity` |
| 2026-07-28 | Beat-assist protocol slice | Domain Profile handbook [`domain-profile-narrative-structure.md`](specs/domain-profile-narrative-structure.md); Harbor ordered-moment beat chain + KE-scoped `precedes`; pure timeline sequence helpers (TS + Rust); knowledge `beat-assist-moment-sequence` |
| 2026-07-27 | Host capability collaboration | `HostCapabilityManifest` schema + codegen (inventory **24**); five-role / ns-exclusivity / authority normative specs; baseline-required `HostManifestPort` (TS+Rust); toy-world dual manifests + fixture-backed adapters |
| 2026-07-27 | ToyWorldAdapter reference examples | Runnable TS `ToyWorldAdapter` (`fixtures/toy-world/src/adapter/`) + Rust `spoke-fixture-toy-world` (`fixtures/toy-world/rust/`); CI `cargo test -p spoke-fixture-toy-world` |
| 2026-07-27 | Adapter aliases | `*Adapter` composed-port aliases in TS/Rust ops; integrator docs point to operations + `fixtures/toy-world/` |
| 2026-07-26 | Adapter interfaces + injection orchestration | Capability-sliced ports + `orchestrate*` / `orchestrate_*` in TS and Rust ops packages; `CAPABILITY_PORT_MISSING`; mock-backed orchestration tests; normative matrix in [spoke-operations.md](specs/spoke-operations.md) |
| 2026-07-26 | L2 typed closed body + trait helpers | Closed `body` with optional `summary` / `tags` / `BodyAttribute[]`; codegen; Mira traits fixture; pure TS/Rust trait read helpers; SemVer-agnostic `bump-version` tests; knowledge `l2-closed-body-and-traits` + `release-bump-tests-version-agnostic` |
| 2026-07-25 | CI / codegen harden | `OpaqueJson` empty schema for opaque log fields; TS/Rust regen; Rust typify duplication strategy A + import guide; schema-count surfaces at **23**; `test:release` lockstep assert/bump unit tests in CI |
| 2026-07-25 | Rust ops CI + publish | CI/release verify `cargo test -p spoke-operations`; lockstep assert for ops crate; crates.io `spoke-operations` after `spoke-schemas`; README EN/CN Rust ops pin/install |
| 2026-07-25 | Registry publish | CI `publish-npm` + `publish-crates` on stable tags after verify; npm `@42ch/spoke-schemas` / `@42ch/spoke-operations`; crates.io `spoke-schemas`; skip `-rc.` tags |
| 2026-07-25 | CHANGELOG release notes | `CHANGELOG.md` via git-cliff; bump regenerates sections; GitHub Release body prefers changelog section over tag annotation |
| 2026-07-25 | Unified version release | Lockstep assert + `release:bump` tooling; CI `verify-version` + tag-gated `release.yml`; README EN/CN pinning + maintainer how-to; [spoke-version-release.md](specs/spoke-version-release.md) |
| 2026-07-24 | Optional Fork (`l5-fork`) | `ForkId`; TimelineEvent `fork_id` / `parent_fork_id`; `Scope.fork_id` + matcher; fixtures; Moment `computable_logs` schema example; schema-count **23** |
| 2026-07-24 | Optional Computable (`l2-computable`) | `body.state` / `body.computable`, Moment `computable_logs`, Session lifecycle normative; optional `project`/`compute` ops; pure validators; fixtures; schema-count **23** |
| 2026-07-23 | KnowledgeEntry / TimelineEvent terminology | Wire locks `KnowledgeEntry` / `TimelineEvent`; ops API + `*KNOWLEDGE_ENTRY*` error codes; fixtures dual-concern pair; product expand **Standardized Programmable Ontology Knowledge Engine** (SPOKE acronym kept) |
| 2026-07-23 | Fixture harness ownership + CI harden | AJV/Vitest under `fixtures/toy-world/tests/` (`@42ch/spoke-fixture-toy-world`); removed from `@42ch/spoke-operations`; `AGENTS.md` boundary; CI `test:fixtures`; `verify-codegen` schema-count assert |
| 2026-07-23 | Operations library deepen + fixtures | OCC compare, KnowledgeEntry status, uniqueness, Scope/upsert/relate gates, error-envelope map; `fixtures/toy-world/` protocol JSON graph |
| 2026-07-23 | Protocol layers + Rule/TimelineEvent | Normative L0–L8 + capability levels; `Rule` + `TimelineEvent` schemas; Scope / error-envelope / Rule-aware `check` |
| 2026-07-23 | Operations library first slice | `@42ch/spoke-operations`: promote, Finding transitions, extensions preserve, AssemblePacket builders; consumer README EN/CN |
| 2026-07-23 | v0.1 bootstrap | `schemas/` SSOT, codegen `@42ch/spoke-schemas` + Rust `spoke-schemas`, CI verify gate |

### Baseline inventory (as of last Done)

| Area | Status |
|------|--------|
| Data wire | KnowledgeEntry (closed L2 `body`: optional `summary`, `tags`, `attributes`; optional `body.state`/`body.computable`, `computable_logs` under `l2-computable`), **Relation (optional `revision` for OCC parity with KnowledgeEntry)**, SourceAnchor, Finding, AssemblePacket, **HostCapabilityManifest**, Rule, TimelineEvent + `extensions`; optional `fork_id`/`parent_fork_id` on TimelineEvent (`l5-fork`); `OpaqueJson` for opaque log field values |
| Ops wire | upsert / promote / relate / check / assemble (+ Scope, error-envelope); optional project / compute (`l2-computable`). **`relate` is OCC-deep-integrated (load→validate→OCC→put) symmetric with upsert** |
| Ops library | Pure TS + Rust helpers over wire types (incl. `Scope.fork_id` TimelineEvent match; KnowledgeEntry / TimelineEvent naming) plus adapter port contracts (six baseline families incl. `HostManifestPort`), `*Adapter` composed-port aliases, and injection orchestration (`spoke-baseline`, optional `l2-computable`, optional `l5-fork`) |
| Fixtures | `fixtures/toy-world/` samples + conformance + reference Adapter examples (dual-concern ontology `"event"` + TimelineEvent; Fork-aware TimelineEvent sample under `l5-fork`) |
| Codegen / CI | Protocol schema inventory **24** (`HostCapabilityManifest` included); `verify-codegen`; `test:release` for lockstep assert/bump; Rust typify strategy A documented |
| Specs / vocabulary | Umbrella, layers, data-model, ops wire, operations library under `.mstar/specs/`; CONCEPTS + knowledge vocabulary pattern |

---

## Out of scope (durable)

Do not schedule these into SPOKE itself unless strategy is explicitly reversed:

- Shared daemon / MCP server / single multi-product runtime
- I/O, LLM, ranking, retrieval, or product detectors inside `@42ch/spoke-operations`
- Protocol-fixture AJV/fs harness inside `@42ch/spoke-operations` (belongs under `fixtures/`)
- In-repo `adapters/*` product binding packages — product DTO↔SPOKE adapters live in consumer repositories; this repo keeps port contracts + `fixtures/toy-world/` reference examples only
- Default full manuscript text on the wire
- Closed forever enums that freeze product ontology growth
- Creator Memory / unpromoted chat as KnowledgeEntry graph canon
- Publishing fixture or codegen packages to registries

---

## Pointers

| Doc / path | Use for |
|------------|---------|
| [`STRATEGY.md`](../STRATEGY.md) | Why / principles / three-column architecture |
| [`CONCEPTS.md`](../CONCEPTS.md) | KnowledgeEntry / TimelineEvent spelling + dual-concern |
| [`.mstar/specs/spoke-protocol.md`](specs/spoke-protocol.md) | Normative umbrella |
| [`.mstar/specs/spoke-version-release.md`](specs/spoke-version-release.md) | Lockstep SemVer, tags, CI-gated GitHub Release, registry publish |
| [`.mstar/specs/spoke-protocol-layers.md`](specs/spoke-protocol-layers.md) | L0–L8 + capability levels |
| [`.mstar/specs/domain-profile-narrative-structure.md`](specs/domain-profile-narrative-structure.md) | Narrative-structure Domain Profile — Beat mapping |
| [`knowledge/architecture-patterns/l5-fork-timeline-event-wire.md`](knowledge/architecture-patterns/l5-fork-timeline-event-wire.md) | Compound note on optional Fork wire |
| [`knowledge/architecture-patterns/spoke-codegen-pipeline.md`](knowledge/architecture-patterns/spoke-codegen-pipeline.md) | Codegen inventory, OpaqueJson, Rust typify strategy A |
| [`schemas/`](../schemas/) | Wire SSOT |
| [`packages/spoke-operations/`](../packages/spoke-operations/) | Pure behavior library (TypeScript) |
| [`crates/spoke-operations/`](../crates/spoke-operations/) | Pure behavior library (Rust) |
| [`fixtures/toy-world/`](../fixtures/toy-world/) | Protocol samples, harness, and reference Adapter examples |
