---
title: Ops wire reference
---

# Ops wire reference

The ops layer defines transport-agnostic request/response envelopes for core KnowledgeEntry operations. Products carry these JSON payloads over any transport — in-process calls, message queues, or HTTP mappings inside adapters — and the wire stays transport-agnostic. Field tables below trace to the committed schemas in [`schemas/ops/`](https://github.com/42ch-dev/spoke/tree/main/schemas/ops).

<img class="diagram-light" alt="KnowledgeEntry status transitions and terminal states" src="/diagrams/knowledge-entry-status.light.svg">
<img class="diagram-dark" alt="KnowledgeEntry status transitions and terminal states" src="/diagrams/knowledge-entry-status.dark.svg">

## Baseline operations

| Op | Request | Response | Semantics |
|----|---------|----------|-----------|
| `upsert` | `UpsertRequest` | `UpsertResponse` | Create or update KnowledgeEntries by stable id (1..n entries; optional idempotency key) |
| `extract→promote` | `PromoteRequest` | `PromoteResponse` | Promote an extracted candidate to a durable KnowledgeEntry (optional merge target) |
| `relate` | `RelateRequest` | `RelateResponse` | Create or update a Relation |
| `check` | `CheckRequest` | `CheckResponse` | Run checker(s) over a `Scope`; returns `Finding[]` (rules via `rule_refs` and/or embedded `rules[]`) |
| `assemble` | `AssembleRequest` | `AssembleResponse` | Return an `AssemblePacket` for a `Scope` (structure only) |

Each operation has a paired request and response schema. Optional families add their own pairs: `project` / `compute` under `l2-computable`, `extract` under `ke-extraction`, and the 33-op `spoke-memory` family.

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
| `viewpoint` | string | Reader context (`ke-ownership`) — the reader's holder KnowledgeEntry `entry_id`; absent names no subject and grants no private visibility |
| `cognition` | CognitionSelection | Cognitive refinement (`spoke-cognition`) — requires the negotiated flag and serves the `current` view only; see [Cognition selection](#cognition-selection-spoke-cognition) |
| `extensions` | ExtensionMap | Product-scoped query metadata; protocol matchers ignore it, adapters round-trip it |

List refinements (`entry_ids`, `entry_types`, `timeline_event_ids`) match by presence: an omitted list is absent and matches everything, while an explicitly present list — including `[]` — restricts the result, so `[]` matches nothing.

## Cognition selection (`spoke-cognition`)

`Scope.cognition` carries a `CognitionSelection` on `check`, `assemble` and the scoped KnowledgeEntry / TimelineEvent reads. This carrier serves the `current` view only: `{ view: "current", filter, selection_policy }`. The `recorded_at` form belongs to the Memory source view; supplying it here rejects the whole request instead of returning an empty projection.

The selection is applied after authorization and before projection or any checker side effect: a KnowledgeEntry is kept only when at least one carried record satisfies the complete one-record filter conjunction, and entries carrying no matching record are excluded. `filter` is the closed grammar in [Data model reference §Selection and projection](/reference/data-model#selection-and-projection) — every supplied predicate is ANDed on the same record, and no predicate joins across records or targets.

`selection_policy` is required in the `current` form and names the product policy as `{ policy_id, policy_version }`. The product supplies the algorithm; the library verifies only that `selected_record_id` belongs to the traced `input_record_ids`. The result is attribution-preserving: every item keeps its own `holder`, `target` and `assessment_context`, and retained disagreements are never averaged or replaced.

An unknown predicate, an unimplemented dimension or an unavailable view rejects the entire request — no silent stripping, partial response or unfiltered fallback:

```json
{
  "error": {
    "code": "invalid_input",
    "message": "Unsupported cognition requirement",
    "details": { "reason": "unsupported_cognition_requirement", "field": "scope.cognition.filter.q" }
  }
}
```

Requests carrying `Scope.cognition` require the negotiated `spoke-cognition` flag; when the negotiation lacks it, the request takes the existing `capability_missing` branch with no handler side effect. See [Protocol reference §Cognition negotiation and rejection](/reference/protocol#cognition-negotiation-and-rejection).

## Envelope field tables

### UpsertRequest / UpsertResponse

`UpsertRequest` required: `knowledge_entries`. Response: `{ knowledge_entries: [...], rejected? }` **or** `{ error }`. `rejected[]` carries per-entry `{ entry_id, code, message }` on the success branch.

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

When `scope.cognition` is present, `packet.cognition` is a `CognitionProjection` `{ view, items[], extensions }` selected over the authorized source set. Each item carries its own `holder`, `target`, `assessment_context`, `presence` (`assessed` or `no_record`) and `assessments[]`; a `current` assessed item also carries `selection: { policy, input_record_ids[], selected_record_id }`. The projection is computed before `max_entries` truncation — the limit narrows the slim packet entries, never which authorized cognition was requested — and is absent when the request carries no cognitive selection. See [Data model reference §Selection and projection](/reference/data-model#selection-and-projection).

## Optional op (`ke-extraction`)

`extract` is an optional op family under the `ke-extraction` capability flag, provided by hosts in the existing `input-source` role; the five baseline op families stay unchanged. It proposes provisional `KnowledgeEntry` candidates from referenced source material.

| Op | Intent | Request | Response |
|----|--------|---------|----------|
| `extract` | Propose provisional `KnowledgeEntry` candidates from referenced source material | `ExtractRequest` | `ExtractResponse` |

### ExtractRequest / ExtractResponse

`ExtractRequest` required: `run_id`, `sources`. Input is reference-only: the request carries source anchors with their optional spans.

| Field | Notes |
|-------|-------|
| `run_id` | Non-empty opaque correlation identity; echoed verbatim as `run.run_id` |
| `sources` | Non-empty `SourceAnchor[]`; the extraction range is this list plus each anchor's optional span (an absent span means the referenced artifact as a whole) |
| `entry_types` | Optional advisory hints naming the candidate types the caller expects; the serving host may use them or ignore them |
| `extensions` | Optional transport metadata |

`ExtractResponse` carries one branch: success `{ candidates, run }` — **or** failure `{ error }`.

| Success field | Notes |
|---------------|-------|
| `candidates` | `KnowledgeEntry[]` — every returned candidate carries `status: "provisional"`; an empty array is a valid zero-result run |
| `run` | `ExtractionRunMetadata` — required `run_id` (the exact request echo), optional non-empty `method`, optional opaque `coverage_hint` |
| `extensions` | Optional transport metadata |

| Failure field | Notes |
|---------------|-------|
| `error` | `ErrorEnvelope` — the response's failure branch for this run |
| `extensions` | Optional transport metadata |

**Provisional invariant.** The library admits only candidates whose `status` is `provisional`. When any candidate carries another status, the whole set is rejected and the error branch is returned — `CANDIDATE_TERMINAL_STATUS` for a `merged` / `deleted` entry, `CANDIDATE_NOT_PROVISIONAL` for any other status; every candidate that is admitted keeps the status the extractor produced, and a success response echoes the request's `run_id` verbatim.

**`extract` vs `extract→promote`.** The baseline `extract→promote` row covers admission: `promote` admits one provisional candidate to durable storage. The optional `extract` op covers production: it proposes provisional candidates from referenced sources. Extraction output reaches durable storage through promote.

## Optional family (`spoke-memory`)

Thirty-three ops across three capability flags for per-subject memory. Every op has a paired request/response schema named `memory-<segments>-{request,response}.schema.json` — `memory.records.promote` becomes `memory-records-promote-request.schema.json` / `memory-records-promote-response.schema.json`.

| Capability | Ops |
|------------|-----|
| `spoke-memory` | 25 core ops: `memory.describe`, `memory.spaces.list` / `.freeze` / `.unfreeze`, `memory.subjects.ensure` / `.get` / `.fence`, `memory.grants.list` / `.grant` / `.revoke`, `memory.capture`, `memory.candidates.list` / `.get` / `.dismiss` / `.review`, `memory.records.create` / `.get` / `.update` / `.delete` / `.list` / `.promote`, `memory.recall`, `memory.profile.get` / `.update`, `memory.receipts.get` |
| `spoke-memory-reflection` | `memory.reflections.state`, `.list`, `.generate` |
| `spoke-memory-transfer` | `memory.exports.create`, `.get`, `memory.imports.stage`, `.activate`, `.discard` |

**Family rules.** Read ops return their payload; every mutating op returns a `MemoryReceipt` (`committed` / `pending` / `failed`) whose `result` carries IDs, revisions, counts, outcome enums, digests, and cost facts only. The failure branch is `{ error }` — the shared envelope shape with `details.retry_class` required (`retryable` / `terminal` / `uncertain`). Every mutating op except `memory.subjects.ensure` requires `idempotency_key`, and conditional writes require `expected_revision`. Per-op grant capability, request fields, and error codes are tabulated in the [Memory reference](/reference/memory) and the [family spec](https://github.com/42ch-dev/spoke/blob/main/.agents/specs/memory/spoke-memory.md).

## Shared rules

- **Check ≠ Assemble** — `check` returns findings only; `assemble` returns a packet only.
- **`$ref` composition** — ops schemas `$ref` data-layer types, with each type defined once.
- **Purity** — the operations library is pure relative to host I/O: storage access, LLM calls, ranking, retrieval, and transport binding are supplied by products through injected adapter ports. The library runs the protocol gates; adapters own persistence.

## Related

- [Protocol reference](/reference/protocol) — the three columns and capability flags.
- [Orchestrate operations](/how-to/orchestrate-ops) — calling each orchestrator with real signatures.
- [Data model reference](/reference/data-model) — the objects these envelopes carry.
- [Cognition records](/reference/data-model#cognition-spoke-cognition) — record shapes, `q`, the closed filter grammar and projections.
- [Connect reference](/reference/connect) — ops envelopes wrapped as opaque invoke payloads.
- [Memory reference](/reference/memory) — the optional `spoke-memory` family's 33 ops, capability split, and provider guidance.
