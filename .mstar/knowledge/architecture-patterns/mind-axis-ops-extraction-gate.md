---
module: spoke-operations / mind-axis
date: 2026-08-14
problem_type: architecture_pattern
category: architecture-patterns
severity: medium
applies_when: ["evaluating whether a research or paper candidate belongs in spoke-operations", "mind-axis ops demand gate triggers (≥2 consumers)", "freezing handbook-defined inner shapes into closed schema", "adding any new optional capability with temporal semantics"]
tags: [l5-mind, mind-axis, spoke-operations, pure-library-boundary, demand-gate, MindDelta, operations-extraction, research-pack]
---

# Mind-axis ops extraction gate — what belongs in spoke-operations

## Context

The `l5-mind` capability ships wire + one pure validator only (`validateMindState` / `validate_mind_state`). The underlying research (Mental World Modeling, arXiv 2607.27201; OmniToM, arXiv 2605.26322) contains substantial operational content beyond wire shapes — check patterns, belief-staleness logic, observation-access derivation, transition modeling. Every future slice that touches the mind axis re-opens the question "should this go into `spoke-operations`?" This pattern records the evaluation so it is not re-derived from the research pack each time.

## Guidance

### The boundary (normative)

`@42ch/spoke-operations` / `spoke-operations` ship only **pure, deterministic shape and plumbing operations over wire types**. No inference, belief revision, ranking, matching, rendering, or I/O. Concretely for the mind axis:

1. **Checker Finding patterns are output contracts, not shipped engines.** The handbook documents `stale_belief_drift`, `dramatic_irony_asymmetry`, `access_violation`, and `action_content_mismatch` as what a product-local checker emits. `orchestrateCheck` already injects the product's checker callback. Implementing these patterns in the ops library duplicates product engines and violates the pure-library contract.
2. **Handbook-defined inner shapes get no validators until frozen into closed schema.** `modules.belief` seven-dimension labels and `modules.observation` `access` keys are open objects (handbook-defined). A validator over an open shape over-constrains it. Freezing an inner field table into closed JSON Schema is a separate, demand-gated wire decision; only then does a validator make sense.
3. **New helper families need a protocol-surface consumer.** Existing sequence helpers (`filterTimelineEventsByMomentScale`, `orderTimelineEventsByIds`, …) exist because check/assemble selectors and orchestration consume timeline events. A speculative twin for `MindState` (ordering/filtering) has no selector, port, or orchestration consumer today — symmetry alone is not a reason.
4. **Every candidate ships dual-language parity + golden vectors** (TS ↔ Rust, shared fixtures). The parity cost is real; demand evidence must justify it.

### The demand gate

Mind-axis ops beyond `validateMindState` are gated on **≥ 2 independent consumers requesting them** (registered on the roadmap; tracked as durable Up-next work). When the gate triggers, revisit candidates in priority order:

1. **`applyMindDeltas` / `apply_mind_deltas`** — apply `MindDelta[]` (`{path, previous?, next?}`) to a baseline `MentalFieldMap` to derive a snapshot. Pure mechanical change application (no inference). Two prerequisites before committing: (a) family symmetry — `ComputableLogChange` has no apply helper either; decide whether both get one together; (b) `path` semantics inside the open `MentalFieldMap` shape.
2. **`MindState` ordering/filtering helpers** (e.g., `orderMindStatesByOccurredAt`, `filterMindStatesByHolder`) — only if a selector, port surface, or orchestration path starts consuming `MindState`.

### Rejected candidates (stable unless the boundary changes)

| Candidate (research source) | Verdict | Why |
|-----------------------------|---------|-----|
| D9 checker patterns: stale-belief drift, dramatic-irony asymmetry, access violation, action-content mismatch | Out of ops | Checker output contracts; checker itself product-local; `orchestrateCheck` injects it |
| Observation-access derivation ("who could perceive E", κ rendering) | Out of ops | Observation *rendering* is product-local by handbook boundary table |
| Belief staleness determination / belief revision | Out of ops | Update logic is product-local |
| `mergeMentalFieldMaps` | No | Open field map; shallow merge silently collides on nine-field scalar keys; `mergeModuleMaps` precedent does not transfer (ModuleMap is a closed map shape) |
| Seven-dimension belief label validators | Deferred | Inner shapes open until schema freeze; freeze is demand-gated |

## Why This Matters

The pure-library contract is a hard repo boundary (root AGENTS.md: no I/O, storage, LLM, HTTP, MCP, ranking, retrieval, or silent auto-promote). Crossing it with checker logic or derivations breaks the contract and duplicates product engines — the exact failure the `l5-mind` design rejected (engines stay product-local). Crossing it with speculative helpers commits dual-language parity cost with no consumer. The gate exists so the boundary is enforced by demand evidence, not by re-reading the research pack.

## When to Apply

- Evaluating any research-pack or paper candidate for inclusion in `spoke-operations`
- The mind-axis ops demand gate fires (≥ 2 consumers request ops / Scope surface)
- Before freezing a handbook-defined inner shape into closed schema (the validator question reopens at that point)
- Designing any new optional capability with temporal semantics — the same gate shape applies (wire + minimal validator first; ops only on consumer evidence)

## Examples

**`l5-mind` (shipped):** wire (`MindState` schema, `TimelineEvent.modules`), one pure validator (`validateMindState` / `validate_mind_state`), handbook check patterns as contracts. Nothing else.

**Gate triggered (hypothetical ≥ 2 consumers):** ship `applyMindDeltas` first (mechanical, mirrors the derivative-record philosophy), then ordering helpers only if a selector consumes `MindState`.

**Gate not triggered:** D9 Finding patterns stay documented contracts; a product checker emits `Finding` objects (`kind: stale_belief_drift` etc.) through `orchestrateCheck`.

## See also

- [`.mstar/specs/l5-mind-capability-adr.md`](../specs/l5-mind-capability-adr.md) — normative ADR; capability boundary
- [`.mstar/specs/domain-profile-mental-state.md`](../specs/domain-profile-mental-state.md) — handbook; Finding check patterns as contracts
- [`mind-axis-ownership-boundary.md`](mind-axis-ownership-boundary.md) — companion: where the data lives (settled home vs derivative records)
- [`spoke-operations-pure-actions.md`](spoke-operations-pure-actions.md) — the pure-library precedent
- [`capability-flagged-optional-bag.md`](capability-flagged-optional-bag.md) — demand-gate resolution via capability flags
