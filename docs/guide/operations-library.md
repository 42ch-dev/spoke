---
title: Operations library
---

# Operations library

The operations library is the hand-written behavior layer over the generated wire types: pure lifecycle helpers plus capability-sliced adapter ports and injection orchestration. It ships twice — `@42ch/spoke-operations` (TypeScript) and `spoke-operations` (Rust) — with behavioral parity at lockstep SemVer.

## Pure helpers

- Extension and module map merge + round-trip preservation (`mergeExtensionMaps`, `preserveModuleMaps`, …).
- Finding status transition validation and apply.
- Promote acceptance gates (provisional requirement, terminal-status rejects, revision bump) — pure and pre-persist.
- KnowledgeEntry status transitions and active-uniqueness checks over caller-supplied sets.
- Scope match helpers for KnowledgeEntry and TimelineEvent; timeline ordering helpers (`orderTimelineEventsByPrecedes`, …).
- AssemblePacket builders with order-preserving truncation.
- Unified `SpokeResult` / `SpokeRejectCode` on every reject path — expected rejects return `SpokeReject` instead of throwing.

## Adapter ports and orchestration

Implement the port families your claimed capabilities require (`KnowledgeEntryPort`, `RelationPort`, `ScopeQueryPort`, `FindingPort`, `RuleQueryPort`, `HostManifestPort`, plus optional `ComputablePort` / `ForkTimelineQueryPort`), then call the matching orchestrator:

```ts
// Illustrative — full signatures in the package READMEs.
import { orchestrateUpsert, type BaselineAdapter } from "@42ch/spoke-operations";

declare const adapter: BaselineAdapter; // your product adapter implements the ports
// orchestrateUpsert(adapter, request) → loads scope, applies gates, persists via ports
```

Composed aliases (`BaselineAdapter`, `ComputableAdapter`, `ForkAdapter`, `FullAdapter`) name the port intersections for each capability level. Host collaboration runs through `HostManifestPort` (`getHostCapabilityManifest`, `listPeerHostCapabilityManifests`).

## Purity boundary

The library performs no storage I/O, LLM calls, ranking, retrieval, or transport binding — products supply those through injected ports and adapters. Reference adapters and conformance demos live in `fixtures/toy-world/`.

## Normative references

- [spoke-operations.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-operations.md) — helper contracts, adapter interfaces, injection orchestration
- [@42ch/spoke-operations README](https://github.com/42ch-dev/spoke/blob/main/packages/spoke-operations/README.md) — TypeScript usage
- [spoke-operations README (Rust)](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-operations/README.md) — Rust usage
- [fixtures/toy-world/](https://github.com/42ch-dev/spoke/tree/main/fixtures/toy-world) — reference adapter examples and conformance graph
