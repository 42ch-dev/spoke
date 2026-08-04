---
title: Ops wire reference
---

# Ops wire reference

The ops layer defines transport-agnostic request/response envelopes for core KnowledgeEntry operations. Products carry these JSON payloads over any transport — in-process calls, message queues, or HTTP mappings inside adapters — and the wire stays transport-agnostic. Field tables below trace to the committed schemas in [`schemas/ops/`](https://github.com/42ch-dev/spoke/tree/main/schemas/ops).

## Baseline operations

| Op | Request | Response | Semantics |
|----|---------|----------|-----------|
| `upsert` | `UpsertRequest` | `UpsertResponse` | Create or update KnowledgeEntries by stable id (1..n entries; optional idempotency key) |
| `extract→promote` | `PromoteRequest` | `PromoteResponse` | Promote an extracted candidate to a durable KnowledgeEntry (optional merge target) |
| `relate` | `RelateRequest` | `RelateResponse` | Create or update a Relation |
| `check` | `CheckRequest` | `CheckResponse` | Run checker(s) over a `Scope`; returns `Finding[]` (rules via `rule_refs` and/or embedded `rules[]`) |
| `assemble` | `AssembleRequest` | `AssembleResponse` | Return an `AssemblePacket` for a `Scope` (structure only) |

Each operation has a paired request and response schema. Optional `project` / `compute` families under `l2-computable` add Session-scoped computable I/O.

## One failure dialect

Every response is a `oneOf` of the success payload or `{ "error": ErrorEnvelope }` — success and error branches are mutually exclusive:

| ErrorEnvelope field | Type | Notes |
|---------------------|------|-------|
| `code` | string, required | Machine-readable error code (open vocabulary) |
| `message` | string, required | Human-readable error message |
| `details` | open object, optional | Structured error context |
| `extensions` | ExtensionMap, required | Product namespace bag |

Expected rejects from the operations library arrive as `SpokeResult` with stable `SpokeRejectCode` strings shared between TypeScript and Rust (`REVISION_CONFLICT`, `STORED_REVISION_STALE`, `CANDIDATE_NOT_PROVISIONAL`, `CANDIDATE_TERMINAL_STATUS`, `EMPTY_CANONICAL_NAME`, `RELATION_SELF_EDGE`, `RELATION_MISSING_ENDPOINT`, `CAPABILITY_PORT_MISSING`, `INTERNAL_ERROR`, …).

## Scope selector

`check` and `assemble` share the `Scope` selector. Required `scope_id`; all refinements optional:

| Field | Type | Notes |
|-------|------|-------|
| `scope_id` | string, required | Protocol-neutral opaque selector. Products map World / Book / chapter / manuscript ids via adapters or op extensions |
| `entry_ids` | string[] | Narrow scope to explicit KnowledgeEntries |
| `entry_types` | string[] | Filter by open `entry_type` vocabulary |
| `timeline_event_ids` | string[] | Narrow scope to explicit L5 TimelineEvent ids |
| `source_id` | string | Provenance or manuscript locator scope |
| `timeline_scale` | TimelineScale | L5 tier filter (`brief`, `narrative`, `moment`) |
| `fork_id` | ForkId | L5 branch filter — strict equality on `TimelineEvent.fork_id` (`l5-fork`) |
| `extensions` | ExtensionMap | Product-scoped query metadata; protocol matchers ignore it, adapters round-trip it |

## Envelope field tables

### UpsertRequest / UpsertResponse

`UpsertRequest` required: `knowledge_entries`. Response: `{ knowledge_entries: [...] }` **or** `{ error }`.

| Field | Notes |
|-------|-------|
| `knowledge_entries` | KnowledgeEntries to create or update |
| `idempotency_key` | Opaque idempotency hint (no server semantics in protocol v0.1) |
| `extensions` | Optional transport metadata |

### PromoteRequest / PromoteResponse

`PromoteRequest` required: `candidate`. Response: `{ knowledge_entry, superseded_id? }` **or** `{ error }`.

| Field | Notes |
|-------|-------|
| `candidate` | Candidate KnowledgeEntry (typically status `provisional`) |
| `target_entry_id` | Optional merge target KnowledgeEntry id; the response then carries `superseded_id` |

### RelateRequest / RelateResponse

`RelateRequest` required: `relation`. Response: `{ relation }` **or** `{ error }`.

| Field | Notes |
|-------|-------|
| `relation` | Relation to create or update (OCC via `revision`) |

### CheckRequest / CheckResponse

`CheckRequest` required: `scope`. Response: `{ findings: [...] }` **or** `{ error }`.

| Field | Notes |
|-------|-------|
| `scope` | Checker scope selector |
| `rule_refs` | Opaque rule ids or URIs; resolved by the receiver when not overridden by `rules[]` |
| `rules` | Optional embedded Rule objects for portable interchange (override by `rule_id`) |
| `checker_kinds` | Optional checker kind filters |
| `extensions` | Optional transport metadata |

### AssembleRequest / AssembleResponse

`AssembleRequest` required: `scope`. Response: `{ packet }` **or** `{ error }`.

| Field | Notes |
|-------|-------|
| `scope` | Assembly scope selector |
| `max_entries` | Optional entry limit hint (not enforced by the protocol) |
| `extensions` | Optional transport metadata |

## Shared rules

- **Check ≠ Assemble** — `check` returns findings only; `assemble` returns a packet only.
- **`$ref` composition** — ops schemas `$ref` data-layer types, with each type defined once.
- **Purity** — the operations library is pure relative to host I/O: storage access, LLM calls, ranking, retrieval, and transport binding are supplied by products through injected adapter ports. The library runs the protocol gates; adapters own persistence.

## Related

- [Protocol reference](/reference/protocol) — the three columns and capability flags.
- [Orchestrate operations](/how-to/orchestrate-ops) — calling each orchestrator with real signatures.
- [Data model reference](/reference/data-model) — the objects these envelopes carry.
- [Connect reference](/reference/connect) — ops envelopes wrapped as opaque invoke payloads.
