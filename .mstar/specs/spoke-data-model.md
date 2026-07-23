# SPOKE Data Model

> **Status:** Normative (v0.1)  
> **Document class:** Detail — data layer  
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)  
> **Schema home:** `schemas/data/`, `schemas/common/`

## Purpose

Define durable **data** wire shapes for narrative Keyblocks and related objects. This layer is transport-agnostic and runtime-agnostic.

## Core objects (v0.1)

Five required wire objects in v0.1; `Rule` is deferred (§Rule deferral).

| Object | Role | v0.1 required | Schema file |
|--------|------|---------------|-------------|
| **Keyblock** | Identity + typed body + provenance envelope | Yes | `schemas/data/keyblock.schema.json` |
| **Relation** | Directed link between Keyblocks (or anchors) | Yes | `schemas/data/relation.schema.json` |
| **SourceAnchor** | Pointer to manuscript/source span | Yes | `schemas/data/source-anchor.schema.json` |
| **Finding** | Checker output (consistency, style, structure, …) | Yes | `schemas/data/finding.schema.json` |
| **AssemblePacket** | Context-assembly payload (structure only) | Yes | `schemas/data/assemble-packet.schema.json` |
| **Rule** | Declarative check-rule reference | **Deferred** (see §Rule deferral) | — |

Product invariant: each **required** object participates in the `extensions` round-trip contract (§Extensions).

---

## Rule deferral (v0.1 decision)

**Decision:** `Rule` is **out of scope** for v0.1 schema authoring.

| Aspect | v0.1 position |
|--------|---------------|
| Wire object | No `schemas/data/rule.schema.json` |
| Ops impact | [`check`](spoke-ops.md#check) request MAY carry opaque `rule_refs: string[]` (rule ids / URIs); no `Rule` body shape |
| Product mapping | Creader `KnowledgeEntryType: "rule"` and Nexus `rule_suggestion` on findings map in **adapter specs** (next iteration) |
| Revisit trigger | When adapter packages need a portable declarative-rule envelope distinct from `Finding` |

**Rationale:** `Finding` covers checker **output**; declarative rules are product-local today and overlap with extension bags. Shipping a stub `Rule` schema without adapter mapping would be dead weight.

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
- **World KB / Author Memory** — product-local stores; mapped via adapters in a later iteration, not redefined here
- **Finding** — checker output, not a Keyblock body
- **Rule** — deferred wire object (§Rule deferral); not synonymous with `block_type: "rule"` which remains a valid open string if products use it

---

## Acceptance (data layer)

- [ ] Each **required** object above has a draft-07 schema under `schemas/data/` (or `schemas/common/` for shared defs)
- [ ] Umbrella + this doc list the same required object set (`Rule` explicitly excluded)
- [ ] Sample valid Keyblock instance (inline above or schema `examples`) shows `extensions` usage — **no fixture directory required**
- [ ] `block_type` / `status` fields are `type: string` without `enum`; core vocabulary appears in `description`

## Non-goals (data layer, v0.1)

- Nexus/Creader object mapping implementations
- Closed enums for all block types
- `Rule` wire schema
- Fork / world-history semantics as required protocol fields
- WASM or computable Keyblock bodies

## See also

| Doc | Topic |
|-----|-------|
| [`spoke-protocol.md`](spoke-protocol.md) | Umbrella framing, extensions, codegen layout |
| [`spoke-ops.md`](spoke-ops.md) | Ops that consume these data shapes (`check`, `assemble`, …) |
| [`spoke-operations.md`](spoke-operations.md) | Lifecycle helpers (extensions, Finding status, promote, AssemblePacket builders) |
| [`schemas/README.md`](../../schemas/README.md) | Schema file checklist |
| [`CONCEPTS.md`](../../CONCEPTS.md) | Vocabulary boundaries (Keyblock vs product stores) |
