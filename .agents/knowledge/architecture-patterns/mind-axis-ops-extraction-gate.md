---
module: spoke-operations / cognition family
date: 2026-08-14
last_updated: 2026-11-07
problem_type: architecture_pattern
category: architecture-patterns
severity: medium
applies_when:
  - evaluating whether a research or paper candidate belongs in spoke-operations
  - the cognition ops demand gate fires (two or more consumers)
  - freezing a handbook-defined inner shape into closed schema
  - adding a new optional capability with temporal semantics
tags:
  - spoke-operations
  - pure-library-boundary
  - cognition-validators
  - cognition-selection
  - demand-gate
  - operations-extraction
---

# Cognition ops extraction gate — what belongs in spoke-operations

## Context

`spoke-cognition` ships closed standard record shapes, one shared pure validator set and the deterministic selection/projection path. The research and handbook material around it (check patterns, belief staleness, observation-access derivation, transition modeling) contains far more than wire shapes, and every later slice re-opens the question "does this belong in `spoke-operations`?" This document fixes the boundary so it is not re-derived from the research pack each time.

## Guidance

### The boundary (normative)

`@42ch/spoke-operations` / `spoke-operations` ship only **pure, deterministic shape and plumbing operations over wire types**. No inference, belief revision, ranking, product-level semantic matching engines, rendering, generation or I/O. Deterministic filter matching over wire types stays in scope (item 1). Concretely:

1. **The shipped surface, and why it qualifies.** Closed-shape validators — `validateCognitionRecord`, `validateCognitionContent`, `validateCognitionModule`, `validateObservationModule`, `validateCognitionFilter`, `validateCognitionSelection`, `validateCognitionProjection` (Rust: `validate_cognition_record`, `validate_cognition_content`, `validate_cognition_module`, `validate_observation_module`, `validate_cognition_filter`, `validate_cognition_selection`, `validate_cognition_projection`) — plus deterministic selection: `cognitionRecordMatchesFilter`, `selectCognitionRecords`, `projectCognitionRecords` (Rust: `cognition_record_matches_filter`, `select_cognition_records`, `project_cognition_records`). They serve real consumers: KE scope/check/assemble through `Scope.cognition`, and Memory list/recall/reflection through `cognition_filter`. Projection applies an injected product `selection_policy` and verifies only that the selected id belongs to the traced inputs — it supplies no policy algorithm.
2. **Checker Finding patterns are output contracts, not shipped engines.** `stale_belief_drift`, `dramatic_irony_asymmetry`, `access_violation` and `action_content_mismatch` describe what a product-local checker emits; `orchestrateCheck` injects that callback. Implementing the patterns here would duplicate product engines and violate the pure-library contract.
3. **Inner shapes get validators only once schema freezes them.** The nine mental fields stay open in `modules.mental`, and `ObservationAccess` keeps unknown keys — a validator over an open shape over-constrains it. The seven Belief dimensions and the record/estimand vocabularies are closed in `cognition.schema.json`, so those are validated; anything still handbook-defined is not.
4. **A new helper family needs a protocol-surface consumer.** Existing sequence helpers (`filterTimelineEventsByMomentScale`, `orderTimelineEventsByIds`, …) exist because check/assemble selectors and orchestration consume timeline events. Symmetry alone is not a reason.
5. **Every candidate ships dual-language parity plus shared vectors** (TS ↔ Rust, shared accept/reject fixtures). The parity cost is real; demand evidence must justify it.

### The demand gate

Ops beyond the shipped validator/selection surface are gated on **two or more independent consumers requesting them**, recorded as durable roadmap work. When the gate triggers, evaluate in this order:

1. **`applyMindDeltas` / `apply_mind_deltas`** — apply `MindDelta[]` (`{path, previous?, next?}`) to a baseline `MentalFieldMap` to derive a snapshot. Not shipped. Two prerequisites before committing: (a) family symmetry — `ComputableLogChange` has no apply helper either; decide whether both get one together; (b) `path` semantics inside the open `MentalFieldMap` shape.
2. **`MindState` ordering/filtering helpers** (e.g. `orderMindStatesByOccurredAt`, `filterMindStatesByHolder`) — only if a selector, port surface or orchestration path starts consuming `MindState` itself.

### Rejected and deferred candidates (stable unless the boundary changes)

| Candidate | Verdict | Why |
|-----------|---------|-----|
| Checker Finding patterns: stale-belief drift, dramatic-irony asymmetry, access violation, action-content mismatch | Out of ops | Output contracts; the checker is product-local and injected |
| Observation-access derivation ("who could perceive E", κ rendering) | Out of ops | Rendering and inference are product-local; observation carries could-perceive metadata only |
| Belief staleness determination / belief revision | Out of ops | Update policy is product-local |
| `mergeMentalFieldMaps` | No | Open field map; a shallow merge collides on scalar keys; `mergeModuleMaps` does not transfer (ModuleMap is a closed map shape) |
| Delta application, replay or history engine | Deferred, absent | Gate not met; see the demand-gate prerequisites |
| Cognition ordering/aggregation helpers | Deferred, absent | No consumer; selection is filter plus projection, not ranking |
| Metric scoring, calibration or aggregation | Out of ops | The protocol stores declared estimates; it never computes, combines or calibrates them |

## Why This Matters

The pure-library contract is a hard repository boundary (root `AGENTS.md`: no I/O, storage, LLM, HTTP, MCP, ranking, retrieval or silent auto-promote). Crossing it with checker logic or derivations breaks the contract and duplicates product engines; crossing it with speculative helpers commits dual-language parity cost with no consumer. The gate enforces the boundary by demand evidence instead of by re-reading the research pack.

## When to Apply

- Evaluating a research-pack or paper candidate for inclusion in `spoke-operations`
- The cognition ops demand gate fires (two or more consumers request ops or Scope surface)
- Before freezing a handbook-defined inner shape into closed schema (the validator question reopens at that point)
- Designing any new optional capability with temporal semantics — wire plus minimal pure validation first; ops only on consumer evidence

## Examples

**Shipped surface:** closed record/module/filter/selection/projection validators, deterministic match/select/project helpers in both languages, and MindState wire validation including the RFC 6901 delta-pointer gate. Nothing else.

**Gate not triggered:** Finding patterns stay documented contracts; a product checker emits `Finding` objects through `orchestrateCheck`. No delta-apply, ordering or history helper exists, and none is planned by symmetry.

## See also

- [`.agents/adr/implemented/architecture/2026-11-07-spoke-cognition-family.md`](../../adr/implemented/architecture/2026-11-07-spoke-cognition-family.md) — family boundary and ADR D8 admission boundary
- [`.agents/specs/cognition/spoke-cognition-data-contract.md`](../../specs/cognition/spoke-cognition-data-contract.md) — §8 pure validation boundary
- [`.agents/specs/cognition/spoke-cognition-consumer-contract.md`](../../specs/cognition/spoke-cognition-consumer-contract.md) — served ports and consumer map
- [`mind-axis-ownership-boundary.md`](mind-axis-ownership-boundary.md) — where the data lives (current authority vs derivatives)
- [`spoke-operations-pure-actions.md`](spoke-operations-pure-actions.md) — the pure-library precedent
- [`capability-flagged-optional-bag.md`](capability-flagged-optional-bag.md) — demand-gate resolution via capability flags
