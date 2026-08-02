---
title: Ops wire
---

# Ops wire

The ops layer defines transport-agnostic request/response envelopes for core KnowledgeEntry operations. Products carry these JSON payloads over any transport — in-process calls, message queues, or HTTP mappings inside adapters; the wire itself carries no transport fields.

## Baseline operations

- **upsert** — create or update KnowledgeEntries by stable id (1..n entries; optional idempotency key).
- **extract→promote** — promote an extracted candidate to a durable KnowledgeEntry (`promote-*` family; optional merge target).
- **relate** — create or update a Relation.
- **check** — run checker(s) over a `Scope` and return `Finding[]` (rules supplied by `rule_refs` and/or embedded `rules[]`).
- **assemble** — return an `AssemblePacket` for a `Scope` (structure only).

Each operation has a paired request and response schema. Optional `project` / `compute` families (under `l2-computable`) add Session-scoped computable I/O.

## Shared rules

- **Scope selector** — `check` and `assemble` require a shared `Scope` with an opaque `scope_id` plus optional refinements (`entry_ids`, `entry_types`, `timeline_event_ids`, `source_id`, `timeline_scale`, `fork_id`, `extensions`). World / book / product ids map via `extensions` or adapters.
- **One failure dialect** — every response is a `oneOf` of the success payload or `{ "error": ErrorEnvelope }`; success and error branches are mutually exclusive.
- **Check ≠ Assemble** — `check` returns findings only; `assemble` returns a packet only.
- **No inline copies** — ops schemas `$ref` data-layer types instead of duplicating them.

## Minimal request sketch

```json
// Illustrative only — the committed schemas under schemas/ops/ are authoritative.
{
  "scope": { "scope_id": "book-harbor", "entry_types": ["character"] },
  "rules": [
    { "rule_id": "r-1", "kind": "rule", "canonical_name": "foreshadow", "extensions": {} }
  ]
}
```

## Normative references

- [spoke-ops.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-ops.md) — full per-operation contracts, Scope, error envelope, optional ops
- [schemas/ops/](https://github.com/42ch-dev/spoke/tree/main/schemas/ops) — committed ops schemas
- [spoke-operations.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-operations.md) — lifecycle gates and orchestration over the wire
- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) — Scope and error-envelope vocabulary
