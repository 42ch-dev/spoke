# SPOKE Data Model

> **Status:** Normative (v0.1 baseline; v0-iter003 deepen)  
> **Document class:** Detail — data layer  
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)  
> **Schema home:** `schemas/data/`, `schemas/common/`

## Purpose

Define durable **data** wire shapes for narrative Keyblocks and related objects. This layer is transport-agnostic and runtime-agnostic.

## Core objects

### v0.1 baseline (delivered)

Five required wire objects in v0.1:

| Object | Role | Schema file |
|--------|------|-------------|
| **Keyblock** | Identity + typed body + provenance envelope | `schemas/data/keyblock.schema.json` |
| **Relation** | Directed link between Keyblocks (or anchors) | `schemas/data/relation.schema.json` |
| **SourceAnchor** | Pointer to manuscript/source span | `schemas/data/source-anchor.schema.json` |
| **Finding** | Checker output (consistency, style, structure, …) | `schemas/data/finding.schema.json` |
| **AssemblePacket** | Context-assembly payload (structure only) | `schemas/data/assemble-packet.schema.json` |

### v0-iter003 deepen (architect-locked target wire)

| Object | Layer | Role | Schema file |
|--------|-------|------|-------------|
| **Rule** | L6 | Declarative constraint **input** to checkers (not Finding output) | `schemas/data/rule.schema.json` |
| **Event** | L5 | Temporal when-axis object | `schemas/data/event.schema.json` |

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
| `target_block_types` | string[] | Optional ontology filter — open strings matching Keyblock `block_type` vocabulary |
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
  "statement": "Character death reversals require a prior foreshadowing Keyblock.",
  "target_block_types": ["character", "event"],
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

## Event (L5)

First-class **when-axis** object. Distinct from Keyblock `block_type: "event"` (ontology label on a Keyblock body).

### Required fields

| Field | Type | Semantics |
|-------|------|-----------|
| `schema_version` | integer | Wire version |
| `event_id` | string | Stable id (opaque to protocol) |
| `canonical_name` | string | Human-stable label (min length 1) |
| `extensions` | object | Namespace map (§Extensions) |

### Optional protocol fields

| Field | Type | Semantics |
|-------|------|-----------|
| `timeline_scale` | `TimelineScale` | L5 projection tier — see §TimelineScale |
| `occurred_at` | string | When the event happened — RFC 3339 **or** opaque fuzzy label (e.g. `"Third Age"`) |
| `description` | string | Longer narrative summary |
| `participant_keyblock_ids` | string[] | Related Keyblock ids (characters, locations, …) |
| `source_anchor` | `SourceAnchor` | Manuscript / scene anchor |
| `sort_key` | string | Opaque ordering hint within a timeline (products define grammar) |
| `created_at` | string (RFC 3339) | Creation timestamp |
| `updated_at` | string (RFC 3339) | Last mutation timestamp |

**Fork (explicitly optional):** baseline `Event` MUST NOT require `fork_id` or branch metadata. Fork semantics remain optional capability `l5-fork` — future wire fields, not v0-iter003 baseline.

### Illustrative instance

```json
{
  "schema_version": 1,
  "event_id": "evt_01HXYZ",
  "canonical_name": "Treaty of Ashford",
  "timeline_scale": "narrative",
  "occurred_at": "1421-06-03T00:00:00Z",
  "participant_keyblock_ids": ["kb_mira", "kb_ashford"],
  "extensions": {
    "nexus": { "world_id": "wld_abc" }
  }
}
```

Product world/book ids belong in `extensions.<namespace>` — not protocol siblings on `Event`.

---

## TimelineScale (L5 vocabulary)

Shared JSON Schema fragment: `common.schema.json#/definitions/TimelineScale`.

| Property | Value |
|----------|-------|
| JSON type | `string` (open — no `enum` in schema) |
| Core vocabulary | `brief`, `narrative`, `moment` (lowercase) |
| Wire field name | **`timeline_scale`** (not `tier`, `projection`, or product UI strings) |
| Appears on | `Event.timeline_scale` (optional); `Scope.timeline_scale` refinement (optional) |

| Value | Semantics on the when-axis |
|-------|----------------------------|
| `brief` | Coarse world shape / era / age-at-a-glance |
| `narrative` | Human-paced ordered events (days–years) |
| `moment` | Scene / beat / sub-scene precision |

Products MAY emit values outside the core trio; adapters MUST round-trip unknown values. Tier names standardize **Timeline dimension semantics** — not Nexus/Creader canvas surface requirements.

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

**Historical note:** v0.1 deferred the `Rule` wire object; `check` accepted opaque `rule_refs: string[]` only. v0-iter003 ships portable `Rule` and `Event` shapes — field tables above are normative.

| Related concern | Product rule |
|-----------------|--------------|
| Adapter mapping | Creader `KnowledgeEntryType: "rule"` and Nexus overlays map in **future adapter specs** — not blockers for wire shapes |
| Keyblock `block_type: "rule"` | Valid open ontology label on a Keyblock — distinct from L6 `Rule` wire object |
| Fork | Optional L5 capability — not required with `Event` |

---

## Shared envelope pattern

Every durable data object schema MUST:

1. Declare `"$schema": "http://json-schema.org/draft-07/schema#"` and a stable `$id` under `https://spoke42.invalid/schemas/...`.
2. Include top-level `schema_version` (integer ≥ 1) in `required`.
3. Set `additionalProperties: false` on the protocol object.
4. Include `extensions` (see §Extensions) in `required` — use `{}` when empty.
5. Reference shared defs from `schemas/common/common.schema.json` via `$ref` (identifiers, timestamps, extension map).

---

## Keyblock envelope

### Required fields

| Field | Type | Semantics |
|-------|------|-----------|
| `schema_version` | integer | Wire version; align with `common.SchemaVersion` |
| `keyblock_id` | string | Stable id (opaque to protocol; products choose prefix/format) |
| `block_type` | string | Open string; core vocabulary in §Open vocabulary |
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

### Illustrative instance

```json
{
  "schema_version": 1,
  "keyblock_id": "kb_01HXYZ",
  "block_type": "character",
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
    "nexus": { "world_id": "wld_abc" }
  }
}
```

---

## Relation

Directed edge between two Keyblocks (or Keyblock ↔ SourceAnchor when products need anchor linkage).

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

Checker output — **not** a Keyblock body.

| Field | Required | Type |
|-------|----------|------|
| `schema_version` | yes | integer |
| `finding_id` | yes | string |
| `severity` | yes | string (open; core: `info`, `warning`, `error`) |
| `status` | yes | string (open; core: `open`, `resolved`, `dismissed`) |
| `title` | yes | string |
| `description` | yes | string |
| `extensions` | yes | object |

Optional: `kind`, `target_keyblock_id`, `source_anchor`, `suggested_fix`, `text_position` (object), `created_at`, `updated_at`.

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
| `keyblock_id` | yes | string |
| `block_type` | yes | string |
| `canonical_name` | yes | string |
| `snippet` | no | string (trimmed text for context window) |

`entries` MAY embed full `Keyblock` objects only when an op response schema explicitly `$ref`s `keyblock.schema.json` instead of `AssembleEntry` — default is the slim entry shape above.

**Out of scope in v0.1 data schema:** ranking scores, retrieval provenance, token budgets, model routing hints — products place those under `extensions.<namespace>` if needed.

---

## Extensions (normative)

```json
"extensions": {
  "nexus": { },
  "creader": { }
}
```

| Rule | Requirement |
|------|-------------|
| Namespace keys | Product ids (`nexus`, `creader`, …) — `^[a-z][a-z0-9_-]*$` |
| Values | Opaque JSON objects (`additionalProperties: true` per namespace value) |
| Unknown namespaces | Adapters MUST preserve on round-trip |
| Unknown keys inside a namespace | Adapters MUST preserve on round-trip |
| Merge / preserve semantics | [`spoke-operations.md`](spoke-operations.md) (`mergeExtensionMaps`, `preserveExtensionMaps`) |
| Core fields | MUST NOT use open `additionalProperties` on the protocol object as a substitute for `extensions` |
| Empty | `extensions: {}` is valid |

Shared JSON Schema fragment: `common.schema.json#/definitions/ExtensionMap`.

---

## Open vocabulary

`block_type`, Keyblock `status`, `relation_type`, and Finding `severity`/`status` are **open strings** in v0.1 schemas (`type: string` with no `enum`). Schemas document the core vocabulary in `description` fields; closure to `enum` waits until adapter specs prove stability.

### Core `block_type` vocabulary (documented, not enforced)

Cross-product narrative set (union of Nexus + Creader research inputs):

| Value | Typical use |
|-------|-------------|
| `character` | Person / agent |
| `location` | Place |
| `event` | Timeline / plot event |
| `scene` | Scene unit |
| `organization` | Group / faction (Nexus) |
| `item` | Object / artifact |
| `conflict` | Dramatic conflict unit |
| `info_point` | Foreshadowing / revelation hook |
| `era` | World-timeline era / brief marker |
| `note` | Free-form author note (Creader) |
| `worldbuilding` | Encyclopedia / lore entry (Creader) |
| `research` | External research note (Creader) |
| `act` | Structural act (script / Creader) |

**Extension policy:** products MAY emit values outside this list. Adapters MUST round-trip unknown values without normalization. When a product needs profile-specific types (e.g. Nexus `species`, `dialogue`, `beat`), document them in the adapter spec; do not expand the core list until a cross-product agreement exists.

### Core Keyblock `status` vocabulary (documented, not enforced)

| Value | Semantics |
|-------|-----------|
| `provisional` | Candidate / unreviewed |
| `confirmed` | Accepted canonical |
| `deprecated` | Superseded but retained |
| `merged` | Absorbed into another Keyblock |
| `deleted` | Tombstone / soft delete |

### Core `relation_type` vocabulary (starter set)

`related_to`, `parent_of`, `member_of`, `located_in`, `participates_in`, `causes`, `foreshadows`

### Extension policy summary

| Concern | Rule |
|---------|------|
| Unknown values | Round-trip verbatim |
| Closed enums | Deferred until adapter iteration |
| Product-only types | `extensions.<namespace>` or open `block_type` string |
| Documentation | Adapter specs (next iteration) own per-product tables |

---

## Vocabulary boundaries (CONCEPTS alignment)

- **Keyblock** — atomic narrative knowledge unit in SPOKE wire form
- **Scope** — shared `Scope` object (`scope_id` required) for `check` / `assemble`; World/Book ids in `extensions` or adapters — see [`spoke-ops.md`](spoke-ops.md) §Scope
- **TimelineScale** — L5 tier vocabulary (`brief` / `narrative` / `moment`) on `Event` and optional `Scope` filter — see §TimelineScale
- **Domain Profile** — published ontology vocabulary per product/integration; core `block_type` stays open string — see [`spoke-protocol-layers.md`](spoke-protocol-layers.md)
- **Event** — L5 temporal wire object (when-axis); distinct from Keyblock `block_type: "event"` labels
- **World KB / Author Memory** — product-local stores; mapped via adapters in a later iteration, not redefined here
- **Finding** — checker output, not a Keyblock body
- **Rule** — L6 declarative wire object (v0-iter003); not synonymous with `block_type: "rule"` which remains a valid open string if products use it

---

## Acceptance (data layer)

- [x] Each **baseline + v0-iter003** object above has a draft-07 schema under `schemas/data/` (or `schemas/common/` for shared defs)
- [x] Umbrella + this doc list the same object set; Rule/Event shipped in v0-iter003 plan `rule-event`
- [ ] Sample valid Keyblock instance (inline above or schema `examples`) shows `extensions` usage — **no fixture directory required**
- [ ] `block_type` / `status` fields are `type: string` without `enum`; core vocabulary appears in `description`

## Non-goals (data layer)

- Nexus/Creader object mapping implementations (adapter iteration)
- Closed enums for all block types
- Required Fork / world-history fields in baseline compliance
- Required WASM or computable Keyblock bodies (optional `l2-computable` capability only)
- Conformance fixtures / golden round-trips

## See also

| Doc | Topic |
|-----|-------|
| [`spoke-protocol.md`](spoke-protocol.md) | Umbrella framing, extensions, codegen layout |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | L0–L8 map, capability levels, Rule vs Finding |
| [`spoke-ops.md`](spoke-ops.md) | Ops that consume these data shapes (`check`, `assemble`, …) |
| [`spoke-operations.md`](spoke-operations.md) | Lifecycle helpers (extensions, Finding status, promote, AssemblePacket builders) |
| [`schemas/README.md`](../../schemas/README.md) | Schema file checklist |
| [`CONCEPTS.md`](../../CONCEPTS.md) | Vocabulary boundaries (Keyblock vs product stores) |
