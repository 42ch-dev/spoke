# Domain Profile — Mental State

> **Status:** Domain Profile — research candidate  
> **Document class:** Handbook — capability-flagged dialect field tables  
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)  
> **Capability:** `l5-mind` + `narrative-modules`  
> **Bag placement authority:** [`spoke-extension-modules.md`](spoke-extension-modules.md)  
> **Wire SSOT:** `schemas/` — optional `modules` (`ModuleMap`) shipped on KnowledgeEntry (and AssemblePacket); `MindState` as a standalone wire object (`schemas/data/mind-state.schema.json`) on the L5 when-axis under `l5-mind`; `modules.observation` on `TimelineEvent.modules`

## Purpose

This Domain Profile documents how narrative hosts exchange **mental-state data** — what each actor believes, attends to, wants, intends, feels, is disposed toward, regards as normatively binding, and is constrained by — **without sharing an engine**.

False-belief structures ("Bo believes the marble is in the box" while the world fact records the basket) and dramatic-irony structures ("the reader and Ana know the transfer happened; Bo does not") are **interchange facts**, not query-time inferences. An engine that stores these as data turns its hardest reasoning into a lookup; an engine that stores only physical facts defers that reasoning to every consumer. This handbook publishes the **shape of that data** so multiple hosts converge on one mental-state layout.

Three companion dialects carry the data:

| Module key | Home | Role |
|------------|------|------|
| **`modules.mental`** | holder KnowledgeEntry | The actor's / group's nine mental fields — durable, queryable authority |
| **`modules.belief`** | holder KnowledgeEntry | Per-proposition belief records with seven closed-label dimensions |
| **`modules.observation`** | event (`TimelineEvent`) | Who could perceive an event + perceptual-access constraints |

A strictly temporal, derivative record — **`MindState`** on the L5 when-axis — records *how* mental fields change over time; it is never a second authority. Naming, placement, and ownership for `MindState` are locked in [`l5-mind-capability-adr.md`](l5-mind-capability-adr.md); this handbook sketches the record only (see §MindState record sketch).

Engines stay product-local: belief revision, ToM inference, observation rendering, branch value evaluation, and transition simulation are **not** described here. This handbook documents the dialect shapes a host emits and a checker queries.

---

## Placement — triad reminder

| Bag | Role for mental state |
|-----|-----------------------|
| **Core fields** | Identity and body (`entry_id`, `canonical_name`, `body.summary`, `body.attributes[]`, `participant_entry_ids`) — closed protocol objects |
| **`modules.mental` / `modules.belief` / `modules.observation`** | Cross-product **functional** dialects: mental fields, belief labels, observation access |
| **`extensions.<product>`** | One product's private mental model state (interim belief store, UI mood flags, host-only inference cache) |

Category rule (normative triad ADR): shared functional dialects use `modules.*`. Product data uses `extensions.<product>`. Publishing mental-state or observation as a shared key under `extensions.*` is a category error — see [`spoke-extension-modules.md`](spoke-extension-modules.md).

`ModuleMap` is an open bag; inner shapes (the field tables below) are handbook-defined. Capability-flagged hosts emit/parse these namespaces; baseline hosts leave `modules` absent.

---

## Envelope status (read first)

| Namespace | Envelope | Shipped today |
|-----------|----------|---------------|
| `modules.mental` | KnowledgeEntry `modules` | **Yes** — `modules` (`ModuleMap`) is shipped, capability-flagged (`narrative-modules`) |
| `modules.belief` | KnowledgeEntry `modules` | **Yes** — same envelope |
| `modules.observation` | event `modules` (companion to `l5-mind`) | **Inner shape handbook-defined.** A `modules` bag on `TimelineEvent` is the companion wire-slice to the `l5-mind` temporal record; the closed `TimelineEvent` schema carries `participant_entry_ids` (core) and `extensions.<product>` today. Products that carry observation metadata ahead of the envelope emit the documented shape under `extensions.<product>` and migrate to `modules.observation` when the bag lands. |

This profile states the **current dialect shape** for each namespace. Freezing any inner field table into a closed JSON Schema definition is a separate, demand-gated wire decision (the `l5-mind` wire-slice).

---

## `modules.mental` — nine-field mental-state vocabulary

**Status:** Handbook-defined inner object under KnowledgeEntry `modules`. Capability-flagged (`narrative-modules`); opt-in. The nine-field vocabulary is the MWM (arXiv 2607.27201) individual mental-state taxonomy: a fixed, enumerable field set that makes mental state *queryable* ("which actors intend X", "who is bound by norm N") and *comparable* across products.

### Field table

All nine fields are optional; a holder carries the subset the author or engine has populated. The holder KnowledgeEntry's own `entry_id` / `canonical_name` is the identity anchor; `identity` below carries mental-identity attributes (self-concept, role, occupation) beyond the core entry identity.

| Field | Value type | Semantics |
|-------|------------|-----------|
| `identity` | object **or** scalar | Who/what the entity takes itself to be — self-concept, occupation, role-class (e.g. `{ role: "teacher" }`, `"captain"`). Mental identity, not the wire `entry_id`. |
| `beliefs` | array of proposition records **or** reference list | Propositions the entity takes to be true. Authoritative per-proposition records (with seven labels) live in `modules.belief`; this field MAY hold a summary, a count, or `entry_id` references to belief nodes. |
| `attention` | scalar **or** object | Current focus of perception or thought — a target `entry_id`, a free-text focus, or `{ target, modality }`. |
| `goals` | scalar **or** array of objects | Desired end states (e.g. `"catch the train"`, `{ goal: "win", status: "active" }`). |
| `intentions` | scalar **or** array of objects | Planned courses of action toward goals (e.g. `"wait at the platform"`). |
| `emotions` | scalar **or** array of objects | Affective state (e.g. `"anxious"`, `[{ emotion: "relief", intensity: 0.7 }]`). |
| `dispositions` | array of scalars/objects | Preferences, values, personality traits (e.g. `"values fairness"`, `{ trait: "introverted" }`). |
| `norms` | array of strings/objects | Rules and customs the entity regards as binding (e.g. `"greet others politely"`, tournament rules). |
| `constraints` | array of strings/objects | Obligations and prohibitions on behavior (e.g. `"must follow the rules"`, `"cannot push others"`). |

### Scalar vs nested guidance

Each field admits a **scalar** (single dominant value) or a **nested** (structured) value:

| Shape | Where it lives | When to use |
|-------|----------------|-------------|
| **Scalar** value | `modules.mental.<field>` **and/or** `body.attributes[]` | A single dominant value (one dominant emotion, one active goal). A scalar MAY also be expressed as a `BodyAttribute` with `trait_type: "mental.<field>"` (e.g. `mental.emotion`) for hosts that surface mental state as searchable traits. |
| **Nested** object / array | `modules.mental.<field>` (module object) | Structured values: multiple beliefs, a goal with sub-goals and status, emotions with intensity, norms with scope. Nested values live in the module object, never flattened into a single `BodyAttribute`. |

`modules.mental` is the durable, queryable home for the field set. `body.attributes[]` is an allowed **scalar mirror** — never the home for structured mental data. Product-private mental model state stays in `extensions.<product>`.

### Group reuse (collective actors)

A group (team, faction, audience) is a **mental actor** with the **same nine-field structure** as an individual — a team believes, wants, intends, feels, and is constrained. Emit `modules.mental` on the group's holder KnowledgeEntry using the identical field table; do not invent a separate collective shape.

Two non-entity layers from the source taxonomy are **not** per-holder fields:

| Layer | Home | Note |
|-------|------|------|
| **Mental relations** (attitudes, roles) | `Relation` (L4) — `attitude:*`, `role:*`, profile-open types | Traversable graph edges ("who distrusts whom", "who is coach of whom"), not a `modules.mental` field. |
| **Atmosphere** (scene-level social mood) | scene / `TimelineEvent` metadata | Scene-scoped value (tense, cooperative, festive, awkward), not a per-entity field. |

### Illustrative shape (handbook-defined inner dialect)

```text
// modules.mental — individual actor (nine-field subset)
{
  "identity": { "role": "harbor_master" },
  "beliefs": { "ref": "kb_hm_beliefs", "count": 12 },
  "attention": { "target": "kb_tw_dawn_dock", "modality": "visual" },
  "goals": [{ "goal": "clear the dawn berths", "status": "active" }],
  "emotions": [{ "emotion": "alert", "intensity": 0.6 }],
  "norms": ["greet arriving captains"],
  "constraints": ["cannot waive dockside law"]
}
```

```text
// modules.mental — group actor (same nine-field structure)
{
  "identity": { "role": "crew_team_a" },
  "goals": [{ "goal": "win the race", "status": "active" }],
  "emotions": [{ "emotion": "determined", "intensity": 0.8 }],
  "norms": ["stay in lane", "follow race rules"],
  "constraints": ["cannot cross the start early"]
}
```

---

## `modules.belief` — belief proposition record

**Status:** Handbook-defined inner array under KnowledgeEntry `modules`. Capability-flagged (`narrative-modules`); opt-in. One proposition record serves **both** narrated world facts and actor beliefs — the `holder` field decides the layer (`world` = narrated fact; an `entry_id` = an actor's mental content). Higher-order beliefs are flat propositions with an `order` label, not nested objects.

**Hosting (normative):** `modules.belief` is an array on a KnowledgeEntry `modules` bag. Rows with `holder: "world"` live on a **designated world-state KnowledgeEntry** — a product's chosen entry for narrated world facts. Rows with `holder: <actor_entry_id>` live on **that actor's** KnowledgeEntry. The `holder` field is the semantic discriminator: the same array shape carries world facts and actor beliefs; placement on the correct KE is what makes them queryable per actor. A product MAY co-locate both row types on a single world-state KE or distribute actor rows to their own entries — `holder` always disambiguates.

### Field table (per array element)

| Field | Required | Type | Semantics |
|-------|----------|------|-----------|
| `holder` | **yes** | `string` | Belief holder: an `entry_id`, a group id, **or the special `world`** marking a narrated fact not attributed to any actor's internal state. `world` is the fact/belief split. |
| `proposition` | **yes** | `string` | Minimal content being represented — what the holder takes to be true. Free text; semantics live in the labels. |
| `order` | **yes** | integer `0`–`3` | Recursive belief depth (dimension 1). `0` = world-level narrated fact (`holder: world`); `1` = first-order belief about the world; `2` = belief about another's belief; `3` = deeper nesting. **Depth cap 3.** An order-4+ record is a smell. |
| `truth` | no | enum | Truth Status (dimension 2) — see §Closed label spaces. |
| `access` | no | enum | Knowledge Access (dimension 3). |
| `representation` | no | enum | Representation (dimension 4). |
| `content_type` | no | enum | Content Type (dimension 5). |
| `source` | no | enum | Mental Source (dimension 6). |
| `context` | no | enum | Context (dimension 7). |

### Closed label spaces (exact)

The seven dimensions carry **closed** label spaces. Adapters MUST round-trip the record object verbatim; **emitters use only the labels listed** for each dimension. An unknown label is an emitter error, not an extension point.

| Dimension | Field | Closed label space |
|-----------|-------|--------------------|
| 1 — Order | `order` | `0`, `1`, `2`, `3` (integer; depth cap 3) |
| 2 — Truth Status | `truth` | `True`, `False`, `Unknown` |
| 3 — Knowledge Access | `access` | `Private`, `Shared`, `Public` |
| 4 — Representation | `representation` | `Explicit`, `Implicit` |
| 5 — Content Type | `content_type` | `Location`, `Contents/Physical State`, `Identity/Relation`, `Epistemic`, `Desire/Intention`, `Emotion`, `Trait/Value`, `Action/Event` |
| 6 — Mental Source | `source` | `Narration`, `Perception`, `Memory`, `Testimony`, `Inference`, `Imagination`, `Unknown` |
| 7 — Context | `context` | `Deceptive`, `Temporal`, `Counterfactual`, `Neutral` |

| Dimension | Captures |
|-----------|----------|
| Order | Recursive depth of attribution (world fact → belief → meta-belief) |
| Truth Status | Belief attribution **vs** factual correctness — central to false belief and appearance–reality. `Unknown` covers the engine not knowing. |
| Knowledge Access | How information is distributed across actors — ignorance, asymmetry, deception are Access patterns, not content |
| Representation | Directly stated (`Explicit`) vs pragmatically inferred (`Implicit`) beliefs |
| Content Type | What is believed — localizes errors by semantic type |
| Mental Source | How the belief was acquired — the update rule's key: an event updates a belief only via `Perception` (observed) or `Testimony` (told); `Inference` / `Memory` / `Imagination` are derived paths |
| Context | Story framing that modulates interpretation and belief updating |

### False belief is one labeled row

False belief needs **no special mechanism**. It is a `world` fact (`truth: True`) plus a diverged actor belief (`truth: False`) about the same proposition. The two records share one shape and one proposition format; the `holder` and `truth` labels keep them distinguishable:

```text
// world fact — True
{ "holder": "world", "proposition": "The marble is in the basket",
  "order": 0, "truth": "True",  "access": "Public", "representation": "Explicit",
  "content_type": "Location", "source": "Narration", "context": "Temporal" }

// actor belief — False (stale; not updated by the hidden transfer)
{ "holder": "kb_bo", "proposition": "The marble is in the box",
  "order": 1, "truth": "False", "access": "Private", "representation": "Implicit",
  "content_type": "Location", "source": "Perception", "context": "Temporal" }
```

A `truth: False` actor belief against a `truth: True` world fact is a **deliberate false-belief structure** until a checker finds no supporting update — then it is a consistency bug (see §False-belief consistency check pattern).

### Array wrapper (illustrative)

The records above are **elements of the `modules.belief` array** on the holder KnowledgeEntry. Integrators must not emit bare objects — they live inside `modules`:

```text
// KnowledgeEntry.modules — belief array (world fact + actor belief)
{
  "modules": {
    "belief": [
      { "holder": "world", "proposition": "The marble is in the basket",
        "order": 0, "truth": "True", "access": "Public", "representation": "Explicit",
        "content_type": "Location", "source": "Narration", "context": "Temporal" },
      { "holder": "kb_bo", "proposition": "The marble is in the box",
        "order": 1, "truth": "False", "access": "Private", "representation": "Implicit",
        "content_type": "Location", "source": "Perception", "context": "Temporal" }
    ]
  }
}
```

### Higher-order beliefs are flat

Order-2 / order-3 beliefs are ordinary propositions whose content references another actor's mental state, labeled by `order`. Do not build nested belief objects.

```text
// order-2: Bo's (false) belief about Ana's belief
{ "holder": "kb_bo", "proposition": "Ana thinks the marble is in the box",
  "order": 2, "truth": "False", "access": "Private", "representation": "Implicit",
  "content_type": "Epistemic", "source": "Inference", "context": "Neutral" }
```

---

## `modules.observation` — event observation metadata

**Status:** Handbook-defined inner object under an event's `modules` bag (companion wire-slice to `l5-mind`). Opt-in. False belief is an **absence mechanic**: an event unobserved by an actor leaves their belief stale. Observation metadata records *who could perceive* an event and the perceptual constraints on that access — the input every "who knows what" derivation needs.

### Relationship to the existing core field

`TimelineEvent.participant_entry_ids` (core field) lists who **participates** in (acts in) an event. `modules.observation.observers` lists who could **perceive** it. The two overlap but differ: an observer MAY be a non-participant bystander; a participant is typically also an observer. The core field stays the participation list; the module carries the perceptual-access dimension the core field does not.

### Field table

| Field | Required | Type | Semantics |
|-------|----------|------|-----------|
| `observers` | **yes** | `entry_id[]` | Actors who could perceive the event (participants who were present **and** non-participant bystanders in perceptual range). An actor absent from this list did not observe the event. |
| `access` | no | object | Perceptual constraints (MWM κ) qualifying how each observer could perceive. Open object; documented keys below. |

**Absence semantics (normative):** An **absent** `modules.observation` on an event means **no observation metadata recorded** — not "no observers". An **empty** `observers: []` means **explicitly no observers** — the event was not perceivable by any actor (or is explicitly private). Consumers and checkers MUST distinguish the two: absent metadata is silent; an empty observer list is an explicit claim.

### `access` keys (documented; open object)

| Key | Type | Semantics |
|-----|------|-----------|
| `position` | `string` / object | Observer position relative to the event (e.g. `"in-room"`, `{ room: "kitchen" }`). |
| `line_of_sight` | `boolean` / object | Whether the event was visually visible to the observer; an object MAY scope by sub-action. |
| `hearing_range` | `boolean` / object | Whether the event was audible. |
| `modality` | `string[]` | Which perceptual modalities were available (e.g. `["visual", "auditory"]`). Absence of a modality narrows what could be perceived. |

`access` is an open object; unknown keys round-trip. Modality availability is the basis for partial observation (an actor hears but does not see).

### Knowledge Access derivation

Observation metadata is the bridge from *what happened* to *who knows*. The derivation chain is normative:

```text
unobserved event  ⇒  actor's belief not updated by that event  ⇒  stale belief  ⇒  false belief
                                                                                   (vs current world fact)
```

An actor absent from a world-changing event's `observers` retains the pre-event belief. If the world fact moved on, that belief is now `truth: False`. The checker compares derived beliefs (from events + observation access) against stored `modules.belief` records and emits Findings on drift (see §False-belief consistency check pattern).

### Illustrative shape

```text
// TimelineEvent: "Ana moves the marble from the box to the basket" (hidden transfer)
{
  "timeline_event_id": "evt_transfer",
  "canonical_name": "Marble moved box → basket",
  "timeline_scale": "moment",
  "participant_entry_ids": ["kb_ana", "kb_marble"],
  "modules": {
    "observation": {
      "observers": ["kb_ana"],
      "access": {
        "line_of_sight": true,
        "hearing_range": true,
        "modality": ["visual", "auditory"]
      }
    }
  },
  "extensions": {}
}
```

Bo (`kb_bo`) is absent from `observers` — he left the room — so the transfer does not update his belief; his `truth: False` belief "the marble is in the box" is the correct stale-belief structure, not a bug.

---

## MindState record sketch

**Status:** Non-normative pointer. Naming, placement, and the authority/derivative ownership boundary for `MindState` are locked in [`l5-mind-capability-adr.md`](l5-mind-capability-adr.md); the closed JSON Schema definitions (`schemas/`) are the `l5-mind` wire-slice and are **not** in this handbook.

`MindState` is a **strictly temporal, derivative** record on the L5 when-axis — it records *how* mental fields changed across the timeline, exactly as `ComputableLogEntry` records how computable fields changed. The **authority** is `modules.mental` / `modules.belief` on the holder KnowledgeEntry; `MindState` is never a second authority.

| Field | Type | Role |
|-------|------|------|
| `mind_state_id` | `string` | Stable id |
| `holder_entry_id` | `string` | The actor / group KnowledgeEntry this change applies to |
| `occurred_at` | `Timestamp` | When-axis placement of the change |
| `snapshot` | `MentalFieldMap` | Optional full-field snapshot of `modules.mental` at this point |
| `deltas` | `MindDelta[]` | Change-units pointing at paths within `modules.mental` / `modules.belief` |
| `extensions` | `ExtensionMap` | Product-private temporal metadata |

**`MentalFieldMap`** is an open object matching the `modules.mental` nine-field vocabulary (see §`modules.mental` field table). The closed JSON Schema typedef is deferred to the `l5-mind` wire-slice (`schemas/data/mind-state.schema.json`); this handbook defines only the field-level vocabulary it inherits.

**`MindDelta`** mirrors `ComputableLogChange` (`{ path, previous?, next? }`):

| Field | Type | Role |
|-------|------|------|
| `path` | `string` (required) | Dot-path / JSON Pointer to the changed field within `modules.mental` or `modules.belief` |
| `previous` | `OpaqueJson` (optional) | Value before the change |
| `next` | `OpaqueJson` (optional) | Value after the change |

Single authority per fact: no mental fact has two homes. `MindState` records the change; the holder `modules.*` records the current value. See the ADR for the full ownership boundary and rejected alternatives (mind-entity Entity class, `l9-mind`, `mind-axis`).

---

## False-belief consistency check pattern

**Status:** Checker **Finding** patterns — checker output, **never** entry bodies, and **not** a shipped engine. These document what a product-local checker emits against the dialect shapes above. A checker compares derived beliefs (from events + `modules.observation`) against stored `modules.belief` records and emits [`Finding`](../../schemas/data/finding.schema.json) objects (`kind`, `target_entry_id`, `severity`, `description`, optional `suggested_fix`).

### 1. Stale-belief drift

| Aspect | Detail |
|--------|--------|
| `kind` | `stale_belief_drift` |
| Detects | An actor belief (`truth: False` vs the current world fact) whose staleness is **unexplained by observation records** — either (a) the actor is listed in the informing event's `modules.observation.observers` (should have observed the change, yet the belief is stale), or (b) no world-changing event exists for the proposition (the stale belief has no informational basis). A deliberate false belief — the actor is **absent** from the world-changing event's `observers` — is the correct stale-belief structure and routes to `dramatic_irony_asymmetry`, **not** this Finding. |
| Reading | A consistency bug: the observation records contradict the stale belief. The deliberate false-belief case (actor absent from `observers`) is structurally correct — `dramatic_irony_asymmetry` reports it, not this checker. |
| Inputs | `modules.belief` (actor row `truth: False` + matching `world` row `truth: True`); `modules.observation` on the world-changing event; the event timeline. |

### 2. Dramatic irony (Access / observation asymmetry)

| Aspect | Detail |
|--------|--------|
| `kind` | `dramatic_irony_asymmetry` |
| Detects | The `world` (or other observers) hold a fact an actor did **not** observe — a world fact `truth: True` whose world-changing event's `modules.observation.observers` excludes that actor, while the actor retains a divergent belief. |
| Reading | The structural basis for dramatic irony: the reader/audience and some actors know what another actor does not. Pure data — no irony detection engine shipped. |
| Inputs | `modules.belief` (world row + actor row); `modules.observation` (observer set); `access` (modality gaps for partial-observation irony). |

### 3. Access violation

| Aspect | Detail |
|--------|--------|
| `kind` | `access_violation` |
| Detects | A belief whose `access` label contradicts its acquisition path — e.g. labeled `Public` (or `Shared`) but the informing event's `modules.observation.observers` excludes the holder, or the `source` is `Perception` of an event the holder could not perceive. |
| Reading | A labeling error: the Access dimension says "everyone/many know", but the observation record says the holder could not have learned it this way. |

### 4. Action-content mismatch

| Aspect | Detail |
|--------|--------|
| `kind` | `action_content_mismatch` |
| Detects | An event whose physical carrier implies a content channel (e.g. speech, gesture) but the corresponding mental content is missing or contradicts the carrier — the carrier/content decomposition (MWM `a = (a^phy, a^ment)`) is inconsistent. |
| Reading | The observable carrier and the intended content are two dimensions of one action; a record storing one without the other, or in conflict, loses intent (deception, irony, politeness) or consequence. |

`severity` is an open string (core vocabulary: `info`, `warning`, `error`); `status` is open (`open`, `resolved`, `dismissed`). Findings feed author review, engine decisions, and cross-host telemetry — they do not alter entry bodies.

---

## Worked example — false-belief box/basket story

Adapted from a classic false-belief structure (OmniToM (arXiv 2605.26322) Fig 2 lineage); protocol-neutral. The same story is the dramatic-irony exemplar: the world and Ana hold the transfer; Bo does not.

**Story:** *Ana and Bo are in a room with a marble. The marble starts in the box. Bo leaves the room. Ana moves the marble from the box to the basket. Bo returns.*

### `modules.belief` rows (state after the hidden transfer)

| `holder` | `proposition` | `order` | `truth` | `access` | `representation` | `content_type` | `source` | `context` |
|----------|---------------|---------|---------|----------|------------------|----------------|----------|-----------|
| `world` | The marble is in the basket | 0 | `True` | `Public` | `Explicit` | `Location` | `Narration` | `Temporal` |
| `world` | Bo left the room | 0 | `True` | `Public` | `Explicit` | `Action/Event` | `Narration` | `Temporal` |
| `kb_ana` | The marble is in the basket | 1 | `True` | `Private` | `Implicit` | `Location` | `Perception` | `Temporal` |
| `kb_ana` | Bo is not in the room | 1 | `True` | `Private` | `Implicit` | `Location` | `Perception` | `Neutral` |
| `kb_bo` | The marble is in the box | 1 | `False` | `Private` | `Implicit` | `Location` | `Perception` | `Temporal` |
| `kb_bo` | Ana thinks the marble is in the box | 2 | `False` | `Private` | `Implicit` | `Epistemic` | `Inference` | `Neutral` |

What this encodes as **pure data**:

- Bo's belief (`kb_bo`, order 1, `truth: False`) vs the world fact (`world`, order 0, `truth: True`) — false belief is one labeled row, not a special mechanism.
- Bo's order-2 belief about Ana is also `False` — nested attribution with its own truth value, stored as a flat proposition.
- Every actor belief is `Private` / `Implicit`: the story never states what anyone thinks, and the actors never tell each other. Access + Representation are derivable from events.

### `modules.observation` on the hidden-transfer `TimelineEvent`

```text
{
  "timeline_event_id": "evt_transfer",
  "canonical_name": "Ana moves the marble from box to basket",
  "timeline_scale": "moment",
  "participant_entry_ids": ["kb_ana", "kb_marble"],
  "modules": {
    "observation": {
      "observers": ["kb_ana"],
      "access": { "line_of_sight": true, "hearing_range": true, "modality": ["visual", "auditory"] }
    }
  },
  "extensions": {}
}
```

Bo (`kb_bo`) is absent from `observers` (he left the room). By the Knowledge Access derivation:

```text
evt_transfer unobserved by Bo  ⇒  Bo's belief not updated  ⇒  "marble in box" stays  ⇒  False vs world "marble in basket"
```

The checker reading for this story: Bo's `truth: False` belief is a **correct** stale-belief structure (a deliberate false belief / dramatic irony), not a `stale_belief_drift` bug — there is a world-changing event, and Bo correctly did not observe it. A checker would instead flag `dramatic_irony_asymmetry` (world + Ana know; Bo does not).

---

## Boundaries — engines are product-local

| Concern | Owner |
|---------|-------|
| Belief revision / update logic | **Product-local** engine |
| ToM inference (rendering each actor's belief from observation) | **Product-local** |
| Observation rendering (`o^ϵ = Ω^ϵ(s)`) | **Product-local** |
| Branch value evaluation / candidate enumeration | **Product-local** |
| Transition simulation (physical + mental) | **Product-local** |
| Round-trip field names, label spaces, pack import mapping | `modules.mental` / `modules.belief` / `modules.observation` (this handbook) |
| Pure library helpers | `@42ch/spoke-operations` / `spoke-operations` — **no** inference, revision, ranking, or rendering code |
| Baseline KnowledgeEntry / TimelineEvent | Optional `modules` is **never required**; absent/empty valid |

This handbook describes **no engines** — no transition kernels, no inference distributions, no ranking or scoring, no belief-update rules. It documents the data shapes a host emits, a checker queries, and a product reads. Checker Findings are output contracts; the checker itself is product-local.

---

## Integrator checklist

An integrator can implement mental-state storage + interchange from this handbook alone when:

1. Actor / group mental state lives under `modules.mental` (nine-field table) on the holder KnowledgeEntry.
2. Per-proposition beliefs live under `modules.belief` with the exact seven closed-label dimensions; `world` holder splits narrated facts from actor beliefs; `order` caps at 3.
3. Event observation metadata lives under `modules.observation` (`observers` + optional `access`) on the event; `participant_entry_ids` stays the participation list.
4. Mental-state changes over the when-axis use the `MindState` derivative record (sketch here; full definition in the ADR + wire-slice), never a second authority.
5. False belief and dramatic irony are **data** (labeled rows + observation asymmetry), checked by product-local Findings — not query-time inference.
6. Engines (revision, ToM, rendering, ranking) stay in the product; `spoke-operations` gains no matchers or scorers.
7. Optional `modules` is capability-flagged (`narrative-modules`); `MindState` is `l5-mind`; inner shapes stay handbook-defined; product-private mental model state stays in `extensions.<product>`.

---

## Acceptance (profile handbook)

- [x] `modules.mental` nine-field vocabulary documented (identity / beliefs / attention / goals / intentions / emotions / dispositions / norms / constraints); field semantics, scalar-vs-nested guidance, group reuse
- [x] `modules.belief` proposition record documented (`holder` / `proposition` / `order` + seven dimensions); closed label spaces exact; `world` holder + false belief as one labeled row; higher-order beliefs flat
- [x] `modules.observation` documented (`observers` + optional `access`); relationship to `participant_entry_ids`; Knowledge Access derivation chain
- [x] MindState record sketch + pointer to ADR; strictly temporal/derivative; `MindDelta` mirrors `ComputableLogChange`
- [x] False-belief consistency check pattern (stale-belief drift, dramatic irony asymmetry, access violation, action-content mismatch) as checker Findings
- [x] Worked example maps the box/basket false-belief story to `modules.belief` rows + one `modules.observation` on the hidden-transfer event (protocol-neutral; dramatic-irony exemplar)
- [x] Boundaries: no engines described (no transition/inference/rendering/ranking code); engines product-local
- [x] Envelope status stated per namespace (shipped KnowledgeEntry `modules`; event `modules` companion to `l5-mind` wire-slice)
- [x] Triad ADR cited; inner shapes handbook-defined; no iteration ids; current dialect shape only

---

## See also

| Doc | Topic |
|-----|-------|
| [`l5-mind-capability-adr.md`](l5-mind-capability-adr.md) | `l5-mind` flag; `MindState` naming, placement, ownership boundary; rejected alternatives |
| [`spoke-extension-modules.md`](spoke-extension-modules.md) | Core / `modules.*` / `extensions.<product>` triad; capability-flagged envelope; placement authority |
| [`domain-profile-lore-activation.md`](domain-profile-lore-activation.md) | Sister Domain Profile — `modules.activation` (handbook style precedent) |
| [`assemble-module-recipes.md`](assemble-module-recipes.md) | Sister Domain Profile — `modules.placement` + `modules.activation_trace` (recipe style precedent) |
| [`domain-profile-narrative-structure.md`](domain-profile-narrative-structure.md) | Sister Domain Profile — Beat / structural mapping |
| [`spoke-protocol.md`](spoke-protocol.md) | Umbrella protocol; §Extensions |
| [`spoke-data-model.md`](spoke-data-model.md) | KnowledgeEntry, TimelineEvent, BodyAttribute, ModuleMap, `ComputableLogChange` |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | L0–L8 layer model; optional flags (`l5-fork`, `narrative-modules`); L5 Temporal — `l5-mind` to be registered in spec-sync (companion wire-slice) |
| [`CONCEPTS.md`](../../CONCEPTS.md) | Domain Profile; Modules (capability-flagged); TimelineEvent dual-concern |
