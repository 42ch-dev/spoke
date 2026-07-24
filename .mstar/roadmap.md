# SPOKE Roadmap

> Living **project** roadmap (tracked result). Strategy and architecture live in [`STRATEGY.md`](../STRATEGY.md) and [`.mstar/specs/`](specs/). Per-slice execution detail stays in local `delivery-compass.md` (process; gitignored).

**Updated:** 2026-07-25  
**North star:** Cross-product KnowledgeEntry dialect for check + assemble I/O — **without** a shared runtime.

---

## Now (in progress)

| Item | Outcome | Normative doc |
|------|---------|---------------|
| **Unified version release** | Lockstep SemVer + annotated `vX.Y.Z` tags + CI-gated GitHub Release (no npm/crates.io) | [`.mstar/specs/spoke-version-release.md`](specs/spoke-version-release.md) |

**Remaining for this track:** CI `verify-version` + `release.yml` workflow; bump/assert tooling; README EN/CN consumer pinning and maintainer how-to (scheduled with operator tooling docs).

---

## Up next (planned)

Ordered by likely value; lock scope in a delivery compass before implement. Items may be reordered or dropped.

| Priority | Item | Outcome | Notes |
|----------|------|---------|-------|
| 1 | CI / codegen harden (residuals) | e.g. TS codegen narrowing for ComputableLogChange; Rust generated-type duplication strategy; keep schema-count (**23**) in sync | Open: ComputableLogChange previous/next narrowing |
| 2 | **Rust** `spoke-operations` crate (optional) | Pure helpers mirrored for Rust consumers | TS library is SSOT today |

**Explicitly not on this roadmap right now:** product adapter packages under `adapters/<product>/`. The `adapters/` tree is a **placeholder README only** — not a delivery track until a product binding sprint is scheduled.

---

## Done (delivered)

Newest first. Dates are delivery dates on `main`.

| When | Slice | What landed |
|------|-------|-------------|
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
| Data wire | KnowledgeEntry, Relation, SourceAnchor, Finding, AssemblePacket, Rule, TimelineEvent + `extensions`; optional `body.state`/`body.computable`, `computable_logs` (`l2-computable`); optional `fork_id`/`parent_fork_id` on TimelineEvent (`l5-fork`) |
| Ops wire | upsert / promote / relate / check / assemble (+ Scope, error-envelope); optional project / compute (`l2-computable`) |
| Ops library | Pure TS helpers over wire types (incl. `Scope.fork_id` TimelineEvent match; KnowledgeEntry / TimelineEvent naming; no I/O, no fixture harness) |
| Fixtures | `fixtures/toy-world/` samples + conformance (dual-concern ontology `"event"` + TimelineEvent; Fork-aware TimelineEvent sample under `l5-fork`) |
| Specs / vocabulary | Umbrella, layers, data-model, ops wire, operations library under `.mstar/specs/`; CONCEPTS + knowledge vocabulary pattern |

---

## Out of scope (durable)

Do not schedule these into SPOKE itself unless strategy is explicitly reversed:

- Shared daemon / MCP server / single multi-product runtime
- I/O, LLM, ranking, retrieval, or product detectors inside `@42ch/spoke-operations`
- Protocol-fixture AJV/fs harness inside `@42ch/spoke-operations` (belongs under `fixtures/`)
- Default full manuscript text on the wire
- Closed forever enums that freeze product ontology growth
- Creator Memory / unpromoted chat as KnowledgeEntry graph canon
- npm / crates.io publish from CI (workspace-local consumption; see version release spec)

---

## Pointers

| Doc / path | Use for |
|------------|---------|
| [`STRATEGY.md`](../STRATEGY.md) | Why / principles / three-column architecture |
| [`CONCEPTS.md`](../CONCEPTS.md) | KnowledgeEntry / TimelineEvent spelling + dual-concern |
| [`.mstar/specs/spoke-protocol.md`](specs/spoke-protocol.md) | Normative umbrella |
| [`.mstar/specs/spoke-version-release.md`](specs/spoke-version-release.md) | Lockstep SemVer, tags, CI-gated GitHub Release |
| [`.mstar/specs/spoke-protocol-layers.md`](specs/spoke-protocol-layers.md) | L0–L8 + capability levels |
| [`knowledge/architecture-patterns/l5-fork-timeline-event-wire.md`](knowledge/architecture-patterns/l5-fork-timeline-event-wire.md) | Compound note on optional Fork wire |
| [`schemas/`](../schemas/) | Wire SSOT |
| [`packages/spoke-operations/`](../packages/spoke-operations/) | Pure behavior library |
| [`fixtures/toy-world/`](../fixtures/toy-world/) | Protocol samples + harness |
