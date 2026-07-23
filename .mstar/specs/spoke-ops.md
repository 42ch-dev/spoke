# SPOKE Operations

> **Status:** Draft (iteration v0.1)  
> **Document class:** Detail — ops / behavioral layer  
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)  
> **Schema home:** `schemas/ops/`, `schemas/common/`

## Purpose

Define transport-agnostic **request/response** wire shapes for core Keyblock operations. Ops schemas are separate families from data envelopes in `schemas/data/`.

**Transport note:** SPOKE ops are **not** HTTP routes, gRPC services, or MCP tools in v0.1. They are JSON payloads products may carry over any transport (in-process function args, message queue, future REST mapping). Binding to HTTP paths, status codes, or auth headers is explicitly out of scope.

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
- **Error path:** failures use `schemas/common/error-envelope.schema.json` (shared across ops). Success responses do not embed error fields.

### Per-operation contract (field-level intent)

#### upsert

| Direction | Core payload |
|-----------|--------------|
| Request | `keyblocks: Keyblock[]` (1..n); optional `idempotency_key: string` (opaque; no server semantics in v0.1) |
| Response | `keyblocks: Keyblock[]` (persisted view); optional `rejected: { keyblock_id, code, message }[]` |

#### extract→promote (`promote-*`)

| Direction | Core payload |
|-----------|--------------|
| Request | `candidate: Keyblock` (typically `status: provisional`); optional `target_keyblock_id` for merge |
| Response | `keyblock: Keyblock` (promoted); optional `superseded_id` when merging |

#### relate

| Direction | Core payload |
|-----------|--------------|
| Request | `relation: Relation` |
| Response | `relation: Relation` |

#### check

| Direction | Core payload |
|-----------|--------------|
| Request | `scope: { keyblock_ids?: string[], source_id?: string }`; optional `rule_refs: string[]` (opaque ids; **no `Rule` object** in v0.1 — see [data model §Rule deferral](spoke-data-model.md#rule-deferral-v01-decision)); optional `checker_kinds: string[]` |
| Response | `findings: Finding[]` |

#### assemble

| Direction | Core payload |
|-----------|--------------|
| Request | `scope: { keyblock_ids?: string[], block_types?: string[], source_id?: string }`; optional `max_entries: integer` (hint only) |
| Response | `packet: AssemblePacket` (see [data model §AssemblePacket](spoke-data-model.md#assemblepacket)) **or** `error: ErrorEnvelope` when a product chooses to signal failure on the wire |

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

Ops responses use **either** the success shape **or** wrap `error: ErrorEnvelope` at the top level — not both. HTTP mapping (4xx/5xx) is adapter concern.

---

## Relationship to adapters

v0.1 delivers schema shapes only. Mapping Nexus daemon routes or Creader API handlers to these ops is a **next-iteration** adapter concern.

---

## Acceptance (ops layer)

- [ ] Each operation above has request + response schemas under `schemas/ops/`
- [ ] `.mstar/specs/spoke-ops.md` and `schemas/ops/` enumerate the same op set (5 ops, 10 schema files)
- [ ] `assemble` response `$ref`s `AssemblePacket` from the data layer
- [ ] `schemas/common/error-envelope.schema.json` exists and is referenced by ops schemas
- [ ] No transport-specific fields (HTTP method, URL path, gRPC service name) in ops schemas

## Non-goals (ops layer, v0.1)

- HTTP/gRPC/MCP transport bindings
- Server or daemon implementation
- Checker execution engine
- Assemble ranking / retrieval algorithms
- Conformance fixtures or golden round-trips

## See also

| Doc | Topic |
|-----|-------|
| [`spoke-protocol.md`](spoke-protocol.md) | Umbrella framing and v0.1 acceptance |
| [`spoke-data-model.md`](spoke-data-model.md) | Data types referenced by ops (`Keyblock`, `AssemblePacket`, …) |
| [`schemas/README.md`](../../schemas/README.md) | Ten op schema files under `schemas/ops/` |
