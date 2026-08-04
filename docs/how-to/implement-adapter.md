---
title: Implement an adapter
---

# Implement an adapter

Your product speaks SPOKE by implementing the **port families your claimed capabilities require** on one adapter type, then calling the matching `orchestrate*` entrypoints from the operations library. The adapter is the only place your product's storage and I/O touch the protocol — every read and write flows through it, and every protocol rule runs in the library before persistence.

## 1. Choose your capability level

The adapter port types are sliced by capability. Pick the alias that matches the capabilities your host claims:

| Capability flags | Ports to implement | Composed alias |
|------------------|--------------------|----------------|
| `spoke-baseline` | `KnowledgeEntryPort`, `RelationPort`, `ScopeQueryPort`, `FindingPort`, `RuleQueryPort`, `HostManifestPort` | `BaselineAdapter` |
| `spoke-baseline` + `l2-computable` | baseline + `ComputablePort` | `ComputableAdapter` |
| `spoke-baseline` + `l5-fork` | baseline + `ForkTimelineQueryPort` | `ForkAdapter` |
| all three | full composition | `FullAdapter` |

The aliases name the same port intersections as `BaselinePorts` / `ComputablePorts` / `ForkPorts` / `FullPorts`. Import them from the operations package:

```ts
import type {
  BaselineAdapter,
  ComputableAdapter,
  ForkAdapter,
  FullAdapter,
  KnowledgeEntryPort,
  RelationPort,
  ScopeQueryPort,
  FindingPort,
  RuleQueryPort,
  HostManifestPort,
  ComputablePort,
  ForkTimelineQueryPort,
} from "@42ch/spoke-operations";
```

## 2. Implement the ports

### KnowledgeEntryPort — entry persistence

```ts
getKnowledgeEntry(entryId: string): SpokeResult<KnowledgeEntry>;
putKnowledgeEntry(
  entry: KnowledgeEntry,
  expectedBaseRevision: number | null,
): SpokeResult<KnowledgeEntry>;
```

`putKnowledgeEntry` is optimistic-concurrency controlled: `expectedBaseRevision: null` means the entry must be absent (create); a non-null value means the store's current revision must equal it (update). Reject with `STORED_REVISION_STALE` or `REVISION_CONFLICT` otherwise. For real concurrency safety, implement an atomic compare-and-put (CAS) in the adapter — the library stays I/O-free.

### RelationPort — relation persistence

```ts
getRelation(relationId: string): SpokeResult<Relation>;
putRelation(
  relation: Relation,
  expectedBaseRevision: number | null,
): SpokeResult<Relation>;
```

Revision assignment is adapter-owned: on create (`expectedBaseRevision: null`) seed `revision = 1`; on an accepted update persist `revision = stored + 1`. The returned Relation carries the assigned revision — callers do not set it.

### ScopeQueryPort — scoped reads for check/assemble

```ts
listKnowledgeEntries(scope: Scope): SpokeResult<KnowledgeEntry[]>;
listTimelineEvents(scope: Scope): SpokeResult<TimelineEvent[]>;
```

The orchestrators load scoped data through these ports, then apply the scope-filtering helpers (`filterKnowledgeEntriesByScope`, `filterTimelineEventsByScope`) to narrow to the request's `entry_ids` / `entry_types` / `timeline_scale` refinements.

### FindingPort — checker output persistence

```ts
putFindings(findings: Finding[]): SpokeResult<Finding[]>;
```

Findings are checker output, persisted through this port after `orchestrateCheck` runs your checker callback.

### RuleQueryPort — rule resolution

```ts
listRules(ruleRefs: string[]): SpokeResult<Rule[]>;
```

Resolves `check` rule references. Embedded `rules[]` in the request override by `rule_id`; unresolved refs reject the check.

### HostManifestPort — collaboration metadata

```ts
getHostCapabilityManifest(): SpokeResult<HostCapabilityManifest>;
listPeerHostCapabilityManifests(): SpokeResult<HostCapabilityManifest[]>;
```

The self manifest and product-known peer manifests. This port is baseline-required — it is the in-process collaboration surface (host roles, capability flags, owned namespaces).

### ComputablePort — optional `l2-computable`

```ts
project(request: ProjectRequest): SpokeResult<ProjectResponse>;
compute(request: ComputeRequest): SpokeResult<ComputeResponse>;
```

Session-scoped computable I/O. `orchestrateProject` / `orchestrateCompute` validate the request, then delegate to these methods; absent methods surface `CAPABILITY_PORT_MISSING` at dynamic boundaries.

### ForkTimelineQueryPort — optional `l5-fork`

```ts
listForkTimelineEvents(
  scope: Scope & { fork_id: ForkId },
): SpokeResult<TimelineEvent[]>;
```

Fork-scoped timeline reads. One object may satisfy both `ScopeQueryPort` and this port.

## 3. Keep the adapter I/O-bound

The operations library is pure relative to host I/O: storage access, LLM calls, ranking, retrieval, and transport binding are supplied by your product through these injected ports. The adapter implements the ports; the library runs the gates. The reference `ToyWorldAdapter` in [`fixtures/toy-world/`](https://github.com/42ch-dev/spoke/tree/main/fixtures/toy-world) demonstrates the pattern — see [Walk the ToyWorld reference adapter](/how-to/walk-toy-world) for the walkthrough.

## 4. Integrator notes

- **Transaction boundaries are adapter-owned.** Multi-entry upsert and other multi-write sequences span several `put*` calls; your adapter decides where the atomic boundary lies.
- **Active-uniqueness helpers take caller-supplied peer sets.** Orchestration supplies batch-local peers; pass a store-wide snapshot when uniqueness must span the whole store.
- **Missing optional ports surface `CAPABILITY_PORT_MISSING`** when an orchestrator needs them at a dynamic boundary. `HostManifestPort` is baseline-required and never gated behind that code.

## Next steps

- [Orchestrate operations](/how-to/orchestrate-ops) — the `orchestrate*` calls your adapter enables.
- [Walk the ToyWorld reference adapter](/how-to/walk-toy-world) — a complete `FullAdapter` in TypeScript and Rust, with a conformance harness.
- [Data model reference](/reference/data-model) — the wire objects your ports persist.
- [Ops wire reference](/reference/ops) — request/response envelope shapes.
