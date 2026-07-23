# SPOKE Roadmap

> Living **project** roadmap (tracked result). Strategy and architecture live in [`STRATEGY.md`](../STRATEGY.md) and [`.mstar/specs/`](specs/). Per-slice execution detail stays in local `delivery-compass.md` (process; gitignored).

**Updated:** 2026-07-23  
**North star:** Cross-product Keyblock dialect for check + assemble I/O — **without** a shared runtime.

---

## Now (in progress)

_Nothing scheduled._ Baseline protocol surfaces (data wire, ops wire, ops library, fixtures) are delivered. Pick the next item from **Up next** when starting a new delivery slice.

---

## Up next (planned)

Ordered by likely value; lock scope in a delivery compass before implement. Items may be reordered or dropped.

| Priority | Item | Outcome | Notes |
|----------|------|---------|-------|
| 1 | Optional **Fork** wire (`l5-fork` capability) | Schema + normative text for world-history branches when a product needs them | Optional capability — not baseline |
| 2 | Optional **`project` / compute** op family | Transport-agnostic I/O for programmable body state (Nexus-class) | Optional; Creader-class products omit |
| 3 | Optional **`l2-computable`** body fields | Documented optional `body.state` / computable shape on Keyblock | Capability flag already named in layers spec |
| 4 | **Rust** `spoke-operations` crate (optional) | Pure helpers mirrored for Rust consumers | TS library is SSOT today |
| 5 | CI / codegen harden (residuals) | e.g. Rust generated-type duplication strategy; keep schema-count (19) in sync when adding schemas | See open residuals in local harness status when present |

**Explicitly not on this roadmap right now:** product adapter packages (`adapters/nexus`, `adapters/creader`). The `adapters/` tree is a **placeholder README only** — not a delivery track until a product binding sprint is scheduled.

---

## Done (delivered)

Newest first. Dates are delivery dates on `main`.

| When | Slice | What landed |
|------|-------|-------------|
| 2026-07-23 | Fixture harness ownership + CI harden | AJV/Vitest under `fixtures/toy-world/tests/` (`@42ch/spoke-fixture-toy-world`); removed from `@42ch/spoke-operations`; `AGENTS.md` boundary; CI `test:fixtures`; `verify-codegen` schema-count assert (19) |
| 2026-07-23 | Operations library deepen + fixtures | OCC compare, Keyblock status, uniqueness, Scope/upsert/relate gates, error-envelope map; `fixtures/toy-world/` protocol JSON graph |
| 2026-07-23 | Protocol layers + Rule/Event | Normative L0–L8 + capability levels; `Rule` + `Event` schemas; Scope / error-envelope / Rule-aware `check` |
| 2026-07-23 | Operations library first slice | `@42ch/spoke-operations`: promote, Finding transitions, extensions preserve, AssemblePacket builders; consumer README EN/CN |
| 2026-07-23 | v0.1 bootstrap | `schemas/` SSOT, codegen `@42ch/spoke-schemas` + Rust `spoke-schemas`, CI verify gate |

### Baseline inventory (as of last Done)

| Area | Status |
|------|--------|
| Data wire | Keyblock, Relation, SourceAnchor, Finding, AssemblePacket, Rule, Event + `extensions` |
| Ops wire | upsert / promote / relate / check / assemble (+ Scope, error-envelope) |
| Ops library | Pure TS helpers over wire types (no I/O, no fixture harness) |
| Fixtures | `fixtures/toy-world/` samples + conformance package |
| Specs | Umbrella, layers, data-model, ops wire, operations library under `.mstar/specs/` |

---

## Out of scope (durable)

Do not schedule these into SPOKE itself unless strategy is explicitly reversed:

- Shared daemon / MCP server / single multi-product runtime
- I/O, LLM, ranking, retrieval, or product detectors inside `@42ch/spoke-operations`
- Protocol-fixture AJV/fs harness inside `@42ch/spoke-operations` (belongs under `fixtures/`)
- Default full manuscript text on the wire
- Closed forever enums that freeze product ontology growth
- Creator Memory / unpromoted chat as Keyblock graph canon

---

## Pointers

| Doc / path | Use for |
|------------|---------|
| [`STRATEGY.md`](../STRATEGY.md) | Why / principles / three-column architecture |
| [`.mstar/specs/spoke-protocol.md`](specs/spoke-protocol.md) | Normative umbrella |
| [`.mstar/specs/spoke-protocol-layers.md`](specs/spoke-protocol-layers.md) | L0–L8 + capability levels |
| [`schemas/`](../schemas/) | Wire SSOT |
| [`packages/spoke-operations/`](../packages/spoke-operations/) | Pure behavior library |
| [`fixtures/toy-world/`](../fixtures/toy-world/) | Protocol samples + harness |
