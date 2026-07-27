# Beat-assist moment sequence (dual KE + precedes)

> **Category:** architecture-patterns  
> **Source:** compound 2026-07-28 (beat-assist protocol slice); promoted from iteration guide  
> **Packages:** `@42ch/spoke-operations` / `spoke-operations`; fixtures under `fixtures/toy-world/`

## Problem

Craft “beat sheets” need an ordered chain of scene atoms. Integrators often try to point `Relation.from_id` / `to_id` at `TimelineEvent` ids. That is not legal on the Relation wire — endpoints are KnowledgeEntry (or SourceAnchor) ids only — so ordering must be assembled through the dual-concern link.

## Pattern

1. **Atomic beat** = `TimelineEvent` with `timeline_scale: "moment"` (optional SourceAnchor for dialogue spans).
2. **Optional dual KnowledgeEntry** = `entry_type: "event"` or profile `"beat"`, linked by `TimelineEvent.extensions.spoke.timeline_entry_id` → KE `entry_id`.
3. **Order via Relation** = `relation_type: "precedes"` (open vocab) between **KE** `from_id` / `to_id`.
4. **Alternate order** = caller-owned `Scope.timeline_event_ids` without Relations.
5. **Pure helpers** (caller-supplied arrays only):
   - `filterTimelineEventsByMomentScale` / `filter_timeline_events_by_moment_scale`
   - `orderTimelineEventsByIds` / `order_timeline_events_by_ids`
   - `orderTimelineEventsByPrecedes` / `order_timeline_events_by_precedes`  
   Kahn topo-sort; ready-queue tie-break = UTF-8 lexicographic `timeline_event_id`; cycle → `INVALID_INPUT` + `details.precedes_cycle`.

## Harbor sample

Dawn → market → customs → berth: moment events + dual KEs + KE-scoped `precedes` rows under `fixtures/toy-world/` (`evt_tw_harbor_*`, `kb_tw_harbor_*`, `rel_tw_harbor_precedes_*`).

## Gotchas

- Do not invent Relation endpoints on `timeline_event_id`.
- Unlinked moment events append after the linked topo subset in **input order**.
- Duplicate `timeline_entry_id` across input events: helpers keep all linked events in the linked set; entry→event map for edge endpoints is last-wins (caller should avoid duplicates).
- Structural beat-sheet roles use `body.attributes` traits (e.g. `structural_role`) and/or open Relation strings — not new schema fields.
- Profile `beat` stays profile-only; no Beat/BeatSheet core wire.

## See also

- `.mstar/specs/domain-profile-narrative-structure.md` — Beat senses → wire mapping handbook
- `.mstar/specs/spoke-operations.md` §14 — narrative sequence helper contracts
- `architecture-patterns/knowledge-entry-timeline-event-vocabulary.md` — dual-concern pair
- `architecture-patterns/timeline-projection-tiers.md` — L5 `moment` tier
- `architecture-patterns/spoke-operations-pure-actions.md` — purity + SpokeResult model
