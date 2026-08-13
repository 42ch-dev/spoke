---
title: Domain profiles
---

# Domain profiles

A **Domain Profile** publishes ontology vocabulary over the core wire shapes: profile-specific `entry_type` labels, `relation_type` values, body-attribute traits, and `modules.*` dialects. The core schemas stay unchanged — the vocabulary rides open strings and the optional `modules` bag. Four profiles are documented on this page; each section lists the published open-string vocabulary integrators can emit and consume.

## Narrative structure

**Beat-assisted narrative outlining** over existing SPOKE wire shapes: ordered story pivots, scene atoms, and structural roles.

| Published vocabulary | Wire home | Values |
|----------------------|-----------|--------|
| Profile entry type | `KnowledgeEntry.entry_type` | `beat` (profile-only label, valid open string) |
| Structural roles | `KnowledgeEntry.body.attributes[].trait_type` | `structural_role` with slots such as `midpoint`, `catalyst`, `finale` |
| Beat ordering | `Relation.relation_type` | `precedes` (or `follows`) between dual KnowledgeEntry ids |
| Moment tier | `TimelineEvent.timeline_scale` / `Scope.timeline_scale` | `moment` — atomic / scene beats are TimelineEvents at this tier |
| Beat–entry link | `TimelineEvent.extensions.spoke` | `timeline_entry_id` pointing at the dual KnowledgeEntry |
| Parenthetical pauses | `SourceAnchor.span` | screenplay `(beat)` pauses on dialogue text |

Key statements: an atomic / scene beat is a `TimelineEvent` with `timeline_scale: "moment"`, optionally paired with a KnowledgeEntry (dual-concern); a structural beat is a `BodyAttribute` with `trait_type: "structural_role"`; ordering runs through `precedes` Relations; selection uses `Scope` filters with `timeline_scale: "moment"`, `timeline_event_ids`, or `entry_types` including `beat`. The operations library exports moment-scale filters and beat-sheet ordering helpers (`filterTimelineEventsByMomentScale`, `orderTimelineEventsByIds`, `orderTimelineEventsByPrecedes`) — pure functions over caller-supplied arrays.

## Lore activation

**Lore activation** defines the fire conditions under which a KnowledgeEntry prefers to surface into assembled context. It lives as the `modules.activation` inner dialect on the optional `modules` bag (capability-flagged `narrative-modules`); matching, scanning, and ranking stay product-local.

| Published vocabulary | Wire home | Values |
|----------------------|-----------|--------|
| Trigger keys | `modules.activation.keys` | primary activation triggers (aliases, names, phrases) |
| Selective combination | `modules.activation.secondary_keys` + `.logic` | `and_any`, `and_all`, `not_any`, `not_all` |
| Always-on seed | `modules.activation.constant` | `true` marks an always-on seed candidate (valid with empty `keys`) |
| Insertion hints | `modules.activation.order` / `.priority` | ordering and tie-break hints |
| Placement hints | `modules.activation.position_hint` / `.outlet` | `before_defs`, `after_defs`, `depth`, `outlet` |
| Match flavor | `modules.activation.match` | literal, regex, or whole-word key matching (flavor defined by the product scanner) |

Key statements: `body.summary` and `AssemblePacket` entry `snippet` read as complete lore facts without the trigger keys (standalone snippet); `constant: true` entries feed the always-on seed set while keyed entries feed the activation pool; lore adjacency expands through `Relation` edges.

## Knowledge pack

A **Narrative Knowledge Pack** is a portable lore bundle: an ordered set of KnowledgeEntries, Relations, and optional SourceAnchors that travel between narrative hosts. Packs use existing wire atoms plus a product transport envelope for catalog metadata.

| Published vocabulary | Wire home | Values |
|----------------------|-----------|--------|
| Pack atoms | existing wire objects | KnowledgeEntry (lore nodes), Relation (graph edges), SourceAnchor (optional provenance) |
| Catalog metadata | product transport envelope | `title`, `version`, `creator`, optional `description` — not `modules.*` on KE or AssemblePacket |
| Per-entry fire conditions | `modules.activation` per KnowledgeEntry | travels with each entry when the pack carries activation |
| Round-trip | `extensions` / `modules` / open strings | importers preserve unknown namespaces, unknown module keys, and open-string vocabulary verbatim |

Key statements: pack ≠ AssemblePacket (durable library interchange vs ephemeral assemble output); narrative hosts stack multiple packs product-side (world pack, character pack, session pack) — import atoms preserving export order, merge Relation graphs by id, union seed and pool sets, then run activation / scope / budget after the merge; stack policy (priority, override, soft-delete) is product-local.

## Assemble recipes

Two **packet-level** companion dialects under the optional `AssemblePacket.modules` bag: where each assembled entry prefers to inject, and why it was activated.

| Published vocabulary | Wire home | Values |
|----------------------|-----------|--------|
| Placement rows | `AssemblePacket.modules.placement[]` | `entry_id` + `position_hint` (`before_defs`, `after_defs`, `depth`, `outlet`), optional `depth` / `outlet` |
| Activation trace rows | `AssemblePacket.modules.activation_trace[]` | `entry_id` + `reason` (`constant`, `key`) with optional matched-key detail |

Key statements: array order is the interchange order — hosts apply their own layout after reading the hints, and an entry may omit a placement row (hosts then use a local default); join `placement[]` / `activation_trace[]` to `entries[]` by `entry_id`; entry-level `modules.activation` is the durable authoring home for preferences, while packet-level `placement[]` is the assembled snapshot for this packet; baseline `AssemblePacket` stays wire-only slim entries — `modules` is opt-in via `narrative-modules`.

## Mental state

**Mental state** defines how narrative hosts exchange mental-state data (beliefs, goals, intentions, emotions, observation access) without a shared engine. It lives as three `modules.*` dialects under capability flags `l5-mind` + `narrative-modules`; a temporal record object (`MindState`) carries derivative snapshots/deltas.

| Published vocabulary | Wire home | Values |
|----------------------|-----------|--------|
| Mental fields | `KnowledgeEntry.modules.mental` | nine-field vocabulary: `identity`, `beliefs`, `attention`, `goals`, `intentions`, `emotions`, `dispositions`, `norms`, `constraints` |
| Belief labels | `KnowledgeEntry.modules.belief` | `holder` (entry_id or `world`), `proposition`, `order` (0–3), seven-dimension labels (Truth/Access/Representation/Content/Source/Context) |
| Event observation | `TimelineEvent.modules.observation` | `observers: entry_id[]`, optional `access` (perceptual constraints) |
| Temporal record | `MindState` wire object | `mind_state_id`, `holder_entry_id`, `snapshot`, `deltas` — strictly derivative |

Key statements: the settled home of mental fields and belief labels is the holder KnowledgeEntry `modules.mental`/`modules.belief`; `MindState` is strictly temporal/derivative (no dual authority); false belief = True world fact + False actor belief (one labeled row); unobserved event ⇒ stale belief ⇒ false belief (Knowledge Access derivation).

## Related

- [Data model reference](/reference/data-model) — the open-string fields these vocabularies ride.
- [Concepts](/explanation/concepts) — open-vocabulary posture and capability flags.
- [Walk the ToyWorld reference adapter](/how-to/walk-toy-world) — profile `entry_type: "beat"` in the committed fixture graph.
