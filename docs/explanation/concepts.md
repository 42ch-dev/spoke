---
title: Concepts
---

# Concepts

SPOKE defines its vocabulary in **wire terms** — every concept below is a concrete JSON shape or open-string vocabulary on the protocol surface. This page is the key-statement tour: what the nine layers are, what capability flags claim, and which pairs the protocol keeps deliberately separate. Field-level detail lives on the [Reference](/reference/protocol) pages.

<img class="diagram-light" alt="The nine SPOKE protocol layers with their primary wire artifacts" src="/diagrams/protocol-layers.light.svg">
<img class="diagram-dark" alt="The nine SPOKE protocol layers with their primary wire artifacts" src="/diagrams/protocol-layers.dark.svg">

## The nine layers (L0–L8)

| Layer | What it carries |
|-------|-----------------|
| **L0 Envelope** | Identity + `schema_version` on all durable objects |
| **L1 Ontology** | Open `entry_type` strings + Domain Profiles |
| **L2 Body** | Closed `body`: summary, tags, scalar `attributes` (optional `state` / `computable` under `l2-computable`) |
| **L3 Provenance** | SourceAnchor pointers |
| **L4 Graph** | Typed directed Relations (with optional `revision` OCC) |
| **L5 Temporal** | TimelineEvent when-axis + projection tiers `brief` / `narrative` / `moment` (optional Fork under `l5-fork`; optional `MindState` mental-state records under `spoke-cognition`) |
| **L6 Constraint** | Rule declarative input to `check` |
| **L7 Finding** | Checker output + status lifecycle |
| **L8 Context** | AssemblePacket wire shape (assembly itself stays product-local) |

## Capability levels

A product claims compliance at a declared capability level. **`spoke-baseline`** covers L0–L8 semantics via the five ops wire families, `HostCapabilityManifest` + baseline `HostManifestPort`, and the shared `Scope` / `error-envelope` definitions. Optional flags are additive — baseline compliance stands alone:

- **`l2-computable`** — computable knowledge state and its history, with the `project` / `compute` ops.
- **`l5-fork`** — world-history branch metadata on the timeline.
- **`spoke-cognition`** — one family for standard, attributable cognition: `modules.cognition` records (Belief, epistemic knowledge/ignorance/undetermined, Perception) with mandatory `q`, `Scope.cognition` filters and attribution-preserving projections, `modules.observation` on the timeline, Memory `Content.cognition`, and the derivative `MindState` temporal view — see the [MindState reference](/reference/mind-state) and [Cognition records](/reference/data-model#cognition-spoke-cognition).
- **`narrative-modules`** — the optional `modules` bag for cross-product functional dialects.
- **`ke-extraction`** — the optional `extract` op: referenced source material in, provisional KnowledgeEntry candidates out.
- **`ke-ownership`** — ownership and disclosure on the KnowledgeEntry envelope, with the `Scope.viewpoint` reader selector.
- **`spoke-connect`** — the opt-in cross-process interaction envelope family.
- **`spoke-memory`** — per-subject memory: thirteen models and 33 `memory.*` ops behind the standalone `MemoryPort`, with optional `spoke-memory-reflection` and `spoke-memory-transfer` sub-flags — see the [Memory reference](/reference/memory).

The connect family's session lifecycle, envelope authentication, and capability routing are explained in [Connect architecture](/explanation/connect).

## Dual-concern pairs

The protocol keeps three pairs deliberately separate:

- An ontology `entry_type: "event"` label on a KnowledgeEntry versus the L5 `TimelineEvent` when-axis object — one local concept may map to one or both wire shapes.
- An ontology `entry_type: "rule"` label versus the L6 `Rule` checker input — Rule is declarative input, Finding is checker output.
- An ontology `entry_type: "character"` / profile `mind` label versus the L5 `MindState` temporal record — the label classifies a knowledge node; `MindState` presents a derivative snapshot / delta of the holder's mental state, while the holder's `modules.cognition` records and `modules.mental` fields carry the authoritative values.

The names stay separate so `check` / `assemble` selectors remain unambiguous.

## Memory beside knowledge

Canonical knowledge and subjective memory are two different jobs, so the protocol keeps them in two families. A KnowledgeEntry is a shared, reviewed node other hosts can check and assemble; retained memory carries two admission paths: captured material becomes a `MemoryCandidate` and counts only after `memory.candidates.review`, while `memory.records.create` writes a retained record directly as an authored management write. Reads follow the caller's grants.

| Aspect | KnowledgeEntry | Memory record |
|--------|----------------|---------------|
| Role | Canonical knowledge node on the shared surface | Per-subject impression captured by a host |
| Subject | Not subject-scoped | Exactly one `subject_id` (flat subjects) |
| Admission | `upsert`, `promote` | Captured: `memory.capture` → `MemoryCandidate` → `memory.candidates.review`. Authored management write: `memory.records.create` |
| Content trust | Envelope carries `ke-ownership` disclosure fields | Record and candidate content is untrusted data; profile documents are trusted author material |
| Forgetting | Status lifecycle (`deprecated`, `merged`, `deleted`) | Explicit operations only: drop/dismiss, tombstone delete, subject archive, import discard |
| Read axis | `check` / `assemble` over shared `Scope` | `ReadSelection` placement modes plus `ProvenanceFilter` |
| Authorization | `ke-ownership` disclosure predicate over `Scope.viewpoint` | Self-contained `MemoryGrant` |

Three consequences follow from that split. Emotion travels as an opaque convention inside `content.structured` (for example `{"affect":{"valence":0.6}}`), so mood taxonomies stay product policy. Author profiles (`ProfileDocument`) carry the refined authored basis, records carry concrete experiences, and reflections are the derived synthesis across records — three layers with distinct roles. And provider replacement is a wire procedure: a versioned, digested export/import with an operator-timed cutover and a single authority throughout.

Standard cognition crosses the two families in a defined way: the holder KnowledgeEntry carries current authority in `modules.cognition`, and a retained record keeps its recorded-at-time view in `Content.cognition`, filtered through `cognition_filter`. Each placement preserves its own attribution and times.

## Open vocabulary and Domain Profiles

Core objects keep closed envelopes (`additionalProperties: false`), while core vocabulary stays **open**: `entry_type`, `relation_type`, and statuses are open strings with documented core lists, not closed enums. A Domain Profile publishes ontology vocabulary over those open strings — beat mapping, lore activation, knowledge packs — and products express profile-specific types with open strings and `extensions.<namespace>`. See [Domain profiles](/explanation/domain-profiles).

## Selectors and extension points

- **Scope** — the shared ops selector for `check` / `assemble`: required opaque `scope_id` plus optional refinements (`entry_ids`, `entry_types`, `timeline_scale`, `fork_id`, `viewpoint`, `cognition`, …); `viewpoint` is the `ke-ownership` reader selector, and `cognition` is the `spoke-cognition` selection/projection carrier.
- **Extensions** — `extensions.<namespace>` product bag on every durable object; adapters round-trip unknown namespaces verbatim.
- **Modules** — the optional `modules.*` bag (capability-flagged `narrative-modules`) for cross-product functional dialects on KnowledgeEntry, AssemblePacket, and TimelineEvent.

## Pure-library posture

The operations library is a pure behavior layer over the generated wire types: lifecycle helpers, capability-sliced adapter ports, and injection orchestration — no I/O of its own. Storage access, LLM calls, ranking, retrieval, and transport binding are supplied by products through injected ports; the reference adapters in `fixtures/toy-world/` demonstrate the pattern.

## Related

- [Protocol reference](/reference/protocol) — schema inventory, extensions contract, capability flags.
- [Data model reference](/reference/data-model) — field tables per layer.
- [Domain profiles](/explanation/domain-profiles) — the published open-string vocabularies.
- [MindState reference](/reference/mind-state) — the derivative L5 temporal view (`spoke-cognition`).
- [Cognition records](/reference/data-model#cognition-spoke-cognition) — `q`, metrics, filter grammar and projection rules.
- [Memory reference](/reference/memory) — the optional `spoke-memory` family and how it sits beside canonical knowledge.
