# l5-mind Capability — Mental State as First-Class Temporal Concern

> **Status:** Normative ADR  
> **Document class:** Normative — capability naming and ownership boundary  
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)  
> **Wire SSOT:** `schemas/`

## Purpose

Narrative engines need to interchange "who believes, wants, and feels what at time *t*" — not as an endpoint answer, but as **data** on the wire. False-belief structures ("Bob believes the object is in the box" while the world fact is True elsewhere) and dramatic-irony structures ("the audience knows what the character does not") are **interchange facts**, not query-time inferences. An engine that stores these as data turns its hardest reasoning into a lookup; an engine that stores only facts defers that reasoning to every consumer.

Endpoint QA can hide whether a model constructs the underlying mental-state representation at all — a model can answer a false-belief question correctly while failing to track what each actor actually believes. The two dimensions that make this hardest for current models (Knowledge Access — who could know/share — and Representation — stated vs inferred) are exactly the dimensions that pure endpoint systems never represent. Putting them on the wire as data is the structural fix.

This ADR locks the **naming, placement, and ownership boundary** for mental-state data on the SPOKE wire: the optional capability flag that carries it, the wire object that records mental change over the when-axis, and the single-authority rule that prevents dual sources of truth.

## Decision

### Capability flag: `l5-mind`

An optional capability flag **`l5-mind`** on the L5 Temporal layer, following the `l<N>-<concern>` convention established by `l5-fork` and `l2-computable`. Declaring `l5-mind` means a product implements the optional **`MindState`** temporal record on `TimelineEvent`. The flag is opt-in; `spoke-baseline` hosts need not emit or parse it.

The flag declares a layer-prefixed capability, not a new protocol layer. The layer model remains L0–L8 ([`spoke-protocol-layers.md`](spoke-protocol-layers.md)). Mental content is cross-cutting — L1 ontology (`entry_type: "belief"` / `"mental_state"`), L2 body (`modules.mental` / `modules.belief`), L4 relations (attitude / role edges), L5 temporal (`MindState`) — and `l5-mind` declares only the L5 temporal artifact.

### Wire object: `MindState`

**`MindState`** is a first-class temporal wire object on the L5 when-axis — a strictly derivative snapshot or change record for mental fields, carried on `TimelineEvent` under `l5-mind`. It is **distinct from** KnowledgeEntry `entry_type: "belief"` (ontology label on a KB entry body), following the same dual-concern separation as TimelineEvent vs `entry_type: "event"` ([`CONCEPTS.md`](../../CONCEPTS.md) §Dual-concern).

`MindState` records **how mental fields changed** across the timeline — exactly as `ComputableLogEntry` records how computable fields changed. It is a **derived change record, never a second authority** for mental state.

## Ownership boundary (normative)

A single authority holds each mental fact. The holder KnowledgeEntry is the durable, queryable authority; `MindState` is the temporal derivative.

| Concern | Settled home | Role |
|---------|-------------|------|
| Nine mental fields (identity, beliefs, attention, goals, intentions, emotions, dispositions, norms, constraints) | holder KnowledgeEntry **`modules.mental`** | **Authority** — durable, queryable mental state of the actor / group |
| Seven belief dimensions (Order, Truth, Access, Representation, Content Type, Mental Source, Context) | holder KnowledgeEntry **`modules.belief`** | **Authority** — per-proposition label space |
| `MindState` (mental snapshot / delta) | **L5 TimelineEvent** (temporal) | **Derivative** — strictly temporal change record; never a second authority |

**Single authority per fact.** The nine fields and seven labels live authoritatively on the holder KnowledgeEntry via the `modules` bag — the place a checker queries and a product reads. `MindState` is strictly temporal and derivative: it records field-level changes across the when-axis, pointing at paths within `modules.mental` / `modules.belief`. No fact has two homes.

### Temporal change record — `MindDelta`

Each `MindState` entry carries **`MindDelta`** change-units mirroring the `ComputableLogChange` pattern:

| Field | Type | Role |
|-------|------|------|
| `path` | `string` (required) | Dot-path or JSON Pointer to the changed field within `modules.mental` or `modules.belief` |
| `previous` | `OpaqueJson` (optional) | Value before the change |
| `next` | `OpaqueJson` (optional) | Value after the change |

This is the exact shape of `ComputableLogChange` (`{ path, previous?, next? }`) in [`schemas/common/common.schema.json#/definitions/ComputableLogChange`](../../schemas/common/common.schema.json), which records field-level changes within `body.computable`. The authority/derivative split is identical: `ComputableLogEntry` is a derived change record on `TimelineEvent.computable_logs[]`; the authority is `body.computable` / `body.state` on the KnowledgeEntry. `MindState` follows the same pattern for mental fields.

### Capability placement

| Surface | Capability flag | Status |
|---------|----------------|--------|
| `modules.mental` / `modules.belief` on KnowledgeEntry | **`narrative-modules`** (existing) | The standard home for `ModuleMap` namespaces; adapters round-trip unknown keys verbatim |
| `MindState` on TimelineEvent | **`l5-mind`** (new) | Optional L5 temporal record; opt-in, not `spoke-baseline` |

## Rejected alternatives

### Mind-entity Entity class

A new first-class Entity / KnowledgeEntry superset carrying the nine fields and seven labels as its own durable object, with a separate identity and lifecycle from the actor it describes, is **not the SPOKE shape**. Three independent grounds:

| Ground | Detail |
|--------|--------|
| Papers model mind as fields on existing actors | MWM's coupled state decomposes as `s^ment_t = ({m^i_t}, {m^G_t}, R^ment_t, α_t)` — individual mental states `m^i_t` are **components of agents**, not free-standing objects. The formal nine-tuple belongs to agent *i*; groups reuse the same structure ([01-mental-world-modeling.md §1](../references/mental-world-tom/01-mental-world-modeling.md)). |
| Identity split / dual SSOT | A mind-entity must reference the actor it describes, duplicating identity already on the actor KnowledgeEntry. Every query ("who believes X") joins two objects — the dual-SSOT anti-pattern. |
| Closed-core growth-path violation | SPOKE grows via closed-core + open vocabulary + capability-flagged `modules`. A new Entity class means a new `*.schema.json`, an `assert-schema-count` bump, and a full codegen regen — the heaviest change for a need already served by `modules.mental` / `modules.belief` under `narrative-modules`. |

### `l9-mind`

There is no L9 — the layer model is L0–L8 ([`spoke-protocol-layers.md`](spoke-protocol-layers.md)). Mental content is inherently cross-cutting across L1 ontology, L2 body, L4 relations, and L5 temporal; forcing it into a single new layer creates cross-layer coupling, not clean separation. A capability flag on the layer whose wire artifact carries the temporal record (`l5-mind`) is the correct expression.

### `mind-axis`

Capability flags follow `l<N>-<concern>` (or `spoke-connect` for cross-process families). `mind-axis` declares neither which layer's wire artifacts carry the data nor what baseline excludes. It breaks the naming convention without adding clarity.

### Vocabulary-only (`entry_type` label only)

An `entry_type: "belief"` or `"mental_state"` label is an L1 ontology classifier — it says *this node is a belief*, not *where the nine fields live* or *how beliefs change over time*. The label is a necessary node classifier (L1 vocabulary), but it carries no structural home for the field set or the temporal change record. Vocabulary-only is the floor, not the carrier.

## Evidence chain

| Claim | Source |
|-------|--------|
| Mind is fields on existing actors, not a separate entity; the nine-tuple belongs to agent *i* | MWM §1: `s^ment_t = ({m^i_t}, {m^G_t}, R^ment_t, α_t)`; formal nine-tuple `m^i_t = (id, b, q, g, ι, e, d, n, c)`; groups reuse individual structure ([01-mental-world-modeling.md §1](../references/mental-world-tom/01-mental-world-modeling.md)) |
| "The world model is not the agent" — mental state belongs to the agent, not a free object | MWM §2: two computational roles — target agent vs world model ([01-mental-world-modeling.md §2](../references/mental-world-tom/01-mental-world-modeling.md)) |
| Observation is rendered per-perspective from one global state — derivative, not authoritative | MWM §3: `o^ϵ_t = Ω^ϵ(s_t)`; κ perceptual access + ρ social-cognitive perspective ([01-mental-world-modeling.md §3](../references/mental-world-tom/01-mental-world-modeling.md)) |
| Beliefs anchored to an existing actor; no belief entity | OmniToM §1: record `(actor, proposition, order)`; `world` special actor splits fact / belief ([02-omnitom-belief-structure.md §1](../references/mental-world-tom/02-omnitom-belief-structure.md)) |
| Seven-dimension label space is a structural home for `modules.belief` | OmniToM §2: vector `s_i = (o, t, k, r, c, m, x)` — Order, Truth, Access, Representation, Content Type, Mental Source, Context ([02-omnitom-belief-structure.md §2](../references/mental-world-tom/02-omnitom-belief-structure.md)) |
| False belief is one labeled row (False actor belief + True world fact); beliefs Private / Implicit, derived from events — not a special mechanism | OmniToM §3 / Fig 2: Bob #19 False vs world #3 True ([02-omnitom-belief-structure.md §3](../references/mental-world-tom/02-omnitom-belief-structure.md)) |
| Endpoint QA hides whether a model constructs the mental-state representation — belief structures are data, not answers | OmniToM §5: the bottleneck is Knowledge Access and Representation, the two dimensions pure endpoint systems never represent ([02-omnitom-belief-structure.md §5](../references/mental-world-tom/02-omnitom-belief-structure.md)) |
| One proposition record for facts and beliefs (shares the holder, not a separate entity) | OmniToM §6 takeaway 1 ([02-omnitom-belief-structure.md §6](../references/mental-world-tom/02-omnitom-belief-structure.md)) |
| Derived change-record precedent (no dual SSOT) | SPOKE `ComputableLogChange` = `{ path, previous?, next? }` in `ComputableLogEntry.changes[]` on `TimelineEvent.computable_logs[]`; authority is `body.computable` ([`schemas/common/common.schema.json`](../../schemas/common/common.schema.json)) |
| Capability-flag growth path (`modules` bag, not a new Entity class) | `narrative-modules` flag; `ModuleMap` on KnowledgeEntry ([`spoke-extension-modules.md`](spoke-extension-modules.md)) |
| Layer model is L0–L8; capability flags follow `l<N>-<concern>` | [`spoke-protocol-layers.md`](spoke-protocol-layers.md) |

## Scope of authority

| This ADR owns | This ADR does not |
|---------------|-------------------|
| Normative **naming** of the `l5-mind` capability flag | Hard-coding inner dialect field tables into `schemas/**/*.json` |
| Normative **placement** of `MindState` on L5 TimelineEvent | Inner field tables for `modules.mental` (nine fields) and `modules.belief` (seven labels) — handbook-defined |
| Normative **ownership boundary**: authority in `modules.mental` / `modules.belief`, derivative in `MindState` | Exact schema `description` text for `MindState` / `MindDelta` — wire schema work |
| `MindDelta` shape mirrors `ComputableLogChange` (`{ path, previous?, next? }`) | JSON Schema definitions for `MindState` / `MindDelta` (`schemas/`) — wire schema work |
| Rejected alternatives (mind-entity, `l9-mind`, `mind-axis`, vocabulary-only) | Engine implementations, belief-revision logic, observation rendering — product-local |
| `MindState` vs KnowledgeEntry `entry_type: "belief"` dual-concern separation | Relation vocabulary for attitudes / roles (L4) — Domain Profile |

## Placement quick reference

| Need | Place it |
|------|----------|
| Actor / group mental state (nine fields) | holder KnowledgeEntry `modules.mental` (inner shape handbook-defined; under `narrative-modules`) |
| Per-proposition belief labels (seven dimensions) | holder KnowledgeEntry `modules.belief` (inner shape handbook-defined; under `narrative-modules`) |
| Temporal change record for mental fields on the when-axis | `MindState` on `TimelineEvent` (under `l5-mind`) |
| Belief as a KB node (ontology label) | KnowledgeEntry `entry_type: "belief"` (L1 vocabulary; necessary but not a shape decision) |
| Mental attitudes / role relations (likes, distrusts, teacher-of) | `Relation` with Domain Profile types (L4) |
| Product-private mental model data | `extensions.<your-product>` |

## See also

| Doc | Topic |
|-----|-------|
| [`CONCEPTS.md`](../../CONCEPTS.md) | TimelineEvent dual-concern; Modules (capability-flagged) |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | L0–L8 layer model; optional flags (`l5-fork`, `l2-computable`, `narrative-modules`); L5 Temporal |
| [`spoke-extension-modules.md`](spoke-extension-modules.md) | Core / modules / extensions triad; `modules.*` placement authority |
| [`spoke-data-model.md`](spoke-data-model.md) | TimelineEvent; ComputableLogEntry / ComputableLogChange; ModuleMap |
| [`schemas/common/common.schema.json`](../../schemas/common/common.schema.json) | `ComputableLogChange`, `ComputableLogEntry`, `ModuleMap`, `OpaqueJson` definitions |
| [01-mental-world-modeling.md](../references/mental-world-tom/01-mental-world-modeling.md) | MWM digest — coupled physical-mental state; nine-field tuple; observation rendering |
| [02-omnitom-belief-structure.md](../references/mental-world-tom/02-omnitom-belief-structure.md) | OmniToM digest — belief propositions; seven-dimension schema; false-belief data |
