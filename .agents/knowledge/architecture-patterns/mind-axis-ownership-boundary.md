---
module: cognition / capability design
date: 2026-08-13
last_updated: 2026-11-07
problem_type: architecture_pattern
category: architecture-patterns
severity: high
tags:
  - cognition
  - ownership-boundary
  - modules-cognition
  - derivative-records
  - mindstate
  - memory
  - reference-identity
  - no-record
---

# Cognition ownership boundary — one current authority, derivative temporal and recorded records

## Context

A capability that combines **durable current state** with **temporal change** must answer two questions: where does the authoritative copy live, and what is the derivative record? The `spoke-cognition` family answers both for epistemic assessment, Belief and Perception, and the shape of the question recurs for any later state-plus-time capability.

## Guidance

### The rule

**One fact, one authority.** For an actor or group, current cognition lives on that holder's `KnowledgeEntry.modules.cognition` (`CognitionModule { records, world_facts, extensions }`), and those records carry `view: "current"`. `modules.mental` keeps its nine open fields (`identity`, `beliefs`, `attention`, `goals`, `intentions`, `emotions`, `dispositions`, `norms`, `constraints`); its `beliefs` member is a derivative summary/count/reference only, never authority, and no other mental facet acquires mandatory `q`. Temporal and recorded records never carry a second current authority.

World facts are separate: a `WorldFactRecord` on the designated world-state KnowledgeEntry has `holder.kind = "world"`, `order: 0` and `recorded_at`; it is not an actor belief and acquires no mandatory `q` merely by existing.

### The derivative records

- **`MindState` / `MindDelta` (L5 Temporal presentation).** `MindState` keeps its open nine-field `snapshot` and may carry a derivative `cognition` projection. `MindDelta.path` is an RFC 6901 JSON Pointer rooted at `/modules/mental/<field>`, `/modules/cognition/records/...` or `/modules/cognition/world_facts/...`. No path targets `MindState` itself, and no delta application, replay, ordering or history engine exists.
- **Memory `Content.cognition`.** Every entry is `view: "recorded_at"` and is authorized and stored independently. A reassessment appends a new attributed assessment; it never rewrites retained content and never mutates or synchronizes holder-side current authority. Memory review, retention, transfer and reflection never create or mutate a `KnowledgeEntry`.

### The precedent

`ComputableLogChange` (`{path, previous?, next?}` inside `ComputableLogEntry.changes[]`) is the in-repo change-unit precedent, with authority `body.computable` on the KnowledgeEntry. `MindDelta` mirrors that shape and points into settled homes rather than into the derivative record.

### Identity, disagreement and absence

- Reference identity is the complete `(kind, namespace, id)` tuple. Equal `text`, equal bare ids, or the same id under different namespaces never establish identity, and no registry or resolution I/O exists.
- Valid attributed disagreements stay separate records. A current view is an explicitly requested projection, never authority: it carries the product `selection_policy` id/version, the `input_record_ids` and the `selected_record_id` chosen from them. The protocol supplies no latest-wins, averaging, consensus or source-privilege algorithm, and a projection never rewrites a record.
- `presence: "assessed"` and `presence: "no_record"` are distinct. `no_record` is emitted only when the filter addresses holder, target and assessment context exactly and the authorized source holds no record for that compound key before optional category/dimension/`q` predicates; records excluded by those predicates produce no item rather than a no-record claim.
- `known`, `ignorant`, `undetermined`, no-record, missing metadata and explicit field absence remain different facts. `truth: "Unknown"` is not holder ignorance, and missing is never zero.

### The rejected alternatives

| Alternative | Why rejected |
|-------------|--------------|
| New Entity class (cognition as a KnowledgeEntry superset) | Splits identity and creates a dual SSOT with the holder entry; violates the closed-core growth path |
| New cognitive layer | Cognition is inherently temporal and current at once; a new layer adds cross-layer coupling without a distinct concern |
| Layer-anchored family flag | The concern spans KE modules, Scope selection, assembly and Memory, so a single-layer prefix misdescribes its carrier — the `spoke-cognition` family names the actual carrier; see `ke-capability-flag-naming.md` |
| Vocabulary-only (`entry_type: "cognition"`) | Labels carry no features, authority or estimand |
| `MindState` or Memory carrying full current state | Dual SSOT: "the holder believes X" would exist in two places with independent revision drift |

### Checklist for a future state-plus-time capability

1. **Settled home** — which existing durable object owns the authoritative state? (usually a `KnowledgeEntry.modules.*` dialect)
2. **Derivative record** — what temporal or recorded artifact carries the change, and how is it marked derivative?
3. **Change-unit shape** — does it mirror `ComputableLogChange`? If yes, reuse it.
4. **Capability flag** — anchored to the actual carrier; see `ke-capability-flag-naming.md`.
5. **Ownership statement** — the ADR states settled home = X, derivative record = Y, and no dual SSOT.
6. **Absence and identity** — state what missing, explicit absence, no-record and cross-namespace identity mean before freezing shapes.

## Why This Matters

Dual SSOT is the common failure mode when a temporal or recorded record is added to a state system; the advisory precedent is that `TimelineEvent` stays clean precisely because it is a when-axis reference that does not re-describe entry content. The same discipline keeps `MindState` and Memory from becoming second authorities, and keeps a product projection from silently becoming one.

## When to Apply

- Adding or changing a capability that has both current state and temporal/recorded projections
- Designing a wire object that references or derives from an existing durable object
- Deciding whether a concept needs a new entity, a new layer, or a `modules.*` dialect
- Writing a current-view projection or a Memory-side cognition capture

## Examples

**`spoke-cognition` (shipped):**
- Settled home: `KnowledgeEntry.modules.cognition` (records + world facts) for current cognition; open `modules.mental` for unrelated facets, with derivative `beliefs`
- World facts: `WorldFactRecord` with `holder.kind = "world"`, `order: 0`
- Derivatives: `MindState.cognition` projection and JSON-Pointer deltas; Memory `Content.cognition` with `view: "recorded_at"`
- Selection: `CognitionSelection` on the shared `Scope`, producing an attributed projection with a declared policy trace

**`l2-computable` (precedent):**
- Settled home: `body.state` on the KnowledgeEntry
- Derivative record: `ComputableLogEntry` on the TimelineEvent (Moment tier)
- Change-unit: `ComputableLogChange` (`{path, previous?, next?}`)
- Flag: `l2-computable` (L2 + L5)

## See also

- [`.agents/adr/implemented/architecture/2026-11-07-spoke-cognition-family.md`](../../adr/implemented/architecture/2026-11-07-spoke-cognition-family.md) — normative family ADR (authority, conformance, cutover)
- [`.agents/specs/cognition/spoke-cognition-data-contract.md`](../../specs/cognition/spoke-cognition-data-contract.md) — record, module, projection and Memory shapes
- [`.agents/specs/cognition/spoke-cognition-consumer-contract.md`](../../specs/cognition/spoke-cognition-consumer-contract.md) — served-port conformance, negotiation and consumer map
- [`.agents/specs/domain-profiles/domain-profile-mental-state.md`](../../specs/domain-profiles/domain-profile-mental-state.md) — mental-state handbook and check-pattern contracts
- [`spoke-codegen-pipeline.md`](spoke-codegen-pipeline.md) — generated types, nominal clones and Builder boundaries
- [`mind-axis-ops-extraction-gate.md`](mind-axis-ops-extraction-gate.md) — what the ops library ships for this boundary
- [`ke-capability-flag-naming.md`](ke-capability-flag-naming.md) — flag naming for cross-layer concerns
- [`relation-occ-parity.md`](relation-occ-parity.md) — OCC parity (another ownership pattern: revision assignment)
