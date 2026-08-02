---
title: Concepts
---

# Concepts

SPOKE defines its vocabulary in **wire terms** — every concept below is a concrete JSON shape or open-string vocabulary on the protocol surface. The authoritative definitions live in [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md); this page is a tour to help you pick the right artifact for your integration.

## Core objects

- **KnowledgeEntry** — the atomic knowledge-base entry: stable `entry_id`, open `entry_type` / `status` strings, closed `body` (summary, tags, scalar traits), optional provenance, required `extensions`.
- **Relation** — directed edge between two KnowledgeEntries (or an entry and a source), with optional `revision` for optimistic concurrency.
- **SourceAnchor** — pointer to a source artifact span (manuscript, scene, external locator).
- **Finding** — checker output (consistency, style, structure), distinct from a KnowledgeEntry body.
- **Rule** — declarative constraint input to `check` (L6), distinct from an ontology label.
- **TimelineEvent** — first-class when-axis object (L5) with optional `timeline_scale` tiers `brief` / `narrative` / `moment`.
- **AssemblePacket** — wire-only context-assembly payload: slim entries for downstream consumption.
- **HostCapabilityManifest** — adapter self-description: `host_id`, `roles`, `capabilities`, owned `namespaces`.

## Selectors and extension points

- **Scope** — shared ops selector for `check` / `assemble`: required opaque `scope_id` plus optional refinements (`entry_ids`, `entry_types`, `timeline_scale`, `fork_id`, …).
- **Domain Profile** — how integrators publish ontology vocabulary (beat mapping, lore activation, knowledge packs) over open strings; core enums stay open.
- **Extensions** — `extensions.<namespace>` product bag on every durable object; adapters round-trip unknown namespaces verbatim.
- **Modules** — optional `modules.*` bag (capability-flagged `narrative-modules`) for cross-product functional dialects on KnowledgeEntry and AssemblePacket.

## Dual-concern pairs

SPOKE keeps two pairs of concepts deliberately separate: an ontology `entry_type: "event"` label on a KnowledgeEntry versus the L5 `TimelineEvent` when-axis object, and an ontology `entry_type: "rule"` label versus the L6 `Rule` checker input. One local concept may map to one or both wire shapes; the names stay separate so check/assemble selectors remain unambiguous.

## Optional capabilities

- **`l2-computable`** — `body.state` / `body.computable`, `TimelineEvent.computable_logs`, and `project` / `compute` ops.
- **`l5-fork`** — `fork_id` / `parent_fork_id` world-history branch metadata on TimelineEvent.
- **`narrative-modules`** — the optional `modules` bag for shared functional dialects.
- **`spoke-connect`** — the opt-in cross-process interaction envelope family.

## Normative references

- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) — vocabulary SSOT, dual-concern rules, spelling
- [spoke-data-model.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-data-model.md) — field semantics for every data object
- [spoke-protocol-layers.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-protocol-layers.md) — layer model and capability levels
- [spoke-extension-modules.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-extension-modules.md) — core / modules / extensions placement authority
- [spoke-connect.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect.md) — connect vocabulary (`peer_id`, capability token)
