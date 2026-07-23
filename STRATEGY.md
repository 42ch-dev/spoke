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
| **`spoke-schema`** (Rust crate) | Generated Rust types |
| **`adapters/*`** | Empty placeholders only |

## What we do not build (v0.1)

- Shared runtime, daemon, or MCP server
- Nexus ↔ SPOKE or Creader ↔ SPOKE conversion (adapter packages deferred)
- Conformance fixtures or golden round-trips
- `Rule` wire schema (deferred — see data model §Rule deferral)
- npm/crates.io publish (workspace-local packages; CI must not publish)

## Architecture (two layers)

1. **Data** — durable objects: Keyblock, Relation, SourceAnchor, Finding, AssemblePacket
2. **Ops** — transport-agnostic request/response families: `upsert`, extract→promote, `relate`, `check`, `assemble`

Product-specific fields live only in `extensions.<namespace>`. Core protocol objects use `additionalProperties: false`.

## Guiding principles

1. **Wire contracts are truth.** `schemas/` is the only hand-authored source; TypeScript and Rust are generated.
2. **Open vocabulary, closed envelopes.** `block_type` and related fields are strings with documented core lists — not `enum` until adapter specs prove stability.
3. **Preserve on round-trip.** Adapters MUST not drop unknown extension namespaces or keys.
4. **Simplicity over premature abstraction.** v0.1 ships schemas and packages; behavior stays in product repos.

## Roadmap pointer

| Phase | Focus |
|-------|-------|
| **v0.1 (now)** | Spec trio + schemas + codegen + CI verify gate |
| **Next iteration** | Adapter packages, optional `Rule` schema, conformance fixtures |
| **North star** | Cross-product Keyblock dialect for checker and context-assembly I/O |

## See also

| Doc | Topic |
|-----|-------|
| [`CONCEPTS.md`](CONCEPTS.md) | Domain vocabulary and boundaries |
| [`.mstar/specs/spoke-protocol.md`](.mstar/specs/spoke-protocol.md) | Umbrella spec and acceptance criteria |
