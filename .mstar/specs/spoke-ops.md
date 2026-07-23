# SPOKE Operations

> **Status:** Normative (v0.1 baseline; v0-iter003 harden)  
> **Document class:** Detail — ops **wire** layer (column 2)  
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)  
> **Schema home:** `schemas/ops/`, `schemas/common/`  
> **Lifecycle behavior:** [`spoke-operations.md`](spoke-operations.md) (column 3 — hand-written library, not wire)

## Purpose

Define transport-agnostic **request/response** wire shapes for core Keyblock operations. Ops schemas are separate families from data envelopes in `schemas/data/`.

**Integrator framing (v0-iter003):**

| Principle | Meaning |
|-----------|---------|
| **Check ≠ Assemble** | `check` runs checkers and returns `Finding[]`; `assemble` returns `AssemblePacket` only. No merged op, no ranking fields in assemble wire. |
| **Scope neutrality** | Shared `Scope` def in `common.schema.json` (architect-locked target wire — **`ops-harden` plan**); required `scope_id` (opaque protocol-neutral string). World/Book/product scope ids go in op `extensions` or adapters — not `Scope` required fields. |
| **One failure dialect** | All ops responses use `oneOf` success branch **or** `{ "error": ErrorEnvelope }` — same attachment as v0.1 `assemble-response` (R3 closed). |

**Transport note:** SPOKE ops are **not** HTTP routes, gRPC services, or MCP tools. They are JSON payloads products may carry over any transport (in-process function args, message queue, future REST mapping). Binding to HTTP paths, status codes, or auth headers is explicitly out of scope.

---

## Core operations (v0.1)

Five ops, ten schema files under `schemas/ops/` (request + response each).

| Operation | Intent | Request schema | Response schema |
|-----------|--------|----------------|-----------------|
| **upsert** | Create or update Keyblock(s) by stable id | `upsert-request.schema.json` | `upsert-response.schema.json` |
| **extract→promote** | Promote extracted candidate → durable Keyblock | `promote-request.schema.json` | `promote-response.schema.json` |
| **relate** | Create or update Relation | `relate-request.schema.json` | `relate-response.schema.json` |
| **check** | Run checker(s); return Finding(s) | `check-request.schema.json` | `check-response.schema.json` |
| **assemble** | Return `AssemblePacket` shape | `assemble-request.schema.json` | `assemble-response.schema.json` |

File basename `promote-*` is the schema id for **extract→promote** (shorter than `extract-promote-*`).

---

## Request/response families

### Shared rules

- Every op defines paired **request** and **response** schemas.
- Ops MUST `$ref` data-layer types (`Keyblock`, `Relation`, `Finding`, `AssemblePacket`) — no duplicated inline copies of those objects.
- Ops MAY include an optional top-level `extensions` object (same `ExtensionMap` as data layer) for transport metadata products choose to standardize later.
- Ops MUST NOT embed product-specific payloads as protocol siblings on nested data objects; use `extensions` on those objects.
- **Error path (architect-locked):** every `*-response.schema.json` uses `oneOf` — **success variant** (op-specific payload) **or** **failure variant** (`error` → `$ref` `error-envelope.schema.json`). Success responses MUST NOT include `error`. Failure responses MUST NOT include success payload fields (`findings`, `packet`, `keyblocks`, …). Optional top-level `extensions` allowed on both branches.

### Scope (shared — `check` + `assemble`)

> **Target wire (`ops-harden` plan):** Field table below is architect-locked normative text. Committed `schemas/` today uses inline `CheckScope` / `AssembleScope` without `scope_id`, `event_ids`, or `timeline_scale`. Shared `Scope` + `TimelineScale` defs land in `common.schema.json` and `check-request` / `assemble-request` `$ref` updates ship in sibling plan **`ops-harden`** — not yet in tree (17 files committed).

Definition: `schemas/common/common.schema.json#/definitions/Scope`. Both `check-request` and `assemble-request` require top-level `scope` referencing this def.

| Field | Required | Type | Semantics |
|-------|----------|------|-----------|
| `scope_id` | **yes** | string | Protocol-neutral opaque selector. Products map World/Book/chapter/manuscript ids via adapters or op `extensions` — **not** as required `Scope` siblings. |
| `keyblock_ids` | no | string[] | Narrow scope to explicit Keyblocks |
| `block_types` | no | string[] | Filter by open `block_type` vocabulary |
| `event_ids` | no | string[] | Narrow to explicit L5 `Event` ids |
| `source_id` | no | string | Provenance / manuscript locator scope |
| `timeline_scale` | no | `TimelineScale` | L5 tier filter (`brief` / `narrative` / `moment`) |

**Mapping rule:** when a product needs `world_id`, `book_id`, or similar, it MUST use either:

1. `request.extensions.<namespace>.world_id` (or equivalent), or  
2. Adapter-side resolution from `scope_id` to product stores.

Core ops schemas MUST NOT add `world_id`, `book_id`, `manuscript_id`, or product-prefixed ids as required `Scope` fields.

### Per-operation contract (field-level intent)

#### upsert

| Direction | Core payload |
|-----------|--------------|
| Request | `keyblocks: Keyblock[]` (1..n); optional `idempotency_key: string` (opaque; no server semantics in v0.1) |
| Response (success) | `keyblocks: Keyblock[]` (persisted view); optional `rejected: { keyblock_id, code, message }[]` |
| Response (failure) | `error: ErrorEnvelope`; optional `extensions` |

#### extract→promote (`promote-*`)

| Direction | Core payload |
|-----------|--------------|
| Request | `candidate: Keyblock` (typically `status: provisional`); optional `target_keyblock_id` for merge |
| Response (success) | `keyblock: Keyblock` (promoted); optional `superseded_id` when merging |
| Response (failure) | `error: ErrorEnvelope`; optional `extensions` |

Pure promote gates and revision bump before persist: [`spoke-operations.md` §Promote acceptance](spoke-operations.md#3-promote-acceptance--promote).

#### relate

| Direction | Core payload |
|-----------|--------------|
| Request | `relation: Relation` |
| Response (success) | `relation: Relation` |
| Response (failure) | `error: ErrorEnvelope`; optional `extensions` |

#### check

| Direction | Core payload |
|-----------|--------------|
| Request | `scope: Scope` (required); optional `rule_refs: string[]`; optional `rules: Rule[]` (full `$ref` to `rule.schema.json`); optional `checker_kinds: string[]`; optional `extensions` |
| Response (success) | `findings: Finding[]`; optional `extensions` |
| Response (failure) | `error: ErrorEnvelope`; optional `extensions` |

**Rule input pattern (architect-locked):** support **both** `rule_refs` and embedded `rules[]`.

| Mechanism | When to use |
|-----------|-------------|
| `rule_refs` | Receiver already stores Rules by id; ids are opaque strings or URIs |
| `rules[]` | Portable interchange — full declarative `Rule` objects inline |
| Both present | For each `rule_id`, embedded object in `rules[]` **wins** over the matching ref; refs without a matching embed are resolved by the receiver |

No separate “slim embed” type — `Rule` schema is intentionally lean (no checker runtime fields). Do not embed `Finding` shapes in `check` requests.

**Product rule:** `check` MUST NOT return `AssemblePacket`. Checker execution engines remain product-local; the wire carries inputs/outputs only.

#### assemble

| Direction | Core payload |
|-----------|--------------|
| Request | `scope: Scope` (required); optional `max_entries: integer` (hint only); optional `extensions` |
| Response (success) | `packet: AssemblePacket` |
| Response (failure) | `error: ErrorEnvelope`; optional `extensions` |

**Product rule:** `assemble` MUST NOT return `Finding[]` or imply ranking/retrieval in required protocol fields.

---

## `assemble` wire-only boundary (normative)

v0.1 standardizes **only** the `AssemblePacket` shape exchanged when a product performs context assembly. The protocol does **not** specify how packets are produced.

### In scope (MUST be in schemas)

- `assemble-request.schema.json` — selector hints (`scope`, optional limits)
- `assemble-response.schema.json` — `{ "packet": AssemblePacket }` success shape
- `assemble-packet.schema.json` (data layer) — `entries[]` slim shape or explicit `$ref` to full Keyblock

### Out of scope (MUST NOT appear in v0.1 protocol schemas)

| Excluded concern | Where it lives |
|------------------|----------------|
| Ranking / scoring algorithms | Product-local |
| Vector retrieval, embedding search | Product-local |
| Token budgeting / trimming | Product-local |
| LLM prompt assembly | Product-local |
| Caching, staleness, refresh policy | Product-local |
| Daemon routes or CLI subcommands | Adapter / product repos |

### Hard rules

1. **`assemble` response MUST NOT** require fields that imply compute products must implement (e.g. mandatory `rank_scores`, `embedding_model`, `retrieval_trace`).
2. **Products MAY** place compute metadata under `AssemblePacket.extensions.<namespace>` — adapters MUST preserve unknown keys.
3. **Conformance in v0.1** means a product can emit or consume a valid `AssemblePacket` on the wire, not that it runs assembly the same way as another product.
4. **No `assemble` "engine" schema** — there is no `AssembleComputeRequest` or pipeline config in v0.1.

---

## Error envelope

`schemas/common/error-envelope.schema.json`:

| Field | Required | Type |
|-------|----------|------|
| `code` | yes | string (machine-readable) |
| `message` | yes | string (human-readable) |
| `details` | no | object (open) |
| `extensions` | yes | object |

### Attachment pattern (architect-locked — R3)

All five ops response schemas MUST use the same discriminated union:

```json
{
  "oneOf": [
    { "required": ["<success-payload>"], "properties": { "...": "..." }, "additionalProperties": false },
    {
      "required": ["error"],
      "properties": {
        "error": { "$ref": "error-envelope.schema.json" },
        "extensions": { "$ref": "ExtensionMap" }
      },
      "additionalProperties": false
    }
  ]
}
```

| Op | Success branch required field(s) | Notes |
|----|----------------------------------|-------|
| `upsert` | `keyblocks` | `rejected[]` stays on **success** branch (partial business reject, not transport failure) |
| `promote` | `keyblock` | — |
| `relate` | `relation` | — |
| `check` | `findings` | Empty array is valid success |
| `assemble` | `packet` | v0.1 reference implementation |

**Invariant:** `error` and success payload fields MUST NOT co-exist on the same response object.

HTTP mapping (4xx/5xx) is adapter concern.

---

## Relationship to adapters and operations library

v0.1 delivers **ops wire** shapes only. Cross-product lifecycle rules (promote acceptance, Finding status transitions, extension preserve, AssemblePacket builders) live in [`spoke-operations.md`](spoke-operations.md) — adapters and product code MUST call `@42ch/spoke-operations` instead of reimplementing those invariants.

Mapping Nexus daemon routes or Creader API handlers to these wire payloads remains a **follow-on** adapter concern (`@42ch/spoke-operations` delivered in v0-iter002).

---

## Acceptance (ops layer)

- [ ] Each operation above has request + response schemas under `schemas/ops/`
- [ ] `.mstar/specs/spoke-ops.md` and `schemas/ops/` enumerate the same op set (5 ops, 10 schema files)
- [ ] `assemble` response `$ref`s `AssemblePacket` from the data layer
- [ ] `schemas/common/error-envelope.schema.json` exists and is referenced by **all** ops response schemas (R3)
- [ ] `check-request` / `assemble-request` `$ref` shared `Scope` from `common.schema.json`
- [ ] `check-request` supports `rule_refs` and `rules[]` per architect lock
- [ ] No transport-specific fields (HTTP method, URL path, gRPC service name) in ops schemas

## Non-goals (ops layer)

- HTTP/gRPC/MCP transport bindings
- Server or daemon implementation
- Checker execution engine
- Assemble ranking / retrieval algorithms
- Conformance fixtures or golden round-trips
- Adapter route mapping (post–v0-iter003)

## See also

| Doc | Topic |
|-----|-------|
| [`spoke-protocol.md`](spoke-protocol.md) | Umbrella framing and v0.1 acceptance |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | L0–L8, capability levels, Check≠Assemble framing |
| [`spoke-data-model.md`](spoke-data-model.md) | Data types referenced by ops (`Keyblock`, `AssemblePacket`, `Rule`, `Event`, …) |
| [`spoke-operations.md`](spoke-operations.md) | Hand-written lifecycle helpers on top of wire types (column 3) |
| [`schemas/README.md`](../../schemas/README.md) | Ten op schema files under `schemas/ops/` |
| [`STRATEGY.md`](../../STRATEGY.md) | Protocol-not-runtime; ops are transport-agnostic payloads |
