# SPOKE Data Model

> **Status:** Normative (v0.1 baseline; protocol layers deepen; KnowledgeEntry/TimelineEvent terminology, 2026-07-23)  
> **Document class:** Detail — data layer  
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)  
> **Schema home:** `schemas/data/`, `schemas/common/`

## Purpose

Define durable **data** wire shapes for narrative KnowledgeEntries and related objects. This layer is transport-agnostic and runtime-agnostic.

## Core objects

### v0.1 baseline (delivered)

Five required wire objects in v0.1:

| Object | Role | Schema file |
|--------|------|-------------|
| **KnowledgeEntry** | Identity + typed body + provenance envelope | `schemas/data/knowledge-entry.schema.json` |
| **Relation** | Directed link between KnowledgeEntries (or anchors) | `schemas/data/relation.schema.json` |
| **SourceAnchor** | Pointer to manuscript/source span | `schemas/data/source-anchor.schema.json` |
| **Finding** | Checker output (consistency, style, structure, …) | `schemas/data/finding.schema.json` |
| **AssemblePacket** | Context-assembly payload (structure only) | `schemas/data/assemble-packet.schema.json` |

### Protocol layers + Rule/TimelineEvent deepen (committed wire)

| Object | Layer | Role | Schema file |
|--------|-------|------|-------------|
| **Rule** | L6 | Declarative constraint **input** to checkers (not Finding output) | `schemas/data/rule.schema.json` |
| **TimelineEvent** | L5 | Temporal when-axis object | `schemas/data/timeline-event.schema.json` |

Product invariant: each durable object participates in the `extensions` round-trip contract (§Extensions). See [`spoke-protocol-layers.md`](spoke-protocol-layers.md) for capability levels and Rule vs Finding boundaries.

---

## Rule (L6)

Declarative constraint **input** to `check` — never checker output.

### Required fields

| Field | Type | Semantics |
|-------|------|-----------|
| `schema_version` | integer | Wire version; align with `common.SchemaVersion` |
| `rule_id` | string | Stable id (opaque to protocol) |
| `canonical_name` | string | Human-stable name (min length 1) |
| `kind` | string | Open string; core vocabulary: `rule`, `prohibition`, `style` (documented, not `enum`) |
| `extensions` | object | Namespace map (§Extensions) |

### Optional protocol fields

| Field | Type | Semantics |
|-------|------|-----------|
| `statement` | string | Declarative constraint text (human- or machine-readable; products choose grammar) |
| `description` | string | Longer explanation for integrators / authors |
| `target_entry_types` | string[] | Optional ontology filter — open strings matching KnowledgeEntry `entry_type` vocabulary |
| `severity_hint` | string | Optional checker hint (`info`, `warning`, `error` — open string) |
| `source_anchor` | `SourceAnchor` | Provenance pointer when rule is anchored to manuscript |
| `status` | string | Open string; core: `draft`, `active`, `deprecated` |
| `created_at` | string (RFC 3339) | Creation timestamp |
| `updated_at` | string (RFC 3339) | Last mutation timestamp |

### Illustrative instance

```json
{
  "schema_version": 1,
  "rule_id": "rule_01HXYZ",
  "canonical_name": "No resurrection without foreshadowing",
  "kind": "rule",
  "statement": "Character death reversals require a prior foreshadowing KnowledgeEntry.",
  "target_entry_types": ["character", "event"],
  "severity_hint": "error",
  "status": "active",
  "extensions": {}
}
```

### Open vocabulary (`kind`, `status`)

| Field | JSON type | Core vocabulary (documented, not `enum`) |
|-------|-----------|------------------------------------------|
| `kind` | open string | `rule`, `prohibition`, `style` |
| `status` | open string | `draft`, `active`, `deprecated` |
| `severity_hint` | open string | `info`, `warning`, `error` |

Products MAY emit values outside the core lists; adapters MUST round-trip unknown values verbatim.

---

## TimelineEvent (L5)

First-class **when-axis** object. Distinct from KnowledgeEntry `entry_type: "event"` (ontology label on a KnowledgeEntry body).

### Required fields

| Field | Type | Semantics |
|-------|------|-----------|
| `schema_version` | integer | Wire version |
| `timeline_event_id` | string | Stable id (opaque to protocol) |
| `canonical_name` | string | Human-stable label (min length 1) |
| `extensions` | object | Namespace map (§Extensions) |

### Optional protocol fields

| Field | Type | Semantics |
|-------|------|-----------|
| `timeline_scale` | `TimelineScale` | L5 projection tier — see §TimelineScale |
| `occurred_at` | string | When the event happened — RFC 3339 **or** opaque fuzzy label (e.g. `"Third Age"`) |
| `description` | string | Longer narrative summary |
| `participant_entry_ids` | string[] | Related KnowledgeEntry ids (characters, locations, …) |
| `source_anchor` | `SourceAnchor` | Manuscript / scene anchor |
| `sort_key` | string | Opaque ordering hint within a timeline (products define grammar) |
| `computable_logs` | `ComputableLogEntry[]` | Optional Moment-scale presentation of computable field changes (`l2-computable` only) — see §Computable logs |
| `fork_id` | `ForkId` | Optional world-history branch identity (`l5-fork`) — see §Fork fields |
| `parent_fork_id` | `ForkId` | Optional parent/base branch reference when product records fork lineage on the event |
| `created_at` | string (RFC 3339) | Creation timestamp |
| `updated_at` | string (RFC 3339) | Last mutation timestamp |

**Fork (`l5-fork`, explicitly optional):** baseline `TimelineEvent` MUST NOT require branch metadata. Fork is **world-history branch identity** on the when-axis — optional protocol fields `fork_id` and `parent_fork_id` on `TimelineEvent` (shared type `ForkId` in `common.schema.json`). `spoke-baseline` excludes required Fork.

| Rule | Requirement |
|------|-------------|
| **Optional capability** | Products omit Fork fields unless they declare `l5-fork` |
| **Wire fields** | `fork_id` — branch this event belongs to; `parent_fork_id` — optional lineage to parent/base branch |
| **Shared type** | `ForkId` — `schemas/common/common.schema.json#/definitions/ForkId` (opaque string, `minLength: 1`) |
| **Tier ≠ Fork** | `timeline_scale` is projection tier — not branch identity |
| **Fork ≠ Profile** | Domain Profile adapts ontology vocabulary — must not fork core schemas |
| **Fork ≠ Session / Finding** | Independent from `l2-computable` Session lifecycle and L7 checker output |
| **Engines** | Branch create, merge, rebase, and world-history stores are product-owned |
| **Extensions folklore** | `extensions.<namespace>` fork hints are adapter convention — not the normative Fork interchange |
| **Lineage prose** | When `parent_fork_id` is present, `fork_id` SHOULD also be present; `parent_fork_id` MUST NOT equal `fork_id` |

### Fork fields (`l5-fork` optional)

Shared JSON Schema fragment: `common.schema.json#/definitions/ForkId`.

| Field | Type | Semantics |
|-------|------|-----------|
| `fork_id` | `ForkId` | Opaque branch identity for the world-history branch this event belongs to |
| `parent_fork_id` | `ForkId` | Optional parent/base branch when the product records fork lineage on the event |

Both fields are optional on `TimelineEvent`. Omitting both remains valid. `Scope` MAY refine by `fork_id` only — see [`spoke-ops.md`](spoke-ops.md) §Scope.

### Illustrative instance (baseline — no Fork)

```json
{
  "schema_version": 1,
  "timeline_event_id": "evt_01HXYZ",
  "canonical_name": "Treaty of Ashford",
  "timeline_scale": "narrative",
  "occurred_at": "1421-06-03T00:00:00Z",
  "participant_entry_ids": ["kb_mira", "kb_ashford"],
  "extensions": {
    "my_product": { "world_id": "wld_abc" }
  }
}
```

Product world/book ids belong in `extensions.<namespace>` — not protocol siblings on `TimelineEvent`.

### Illustrative instance (`l5-fork` optional)

```json
{
  "schema_version": 1,
  "timeline_event_id": "evt_fork_01HXYZ",
  "canonical_name": "Treaty of Ashford (what-if branch)",
  "timeline_scale": "narrative",
  "occurred_at": "1421-06-03T00:00:00Z",
  "fork_id": "fork_what_if_b",
  "parent_fork_id": "fork_mainline_a",
  "participant_entry_ids": ["kb_mira", "kb_ashford"],
  "extensions": {
    "my_product": { "world_id": "wld_abc" }
  }
}
```

`fork_id` and `parent_fork_id` are optional; omit both for baseline TimelineEvents. `Scope.fork_id` filters by `fork_id` only — see [`spoke-ops.md`](spoke-ops.md) §Scope.

### Computable logs (`l2-computable` optional)

When a product records dynamic computable field history on the Moment axis, it MAY attach **`computable_logs`** to a `TimelineEvent` with `timeline_scale: "moment"`.

| Rule | Requirement |
|------|-------------|
| **Not Finding** | Log entries MUST NOT reuse Finding fields (`finding_id`, `severity`, `title`, `suggested_fix`, …) |
| **Not Session wire** | Logs are presentation only — Session lifecycle stays op-correlated via `session_id`, not a durable Session object |
| **Engines** | SPOKE does not define how `previous` / `next` values are computed |

**`ComputableLogEntry`** (`common.schema.json#/definitions/ComputableLogEntry`):

| Field | Required | Type |
|-------|----------|------|
| `logged_at` | yes | RFC 3339 timestamp |
| `entry_id` | yes | KnowledgeEntry id whose computable fields changed |
| `changes` | yes | `ComputableLogChange[]` |
| `session_id` | no | Opaque Session correlation (matches op `session_id`) |
| `message` | no | Human-readable presentation note |

**`ComputableLogChange`:**

| Field | Required | Type |
|-------|----------|------|
| `path` | yes | Dot-path or JSON Pointer to changed field within `body.computable` |
| `previous` | no | Opaque JSON — value before change |
| `next` | no | Opaque JSON — value after change |

---

### Dual-concern example (ontology `"event"` vs TimelineEvent)

The same story beat may appear as **both** wire shapes — products choose mapping; protocol keeps names distinct:

| Wire artifact | Example |
|---------------|---------|
| KnowledgeEntry (`entry_type: "event"`) | KB fact node “Treaty of Ashford” with structured `body` |
| TimelineEvent | When-axis placement with `timeline_scale: "narrative"` and `timeline_event_id` |

Toy-world dual-concern fixture pair: `kb_tw_harbor_dawn_event` + `evt_tw_harbor_dawn` — see [`fixtures/toy-world/`](../../fixtures/toy-world/).

---

## TimelineScale (L5 vocabulary)

Shared JSON Schema fragment: `common.schema.json#/definitions/TimelineScale`.

| Property | Value |
|----------|-------|
| JSON type | `string` (open — no `enum` in schema) |
| Core vocabulary | `brief`, `narrative`, `moment` (lowercase) |
| Wire field name | **`timeline_scale`** (not `tier`, `projection`, or product UI strings) |
| Appears on | `TimelineEvent.timeline_scale` (optional); `Scope.timeline_scale` refinement (optional) |

| Value | Semantics on the when-axis |
|-------|----------------------------|
| `brief` | Coarse world shape / era / age-at-a-glance |
| `narrative` | Human-paced ordered events (days–years) |
| `moment` | Scene / beat / sub-scene precision |

Products MAY emit values outside the core trio; adapters MUST round-trip unknown values. Tier names standardize **Timeline dimension semantics** — not any product’s canvas surface requirements.

---

## Rule vs Finding (boundary)

`Rule` and `Finding` are **never interchangeable**. Collapsing them breaks `check` I/O semantics.

| Concern | Rule (L6) | Finding (L7) |
|---------|-----------|--------------|
| **Role** | Declarative checker **input** | Checker **output** |
| **Wire schema** | `schemas/data/rule.schema.json` | `schemas/data/finding.schema.json` |
| **Stable id** | `rule_id` | `finding_id` |
| **`check` direction** | Request: `rule_refs[]` and/or embedded `rules[]` | Response: `findings[]` |
| **Severity** | Optional `severity_hint` (checker hint) | Required `severity` |
| **Status vocabulary** | `draft`, `active`, `deprecated` (open string) | `open`, `resolved`, `dismissed` (open string) |
| **Constraint text** | Optional `statement` / `description` | Required `title` + `description` |
| **Remediation** | Not on Rule wire | Optional `suggested_fix`, `text_position` |
| **MUST NOT** | Appear in `findings[]` | Appear in `check` request as rules |

| Related concern | Product rule |
|-----------------|--------------|
| Adapter mapping | Product ontology labels map in **future adapter specs** / Showcases — not blockers for wire shapes |
| KnowledgeEntry `entry_type: "rule"` | Valid open ontology label on a KnowledgeEntry — distinct from L6 `Rule` wire object |
| Fork | Optional L5 capability — not required with `TimelineEvent` |

---

## Shared envelope pattern

Every durable data object schema MUST:

1. Declare `"$schema": "http://json-schema.org/draft-07/schema#"` and a stable `$id` under `https://spoke42.invalid/schemas/...`.
2. Include top-level `schema_version` (integer ≥ 1) in `required`.
3. Set `additionalProperties: false` on the protocol object.
4. Include `extensions` (see §Extensions) in `required` — use `{}` when empty.
5. Reference shared defs from `schemas/common/common.schema.json` via `$ref` (identifiers, timestamps, extension map).

---

## KnowledgeEntry envelope

### Required fields

| Field | Type | Semantics |
|-------|------|-----------|
| `schema_version` | integer | Wire version; align with `common.SchemaVersion` |
| `entry_id` | string | Stable id (opaque to protocol; products choose prefix/format) |
| `entry_type` | string | Open string; core vocabulary in §Open vocabulary |
| `canonical_name` | string | Human-stable name (min length 1) |
| `status` | string | Open string; core vocabulary in §Open vocabulary |
| `body` | object | Structured payload; `additionalProperties: true` on `body` only |
| `extensions` | object | Namespace map (§Extensions) |

### Optional protocol fields

| Field | Type | Semantics |
|-------|------|-----------|
| `revision` | integer ≥ 0 | OCC / optimistic concurrency |
| `source_anchor` | `SourceAnchor` | Provenance pointer (`$ref` to data schema) |
| `created_at` | string (RFC 3339) | Creation timestamp |
| `updated_at` | string (RFC 3339) | Last mutation timestamp |

### Body rules

- `body` is the **only** protocol subtree that allows open keys (`additionalProperties: true`).
- Typed attributes (e.g. `summary`, `tags`, `attributes`) live under `body`, not as sibling protocol keys.
- Product-specific body shapes MUST NOT add protocol siblings; use `extensions.<namespace>` for opaque product fields that must round-trip outside `body`.

### Computable body (`l2-computable` optional)

Products declaring **`l2-computable`** MAY use two documented optional keys under `body`. Both share **`ComputableFieldMap`** (`common.schema.json#/definitions/ComputableFieldMap`): an open JSON object (`additionalProperties: true`) for product domain values. Protocol does **not** require WASM bytecode or executable artifacts in required fields.

| Key | Role | Lifecycle |
|-----|------|-----------|
| **`state`** | Static durable computable state | Authoritative on disk pre-Session and after settle |
| **`computable`** | Dynamic Session-scoped projection | Absent or inert pre-Session; mutates mid-Session only; merged into `state` at settle |

**Session lifecycle (normative):**

| Phase | `body.state` | `body.computable` |
|-------|--------------|-------------------|
| Pre-Session | Present (when capability used) | Absent or inert |
| Session start | Unchanged | Initialized from `state` (typically via `project` op) |
| Mid-Session | MUST NOT be silently rewritten | Only subtree that mutates |
| Session end | Receives merged values from `computable` | Cleared or inert after settle (typically via `compute` op with `settle: true`) |

**Session** is a lifecycle concept — not `entry_type`, not a durable KnowledgeEntry, not a top-level wire object. Correlation uses op-level `session_id` (see [`spoke-ops.md`](spoke-ops.md) §Optional ops).

**Dual-concern:** Moment `computable_logs` on TimelineEvent are presentation for field history — distinct from L7 **Finding** checker output.

---

```json
{
  "schema_version": 1,
  "entry_id": "kb_01HXYZ",
  "entry_type": "character",
  "canonical_name": "Mira Vale",
  "status": "confirmed",
  "body": {
    "summary": "Protagonist; reluctant cartographer.",
    "tags": ["pov"]
  },
  "source_anchor": {
    "schema_version": 1,
    "source_id": "manuscript:book-1:ch-3",
    "span": { "start": 120, "end": 480 },
    "extensions": {}
  },
  "revision": 2,
  "created_at": "2026-07-23T08:00:00Z",
  "updated_at": "2026-07-23T09:15:00Z",
  "extensions": {
    "my_product": { "world_id": "wld_abc" }
  }
}
```

### Illustrative instance (`l2-computable` — mid-Session)

```json
{
  "schema_version": 1,
  "entry_id": "kb_sim_01",
  "entry_type": "item",
  "canonical_name": "Harbor simulation",
  "status": "confirmed",
  "body": {
    "summary": "Tide and cargo model for Ashford harbor.",
    "state": { "tide_level": 2.1, "cargo_tons": 40 },
    "computable": { "tide_level": 2.4, "cargo_tons": 38 }
  },
  "extensions": {}
}
```

Pre-Session and post-settle: omit `computable` or leave inert; `state` holds durable values.

---

## Relation

Directed edge between two KnowledgeEntries (or KnowledgeEntry ↔ SourceAnchor when products need anchor linkage).

| Field | Required | Type |
|-------|----------|------|
| `schema_version` | yes | integer |
| `relation_id` | yes | string |
| `relation_type` | yes | string (open; core list in §Open vocabulary) |
| `from_id` | yes | string |
| `to_id` | yes | string |
| `extensions` | yes | object |

Optional: `label`, `metadata` (object, open), `created_at`, `updated_at`.

---

## SourceAnchor

Pointer to a source artifact span (manuscript, scene, external URI).

| Field | Required | Type |
|-------|----------|------|
| `schema_version` | yes | integer |
| `source_id` | yes | string (opaque locator; products define grammar) |
| `extensions` | yes | object |

Optional: `span` (`{ "start": number, "end": number }`), `label`, `mime_type`.

---

## Finding

Checker output — **not** a KnowledgeEntry body.

| Field | Required | Type |
|-------|----------|------|
| `schema_version` | yes | integer |
| `finding_id` | yes | string |
| `severity` | yes | string (open; core: `info`, `warning`, `error`) |
| `status` | yes | string (open; core: `open`, `resolved`, `dismissed`) |
| `title` | yes | string |
| `description` | yes | string |
| `extensions` | yes | object |

Optional: `kind`, `target_entry_id`, `source_anchor`, `suggested_fix`, `text_position` (object), `created_at`, `updated_at`.

**Status transitions (cross-product minimum):** enforced by `@42ch/spoke-operations` — see [`spoke-operations.md` §Finding lifecycle](spoke-operations.md#2-finding-lifecycle--finding). Wire schema keeps `status` as open string; library enforces the core transition table.

---

## AssemblePacket

**Wire-only context payload** — no compute semantics in the data schema. Normative ops boundary: [`spoke-ops.md` §`assemble` wire-only](spoke-ops.md#assemble-wire-only-boundary-normative).

| Field | Required | Type |
|-------|----------|------|
| `schema_version` | yes | integer |
| `packet_id` | yes | string |
| `entries` | yes | array of `AssembleEntry` |
| `extensions` | yes | object |

### AssembleEntry (inline definition)

| Field | Required | Type |
|-------|----------|------|
| `entry_id` | yes | string |
| `entry_type` | yes | string |
| `canonical_name` | yes | string |
| `snippet` | no | string (trimmed text for context window) |

`entries` MAY embed full `KnowledgeEntry` objects only when an op response schema explicitly `$ref`s `knowledge-entry.schema.json` instead of `AssembleEntry` — default is the slim entry shape above.

**Out of scope in v0.1 data schema:** ranking scores, retrieval provenance, token budgets, model routing hints — products place those under `extensions.<namespace>` if needed.

---

## Extensions (normative)

```json
"extensions": {
  "my_product": { },
  "other_product": { }
}
```

| Rule | Requirement |
|------|-------------|
| Namespace keys | Product-chosen ids — `^[a-z][a-z0-9_-]*$` |
| Values | Opaque JSON objects (`additionalProperties: true` per namespace value) |
| Unknown namespaces | Adapters MUST preserve on round-trip |
| Unknown keys inside a namespace | Adapters MUST preserve on round-trip |
| Merge / preserve semantics | [`spoke-operations.md`](spoke-operations.md) (`mergeExtensionMaps`, `preserveExtensionMaps`) |
| Core fields | MUST NOT use open `additionalProperties` on the protocol object as a substitute for `extensions` |
| Empty | `extensions: {}` is valid |

Shared JSON Schema fragment: `common.schema.json#/definitions/ExtensionMap`.

---

## Open vocabulary

`entry_type`, KnowledgeEntry `status`, `relation_type`, and Finding `severity`/`status` are **open strings** in v0.1 schemas (`type: string` with no `enum`). Schemas document the core vocabulary in `description` fields; closure to `enum` waits until adapter specs prove stability.

### Core `entry_type` vocabulary (documented, not enforced)

Cross-product narrative set used by the protocol core list. Order: baseline narrative types, authoring extras, then canvas-sync additions.

| Value | Typical use |
|-------|-------------|
| `character` | Person / agent |
| `location` | Place |
| `event` | Ontology label for plot / story-beat facts; **≠** L5 `TimelineEvent` wire object |
| `scene` | Scene unit |
| `act` | Structural act (script / screenplay) |
| `organization` | Group / faction |
| `item` | Object / artifact |
| `conflict` | Dramatic conflict unit |
| `info_point` | Foreshadowing / revelation hook |
| `era` | World-timeline era / brief marker |
| `worldbuilding` | Encyclopedia / lore entry |
| `note` | Free-form author note |
| `research` | External research note |
| `ability` | Skill / power / capability KnowledgeEntry (canvas baseline) |
| `rule` | World rule / constraint **ontology label** on a KnowledgeEntry; **≠** L6 `Rule` wire object (`rule_id`, `kind`, `statement`, `target_entry_types`) |

**Extension policy:** products MAY emit values outside this list. Adapters MUST round-trip unknown values without normalization. Profile-specific types (`dialogue`, `beat`, `species`, `magic_system`, …) belong in **Domain Profile** / adapter specs — not in this core table or in schema `description` core lists.

### Research canvas coverage (ontology)

Normative mirror of the Spoke Protocol Research canvas `TYPE_MAP`. Integrators cite this table — not the canvas alone — for baseline vs profile vs deferred decisions.

| Canvas `spoke` | Decision | Integrator note |
|----------------|----------|-----------------|
| `character` | **Keep** (core) | Baseline |
| `location`* | **Keep** (core) | Open string; `*` = adapter profile annotation in canvas |
| `event` | **Keep** (core) | Ontology label; ≠ `TimelineEvent` wire object |
| `scene` | **Keep** (core) | Baseline |
| `act` | **Keep** (core) | Baseline |
| `organization` | **Keep** (core) | Baseline |
| `item` | **Keep** (core) | Baseline |
| `ability` | **Add** (core) | Skill / power / capability KnowledgeEntry |
| `conflict` | **Keep** (core) | Baseline |
| `info_point` | **Keep** (core) | Foreshadowing / revelation hook |
| `era` | **Keep** (core) | Brief-scale timeline marker |
| `worldbuilding`* | **Keep** (core) | Lore / encyclopedia; `*` = profile variants |
| `rule`* | **Add** (core) | Ontology label `entry_type: "rule"`; **≠** L6 `Rule` object |
| `note`, `research` | **Keep** (core) | Authoring extras; not shown on canvas `TYPE_MAP` |
| `dialogue` | **Profile-only** | Domain Profile / adapter spec |
| `beat` | **Profile-only** | Domain Profile / adapter spec |
| `species`, `magic_system` | **Profile-only** | Typically under worldbuilding profile |

**Dual-concern quick reference:**

| Integrator question | Answer |
|---------------------|--------|
| `entry_type: "rule"` on a KnowledgeEntry — is that the L6 `Rule` object? | **No.** KB ontology label only. L6 rules use `rule.schema.json` + `rule_id`. |
| `target_entry_types` on a `Rule` — what does it filter? | KnowledgeEntry **`entry_type`** strings (e.g. `character`, `event`), not `Rule` object kinds. |
| `entry_type: "event"` vs `TimelineEvent`? | KB fact node vs L5 when-axis object. `Scope` uses `entry_types` vs `timeline_event_ids` separately. |
| Session vs TimelineEvent vs Finding? | Session = lifecycle (`session_id` on ops); TimelineEvent = when-axis; `computable_logs` = presentation; Finding = checker output. |
| Filter TimelineEvents by branch? | Optional `Scope.fork_id` — strict equality on `TimelineEvent.fork_id` (`l5-fork`); events without `fork_id` do not match. |
| Should `dialogue` / `beat` be in the core table? | **No.** Profile-only per baseline lock. |

### Core KnowledgeEntry `status` vocabulary (documented, not enforced)

| Value | Semantics |
|-------|-----------|
| `provisional` | Candidate / unreviewed |
| `confirmed` | Accepted canonical |
| `deprecated` | Superseded but retained |
| `merged` | Absorbed into another KnowledgeEntry |
| `deleted` | Tombstone / soft delete |

**Status transitions (cross-product minimum):** enforced by `@42ch/spoke-operations` — see [`spoke-operations.md` §KnowledgeEntry lifecycle](spoke-operations.md#6-knowledgeentry-lifecycle--knowledgeentry). Wire schema keeps `status` as open string; library enforces the core transition table. **Active** statuses for uniqueness: `provisional`, `confirmed` only.

**`deprecated` → `merged` excluded:** merge absorbs an active canonical KnowledgeEntry into a target; a deprecated row is already superseded — restore to `confirmed` (or merge from `provisional`/`confirmed`) before absorb.

### Core `relation_type` vocabulary (starter set)

`related_to`, `parent_of`, `member_of`, `located_in`, `participates_in`, `causes`, `foreshadows`

### Extension policy summary

| Concern | Rule |
|---------|------|
| Unknown values | Round-trip verbatim |
| Closed enums | Deferred until adapter iteration |
| Product-only types | `extensions.<namespace>` or open `entry_type` string |
| Documentation | Adapter specs (next iteration) own per-product tables |

---

## Vocabulary boundaries (CONCEPTS alignment)

- **KnowledgeEntry** — atomic Knowledge Base entry in SPOKE wire form
- **Scope** — shared `Scope` object (`scope_id` required) for `check` / `assemble`; World/Book ids in `extensions` or adapters — see [`spoke-ops.md`](spoke-ops.md) §Scope
- **TimelineScale** — L5 tier vocabulary (`brief` / `narrative` / `moment`) on `TimelineEvent` and optional `Scope` filter — see §TimelineScale
- **ForkId** — opaque branch identity (`l5-fork`) on `TimelineEvent.fork_id`, `TimelineEvent.parent_fork_id`, and optional `Scope.fork_id` — see §Fork fields
- **Domain Profile** — published ontology vocabulary per product/integration; core `entry_type` stays open string — see [`spoke-protocol-layers.md`](spoke-protocol-layers.md)
- **TimelineEvent** — L5 temporal wire object (when-axis); distinct from KnowledgeEntry `entry_type: "event"` labels
- **Session** — optional `l2-computable` lifecycle (not `entry_type`, not durable wire object); see §Computable body
- **ComputableFieldMap** — open object for `body.state` and `body.computable` under `l2-computable`
- **ComputableLogEntry** — Moment-scale presentation on `TimelineEvent.computable_logs` (not Finding)
- **World KB / Author Memory** — product-local stores; mapped via adapters in a later iteration, not redefined here
- **Finding** — checker output, not a KnowledgeEntry body
- **Rule** — L6 declarative wire object (protocol layers deepen); not synonymous with `entry_type: "rule"` which remains a valid open string if products use it

---

## Acceptance (data layer)

- [x] Each **baseline + protocol layers deepen** object above has a draft-07 schema under `schemas/data/` (or `schemas/common/` for shared defs)
- [x] Umbrella + this doc list the same object set; Rule/TimelineEvent shipped in `rule-event`
- [ ] Sample valid KnowledgeEntry instance (inline above or schema `examples`) shows `extensions` usage — **no fixture directory required**
- [ ] `entry_type` / `status` fields are `type: string` without `enum`; core vocabulary appears in `description`

## Non-goals (data layer)

- Product object mapping implementations (adapter iteration)
- Closed enums for all entry types
- Required Fork / world-history fields in baseline compliance
- Required WASM or computable KnowledgeEntry bodies (optional `l2-computable` capability only)
- Golden product DTO round-trips (protocol `fixtures/toy-world/` delivered fixtures conformance slice — see [`fixtures/toy-world/README.md`](../../fixtures/toy-world/README.md); product DTO maps remain adapter work)

## See also

| Doc | Topic |
|-----|-------|
| [`spoke-protocol.md`](spoke-protocol.md) | Umbrella framing, extensions, codegen layout |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | L0–L8 map, capability levels, Rule vs Finding |
| [`spoke-ops.md`](spoke-ops.md) | Ops that consume these data shapes (`check`, `assemble`, …) |
| [`spoke-operations.md`](spoke-operations.md) | Lifecycle helpers (extensions, Finding status, promote, AssemblePacket builders) |
| [`schemas/README.md`](../../schemas/README.md) | Schema file checklist |
| [`CONCEPTS.md`](../../CONCEPTS.md) | Vocabulary boundaries (KnowledgeEntry vs product stores) |
