# SPOKE Operations Library

> **Status:** Normative (operations library first slice + deepen — delivered 2026-07-23)  
> **Document class:** Detail — hand-written behavior layer (column 3)  
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)  
> **Package:** `@42ch/spoke-operations` under `packages/spoke-operations/`

## Problem & user value

Wire schemas (`@42ch/spoke-schemas`) tell integrators **what** crosses the boundary. They do not encode cross-product **lifecycle invariants** — promote gates, Finding status rules, extension round-trip preservation, or wire-valid `AssemblePacket` construction.

Without a shared operations library, every product (Nexus, Creader, future adapters) reimplements the same pure rules and drifts. **`@42ch/spoke-operations`** is the single hand-written place for those invariants: callable from adapters and product code, with **no** I/O, storage, LLM, ranking, or retrieval.

**Integrator outcome:** import types from `@42ch/spoke-schemas`, import lifecycle helpers from `@42ch/spoke-operations`, bind transport locally — no shared daemon required.

---

## Three-way boundary (normative)

| Layer | Authored how | Owns | Does not own |
|-------|--------------|------|--------------|
| **Wire schemas** | Hand-written JSON Schema in `schemas/` → generated `@42ch/spoke-schemas` | Object shapes, ops request/response envelopes, `extensions` bag presence | Lifecycle transitions, merge semantics, promote gates |
| **Operations library** | Hand-written TypeScript in `packages/spoke-operations/` | Pure functions / small state machines over generated types | HTTP/MCP, persistence, LLM, ranking, retrieval, product-specific detectors |
| **Adapters** | Hand-written per product in `adapters/*` | Product DTO ↔ SPOKE mapping, transport binding | Reimplementing operations invariants (MUST call library instead) |

### Hard In / Out

| **In (library MUST provide)** | **Out (library MUST NOT)** |
|---------------------------------|----------------------------|
| Extension map merge + round-trip preserve | Storage read/write; fetch stored KnowledgeEntry inside library |
| Finding `status` transition validation + apply | HTTP routes, MCP tools, message queues, HTTP status code tables |
| Promote acceptance checks (pure gate before persist) | LLM calls, checker engines, Guardian logic |
| AssemblePacket builders from KnowledgeEntries (structure only) | Ranking, scoring, vector retrieval, token budgeting |
| Unified `SpokeResult` / `SpokeRejectCode` on every reject path | Silent auto-promote bypassing human review semantics |
| Revision bump on promote apply (see §Promote acceptance) | — |
| OCC revision compare (`assertRevisionMatch`) — operations library deepen | — |
| KnowledgeEntry status transitions + active uniqueness — operations library deepen | Product `world_id` / `book_id` as required core fields |
| Scope match, upsert/relate gates, error-envelope map — operations library deepen | `scope_id` parsing; retrieval engines |

### Per-family In / Out

| Family | **In** | **Out** |
|--------|--------|---------|
| **Extensions** | Deep merge; overlay wins scalars; preserve unknown namespaces/keys | Dropping empty `{}` namespaces; mutating inputs |
| **Finding** | Transition table enforcement; no-op same-status; structured reject | Product-specific workflow beyond cross-product minimum |
| **Promote** | Provisional gate; terminal-status reject; revision bump; merge-target id guard; OCC via caller-supplied revisions | Persist; fetch stored KnowledgeEntry |
| **Assemble** | Wire-valid `AssemblePacket`; `snippet` from `body.summary` rule; order-preserving `maxEntries` truncate | Sort, rank, dedupe, token count, embedding search |
| **OCC** | `assertRevisionMatch` on caller-supplied integers | Storage fetch |
| **KnowledgeEntry** | Status transition table; active uniqueness over caller set | Product `world_id` / `book_id` required fields |
| **Scope** | KnowledgeEntry + TimelineEvent refinement filters | `scope_id` parsing; retrieval |
| **Upsert / Relate** | Create/update revision rules; self-edge reject | Persist |
| **Error map** | `SpokeReject` ↔ `ErrorEnvelope` code stability | HTTP/MCP status mapping |

---

## Result / reject envelope (normative)

All helpers that can fail MUST return the same discriminated union — no thrown errors for expected reject paths, no ad-hoc `{ error: string }` shapes.

```typescript
type SpokeOk<T = void> = [T] extends [void]
  ? { ok: true }
  : { ok: true; value: T };

type SpokeReject = {
  ok: false;
  code: SpokeRejectCode;
  message: string;
  details?: Record<string, unknown>;
};

type SpokeResult<T = void> = SpokeOk<T> | SpokeReject;
```

| Rule | Requirement |
|------|-------------|
| Success | `ok: true`; payload helpers use `value`, validators may omit it |
| Failure | `ok: false` with stable `code` + human-readable `message` |
| `details` | Optional structured context (e.g. `{ from, to }` on transition reject) — not a second error channel |
| Throwing | Unexpected programmer errors only; lifecycle rejects are **never** thrown |

### `SpokeRejectCode` (first slice + deepen)

Stable string literals exported from `@42ch/spoke-operations` (e.g. `as const` object + union type). Implementers MUST NOT invent parallel code strings.

| Code | Family | Emitted in first slice | Emitted in deepen slice | Meaning |
|------|--------|----------------------|----------------------|---------|
| `INVALID_INPUT` | shared | yes | yes | Argument fails shape/null checks before domain rules |
| `INVALID_STATUS` | finding | yes | yes | `to` (or current `finding.status`) not in core vocabulary |
| `INVALID_STATUS_TRANSITION` | finding | yes | yes | Disallowed `from` → `to` (see transition table) |
| `CANDIDATE_NOT_PROVISIONAL` | promote | yes | yes | `candidate.status` ≠ `provisional` (default gate) |
| `CANDIDATE_TERMINAL_STATUS` | promote | yes | yes | `candidate.status` is `merged` or `deleted` |
| `EMPTY_CANONICAL_NAME` | promote | yes | yes | `canonical_name` missing or whitespace-only |
| `MERGE_TARGET_SELF` | promote | yes | yes | `target_knowledge_entry_id` equals `candidate.knowledge_entry_id` |
| `MISSING_REQUIRED_FIELD` | promote / upsert | yes | yes | Required KnowledgeEntry field absent (schema-aligned check) |
| `INVALID_PACKET_INPUT` | assemble | yes | yes | e.g. empty `packetId`, negative `maxEntries` |
| `REVISION_CONFLICT` | occ | reserved | **yes** | `actualRevision < expectedRevision` (caller ahead of store) |
| `STORED_REVISION_STALE` | occ | reserved | **yes** | `actualRevision > expectedRevision` (caller behind store) |
| `INVALID_KNOWLEDGE_ENTRY_STATUS` | knowledge-entry | — | **yes** | Proposed KnowledgeEntry `status` not in core vocabulary |
| `INVALID_KNOWLEDGE_ENTRY_STATUS_TRANSITION` | knowledge-entry | — | **yes** | Disallowed KnowledgeEntry `from` → `to` |
| `DUPLICATE_ACTIVE_KNOWLEDGE_ENTRY` | uniqueness | — | **yes** | Second active KnowledgeEntry for same `(scope_key, entry_type, canonical_name)` |
| `KNOWLEDGE_ENTRY_NOT_FOUND` | upsert | — | **yes** | Update path but no `stored` KnowledgeEntry supplied |
| `KNOWLEDGE_ENTRY_ALREADY_EXISTS` | upsert | — | **yes** | Create path but `stored` KnowledgeEntry already present |
| `KNOWLEDGE_ENTRY_TERMINAL_STATUS` | upsert | — | **yes** | Update rejected because `stored.status` is `merged` or `deleted` |
| `RELATION_SELF_EDGE` | relate | — | **yes** | `from_id === to_id` |
| `RELATION_MISSING_ENDPOINT` | relate | — | **yes** | `from_id` or `to_id` missing or whitespace-only |

---

## First-slice helper inventory

Four families. Export names below are **normative** for the first slice; module layout under `src/` may group them, but public `src/index.ts` MUST expose these symbols.

### 1. Extensions — `extensions/*`

| Export | Purpose | Purity |
|-------------------|---------|--------|
| `mergeExtensionMaps(base, overlay)` | Deep-merge two `ExtensionMap`s; overlay wins on scalar conflicts; all namespace keys from both inputs appear in output | Pure, non-mutating |
| `preserveExtensionMaps(source, target)` | Produce merged map: `target` fields win for known keys; **unknown namespaces and unknown keys inside a namespace** from `source` are retained | Pure, non-mutating |

**Product rules encoded:**

- Unknown namespace keys MUST survive round-trip (aligns with [`spoke-data-model.md` §Extensions](spoke-data-model.md#extensions-normative)).
- Empty namespace objects `{}` are valid and MUST NOT be dropped.

**Tests must cover:** unknown `nexus` + `creader` keys preserved; overlay does not delete sibling namespaces.

---

### 2. Finding lifecycle — `finding/*`

| Export | Purpose | Purity |
|-------------------|---------|--------|
| `isValidFindingStatusTransition(from, to)` | Boolean guard for allowed transitions | Pure |
| `transitionFindingStatus(finding, to)` | Return `SpokeResult<Finding>` — updated `status` + `updated_at` on success | Pure, non-mutating input |

**Core vocabulary** (documented in schema, enforced here): `open`, `resolved`, `dismissed`.

**Allowed transitions (first slice):**

| From | To | Notes |
|------|-----|-------|
| `open` | `resolved` | User/checker resolved |
| `open` | `dismissed` | Intentionally ignored |
| `resolved` | `open` | Reopen |
| `dismissed` | `open` | Undismiss / reopen |
| same | same | No-op accept |

**Rejected:** any transition not in the table (e.g. `resolved` → `dismissed` without passing through `open`). Products MAY use extension namespaces for product-specific workflow — library enforces **cross-product minimum**.

**Reject codes:** `INVALID_STATUS`, `INVALID_STATUS_TRANSITION` (see §Result / reject envelope).

**Tests must cover:** each allowed edge, representative rejects, no-op same-status.

---

### 3. Promote acceptance — `promote/*`

| Export | Purpose | Purity |
|-------------------|---------|--------|
| `validatePromoteRequest(request)` | Validate `PromoteRequest` shape + lifecycle rules; return `SpokeResult<void>` | Pure |
| `applyPromoteAcceptance(request)` | On success, return `SpokeResult<KnowledgeEntry>` — promoted view (`status: confirmed`, revision bump per below); does **not** persist | Pure |

**Rules encoded (minimum):**

- `candidate` MUST satisfy KnowledgeEntry required fields (delegate to schema-shaped checks, not a parallel DTO).
- `candidate.status` MUST be `provisional` unless product documents an explicit override path (default: reject non-provisional → `CANDIDATE_NOT_PROVISIONAL`).
- `candidate.canonical_name` MUST be non-empty (`minLength` semantics → `EMPTY_CANONICAL_NAME`).
- If `target_knowledge_entry_id` present: MUST NOT equal `candidate.knowledge_entry_id` → `MERGE_TARGET_SELF`; merge semantics are structural only (no storage fetch).
- Reject `candidate` in terminal KnowledgeEntry statuses (`merged`, `deleted`) → `CANDIDATE_TERMINAL_STATUS`.
- **Human-in-loop invariant:** library never silently upgrades provisional → confirmed without caller explicitly invoking promote acceptance (no hidden side effects).

**Revision bump on apply (normative):**

| `candidate.revision` before apply | `revision` on returned KnowledgeEntry |
|-----------------------------------|---------------------------------|
| absent / `undefined` | `1` |
| integer ≥ 0 | `candidate.revision + 1` |

Returned KnowledgeEntry also sets `status: "confirmed"`. Other fields are shallow-copied from `candidate` unless promote rules explicitly transform them. Library does **not** set `updated_at` unless a later slice adds an optional clock parameter — operations library first slice leaves timestamps to the caller/adapter.

**Reject codes:** `CANDIDATE_NOT_PROVISIONAL`, `CANDIDATE_TERMINAL_STATUS`, `EMPTY_CANONICAL_NAME`, `MERGE_TARGET_SELF`, `MISSING_REQUIRED_FIELD`, `INVALID_INPUT`.

**Tests must cover:** happy path provisional→confirmed, reject deleted/merged candidate, reject empty name, merge-target id collision, revision `undefined`→`1`, revision `2`→`3`.

**OCC before persist (operations library deepen):** upsert update and promote paths SHOULD call `assertRevisionMatch` (§5) with caller-supplied `expectedRevision` and `actualRevision` — library never fetches storage.

---

### 4. AssemblePacket builder — `assemble/*`

| Export | Purpose | Purity |
|-------------------|---------|--------|
| `knowledgeEntryToAssembleEntry(knowledgeEntry)` | Map `KnowledgeEntry` → slim `AssembleEntry` per rules below | Pure |
| `buildAssemblePacket({ packetId, knowledgeEntries, extensions?, maxEntries? })` | Build valid `AssemblePacket`; `maxEntries` truncates **input order** only (no sort/rank) | Pure |

**`knowledgeEntryToAssembleEntry` mapping (normative):**

| Output field | Source |
|--------------|--------|
| `knowledge_entry_id` | `knowledgeEntry.knowledge_entry_id` |
| `entry_type` | `knowledgeEntry.entry_type` |
| `canonical_name` | `knowledgeEntry.canonical_name` |
| `snippet` | See rule below — **omit key** when rule does not apply |

**`snippet` from `body.summary`:**

1. Read `knowledgeEntry.body` as a record; if `body.summary` is **not** a string, omit `snippet`.
2. If it is a string, `trim()` it; if trimmed length is `0`, omit `snippet`.
3. Otherwise set `snippet` to the trimmed string.

Do **not** coerce non-strings, fall back to other `body` keys, or emit `snippet: ""`.

**`buildAssemblePacket`:** maps each input KnowledgeEntry via `knowledgeEntryToAssembleEntry`; when `maxEntries` is a positive integer, keep the first *n* entries in input order; when omitted, include all. Reject invalid args via `INVALID_PACKET_INPUT`.

**Explicitly out:** scoring, embedding search, deduplication by relevance, token counting.

**Tests must cover:** empty knowledge entry list, snippet present/absent/whitespace-only, non-string `body.summary`, `maxEntries` truncation preserves order, `extensions` passthrough.

---

## Helper families (operations deepen)

Five new families (plus error map). Export names are **normative** for the deepen slice; `src/index.ts` MUST expose them alongside first-slice symbols.

### 5. OCC — `occ/*`

| Export | Purpose | Purity |
|--------|---------|--------|
| `assertRevisionMatch(expectedRevision, actualRevision)` | Compare caller-supplied revisions before persist | Pure |

**Rules (normative):**

| Input | Result |
|-------|--------|
| Both integers ≥ 0 and equal | `ok: true` |
| `actualRevision > expectedRevision` | `STORED_REVISION_STALE` — caller read stale base |
| `actualRevision < expectedRevision` | `REVISION_CONFLICT` — caller expected impossible future revision |
| Non-integer, negative, or `NaN` | `INVALID_INPUT` |

**Caller contract:** integrator fetches `actualRevision` from its store and passes `expectedRevision` from the mutation payload. Library performs **no** storage I/O.

**Tests must cover:** match, stale (actual > expected), conflict (actual < expected), invalid inputs.

---

### 6. KnowledgeEntry lifecycle — `knowledge-entry/*`

| Export | Purpose | Purity |
|--------|---------|--------|
| `isValidKnowledgeEntryStatusTransition(from, to)` | Boolean guard for allowed transitions | Pure |
| `transitionKnowledgeEntryStatus(knowledgeEntry, to)` | Return `SpokeResult<KnowledgeEntry>` with updated `status` on success | Pure, non-mutating input |

**Core vocabulary** (aligned with `knowledge-entry.schema.json` `description` and [`spoke-data-model.md` §Core KnowledgeEntry status](spoke-data-model.md#core-knowledgeentry-status-vocabulary-documented-not-enforced)): `provisional`, `confirmed`, `deprecated`, `merged`, `deleted`.

**Terminal statuses:** `merged`, `deleted` — no outbound transitions (except same→same no-op).

**Active statuses (uniqueness gate):** `provisional`, `confirmed` only.

**Allowed transitions (deepen slice):**

| From | To | Notes |
|------|-----|-------|
| `provisional` | `confirmed` | Also via promote acceptance |
| `provisional` | `deprecated` | Discard / park candidate |
| `provisional` | `merged` | Absorb before confirm |
| `provisional` | `deleted` | Drop candidate |
| `confirmed` | `deprecated` | Supersede canonical |
| `confirmed` | `merged` | Absorb into target |
| `confirmed` | `deleted` | Tombstone |
| `deprecated` | `confirmed` | Restore |
| `deprecated` | `deleted` | Tombstone |
| same | same | No-op accept |

**Rejected:** all other pairs (e.g. `merged` → `confirmed`, `deleted` → `provisional`, `deprecated` → `merged`). **`deprecated` → `merged` excluded** — merge requires an active canonical source; restore to `confirmed` first.

**Reject codes:** `INVALID_KNOWLEDGE_ENTRY_STATUS`, `INVALID_KNOWLEDGE_ENTRY_STATUS_TRANSITION` with optional `details: { from, to }`.

**Tests must cover:** each allowed edge, terminal outbound rejects, no-op same-status, invalid vocabulary.

---

### 7. Active uniqueness — `knowledge-entry/*`

| Export | Purpose | Purity |
|--------|---------|--------|
| `assertUniqueActiveKnowledgeEntry({ scope_key, entry_type, canonical_name, candidate, existing })` | Reject duplicate active triple among caller-supplied set | Pure |

**Rules:**

- `scope_key` is an **opaque string** supplied by the caller (typically mapped from `Scope.scope_id` or product World/Book ids). It is **not** a KnowledgeEntry protocol field.
- `existing` is `KnowledgeEntry[]` the caller already holds for that `scope_key`.
- Consider only KnowledgeEntries whose `status` is **active** (`provisional` or `confirmed`).
- Match triple `(scope_key, entry_type, canonical_name)` — `entry_type` and `canonical_name` from KnowledgeEntry wire fields.
- `candidate` is the KnowledgeEntry about to be created or reactivated; reject if another **different** `knowledge_entry_id` in `existing` already occupies the triple.
- Same `knowledge_entry_id` updating in place is allowed (no duplicate).

**Reject code:** `DUPLICATE_ACTIVE_KNOWLEDGE_ENTRY` with `details: { scope_key, entry_type, canonical_name, conflicting_knowledge_entry_id }`.

**Tests must cover:** unique accept, duplicate reject, inactive statuses ignored, same-id update allowed.

---

### 8. Scope match — `scope/*`

| Export | Purpose | Purity |
|--------|---------|--------|
| `knowledgeEntryMatchesScope(knowledgeEntry, scope)` | KnowledgeEntry passes optional `Scope` refinements | Pure |
| `filterKnowledgeEntriesByScope(knowledgeEntries, scope)` | Filter list by `knowledgeEntryMatchesScope` | Pure |
| `timelineEventMatchesScope(timelineEvent, scope)` | TimelineEvent passes optional `Scope` refinements | Pure |
| `filterTimelineEventsByScope(timelineEvents, scope)` | Filter list by `timelineEventMatchesScope` | Pure |

**`Scope` wire shape:** [`spoke-ops.md` §Scope](spoke-ops.md#scope-shared--check--assemble). `scope_id` is required on wire but **not interpreted** by these helpers — caller pre-scopes collections by product binding.

**KnowledgeEntry refinements (AND when present on `scope`):**

| Refinement | Match rule |
|------------|------------|
| `knowledge_entry_ids` | `knowledgeEntry.knowledge_entry_id` ∈ array |
| `entry_types` | `knowledgeEntry.entry_type` ∈ array |
| `source_id` | `knowledgeEntry.source_anchor?.source_id === scope.source_id` |

Ignored on KnowledgeEntry: `timeline_event_ids`, `timeline_scale`.

**TimelineEvent refinements (AND when present on `scope`):**

| Refinement | Match rule |
|------------|------------|
| `timeline_event_ids` | `timelineEvent.timeline_event_id` ∈ array |
| `timeline_scale` | `timelineEvent.timeline_scale === scope.timeline_scale` |

Ignored on TimelineEvent: `knowledge_entry_ids`, `entry_types`, `source_id`.

**Tests must cover:** each refinement on its carrier type, empty refinement pass-through, combined AND.

---

### 9. Upsert gate — `upsert/*`

| Export | Purpose | Purity |
|--------|---------|--------|
| `validateUpsertKnowledgeEntry(candidate, context)` | Create vs update rules before persist | Pure |

`context: { stored?: KnowledgeEntry }` — caller supplies stored view when updating.

**Create** (`stored` absent):

| Rule | Reject |
|------|--------|
| All `knowledge-entry.schema.json` required fields present | `MISSING_REQUIRED_FIELD` |
| `revision` absent, `undefined`, or `0` | accept |
| `revision` ≥ 1 on create | `INVALID_INPUT` |
| Caller passes `stored` by mistake on create path | N/A — use update path |

**Update** (`stored` present):

| Rule | Reject |
|------|--------|
| `candidate.knowledge_entry_id === stored.knowledge_entry_id` | `INVALID_INPUT` on mismatch |
| `candidate.revision` present, integer ≥ 0 | `MISSING_REQUIRED_FIELD` if absent |
| `assertRevisionMatch(candidate.revision, stored.revision ?? 0)` | OCC codes |
| `stored.status` is `merged` or `deleted` | `KNOWLEDGE_ENTRY_TERMINAL_STATUS` |

**Implicit path errors (caller wiring):**

| Situation | Code |
|-----------|------|
| Update path with no `stored` | `KNOWLEDGE_ENTRY_NOT_FOUND` |
| Create path when `stored` provided | `KNOWLEDGE_ENTRY_ALREADY_EXISTS` |

Integrator SHOULD run KnowledgeEntry status transition validation separately when `candidate.status !== stored.status`.

**Tests must cover:** valid create, valid update with OCC, create with revision ≥ 1 reject, update without revision, terminal stored reject.

---

### 10. Relate gate — `relate/*`

| Export | Purpose | Purity |
|--------|---------|--------|
| `validateRelateRequest(relation)` | Shape + lifecycle rules before persist | Pure |

**Rules:**

- `from_id` and `to_id` MUST be non-empty trimmed strings → else `RELATION_MISSING_ENDPOINT`.
- `from_id === to_id` → `RELATION_SELF_EDGE`.
- `relation_type` remains open string (no closed enum in library).

**Tests must cover:** happy path, self-edge, missing endpoint.

---

### 11. Error envelope map — `error/*`

| Export | Purpose | Purity |
|--------|---------|--------|
| `toErrorEnvelope(reject)` | Map `SpokeReject` → ops `ErrorEnvelope` | Pure |
| `fromErrorEnvelope(error)` | Map `ErrorEnvelope` → `SpokeReject` | Pure |

**Rules:**

- `code` MUST round-trip unchanged (same string as `SpokeRejectCode`).
- `message` copies verbatim.
- `details` copies when present; omitted when absent.
- `extensions` on `ErrorEnvelope` MUST be `{}` when converting from `SpokeReject` unless a later slice adds namespace passthrough.
- **Out of scope:** HTTP status codes, MCP error types, gRPC codes, retry hints.

Wire shape: [`spoke-ops.md` §Error envelope](spoke-ops.md#error-envelope).

**Tests must cover:** round-trip for every code used in first-slice + deepen tests; `extensions: {}` on outbound map.

---

## Package contract

| Field | Value |
|-------|-------|
| Name | `@42ch/spoke-operations` |
| Dependency | `@42ch/spoke-schemas` (workspace) only |
| Publish | Private workspace package; no npm publish job in CI |
| Rust | Deferred (`spoke-operations` crate not in first slice) |

Public entry: `src/index.ts` re-exporting all families above plus `SpokeResult`, `SpokeReject`, `SpokeRejectCode` types/constants.

---

## Acceptance (operations layer)

### First slice (delivered)

- [x] This spec + [`spoke-protocol.md`](spoke-protocol.md) cross-link (umbrella column 3)
- [x] Package exists with four helper families and unit tests per table above
- [x] `SpokeResult` / `SpokeRejectCode` exported and used on all reject paths
- [x] No I/O, LLM, ranking, retrieval, or storage imports in package dependency graph
- [x] CI typecheck + test + build includes `packages/spoke-operations/`

### Deepen slice (delivered 2026-07-23)

- [x] OCC, KnowledgeEntry status, uniqueness, Scope, upsert, relate, error-map families implemented per §Helper families (operations deepen)
- [x] `REVISION_CONFLICT` and `STORED_REVISION_STALE` emitted on documented paths
- [x] [`spoke-protocol-layers.md`](spoke-protocol-layers.md) library column updated for L0–L6 rows
- [x] First-slice export behavior unchanged except additive OCC emit on new call sites

## Non-goals (operations layer)

### First slice

- Adapter conversion code
- Rust operations crate
- Conformance fixtures / golden files
- `Rule` evaluation, checker engines, Guardian detectors
- HTTP/MCP binding or daemon routes
- Ranking / retrieval / token-budget helpers

### Deepen slice (unchanged)

- Adapter packages and product DTO field maps
- Storage fetch inside library
- HTTP/MCP status code tables
- Checker Rule evaluation engines
- Fork wire / `project` op / Rust ops crate

---

## Related paths

| Path | Role |
|------|------|
| [`spoke-ops.md`](spoke-ops.md) | Ops **wire** request/response (column 2) |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | L0–L8 map; Check≠Assemble boundary framing |
| [`spoke-data-model.md`](spoke-data-model.md) | Data objects helpers operate on |
| [`.mstar/roadmap.md`](../roadmap.md) | Thrust A column 3 mandate |
| `packages/spoke-operations/` | Implementation (first slice + deepen delivered 2026-07-23) |
