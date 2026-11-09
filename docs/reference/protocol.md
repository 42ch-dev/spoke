---
title: Protocol reference
---

# Protocol reference

SPOKE is a shared **wire dialect** for narrative products: one set of JSON Schema contracts for knowledge data and operations, so products exchange KnowledgeEntry data and ops on a common protocol surface. The protocol spans three columns plus an opt-in connect family.

## The three columns

| Column | Contents |
|--------|----------|
| **Data wire** | Eleven data objects: KnowledgeEntry, Relation, SourceAnchor, Finding, AssemblePacket, HostCapabilityManifest, Rule, TimelineEvent, CognitionRecord, MindState, ToolDescriptor ([`schemas/data/`](https://github.com/42ch-dev/spoke/tree/main/schemas/data)), plus shared definitions in [`schemas/common/`](https://github.com/42ch-dev/spoke/tree/main/schemas/common); the optional `spoke-memory` family adds thirteen memory models under the same authoring rules |
| **Ops wire** | Five baseline operation families — upsert, extract→promote, relate, check, assemble — as transport-agnostic request/response envelopes ([`schemas/ops/`](https://github.com/42ch-dev/spoke/tree/main/schemas/ops)), plus optional `project` / `compute` under `l2-computable`, optional `extract` under `ke-extraction`, and the 33-op `spoke-memory` family |
| **Operations library** | Hand-written behavior over the generated wire types: pure lifecycle helpers, capability-sliced adapter ports, and injection orchestration (`@42ch/spoke-operations` TypeScript; `spoke-operations` Rust, lockstep SemVer) |

## Connect family (opt-in)

Six interaction envelopes ([`schemas/connect/`](https://github.com/42ch-dev/spoke/tree/main/schemas/connect), capability flag `spoke-connect`) add cross-process interaction: a signed hello that embeds `HostCapabilityManifest` by `$ref`, session context, invoke request/response wrapping existing ops envelopes as opaque payloads, and auth challenge/response. The family is additive — baseline compliance and baseline schemas stay unchanged. See [Connect reference](/reference/connect).

## Schema inventory and codegen posture

The wire inventory is **114 committed `*.schema.json` files**: 2 common + 24 data + 82 ops + 6 connect envelopes. The optional `spoke-memory` family contributes 79 of them (13 data models + 33 request/response pairs). Baseline integrators use **21** of those files (2 common + 9 data + 10 baseline ops): the nine baseline data schemas include `ToolDescriptor`, while `CognitionRecord` and `MindState` are the two optional non-memory data files. `schemas/` is the only hand-authored wire truth; generated TypeScript (`@42ch/spoke-schemas`) and Rust (`spoke-schemas`) output is committed and mirrors the schema tree. `pnpm run verify-codegen` fails the build if the generated tree drifts from `schemas/`; schema changes and regenerated output land in the same commit.

## Extensions contract

Every durable record object — KnowledgeEntry, Relation, SourceAnchor, Finding, AssemblePacket, HostCapabilityManifest, Rule, TimelineEvent, CognitionRecord, MindState — carries the required `extensions.<namespace>` bag; core fields stay closed (`additionalProperties: false`). `ToolDescriptor` is a closed descriptor without an extensions bag.

| Bag | Shape | Role |
|-----|-------|------|
| `extensions.<namespace>` | required `ExtensionMap` on every durable data object; namespace keys are product-chosen ids matching `^[a-z][a-z0-9_-]*$`; values are opaque JSON objects | One product's private bag. Adapters preserve unknown namespaces and keys on round-trip |
| `modules.*` | optional `ModuleMap` (capability-flagged `narrative-modules`) on KnowledgeEntry, AssemblePacket, and TimelineEvent; keys are functional-dialect ids (`activation`, `placement`, `activation_trace`, `cognition`, `mental`, `observation`, …); values are structured JSON, inner shapes handbook-defined | Cross-product functional dialects shared by narrative hosts. Unknown module keys round-trip |

Placement rule: a **cross-product functional dialect** uses `modules.*`; **product data** uses `extensions.<product>`. On `HostCapabilityManifest`, `extensions` carries deployment metadata — roles, capabilities, and namespace ownership are core manifest fields.

## Capability flags

| Flag | What it adds |
|------|--------------|
| `spoke-baseline` | L0–L8 semantics via the five ops wire families, `HostCapabilityManifest` + baseline `HostManifestPort`, and the shared `Scope` / `error-envelope` definitions. Baseline compliance stands alone; optional flags are additive |
| `l2-computable` | `body.state` / `body.computable` on KnowledgeEntry, `TimelineEvent.computable_logs`, and `project` / `compute` ops |
| `l5-fork` | `fork_id` / `parent_fork_id` branch metadata on TimelineEvent and `Scope.fork_id` filtering |
| `spoke-cognition` | one family (no subflags) for standard attributable cognition: `modules.cognition` on the holder KnowledgeEntry (Belief, epistemic knowledge/ignorance/undetermined, Perception records plus narrated world facts; every completed Belief, epistemic or Perception record carries a declared foundational `q` and may carry distinct metrics), `Scope.cognition` selection with attribution-preserving projection, `modules.observation` on TimelineEvent (could-perceive access), Memory `Content.cognition` with `cognition_filter`, and derivative `MindState` cognition. Advertisement covers only the surfaces a host actually serves; unsupported requirements reject explicitly — see [Cognition records](/reference/data-model#cognition-spoke-cognition) and [MindState reference](/reference/mind-state) |
| `narrative-modules` | the optional `modules` (`ModuleMap`) bag on KnowledgeEntry + AssemblePacket + TimelineEvent |
| `ke-extraction` | the optional `extract` op family — provisional `KnowledgeEntry` candidates from referenced source material — plus the standalone `ExtractionPort` and the `orchestrateExtract` boundary |
| `ke-ownership` | `KnowledgeEntry.owner` / `KnowledgeEntry.disclosure` governance fields and the shared `Scope.viewpoint` reader selector |
| `spoke-connect` | the opt-in interaction envelope family; hosts that speak it list the flag in `HostCapabilityManifest.capabilities` |
| `spoke-memory` | the optional per-subject memory family: thirteen models and 33 `memory.*` ops with a standalone `MemoryPort`. Two optional sub-flags — `spoke-memory-reflection` (3 reflection ops) and `spoke-memory-transfer` (5 export/import ops) — each require `spoke-memory`. See [Memory reference](/reference/memory) |

## Cognition negotiation and rejection

`spoke-cognition` is one capability flag. A host lists it in `HostCapabilityManifest.capabilities` only when every mandatory rule for the surfaces it actually serves is implemented: a KnowledgeEntry-only host serves the cognition module, `modules.observation` and `Scope.cognition` filtering/projection without Memory; a host serving both KnowledgeEntry and Memory also satisfies the Memory cognition row (recorded-at-time content validation, filtered list/recall, reflection basis, transfer digests, scrub). The manifest string alone is not conformance evidence.

Negotiation reuses the existing signed-hello intersection: `negotiatedCapabilities(local, remote)` grants `spoke-cognition` only when both manifests declare it. Baseline ops still require `spoke-baseline`; a payload-dependent guard adds the cognition requirement only when the dispatched request carries its declared carrier (`scope.cognition` on `check`, `assemble`, `port.scope.list_knowledge_entries`, `port.scope.list_timeline_events`, `port.fork.list_timeline_events`; `cognition_filter` on `memory.recall`, `memory.records.list` and the three `memory.reflections.*` reads). When the negotiated set lacks the family, a request that requires it takes the existing `capability_missing` branch with no handler side effect. Records without optional cognitive fields stay ordinary missing data and raise no error.

After schema decoding, an unknown predicate, a known-but-unimplemented view/dimension/metric filter, or a requested recorded-at-time source on a KnowledgeEntry carrier rejects the whole request through the existing error envelope: `code: "invalid_input"`, `message: "Unsupported cognition requirement"`, `details.reason: "unsupported_cognition_requirement"`, and `details.field` naming the carrier-relative path of the requirement. The response carries no silent stripping, no ignored conjunction, no partial result and no fallback to unfiltered output.

## Repository layout

`schemas/` (SSOT) · `tooling/codegen/` · `packages/spoke-schemas` + `packages/spoke-operations` (TypeScript) · `crates/spoke-schemas` + `crates/spoke-operations` (Rust) · `fixtures/toy-world/` (conformance samples and reference adapters).

## Related

- [Data model reference](/reference/data-model) — field tables for all eleven data objects.
- [Ops wire reference](/reference/ops) — request/response envelopes, `Scope`, `ErrorEnvelope`.
- [Connect reference](/reference/connect) — the opt-in envelope family.
- [Cognition records](/reference/data-model#cognition-spoke-cognition) — `q`, optional metrics, filter grammar and projections.
- [MindState reference](/reference/mind-state) — the derivative L5 temporal view (`spoke-cognition`).
- [Memory reference](/reference/memory) — the optional `spoke-memory` family: models, ops, capability flags, provider guidance.
- [Concepts](/explanation/concepts) — the nine layers and how capabilities map onto them.
- [`schemas/README.md`](https://github.com/42ch-dev/spoke/blob/main/schemas/README.md) — schema file inventory.
