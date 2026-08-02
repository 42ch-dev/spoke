---
title: Protocol umbrella
---

# Protocol umbrella

SPOKE is a shared **wire dialect** for narrative products: one set of JSON Schema contracts for knowledge data and operations, so products exchange KnowledgeEntry data and ops on a common protocol surface. The protocol spans three columns plus an opt-in connect family.

## The three columns

- **Data wire** — eight durable objects: KnowledgeEntry, Relation, SourceAnchor, Finding, AssemblePacket, HostCapabilityManifest, Rule, TimelineEvent (`schemas/data/` plus shared defs in `schemas/common/`).
- **Ops wire** — five baseline operation families (upsert, extract→promote, relate, check, assemble) as transport-agnostic request/response envelopes, plus optional `project` / `compute` under `l2-computable`.
- **Operations library** — hand-written behavior over the generated wire types: pure lifecycle helpers, capability-sliced adapter ports, and injection orchestration (`@42ch/spoke-operations` TypeScript; `spoke-operations` Rust at lockstep SemVer).

## Connect family (opt-in)

Six interaction envelopes (`schemas/connect/`, capability flag `spoke-connect`) add cross-process interaction: a signed hello that embeds `HostCapabilityManifest` by `$ref`, session context, invoke request/response wrapping existing ops envelopes as opaque payloads, and auth challenge/response.

## Codegen posture

- `schemas/` is the only hand-authored wire truth; generated TypeScript (`@42ch/spoke-schemas`) and Rust (`spoke-schemas`) output is committed and mirrors the schema tree.
- `pnpm run verify-codegen` fails the build if the generated tree drifts from `schemas/`; schema changes and regenerated output land in the same commit.
- The wire inventory is 30 committed `*.schema.json` files: 2 common + 8 data + 14 ops + 6 connect envelopes.
- Every durable object carries required `extensions.<namespace>`; core fields stay closed (`additionalProperties: false`).

## Repository layout

`schemas/` (SSOT) · `tooling/codegen/` · `packages/spoke-schemas` + `packages/spoke-operations` (TypeScript) · `crates/spoke-schemas` + `crates/spoke-operations` (Rust) · `fixtures/toy-world/` (conformance samples and reference adapters) · `.mstar/specs/` (normative docs).

## Normative references

- [spoke-protocol.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-protocol.md) — umbrella: problem framing, schema inventory, extensions, acceptance
- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) — protocol vocabulary
- [schemas/README.md](https://github.com/42ch-dev/spoke/blob/main/schemas/README.md) — schema file checklist
- [spoke-protocol-layers.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-protocol-layers.md) — L0–L8 and capability levels
- [spoke-data-model.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-data-model.md) — data objects
- [spoke-ops.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-ops.md) — ops wire
- [spoke-operations.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-operations.md) — operations library
- [spoke-connect.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect.md) — connect family
- [README.md](https://github.com/42ch-dev/spoke/blob/main/README.md) — repository overview and quick start
