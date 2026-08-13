---
module: mind-axis / capability design
date: 2026-08-13
problem_type: architecture-decision
category: architecture-patterns
severity: high
plan_id: 2026-08-13-l5-mind-design-adr
tags: [l5-mind, MindState, ownership-boundary, settled-home, derivative, modules, dual-SSOT, ComputableLogChange, capability-flag, temporal-record]
---

# Mind-axis ownership boundary — settled home vs derivative temporal records

## Context

When adding a new cross-product capability to SPOKE that involves both **durable state** (mental fields, belief labels) and **temporal change** (snapshots, deltas), the design must answer: where does the authoritative copy live, and what is the derivative record?

This pattern emerged from the `l5-mind` capability design (Mental World Modeling + OmniToM research → SPOKE protocol). The same question recurs for any future temporal or state-tracking capability.

## Guidance

### The rule

**One fact, one authority.** The settled home of domain state lives on the holder object (`KnowledgeEntry` for entities, `TimelineEvent` for events) via `modules.*` dialects. Temporal records (`MindState`, future equivalents) carry strictly derivative snapshots/deltas — never a second authority.

### The precedent

`ComputableLogChange` (`{path, previous?, next?}` change-unit inside `ComputableLogEntry.changes[]`) is the in-repo precedent: a derived change log whose authority is `body.computable`. The `MindDelta` mirrors this shape exactly — `{path, previous?, next?}` where `path` points into the settled home (`modules.mental` / `modules.belief`), not into the MindState record itself.

### The rejected alternatives

| Alternative | Why rejected |
|-------------|-------------|
| New Entity class (mind-entity as KnowledgeEntry superset) | Papers model mind as fields on existing actors, not a separate entity; identity split creates dual SSOT; violates closed-core growth path |
| New layer (`l9-mind`) | Mental axis is inherently temporal (L5); cross-layer coupling |
| Functional flag name without layer anchor (`mind-axis`) | Breaks `l<n>-<concern>` convention (`l5-fork`, `l2-computable`) |
| Vocabulary-only (`entry_type: "mind"`) | Labels carry no features; no temporal axis |
| MindState carries full settled state | Dual SSOT: "Bob believes X" exists in two places; revision drift |

### The checklist for future capabilities

When adding a capability that touches both state and time:

1. **Settled home** — which existing durable object owns the authoritative state? (Usually KnowledgeEntry via `modules.*`.)
2. **Derivative record** — what temporal record carries the change? (New first-class wire object, strictly derivative.)
3. **Change-unit shape** — does it mirror `ComputableLogChange` (`{path, previous?, next?}`)? If yes, reuse the pattern.
4. **Capability flag** — `l<n>-<concern>` at the correct layer; not a new layer unless the concern is orthogonal to all existing layers.
5. **Ownership statement** — the ADR must normatively state: settled home = X; derivative record = Y; no dual SSOT.

## Why This Matters

Dual SSOT is the most common failure mode when adding temporal records to a state system. The advisory nit that sharpened this pattern: "TimelineEvent stays clean precisely because it's a when-axis reference that does NOT re-describe entry content." The same discipline applies to any temporal record — it must not re-describe what the settled home already owns.

## When to Apply

- Adding a new optional capability flag with temporal semantics
- Designing a wire object that references or derives from an existing durable object
- Deciding whether a new concept needs a new Entity class, a new layer, or a modules.* dialect

## Examples

**l5-mind (this iteration):**
- Settled home: `modules.mental` (nine fields) + `modules.belief` (seven-dim labels) on holder KnowledgeEntry
- Derivative record: `MindState` (temporal snapshots/deltas, strictly derivative)
- Change-unit: `MindDelta` (`{path, previous?, next?}` mirroring `ComputableLogChange`)
- Flag: `l5-mind` (L5 Temporal, pattern `l5-fork`)

**l2-computable (precedent):**
- Settled home: `body.state` on KnowledgeEntry
- Derivative record: `ComputableLogEntry` on TimelineEvent (Moment tier)
- Change-unit: `ComputableLogChange` (`{path, previous?, next?}`)
- Flag: `l2-computable` (L2 + L5)

## See also

- [`.mstar/specs/l5-mind-capability-adr.md`](../specs/l5-mind-capability-adr.md) — normative ADR with ownership boundary
- [`.mstar/specs/domain-profile-mental-state.md`](../specs/domain-profile-mental-state.md) — handbook with field tables
- [`spoke-codegen-pipeline.md`](spoke-codegen-pipeline.md) — codegen pipeline (schema count, verify-codegen)
- [`relation-occ-parity.md`](relation-occ-parity.md) — OCC parity (another ownership pattern: revision assignment)
