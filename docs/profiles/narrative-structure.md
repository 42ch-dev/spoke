---
title: Domain Profile — narrative structure
---

# Domain Profile — narrative structure

This Domain Profile handbook documents **Beat-assisted narrative outlining** over existing SPOKE wire shapes: ordered story pivots, scene atoms, and structural roles. It publishes open-string vocabulary and mapping guidance — the core schemas stay unchanged.

## Beat senses → wire mapping

- **Atomic / scene beat** — `TimelineEvent` with `timeline_scale: "moment"`, optionally paired with a KnowledgeEntry (dual-concern).
- **Structural beat** — `BodyAttribute` trait (`structural_role`) on a KnowledgeEntry body, e.g. `midpoint`, `catalyst`, `finale` (open vocabulary, not core `enum`).
- **Parenthetical beat** — screenplay `(beat)` pauses map to `SourceAnchor` spans on dialogue text.
- **Ordering** — `Relation` with `relation_type: "precedes"` (or `follows`) between dual KnowledgeEntry ids; moment events link to their KE via `extensions.spoke.timeline_entry_id`.
- **Selection** — `Scope` filters with `timeline_scale: "moment"`, `timeline_event_ids`, or `entry_types` including the profile `beat` label.

Profile-only `entry_type: "beat"` is a valid open string published by this profile — it stays outside the core `entry_type` table and schema description lists.

## Library support

`@42ch/spoke-operations` / `spoke-operations` export moment-scale filters and beat-sheet ordering helpers (`filterTimelineEventsByMomentScale`, `orderTimelineEventsByIds`, `orderTimelineEventsByPrecedes`) — pure functions over caller-supplied arrays, with no I/O or storage.

## Normative references

- [domain-profile-narrative-structure.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/domain-profile-narrative-structure.md) — full mapping matrix, Relation vocabulary, beat-sheet interchange sample
- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) — Domain Profile, dual-concern, TimelineScale
- [spoke-data-model.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-data-model.md) — TimelineEvent, BodyAttribute, Relation
- [spoke-ops.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-ops.md) — Scope refinements
- [spoke-operations.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-operations.md) — moment filter / order helpers
