---
title: Protocol reference
---

# Protocol reference

SPOKE is a shared **wire dialect** for narrative products: one set of JSON Schema contracts for knowledge data and operations, so products exchange KnowledgeEntry data and ops on a common protocol surface. The protocol spans three columns plus an opt-in connect family.

## The three columns

| Column | Contents |
|--------|----------|
| **Data wire** | Eight durable objects: KnowledgeEntry, Relation, SourceAnchor, Finding, AssemblePacket, HostCapabilityManifest, Rule, TimelineEvent ([`schemas/data/`](https://github.com/42ch-dev/spoke/tree/main/schemas/data)), plus shared definitions in [`schemas/common/`](https://github.com/42ch-dev/spoke/tree/main/schemas/common) |
| **Ops wire** | Five baseline operation families — upsert, extract→promote, relate, check, assemble — as transport-agnostic request/response envelopes ([`schemas/ops/`](https://github.com/42ch-dev/spoke/tree/main/schemas/ops)), plus optional `project` / `compute` under `l2-computable` |
| **Operations library** | Hand-written behavior over the generated wire types: pure lifecycle helpers, capability-sliced adapter ports, and injection orchestration (`@42ch/spoke-operations` TypeScript; `spoke-operations` Rust, lockstep SemVer) |

## Connect family (opt-in)

Six interaction envelopes ([`schemas/connect/`](https://github.com/42ch-dev/spoke/tree/main/schemas/connect), capability flag `spoke-connect`) add cross-process interaction: a signed hello that embeds `HostCapabilityManifest` by `$ref`, session context, invoke request/response wrapping existing ops envelopes as opaque payloads, and auth challenge/response. The family is additive — baseline compliance and baseline schemas stay unchanged. See [Connect reference](/reference/connect).

## Schema inventory and codegen posture

The wire inventory is **30 committed `*.schema.json` files**: 2 common + 8 data + 14 ops + 6 connect envelopes. `schemas/` is the only hand-authored wire truth; generated TypeScript (`@42ch/spoke-schemas`) and Rust (`spoke-schemas`) output is committed and mirrors the schema tree. `pnpm run verify-codegen` fails the build if the generated tree drifts from `schemas/`; schema changes and regenerated output land in the same commit.

## Extensions contract

Every durable object carries the required `extensions.<namespace>` bag; core fields stay closed (`additionalProperties: false`).

| Bag | Shape | Role |
|-----|-------|------|
| `extensions.<namespace>` | required `ExtensionMap` on every durable data object; namespace keys are product-chosen ids matching `^[a-z][a-z0-9_-]*$`; values are opaque JSON objects | One product's private bag. Adapters preserve unknown namespaces and keys on round-trip |
| `modules.*` | optional `ModuleMap` (capability-flagged `narrative-modules`) on KnowledgeEntry and AssemblePacket; keys are functional-dialect ids (`activation`, `placement`, `activation_trace`, …); values are structured JSON, inner shapes handbook-defined | Cross-product functional dialects shared by narrative hosts. Unknown module keys round-trip |

Placement rule: a **cross-product functional dialect** uses `modules.*`; **product data** uses `extensions.<product>`. On `HostCapabilityManifest`, `extensions` carries deployment metadata — roles, capabilities, and namespace ownership are core manifest fields.

## Capability flags

| Flag | What it adds |
|------|--------------|
| `spoke-baseline` | L0–L8 semantics via the five ops wire families, `HostCapabilityManifest` + baseline `HostManifestPort`, and the shared `Scope` / `error-envelope` definitions. Baseline compliance stands alone; optional flags are additive |
| `l2-computable` | `body.state` / `body.computable` on KnowledgeEntry, `TimelineEvent.computable_logs`, and `project` / `compute` ops |
| `l5-fork` | `fork_id` / `parent_fork_id` branch metadata on TimelineEvent and `Scope.fork_id` filtering |
| `narrative-modules` | the optional `modules` (`ModuleMap`) bag on KnowledgeEntry + AssemblePacket |
| `spoke-connect` | the opt-in interaction envelope family; hosts that speak it list the flag in `HostCapabilityManifest.capabilities` |

## Repository layout

`schemas/` (SSOT) · `tooling/codegen/` · `packages/spoke-schemas` + `packages/spoke-operations` (TypeScript) · `crates/spoke-schemas` + `crates/spoke-operations` (Rust) · `fixtures/toy-world/` (conformance samples and reference adapters).

## Related

- [Data model reference](/reference/data-model) — field tables for all eight durable objects.
- [Ops wire reference](/reference/ops) — request/response envelopes, `Scope`, `ErrorEnvelope`.
- [Connect reference](/reference/connect) — the opt-in envelope family.
- [Concepts](/explanation/concepts) — the nine layers and how capabilities map onto them.
- [`schemas/README.md`](https://github.com/42ch-dev/spoke/blob/main/schemas/README.md) — schema file inventory.
