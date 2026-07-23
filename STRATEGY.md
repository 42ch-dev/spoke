# Strategy — SPOKE

## Positioning

**SPOKE is a protocol repository, not a runtime.**

It defines JSON Schema wire contracts for narrative Keyblock **data** and **ops** so products (Nexus, Creader, others) can exchange consistency-check and context-assembly I/O without sharing a database, daemon, or deployment.

## What we build (v0.1)

| Deliverable | Role |
|-------------|------|
| **Normative specs** | `.mstar/specs/` — data model, ops, extensions rules |
| **`schemas/`** | Draft-07 SSOT (17 files: 2 common + 5 data + 10 ops) |
| **`@42ch/spoke-schema`** | Generated TypeScript types |
| **`@42ch/spoke-operations`** | Hand-written lifecycle helpers over wire types (v0-iter002+) |
| **`spoke-schema`** (Rust crate) | Generated Rust types |
| **`adapters/*`** | Empty placeholders only |

## What we do not build (v0.1)

- Shared runtime, daemon, or MCP server
- Nexus ↔ SPOKE or Creader ↔ SPOKE conversion (adapter packages deferred)
- Conformance fixtures or golden round-trips
- `Rule` wire schema (deferred — see data model §Rule deferral)
- npm/crates.io publish (workspace-local packages; CI must not publish)

## Architecture (three columns)

SPOKE Thrust A spans **data wire**, **ops wire**, and a **hand-written operations library** — see [`.mstar/roadmap.md`](.mstar/roadmap.md) and [`.mstar/specs/spoke-protocol.md`](.mstar/specs/spoke-protocol.md).

| Column | Responsibility | Artifact |
|--------|----------------|----------|
| **1. Data wire** | Durable objects: Keyblock, Relation, SourceAnchor, Finding, AssemblePacket | `schemas/data/` → `@42ch/spoke-schema` |
| **2. Ops wire** | Transport-agnostic request/response families: `upsert`, extract→promote, `relate`, `check`, `assemble` | `schemas/ops/` → `@42ch/spoke-schema` |
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
| **v0-iter002 (delivered 2026-07-23)** | `@42ch/spoke-operations` first slice + consumer README EN/CN |
| **Next** | Adapter packages, optional `Rule` schema, conformance fixtures |
| **North star** | Cross-product Keyblock dialect for checker and context-assembly I/O |

## See also

| Doc | Topic |
|-----|-------|
| [`CONCEPTS.md`](CONCEPTS.md) | Domain vocabulary and boundaries |
| [`.mstar/specs/spoke-protocol.md`](.mstar/specs/spoke-protocol.md) | Umbrella spec and acceptance criteria |
