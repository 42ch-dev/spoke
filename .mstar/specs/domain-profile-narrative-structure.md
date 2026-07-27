# Domain Profile — Narrative Structure

> **Status:** Normative profile handbook (tracked result)  
> **Document class:** Domain Profile — narrative craft vocabulary over closed SPOKE envelopes  
> **Parent:** [`spoke-protocol-layers.md`](spoke-protocol-layers.md) §Domain Profile  
> **Wire SSOT:** `schemas/` (unchanged by this profile)

## Purpose

This Domain Profile documents how integrators express **Beat-assisted narrative outlining** — ordered story pivots, scene atoms, and structural roles — using existing SPOKE wire shapes. The profile publishes open-string vocabulary and mapping guidance over closed envelopes. Profile `entry_type: "beat"` remains outside the core `entry_type` table.

Integrators read this handbook to map craft “Beat” senses onto **KnowledgeEntry**, **TimelineEvent**, **Relation**, **BodyAttribute**, and **Scope**.

---

## Beat senses (craft vocabulary)

Craft discourse uses “Beat” at multiple scales. This profile maps atomic and structural senses to wire; parenthetical screenplay pauses map to manuscript presentation (see third row).

| Sense | Craft meaning | Profile mapping |
|-------|---------------|-----------------|
| **Atomic / scene beat** | Smallest story pivot on the when-axis (scene atom, micro-turn) | `TimelineEvent` with `timeline_scale: "moment"`; optional dual KnowledgeEntry |
| **Structural beat** | Named position on a beat sheet or act model (e.g. catalyst, midpoint) | `BodyAttribute` trait (e.g. `structural_role`) and/or `Relation` to a when-axis or KB object |
| **Parenthetical beat** | Screenplay pause marker `(beat)` in dialogue | `SourceAnchor` spans on dialogue text; integrator-local dialogue presentation models |

Atomic and structural beats interoperate: a beat-sheet row may reference one or more moment-scale TimelineEvents in order, while structural role labels attach to KB entries or narrative-scale events.

---

## Mapping matrix

| Integrator concept | Primary wire | Secondary / optional | Ordering |
|--------------------|--------------|----------------------|----------|
| Scene atom / micro-beat | `TimelineEvent` (`timeline_scale: "moment"`) | Dual KE: `entry_type: "event"` **or** profile `entry_type: "beat"` | `Scope.timeline_event_ids` and/or `Relation` `precedes` on **linked KE** `from_id` / `to_id` |
| Beat on narrative tier | `TimelineEvent` (`timeline_scale: "narrative"`) | Dual KE `entry_type: "event"` | Same as above |
| Beat-sheet slot label | `BodyAttribute` (`trait_type: "structural_role"`) on a KE body | `Relation` `fulfills` / `foreshadows` between KE `from_id` / `to_id` | Profile open strings; not core `enum` |
| Ordered beat sheet (interchange sample) | Moment `TimelineEvent` sequence + dual KE pairs | `precedes` chain on KE ids (see §Dual-concern link) | Dual-concern link + optional `precedes` chain (see §Beat sheet) |
| Scope filter for beat work | `Scope` with `timeline_scale: "moment"` and/or `timeline_event_ids` | `entry_types` when filtering profile `beat` KEs | [`spoke-ops.md`](spoke-ops.md) §Scope; ops [`filterTimelineEventsByMomentScale`](spoke-operations.md#14-narrative-sequence--timeline) |

**Dual-concern (unchanged):** KnowledgeEntry `entry_type: "event"` is an ontology label; `TimelineEvent` is the L5 when-axis object. The same local beat may map to one or both. See [`spoke-data-model.md`](spoke-data-model.md) §Dual-concern example.

**Dual-concern link (ordering):** `Relation` endpoints are **KnowledgeEntry** or SourceAnchor ids (`from_id`, `to_id`) — not `timeline_event_id`. For beat-sheet `precedes` chains, emit Relations between dual KE ids and link each moment `TimelineEvent` to its KE via `extensions.spoke.timeline_entry_id` (toy-world convention). Committed Harbor pair: `kb_tw_harbor_dawn_event` + `evt_tw_harbor_dawn` with `extensions.spoke.timeline_entry_id` → `kb_tw_harbor_dawn_event`. Documented ops helpers ([`spoke-operations.md`](spoke-operations.md) §14) resolve `precedes` walks through that link. Integrators that omit dual KEs order moments with explicit `Scope.timeline_event_ids` only.

**Profile-only `entry_type: "beat"`:** Valid open string for integrators publishing this profile. It remains **outside** the core `entry_type` table and schema `description` core lists. Adapters round-trip unknown `entry_type` values verbatim.

---

## TimelineEvent — atomic beat

Moment-scale TimelineEvents carry fine-grained when-axis placement:

| Field | Use for atomic beats |
|-------|----------------------|
| `timeline_scale` | Set to `"moment"` for scene-atom precision |
| `timeline_event_id` | Stable id in beat-sheet order and `Scope.timeline_event_ids` |
| `canonical_name` | Human label for the beat (e.g. “Dawn at Harbor”) |
| `occurred_at` / `sort_key` | Product-chosen ordering hints; `precedes` Relations define explicit beat-sheet order |
| `participant_entry_ids` | Characters, locations, or other KE ids in the beat |
| `source_anchor` | Manuscript span for the beat |
| `computable_logs` | Optional under `l2-computable` — presentation history, not beat identity |

Narrative-scale beats use `timeline_scale: "narrative"` when the integrator places act-level pivots on the same when-axis tier vocabulary (`brief` / `narrative` / `moment`).

---

## BodyAttribute — structural role

Structural positions on a beat sheet (act breaks, midpoint, climax) attach as scalar traits on `body.attributes[]`:

| Property | Guidance |
|----------|----------|
| `trait_type` | Use `"structural_role"` (or a profile-published alias documented in adapter specs) |
| `value` | Open string — integrator vocabulary (e.g. `midpoint`, `catalyst`, `dark_night`) |
| `display_type` | Optional presentation hint for UIs |
| Duplicates | Multiple `trait_type` values allowed in the array per [`spoke-data-model.md`](spoke-data-model.md) §BodyAttribute |

Structural role is **not** a new core field on KnowledgeEntry or TimelineEvent. Complex nested beat-sheet metadata belongs in `extensions.<namespace>`.

**Non-normative examples** (illustrative only — not closed protocol enums):

| Example label | Typical craft source |
|---------------|----------------------|
| `opening_image`, `theme_stated`, `catalyst`, `debate`, `break_into_two`, `b_story`, `fun_and_games`, `midpoint`, `bad_guys_close_in`, `all_is_lost`, `dark_night_of_the_soul`, `break_into_three`, `finale`, `final_image` | Save the Cat beat sheet |
| `exposition`, `rising_action`, `climax`, `falling_action`, `resolution` | Three-act / Freytag |
| `inciting_incident`, `first_plot_point`, `midpoint`, `second_plot_point` | Syd Field screenplay model |

Integrators MAY emit values outside these tables. Adapters MUST round-trip unknown `structural_role` values without normalization.

---

## Relation — open vocabulary for beats

`relation_type` is an open string on `Relation`. Wire endpoints are **`from_id`** and **`to_id`** (KnowledgeEntry id or SourceAnchor id per [`relation.schema.json`](../../schemas/data/relation.schema.json)). This profile documents beat-oriented strings integrators MAY publish in adapter specs. None are added to a closed core `enum`.

### Ordering

| `relation_type` | Semantics | Endpoint rule |
|-----------------|-----------|-----------------|
| `precedes` | Directed order: **from** beat occurs before **to** beat in beat-sheet or scene sequence | `from_id` / `to_id` = dual KE `entry_id` values for the beats being ordered |
| `follows` | Inverse narrative of `precedes` (use one convention per graph; document in adapter) | Same KE endpoint rule |

**`precedes` on TimelineEvents (schema-legal):** emit `precedes` between the **dual KnowledgeEntry** ids, not `timeline_event_id` values. Pair each moment `TimelineEvent` with its KE and set `TimelineEvent.extensions.spoke.timeline_entry_id` to that `entry_id`. Harbor committed fixtures demonstrate the dual-concern link (`kb_tw_harbor_dawn_event` ↔ `evt_tw_harbor_dawn`); integrators MAY extend Harbor with KE-scoped `precedes` Relations for ordered beat-sheet samples.

**`precedes` chain:** Beat-sheet interchange samples form a directed acyclic sequence on KE ids. Cycles → `orderTimelineEventsByPrecedes` rejects with `INVALID_INPUT` (`details.precedes_cycle`). See [`spoke-operations.md`](spoke-operations.md) §14.

### Structural and payoff links

| `relation_type` | Semantics | Endpoints |
|-----------------|-----------|-----------|
| `fulfills` | Later beat pays off setup from earlier beat | KE `from_id` → KE `to_id` |
| `foreshadows` | Earlier beat sets up later beat (aligns with core starter `foreshadows` where applicable) | KE `from_id` → KE `to_id` |
| `parallels` | Thematic or structural mirror between beats | KE ↔ KE |
| `escalates_from` | Later beat raises stakes from earlier beat | KE `from_id` → KE `to_id` |

Core starter vocabulary in [`spoke-data-model.md`](spoke-data-model.md) §Core `relation_type` vocabulary (`related_to`, `parent_of`, `member_of`, `located_in`, `participates_in`, `causes`, `foreshadows`) remains valid. Profile strings extend the open set.

### Relation endpoints (normative)

| Endpoint field | Allowed id kinds | Beat assist use |
|----------------|------------------|-----------------|
| `from_id` | KnowledgeEntry `entry_id` or SourceAnchor `source_id` | Dual KE for earlier beat |
| `to_id` | KnowledgeEntry `entry_id` or SourceAnchor `source_id` | Dual KE for later beat |

**TimelineEvent ids are not valid Relation endpoints.** Order moment events by:

1. **Dual-concern + `precedes`:** KE `from_id` / `to_id` + `extensions.spoke.timeline_entry_id` on each `TimelineEvent`, **or**
2. **Scope-only:** explicit `Scope.timeline_event_ids` array order (no Relations required).

Harbor toy-world fixtures demonstrate the dual-concern link (1): `kb_tw_harbor_dawn_event` paired with `evt_tw_harbor_dawn` via `extensions.spoke.timeline_entry_id`. Integrators MAY extend Harbor with `rel_tw_harbor_*` `precedes` rows between dual KE ids for ordered beat-sheet interchange samples.

---

## Scope — beat-sheet selection

| `Scope` field | Beat assist use |
|---------------|-----------------|
| `timeline_scale` | `"moment"` to select scene-atom beats |
| `timeline_event_ids` | Explicit ordered or unordered beat subset |
| `entry_types` | Include profile `"beat"` or ontology `"event"` when filtering KB nodes |
| `entry_ids` | Pin specific beat KE ids |

`scope_id` remains protocol-neutral and opaque. World or manuscript ids live in `extensions` or adapter mapping — not required `Scope` fields.

---

## Beat sheet as interchange sample

A beat sheet in this profile is an **ordered graph** of moment-scale TimelineEvents with dual KE pairs and KE-scoped `precedes` Relations on existing wire shapes:

```text
kb_tw_harbor_beat_a --precedes--> kb_tw_harbor_beat_b --precedes--> kb_tw_harbor_beat_c
        ^                                      ^                                      ^
        | extensions.spoke.timeline_entry_id   |                                      |
evt_tw_harbor_beat_a                    evt_tw_harbor_beat_b                 evt_tw_harbor_beat_c
```

Integrators MAY extend `fixtures/toy-world/` Harbor with 2–4 ordered moment events, matching dual KEs, and `rel_tw_harbor_*` `precedes` rows on KE ids. Sample ids for extension: `kb_tw_harbor_beat_a` … `kb_tw_harbor_beat_c`, `evt_tw_harbor_beat_a` … `evt_tw_harbor_beat_c`, `rel_tw_harbor_beat_*`. See [`fixtures/toy-world/README.md`](../../fixtures/toy-world/README.md) for the committed dual-concern pair.

---

## Profile `entry_type: "beat"`

| Property | Contract |
|----------|----------|
| Core table | **Not listed** — profile-only per baseline lock |
| Schema | Open `entry_type` string on KnowledgeEntry |
| Typical body | `summary`, `tags`, optional `structural_role` in `attributes[]` |
| Pairing | MAY pair with `TimelineEvent` (`moment`) for dual-concern beat nodes |

Use profile `beat` when the integrator wants a dedicated ontology label distinct from generic `event`. Use `entry_type: "event"` when reusing the core ontology row.

---

## Pure ops helpers (documented contracts)

Normative helper contracts for moment-scale filter and beat-sheet ordering live in [`spoke-operations.md`](spoke-operations.md) §14:

| TypeScript (contract name) | Rust (contract name) |
|----------------------------|----------------------|
| `filterTimelineEventsByMomentScale` | `filter_timeline_events_by_moment_scale` |
| `orderTimelineEventsByIds` | `order_timeline_events_by_ids` |
| `orderTimelineEventsByPrecedes` | `order_timeline_events_by_precedes` |

`@42ch/spoke-operations` and `spoke-operations` gain matching exports when the narrative-sequence ops slice lands.

Contract summary (full rules in §14):

- **Filter:** `timeline_scale === "moment"`; preserve input order.
- **Order by ids:** output follows `orderedIds`; unlisted input events append in input order; unknown ids → `INVALID_INPUT`.
- **Order by precedes:** walk `relation_type: "precedes"` on KE `from_id` / `to_id` via `extensions.spoke.timeline_entry_id` links; acyclic only; stable tie-break by ascending `timeline_event_id`.

Helpers accept **caller-supplied** arrays — no I/O, storage, LLM, or ranking.

---

## Acceptance (profile handbook)

- [x] Beat senses separated; parenthetical beat maps to SourceAnchor / integrator dialogue presentation
- [x] Mapping matrix covers atomic beat, structural role, ordering, Scope
- [x] `precedes` and related Relation strings documented as open vocabulary
- [x] `structural_role` BodyAttribute pattern documented
- [x] Genre / beat-sheet names are non-normative examples only
- [x] Wire uses existing types; `beat` remains profile-only `entry_type`
- [x] Relation endpoints schema-legal (`from_id`/`to_id` KE ids; TimelineEvent link via `extensions.spoke.timeline_entry_id`)

---

## See also

| Doc | Topic |
|-----|-------|
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | Domain Profile principles; L5 `moment` tier |
| [`spoke-data-model.md`](spoke-data-model.md) | TimelineEvent, BodyAttribute, Relation, open vocabulary |
| [`spoke-ops.md`](spoke-ops.md) | Scope refinements |
| [`spoke-operations.md`](spoke-operations.md) | Pure moment filter / order helpers |
| [`CONCEPTS.md`](../../CONCEPTS.md) | Domain Profile, dual-concern, TimelineScale |
| [`fixtures/toy-world/`](../../fixtures/toy-world/) | Harbor dual-concern moment event sample (`kb_tw_harbor_dawn_event` ↔ `evt_tw_harbor_dawn`) |
