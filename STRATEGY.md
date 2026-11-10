# Strategy — SPOKE

## Vision

**SPOKE is a protocol for interoperable narrative knowledge.**

It defines JSON Schema wire contracts for KnowledgeEntry data and operations, attributable cognition, and recorded-at-time Memory. Independent products bind these contracts to their own runtime, storage, and governance.

## What we build

| Deliverable | Role |
|-------------|------|
| **Normative specs** | `.agents/specs/` — protocol umbrella, L0–L8 layers, data model, ops wire, operations library |
| **`schemas/`** | Draft-07 SSOT (**114** files — see [`spoke-protocol.md`](.agents/specs/protocol/spoke-protocol.md)) |
| **`@42ch/spoke-schemas`** | Generated TypeScript types (published to npm) |
| **`@42ch/spoke-operations`** | Hand-written lifecycle helpers, capability-sliced adapter port contracts, and injection orchestration over wire types (published to npm) |
| **`spoke-schemas`** (Rust crate) | Generated Rust types (published to crates.io) |
| **`spoke-operations`** (Rust crate) | Hand-written lifecycle helpers, adapter port traits, and injection orchestration over wire types (published to crates.io) |
| **`fixtures/toy-world/`** | Protocol conformance JSON, AJV/Vitest harness, and reference Adapter examples (`ToyWorldAdapter` in TypeScript and Rust) |

## What we don't build

- Shared runtime, daemon, or MCP server
- Product cognitive inference, ranking, retrieval, storage, or history/replay engines
- Product mapping packages in consumer repositories until a product binding sprint schedules them
- Golden product DTO round-trips (protocol `fixtures/toy-world/` — fixtures conformance slice)
- Publishing fixture, codegen, or adapter packages to registries

## Technology Direction

SPOKE combines data wire, operations wire, and a pure hand-written operations library. JSON Schema drives coordinated TypeScript and Rust contracts; existing Connect and native JSON carriers carry the same protocol payloads.

| Column | Responsibility | Artifact |
|--------|----------------|----------|
| **1. Data wire** | KnowledgeEntry, Relation, SourceAnchor, Finding, AssemblePacket, HostCapabilityManifest, Rule, TimelineEvent, and optional cognition/MindState contracts | `schemas/data/` → generated TS/Rust |
| **2. Ops wire** | Transport-agnostic request/response families for entry lifecycle, relations, checking, context assembly, and Memory | `schemas/ops/` → generated TS/Rust |
| **3. Ops library** | Pure lifecycle invariants, cognition validation/filtering/projection, capability-sliced adapter ports, and injection orchestration | `@42ch/spoke-operations` / `spoke-operations` |

Product-specific fields live only in `extensions.<namespace>`. Core protocol objects use `additionalProperties: false`. Product adapters implement the operations port contracts, map product DTOs to wire types, and **call** `@42ch/spoke-operations` / `spoke-operations` orchestration entrypoints for shared lifecycle sequences. Adapters own transport, persistence, and **transaction boundaries** (including multi-entry upsert). Active-uniqueness helpers take **caller-supplied peer sets** — batch-local peers from the orchestrator unless the adapter supplies a wider store snapshot.

## Guiding Principles

1. **Wire contracts are truth.** `schemas/` is the only hand-authored source; TypeScript and Rust are generated.
2. **Open product vocabulary, closed standard contracts.** `entry_type` remains an open string vocabulary; standard cognition shapes and enums follow the accepted family contract, and product-specific fields use extension namespaces.
3. **Preserve on round-trip.** Adapters MUST not drop unknown extension namespaces or keys.
4. **Simplicity over premature abstraction.** Necessary pure validators, deterministic predicates and projections serve the shared data contract; broader reusable algorithms retain their consumer-demand gate.

## Decision Log

- One cross-layer `spoke-cognition` family gives Knowledge, Ignorance, Perception, and Belief shared attribution, identity, absence and filtering semantics across KnowledgeEntry and Memory. L5 remains Temporal; family conformance follows the surfaces a host actually serves.
- Holder KnowledgeEntry cognition is current authority, while Memory retains independently authorized recorded-at-time assessments. This separation preserves historical attribution and avoids turning retention or derived MindState/MindDelta views into another authority.
- Closed standard cognition shapes and necessary pure operations are admitted by the coherent family contract and affected-consumer mapping. Product inference and broader reusable algorithms require their own demonstrated need rather than following automatically from wire standardization.

## See also

| Doc | Topic |
|-----|-------|
| [`CONCEPTS.md`](CONCEPTS.md) | Domain vocabulary and boundaries |
| [`.agents/specs/protocol/spoke-protocol.md`](.agents/specs/protocol/spoke-protocol.md) | Umbrella spec and acceptance criteria |
| [`.agents/specs/protocol/spoke-protocol-layers.md`](.agents/specs/protocol/spoke-protocol-layers.md) | L0–L8, capability levels, TimelineScale vocabulary |
| [`.agents/adr/implemented/architecture/2026-11-07-spoke-cognition-family.md`](.agents/adr/implemented/architecture/2026-11-07-spoke-cognition-family.md) | Cognition concepts, authority, conformance and module boundaries |
| [`.agents/specs/protocol/spoke-version-release.md`](.agents/specs/protocol/spoke-version-release.md) | Lockstep SemVer, annotated tags, CI-gated GitHub Release, registry publish |
