# Strategy — SPOKE

## Positioning

**SPOKE is a protocol repository, not a runtime.**

It defines JSON Schema wire contracts for narrative Keyblock **data** and **ops** so products (Nexus, Creader, others) can exchange consistency-check and context-assembly I/O without sharing a database, daemon, or deployment.

## What we build

| Deliverable | Role |
|-------------|------|
| **Normative specs** | `.mstar/specs/` — protocol umbrella, L0–L8 layers, data model, ops wire, operations library |
| **`schemas/`** | Draft-07 SSOT (19 files — see [`spoke-protocol.md`](.mstar/specs/spoke-protocol.md)) |
| **`@42ch/spoke-schemas`** | Generated TypeScript types |
| **`@42ch/spoke-operations`** | Hand-written lifecycle helpers over wire types (operations library first slice onward) |
| **`spoke-schemas`** (Rust crate) | Generated Rust types |
| **`adapters/`** | README purpose note only (implementation deferred) |

## What we do not build

- Shared runtime, daemon, or MCP server
- Nexus ↔ SPOKE or Creader ↔ SPOKE conversion (adapter packages deferred)
- Golden product DTO round-trips (protocol `fixtures/toy-world/` — fixtures conformance slice)
- npm/crates.io publish (workspace-local packages; CI must not publish)

## Architecture (three columns)

SPOKE Thrust A spans **data wire**, **ops wire**, and a **hand-written operations library** — see [`.mstar/roadmap.md`](.mstar/roadmap.md) and [`.mstar/specs/spoke-protocol.md`](.mstar/specs/spoke-protocol.md).

| Column | Responsibility | Artifact |
|--------|----------------|----------|
| **1. Data wire** | Durable objects: Keyblock, Relation, SourceAnchor, Finding, AssemblePacket; protocol layers deepen adds Rule, Event | `schemas/data/` → `@42ch/spoke-schemas` |
| **2. Ops wire** | Transport-agnostic request/response families: `upsert`, extract→promote, `relate`, `check`, `assemble` | `schemas/ops/` → `@42ch/spoke-schemas` |
| **3. Ops library** | Pure lifecycle invariants JSON Schema cannot express (promote gate, Finding transitions, extensions preserve, AssemblePacket builders) | `@42ch/spoke-operations` |

Product-specific fields live only in `extensions.<namespace>`. Core protocol objects use `additionalProperties: false`. Adapters map product DTOs to wire types and **call** `@42ch/spoke-operations` for shared lifecycle rules — they must not reimplement those invariants.

## Guiding principles

1. **Wire contracts are truth.** `schemas/` is the only hand-authored source; TypeScript and Rust are generated.
2. **Open vocabulary, closed envelopes.** `block_type` and related fields are strings with documented core lists — not `enum` until adapter specs prove stability.
3. **Preserve on round-trip.** Adapters MUST not drop unknown extension namespaces or keys.
4. **Simplicity over premature abstraction.** v0.1 ships schemas and packages; cross-product lifecycle behavior lives in `@42ch/spoke-operations`.

## Roadmap pointer

| Phase | Focus |
|-------|-------|
| **v0.1 (delivered 2026-07-23)** | Spec trio + schemas + codegen + CI verify gate |
| **Operations library first slice (delivered 2026-07-23)** | `@42ch/spoke-operations` first slice + consumer README EN/CN |
| **Protocol layers + Rule/Event (delivered 2026-07-23)** | Normative L0–L8 + capability levels; `Rule` + `Event` wire schemas; ops harden — see [`spoke-protocol-layers.md`](.mstar/specs/spoke-protocol-layers.md) |
| **Operations library deepen + fixtures (delivered 2026-07-23)** | Deepen `@42ch/spoke-operations` (OCC, Keyblock status, Scope/upsert/relate gates, error map) + `fixtures/toy-world/` protocol conformance graph + AJV harness (`fixtures/toy-world/tests/`; `@42ch/spoke-fixture-toy-world`) |
| **Next** | Adapter packages (`adapters/nexus`, `adapters/creader`) |
| **North star** | Cross-product Keyblock dialect for checker and context-assembly I/O |

## See also

| Doc | Topic |
|-----|-------|
| [`CONCEPTS.md`](CONCEPTS.md) | Domain vocabulary and boundaries |
| [`.mstar/specs/spoke-protocol.md`](.mstar/specs/spoke-protocol.md) | Umbrella spec and acceptance criteria |
| [`.mstar/specs/spoke-protocol-layers.md`](.mstar/specs/spoke-protocol-layers.md) | L0–L8, capability levels, TimelineScale vocabulary |
