# SPOKE Data Model

> **Status:** Normative (v0.1 baseline; Rule + TimelineEvent + open vocabulary)
> **Document class:** Detail — data layer  
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)  
> **Schema home:** `schemas/data/`, `schemas/common/`

## Purpose

Define durable **data** wire shapes for narrative KnowledgeEntries and related objects. This layer is transport-agnostic and runtime-agnostic.

## Core objects

### v0.1 baseline (delivered)

Six core durable wire objects for baseline host collaboration and narrative knowledge:

| Object | Role | Schema file |
|--------|------|-------------|
| **KnowledgeEntry** | Identity + typed body + provenance envelope | `schemas/data/knowledge-entry.schema.json` |
| **Relation** | Directed link between KnowledgeEntries (or anchors) | `schemas/data/relation.schema.json` |
| **SourceAnchor** | Pointer to manuscript/source span | `schemas/data/source-anchor.schema.json` |
| **Finding** | Checker output (consistency, style, structure, …) | `schemas/data/finding.schema.json` |
| **AssemblePacket** | Context-assembly payload (structure only) | `schemas/data/assemble-packet.schema.json` |
| **HostCapabilityManifest** | Adapter self-description: roles, capabilities, owned `extensions` namespaces | `schemas/data/host-capability-manifest.schema.json` |

### Protocol layers + Rule/TimelineEvent deepen (committed wire)

| Object | Layer | Role | Schema file |
|--------|-------|------|-------------|
| **Rule** | L6 | Declarative constraint **input** to checkers (not Finding output) | `schemas/data/rule.schema.json` |
| **TimelineEvent** | L5 | Temporal when-axis object | `schemas/data/timeline-event.schema.json` |

Product invariant: each durable object participates in the `extensions` round-trip contract (§Extensions). `HostCapabilityManifest` uses the same `ExtensionMap` for deployment metadata only — host roles and namespace ownership are core manifest fields, not KE `extensions.<ns>` bags. See [`spoke-operations.md`](spoke-operations.md) §Host collaboration. See [`spoke-protocol-layers.md`](spoke-protocol-layers.md) for capability levels and Rule vs Finding boundaries.

---

## HostCapabilityManifest (host collaboration)

First-class wire object for **in-process adapter self-description** — roles, capability flags, and exclusive `extensions` namespace ownership. Distinct from KnowledgeEntry: host metadata MUST NOT be required inside `KnowledgeEntry.extensions.<ns>`.

Schema: `schemas/data/host-capability-manifest.schema.json`.

### Required fields

| Field | Type | Semantics |
|-------|------|-----------|
| `schema_version` | integer | Wire version; align with `common.SchemaVersion` |
| `host_id` | string | Stable host identity in a collaboration context (`minLength: 1`; opaque to protocol) |
| `roles` | string[] | Open strings; `minItems: 1`; `uniqueItems: true` — core vocabulary in §Host roles |
| `capabilities` | string[] | Open strings; `minItems: 1`; `uniqueItems: true` — capability flags in §Host capabilities |
| `namespaces` | string[] | Namespace keys this host owns; `minItems: 1`; `uniqueItems: true`; each item matches `^[a-z][a-z0-9_-]*$` |
| `extensions` | object | `ExtensionMap` — deployment/product metadata only (§Manifest extensions) |

### Optional protocol fields

| Field | Type | Semantics |
|-------|------|-----------|
| `authority` | object | Closed scope pointer for data-store OCC — see §Authority |

### Host roles (open vocabulary)

`roles` is an open `string[]` (no JSON Schema `enum`). Core collaboration vocabulary:

| Role | Purpose | Typical ports / ops |
|------|---------|---------------------|
| `data-store` | Single OCC authority per `entry_id`; settled state via `putKnowledgeEntry` | `KnowledgeEntryPort`; `orchestrateUpsert`, `orchestratePromote` |
| `input-source` | Ingest or propose entries/intent | Product-defined ingest surface (no new port family) |
| `checker` | Emit `Finding[]`; no settled `body.state` write-back | `RuleQueryPort`, `FindingPort`; `orchestrateCheck` |
| `assembler` | Closed-loop context aggregation | `ScopeQueryPort`; `orchestrateAssemble` |
| `computable-engine` | Optional L2 session/compute | `ComputablePort` when `l2-computable` declared |

Only **data-store** commits settled KnowledgeEntry state. Checker, assembler, and computable-engine emit intent or derived artifacts — write-back flows through the data-store authority.

`assembler` is closed-loop core vocabulary (not an optional role label). `computable-engine` is optional and pairs with the `l2-computable` capability flag.

### Host capabilities (open vocabulary)

`capabilities` reuses existing capability flag strings from [`spoke-protocol-layers.md`](spoke-protocol-layers.md):

| Flag | Normative pairing |
|------|-------------------|
| `spoke-baseline` | MUST appear when manifest describes a baseline-compliant adapter |
| `l2-computable` | MUST appear when `computable-engine` ∈ `roles` |
| `l5-fork` | SHOULD appear when fork-aware timeline query is advertised |

### Authority (optional)

When `authority` is present, it is a **closed** object (`additionalProperties: false`) — not a free-form opaque bag:

| Field | Required when `authority` present | Semantics |
|-------|-----------------------------------|-----------|
| `scope_key` | yes | Opaque collaboration scope for OCC / active-uniqueness (aligns with operations `scope_key` folklore in `assertUniqueActiveKnowledgeEntry`) |

`authority` is **not** schema-required when `data-store` ∈ `roles`. When absent, integrators treat this manifest's `host_id` as the implicit write authority for its collaboration scope. No CRDT or vector-clock fields on the wire.

### Namespace exclusivity

Within one **collaboration context**, each namespace string in `namespaces[]` MUST appear on **at most one** manifest (`host_id`). Integrators enforce exclusivity when assembling peer lists and routing `KnowledgeEntry.extensions.<ns>` ownership product-side.

### Manifest extensions vs KnowledgeEntry extensions

| Surface | Role |
|---------|------|
| `HostCapabilityManifest.extensions` | Required `ExtensionMap`; deployment/product metadata only |
| `KnowledgeEntry.extensions.<ns>` | Product bags only — not the host-role channel |

Manifest `extensions` MUST NOT duplicate `roles`, `capabilities`, or `namespaces`.

### Illustrative instance

```json
{
  "schema_version": 1,
  "host_id": "host_toy_primary",
  "roles": ["data-store", "checker", "assembler", "input-source"],
  "capabilities": ["spoke-baseline"],
  "namespaces": ["toy"],
  "authority": { "scope_key": "collab_toy_world" },
  "extensions": {
    "toy_world": { "display_name": "Toy World primary host" }
  }
}
```

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

**`ComputableLogChange`** (`common.schema.json#/definitions/ComputableLogChange`):

| Field | Required | Type |
|-------|----------|------|
| `path` | yes | Dot-path or JSON Pointer to changed field within `body.computable` |
| `previous` | no | `#/definitions/OpaqueJson` — any JSON value (scalar, array, object, or null) |
| `next` | no | `#/definitions/OpaqueJson` — any JSON value (scalar, array, object, or null) |

**`OpaqueJson`** (`common.schema.json#/definitions/OpaqueJson`): empty schema `{}` (draft-07 accepts any instance). Used by `ComputableLogChange.previous` / `.next`.

Generated TypeScript and Rust types MUST reflect the same opacity (not object-only maps). TypeScript: `OpaqueJson` / `unknown`. Rust: `serde_json::Value`.

---

### Dual-concern example (ontology `"event"` vs TimelineEvent)

The same story beat may appear as **both** wire shapes — products choose mapping; protocol keeps names distinct:

| Wire artifact | Example |
|---------------|---------|
| KnowledgeEntry (`entry_type: "event"`) | KB fact node “Treaty of Ashford” with structured `body` |
| TimelineEvent | When-axis placement with `timeline_scale: "narrative"` and `timeline_event_id` |

Toy-world dual-concern fixture pair: `kb_tw_harbor_dawn_event` + `evt_tw_harbor_dawn` — see [`fixtures/toy-world/`](../../fixtures/toy-world/). Beat-assisted outlining (moment-scale atoms, `precedes` order, `structural_role`) — [`domain-profile-narrative-structure.md`](domain-profile-narrative-structure.md).

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
| `body` | object | Closed L2 payload (`additionalProperties: false`); see §Body rules |
| `extensions` | object | Namespace map (§Extensions) |

### Optional protocol fields

| Field | Type | Semantics |
|-------|------|-----------|
| `revision` | integer ≥ 0 | OCC / optimistic concurrency |
| `source_anchor` | `SourceAnchor` | Provenance pointer (`$ref` to data schema) |
| `created_at` | string (RFC 3339) | Creation timestamp |
| `updated_at` | string (RFC 3339) | Last mutation timestamp |
| `modules` | `ModuleMap` | Optional cross-product functional-dialect bag; capability-flagged (`narrative-modules`); see §Modules |

### Body rules

`body` is a **closed** JSON object: `additionalProperties: false`. Only the keys below are valid on the wire. Product-specific or lossy-round-trip fields belong in `extensions.<namespace>` — not as extra `body` keys.

| Key | Required on `body` | Type | Semantics |
|-----|-------------------|------|-----------|
| `summary` | no | string | Short human blurb; `assemble` MAY emit `snippet` from trimmed non-empty `summary` (see [`spoke-operations.md`](spoke-operations.md)) |
| `tags` | no | string[] | Free-form labels |
| `attributes` | no | `BodyAttribute[]` | Trait list; duplicate `trait_type` allowed |
| `state` | no | `ComputableFieldMap` | Static durable computable state (`l2-computable` optional) |
| `computable` | no | `ComputableFieldMap` | Dynamic Session-scoped projection (`l2-computable` optional) |

Empty `body: {}` is valid for `spoke-baseline` — no L2 key is required.

### BodyAttribute

Shared JSON Schema fragment: `common.schema.json#/definitions/BodyAttribute`. ERC721-style trait item for `body.attributes[]`.

| Field | Required | Type | Semantics |
|-------|----------|------|-----------|
| `trait_type` | yes | string (`minLength: 1`) | Trait name / metadata key |
| `value` | yes | string \| number \| boolean | Scalar trait value only — no nested object or array |
| `display_type` | no | string | Optional presentation hint (e.g. `"number"`, `"date"`) |
| `max_value` | no | number | Optional numeric ceiling hint |

| Rule | Requirement |
|------|-------------|
| Item shape | `additionalProperties: false` on each trait object |
| Array level | Duplicate `trait_type` **allowed** — multi-valued metadata uses multiple items |
| Nested values | Not in `value`; use multiple traits or `extensions.<namespace>` |

```json
{
  "trait_type": "affiliation",
  "value": "Guild",
  "display_type": "string"
}
```

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
    "tags": ["pov"],
    "attributes": [
      { "trait_type": "role", "value": "protagonist" }
    ]
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

Optional: `label`, `metadata` (object, open), `revision` (integer ≥ 0, optimistic concurrency), `created_at`, `updated_at`.

### Persisted-entity OCC parity (normative guardrail)

Any SPOKE entity that (a) has a dedicated write `*Port` family and (b) is the subject of a create-or-update orchestrated op MUST carry structural `revision` (integer ≥ 0) and MUST be persisted through `put*(entity, expectedBaseRevision)` where `expectedBaseRevision` is `null`/`None` on create and the stored revision on update. This rule applies to:

| Entity | OCC port | Orchestrated op | Status |
|--------|----------|-----------------|--------|
| `KnowledgeEntry` | `KnowledgeEntryPort` (`getKnowledgeEntry` + `putKnowledgeEntry(entry, expectedBaseRevision)`) | `orchestrateUpsert`, `orchestratePromote` | **OCC parity delivered** |
| `Relation` | `RelationPort` (`getRelation` + `putRelation(relation, expectedBaseRevision)`) | `orchestrateRelate` | **OCC parity delivered** |

**Exemptions — entities that do NOT carry structural OCC or an `expectedBaseRevision` write signature:**

| Entity | Reason (architecture) |
|--------|-----------------------|
| `Finding` | Bulk checker output via `putFindings`; products replace sets atomically, not RMW a single finding id through relate-style per-id OCC (`putFindings` is a batch operation, not a create-or-update on one id) |
| `Rule` | Read/query port (`RuleQueryPort`) only; no create-or-update persistence op on the baseline write surface |
| `HostCapabilityManifest` | Read port (`HostManifestPort`) only; persistence lifecycle stays product-side |
| `TimelineEvent` | Product-owned world history; protocol owns wire shape only, not write-port OCC or a baseline persistence port |
| `SourceAnchor` | Embedded inside `KnowledgeEntry`, not independently persisted |
| `AssemblePacket` | Ephemeral op output; never persisted |

**OCC codes:** `STORED_REVISION_STALE` / `REVISION_CONFLICT` are reused across all persisted-entity update paths (upsert, promote, relate). No parallel per-entity OCC code families.

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
| `modules` | no | `ModuleMap` — optional cross-product functional-dialect bag; capability-flagged (`narrative-modules`); see §Modules |

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

## Modules (normative)

Optional cross-product **functional-dialect** bag on `KnowledgeEntry` and `AssemblePacket`. Distinct from product-owned `extensions`.

```json
"modules": {
  "activation": { },
  "placement": [ ]
}
```

| Rule | Requirement |
|------|-------------|
| Presence | Optional on KnowledgeEntry + AssemblePacket; **not** required; absent and empty valid |
| Capability | Opt-in via `narrative-modules` ([`spoke-protocol-layers.md`](spoke-protocol-layers.md)) |
| Namespace keys | Functional-dialect ids — `^[a-z][a-z0-9_-]*$` (e.g. `activation`, `pack`, `placement`, `activation_trace`) |
| Values | Structured JSON — object **or** array (`ModuleMap` `anyOf`); inner field tables handbook-defined |
| Unknown namespaces | Adapters MUST preserve on round-trip |
| Merge / preserve semantics | [`spoke-operations.md`](spoke-operations.md) (`mergeModuleMaps`, `preserveModuleMaps`) |
| Category | Functional dialects ∈ `modules.*`; product bags ∈ `extensions.<product>` |

Shared JSON Schema fragment: `common.schema.json#/definitions/ModuleMap`.

Bag placement for product `extensions` vs cross-product `modules.*`: [`spoke-extension-modules.md`](spoke-extension-modules.md).

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
| `beat` | **Profile-only** | [`domain-profile-narrative-structure.md`](domain-profile-narrative-structure.md) |
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

**Status transitions (cross-product minimum):** enforced by `@42ch/spoke-operations` — see [`spoke-operations.md` §KnowledgeEntry lifecycle](spoke-operations.md#6-knowledgeentry-lifecycle--knowledge-entry). Wire schema keeps `status` as open string; library enforces the core transition table. **Active** statuses for uniqueness: `provisional`, `confirmed` only.

**`deprecated` → `merged` excluded:** merge absorbs an active canonical KnowledgeEntry into a target; a deprecated row is already superseded — restore to `confirmed` (or merge from `provisional`/`confirmed`) before absorb.

### Core `relation_type` vocabulary (starter set)

`related_to`, `parent_of`, `member_of`, `located_in`, `participates_in`, `causes`, `foreshadows`

### Extension policy summary

| Concern | Rule |
|---------|------|
| Unknown values | Round-trip verbatim |
| Closed enums | Open vocabulary; per-product tables published in adapter specs / Showcases |
| Product-only types | `extensions.<namespace>` or open `entry_type` string |
| Documentation | Adapter specs and Showcases own per-product tables |

---

## Vocabulary boundaries (CONCEPTS alignment)

- **KnowledgeEntry** — atomic Knowledge Base entry in SPOKE wire form
- **Scope** — shared `Scope` object (`scope_id` required) for `check` / `assemble`; optional `extensions` (`ExtensionMap`) carries product-scoped query metadata (matchers ignore); World/Book ids in op `extensions`, `Scope.extensions`, or adapters — full field table in [`spoke-ops.md`](spoke-ops.md) §Scope
- **TimelineScale** — L5 tier vocabulary (`brief` / `narrative` / `moment`) on `TimelineEvent` and optional `Scope` filter — see §TimelineScale
- **ForkId** — opaque branch identity (`l5-fork`) on `TimelineEvent.fork_id`, `TimelineEvent.parent_fork_id`, and optional `Scope.fork_id` — see §Fork fields
- **Domain Profile** — published ontology vocabulary per product/integration; core `entry_type` stays open string — see [`spoke-protocol-layers.md`](spoke-protocol-layers.md); narrative-structure / Beat mapping — [`domain-profile-narrative-structure.md`](domain-profile-narrative-structure.md); lore-activation (`modules.activation`) — [`domain-profile-lore-activation.md`](domain-profile-lore-activation.md)
- **TimelineEvent** — L5 temporal wire object (when-axis); distinct from KnowledgeEntry `entry_type: "event"` labels
- **Session** — optional `l2-computable` lifecycle (not `entry_type`, not durable wire object); see §Computable body
- **ComputableFieldMap** — open object for `body.state` and `body.computable` under `l2-computable`
- **BodyAttribute** — scalar trait item in `body.attributes[]` (`trait_type` + `value`; optional `display_type`, `max_value`)
- **ComputableLogEntry** — Moment-scale presentation on `TimelineEvent.computable_logs` (not Finding)
- **World KB / Author Memory** — product-local stores; mapped via adapters in a later iteration, not redefined here
- **Finding** — checker output, not a KnowledgeEntry body
- **HostCapabilityManifest** — in-process adapter self-description (`host_id`, `roles`, `capabilities`, `namespaces`); distinct from KnowledgeEntry
- **Host role** — open string in `HostCapabilityManifest.roles[]`; core vocabulary: `data-store`, `input-source`, `checker`, `assembler`, `computable-engine`
- **Namespace attribution** — integrator maps manifest `namespaces[]` → owning `host_id` for `extensions.<ns>` folklore; exclusivity per collaboration context

---

## Acceptance (data layer)

- [x] Each **committed** baseline and optional-capability wire object in this doc (KnowledgeEntry through TimelineEvent) has a draft-07 schema under `schemas/data/` (or `schemas/common/` for shared defs)
- [x] Umbrella + this doc list the same object set; `Rule` and `TimelineEvent` schemas committed
- [x] `HostCapabilityManifest` schema committed at `schemas/data/host-capability-manifest.schema.json` with field rules in §HostCapabilityManifest
- [x] `entry_type` / `status` fields are `type: string` without `enum`; core vocabulary appears in `description`

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
| [`spoke-extension-modules.md`](spoke-extension-modules.md) | Core / modules / extensions naming triad |
| [`domain-profile-narrative-structure.md`](domain-profile-narrative-structure.md) | Narrative-structure Domain Profile — Beat mapping |
| [`domain-profile-lore-activation.md`](domain-profile-lore-activation.md) | Lore-activation Domain Profile — `modules.activation` |
| [`spoke-ops.md`](spoke-ops.md) | Ops that consume these data shapes (`check`, `assemble`, …) |
| [`spoke-operations.md`](spoke-operations.md) | Lifecycle helpers (extensions, Finding status, promote, AssemblePacket builders) |
| [`schemas/README.md`](../../schemas/README.md) | Schema file checklist |
| [`CONCEPTS.md`](../../CONCEPTS.md) | Vocabulary boundaries (KnowledgeEntry vs product stores) |
