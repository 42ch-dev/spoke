---
title: Concepts
---

# Concepts

SPOKE defines its vocabulary in **wire terms** — every concept below is a concrete JSON shape or open-string vocabulary on the protocol surface. This page is the key-statement tour: what the nine layers are, what capability flags claim, and which pairs the protocol keeps deliberately separate. Field-level detail lives on the [Reference](/reference/protocol) pages.

## The nine layers (L0–L8)

| Layer | What it carries |
|-------|-----------------|
| **L0 Envelope** | Identity + `schema_version` on all durable objects |
| **L1 Ontology** | Open `entry_type` strings + Domain Profiles |
| **L2 Body** | Closed `body`: summary, tags, scalar `attributes` (optional `state` / `computable` under `l2-computable`) |
| **L3 Provenance** | SourceAnchor pointers |
| **L4 Graph** | Typed directed Relations (with optional `revision` OCC) |
| **L5 Temporal** | TimelineEvent when-axis + projection tiers `brief` / `narrative` / `moment` (optional Fork under `l5-fork`; optional `MindState` mental-state records under `l5-mind`) |
| **L6 Constraint** | Rule declarative input to `check` |
| **L7 Finding** | Checker output + status lifecycle |
| **L8 Context** | AssemblePacket wire shape (assembly itself stays product-local) |

## Capability levels

A product claims compliance at a declared capability level. **`spoke-baseline`** covers L0–L8 semantics via the five ops wire families, `HostCapabilityManifest` + baseline `HostManifestPort`, and the shared `Scope` / `error-envelope` definitions. Optional flags are additive — baseline compliance stands alone:

- **`l2-computable`** — computable knowledge state and its history, with the `project` / `compute` ops.
- **`l5-fork`** — world-history branch metadata on the timeline.
- **`l5-mind`** — `MindState`, the strictly derivative temporal mental-state record over the when-axis — see the [MindState reference](/reference/mind-state).
- **`narrative-modules`** — the optional `modules` bag for cross-product functional dialects.
- **`ke-extraction`** — the optional `extract` op: referenced source material in, provisional KnowledgeEntry candidates out.
- **`ke-ownership`** — ownership and disclosure on the KnowledgeEntry envelope, with the `Scope.viewpoint` reader selector.
- **`spoke-connect`** — the opt-in cross-process interaction envelope family.

The connect family's session lifecycle, envelope authentication, and capability routing are explained in [Connect architecture](/explanation/connect).

## Dual-concern pairs

The protocol keeps three pairs deliberately separate:

- An ontology `entry_type: "event"` label on a KnowledgeEntry versus the L5 `TimelineEvent` when-axis object — one local concept may map to one or both wire shapes.
- An ontology `entry_type: "rule"` label versus the L6 `Rule` checker input — Rule is declarative input, Finding is checker output.
- An ontology `entry_type: "character"` / profile `mind` label versus the L5 `MindState` temporal record — the label classifies a knowledge node; `MindState` is the strictly derivative snapshot / delta of the holder's mental fields (`modules.mental` / `modules.belief` are the settled home).

The names stay separate so `check` / `assemble` selectors remain unambiguous.

## Open vocabulary and Domain Profiles

Core objects keep closed envelopes (`additionalProperties: false`), while core vocabulary stays **open**: `entry_type`, `relation_type`, and statuses are open strings with documented core lists, not closed enums. A Domain Profile publishes ontology vocabulary over those open strings — beat mapping, lore activation, knowledge packs — and products express profile-specific types with open strings and `extensions.<namespace>`. See [Domain profiles](/explanation/domain-profiles).

## Selectors and extension points

- **Scope** — the shared ops selector for `check` / `assemble`: required opaque `scope_id` plus optional refinements (`entry_ids`, `entry_types`, `timeline_scale`, `fork_id`, `viewpoint`, …); `viewpoint` is the `ke-ownership` reader selector.
- **Extensions** — `extensions.<namespace>` product bag on every durable object; adapters round-trip unknown namespaces verbatim.
- **Modules** — the optional `modules.*` bag (capability-flagged `narrative-modules`) for cross-product functional dialects on KnowledgeEntry, AssemblePacket, and TimelineEvent.

## Pure-library posture

The operations library is a pure behavior layer over the generated wire types: lifecycle helpers, capability-sliced adapter ports, and injection orchestration — no I/O of its own. Storage access, LLM calls, ranking, retrieval, and transport binding are supplied by products through injected ports; the reference adapters in `fixtures/toy-world/` demonstrate the pattern.

## Related

- [Protocol reference](/reference/protocol) — schema inventory, extensions contract, capability flags.
- [Data model reference](/reference/data-model) — field tables per layer.
- [Domain profiles](/explanation/domain-profiles) — the published open-string vocabularies.
- [MindState reference](/reference/mind-state) — the L5 temporal mental-state record (`l5-mind`).
