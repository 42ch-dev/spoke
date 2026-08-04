# SPOKE Operations Library

> **Status:** Normative (operations library — pure helpers, capability-sliced adapter ports, and injection orchestration)  
> **Document class:** Detail — hand-written behavior layer (column 3)  
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)  
> **Package (TypeScript):** `@42ch/spoke-operations` under `packages/spoke-operations/`  
> **Crate (Rust):** `spoke-operations` under `crates/spoke-operations/` — behavioral parity with the TypeScript package at lockstep SemVer

## Problem & user value

Wire schemas (`@42ch/spoke-schemas`) tell integrators **what** crosses the boundary. They do not encode cross-product **lifecycle invariants** — promote gates, Finding status rules, extension round-trip preservation, or wire-valid `AssemblePacket` construction.

Without a shared operations library, every product reimplements the same pure rules and drifts. **`@42ch/spoke-operations`** (TypeScript) and **`spoke-operations`** (Rust) are the hand-written surfaces for those invariants: pure helpers plus capability-sliced adapter ports and injection orchestration. The library performs **no** I/O, storage, LLM, ranking, or retrieval; product adapters supply persistence and query through injected ports.

**Integrator outcome:** import types from `@42ch/spoke-schemas` / `spoke-schemas`, implement the port families for the capability flags you claim, call orchestration entrypoints (or pure helpers directly), and bind transport locally.

---

## Three-way boundary (normative)

| Layer | Authored how | Owns | Does not own |
|-------|--------------|------|--------------|
| **Wire schemas** | Hand-written JSON Schema in `schemas/` → generated `@42ch/spoke-schemas` | Object shapes, ops request/response envelopes, `extensions` bag presence | Lifecycle transitions, merge semantics, promote gates |
| **Operations library** | Hand-written TypeScript in `packages/spoke-operations/`; hand-written Rust in `crates/spoke-operations/` | Pure helpers over generated types; capability-sliced port contracts; injection orchestration | HTTP/MCP, persistence engines, LLM, ranking, retrieval, product-specific detectors |
| **Adapters** | Hand-written per product in consumer repositories, or as reference examples in `fixtures/toy-world/` | Product DTO ↔ SPOKE mapping; transport binding; port implementations | Reimplementing operations invariants (MUST call library instead) |

### Hard In / Out

| **In (library MUST provide)** | **Out (library MUST NOT)** |
|---------------------------------|----------------------------|
| Extension map merge + round-trip preserve | Storage read/write; fetch stored KnowledgeEntry inside library |
| Finding `status` transition validation + apply | HTTP routes, MCP tools, message queues, HTTP status code tables |
| Promote acceptance checks (pure gate before persist) | LLM calls, checker engines, Guardian logic |
| AssemblePacket builders from KnowledgeEntries (structure only) | Ranking, scoring, vector retrieval, token budgeting |
| Unified `SpokeResult` / `SpokeRejectCode` on every reject path — **both** language packages | Silent auto-promote bypassing human review semantics |
| Revision bump on promote apply (see §Promote acceptance) | — |
| OCC revision compare (`assertRevisionMatch` / `assert_revision_match`) | — |
| KnowledgeEntry status transitions + active uniqueness | Product `world_id` / `book_id` as required core fields |
| Scope match, upsert/relate gates, error-envelope map | `scope_id` parsing; retrieval engines |
| Body attribute list/filter/read by `trait_type` | Attribute upsert, merge, or validation that rejects unknown traits |
| Computable shape validators (`validateComputableFieldMap`, log entry, project/compute request gates) | Compute engine execution, WASM, Session store I/O |
| Capability-sliced adapter ports + injection orchestration entrypoints | Product DTO field maps; storage/HTTP/LLM imports inside the library |

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
| **Body attributes** | List/filter `body.attributes` by `trait_type`; skip malformed wire elements | Attribute upsert, merge, ranking, defaulting |
| **Upsert / Relate** | Create/update revision rules; self-edge reject | Persist |
| **Error map** | `SpokeReject` ↔ `ErrorEnvelope` code stability | HTTP/MCP status mapping |
| **Computable** | Field-map, log-entry, project/compute request shape gates | Engine execution, WASM, Session I/O |

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

### `SpokeRejectCode` (normative)

Stable string literals exported from `@42ch/spoke-operations` and `spoke-operations` (e.g. TS `as const` object + union type; Rust `SpokeRejectCode` with `as_str()` returning the same literals). Implementers MUST NOT invent parallel code strings.

| Code | Family | Emitted by baseline helpers | Emitted by deepen / orchestration | Meaning |
|------|--------|----------------------------|-----------------------------------|---------|
| `INVALID_INPUT` | shared | yes | yes | Argument fails shape/null checks before domain rules |
| `INVALID_STATUS` | finding | yes | yes | `to` (or current `finding.status`) not in core vocabulary |
| `INVALID_STATUS_TRANSITION` | finding | yes | yes | Disallowed `from` → `to` (see transition table) |
| `CANDIDATE_NOT_PROVISIONAL` | promote | yes | yes | `candidate.status` ≠ `provisional` (default gate) |
| `CANDIDATE_TERMINAL_STATUS` | promote | yes | yes | `candidate.status` is `merged` or `deleted` |
| `EMPTY_CANONICAL_NAME` | promote | yes | yes | `canonical_name` missing or whitespace-only |
| `MERGE_TARGET_SELF` | promote | yes | yes | `target_entry_id` equals `candidate.entry_id` |
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
| `RELATION_NOT_FOUND` | relate | — | **yes** | Update path but no stored Relation |
| `RELATION_ALREADY_EXISTS` | relate | — | **yes** | Create path but stored Relation already present |
| `CAPABILITY_PORT_MISSING` | orchestration | — | **yes** | An optional orchestrator was called without the injected port required by its claimed capability |

---

## First-slice helper inventory

Four families. Export names below are **normative** for the first slice; module layout under `src/` may group them, but public `src/index.ts` MUST expose these symbols.

### 1. Extensions — `extensions/*`

| Export | Purpose | Purity |
|-------------------|---------|--------|
| `mergeExtensionMaps(base, overlay)` | Deep-merge two `ExtensionMap`s; overlay wins on scalar conflicts; all namespace keys from both inputs appear in output | Pure, non-mutating |
| `preserveExtensionMaps(source, target)` | Produce merged map: `target` fields win for known keys; **unknown namespaces and unknown keys inside a namespace** from `source` are retained | Pure, non-mutating |
| `mergeModuleMaps(base, overlay)` | Deep-merge two `ModuleMap`s with the same namespace-merge core; object values deep-merge; array/other values overlay-replace base | Pure, non-mutating |
| `preserveModuleMaps(source, target)` | Produce merged map: `target` wins for known keys; **unknown module namespaces** (object or array values) from `source` are retained | Pure, non-mutating |

**Product rules encoded:**

- Unknown namespace keys MUST survive round-trip (aligns with [`spoke-data-model.md` §Extensions](spoke-data-model.md#extensions-normative) and §Modules).
- Empty namespace objects `{}` are valid and MUST NOT be dropped.
- `ModuleMap` values may be objects **or** arrays; merge deep-merges objects and replaces arrays (no element-wise merge).
- Extension and module helpers share one internal namespace-merge core; thin wrappers preserve separate public names.

**Tests must cover:** unknown keys under two distinct namespaces preserved; overlay does not delete sibling namespaces; module object-valued and array-valued namespaces round-trip; empty/absent `modules` valid.

Bag placement: [`spoke-extension-modules.md`](spoke-extension-modules.md). Capability flag: `narrative-modules` in [`spoke-protocol-layers.md`](spoke-protocol-layers.md).

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
- If `target_entry_id` present: MUST NOT equal `candidate.entry_id` → `MERGE_TARGET_SELF`; merge semantics are structural only (no storage fetch).
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
| `entry_id` | `knowledgeEntry.entry_id` |
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

## Helper families (operations deepen + computable)

Deepen families (§5–§11), computable validators (§12), and body attribute read helpers (§13). Export names are **normative**; `@42ch/spoke-operations` `src/index.ts` MUST expose them alongside first-slice symbols. `spoke-operations` `src/lib.rs` MUST re-export every symbol in TS `index.ts` (snake_case) and MAY additionally export Rust-only typed/wire helpers (see §Rust).

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
- `candidate` is the KnowledgeEntry about to be created or reactivated; reject if another **different** `entry_id` in `existing` already occupies the triple.
- Same `entry_id` updating in place is allowed (no duplicate).

**Reject code:** `DUPLICATE_ACTIVE_KNOWLEDGE_ENTRY` with `details: { scope_key, entry_type, canonical_name, conflicting_entry_id }`.

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
| `entry_ids` | `knowledgeEntry.entry_id` ∈ array |
| `entry_types` | `knowledgeEntry.entry_type` ∈ array |
| `source_id` | `knowledgeEntry.source_anchor?.source_id === scope.source_id` |

Ignored on KnowledgeEntry: `timeline_event_ids`, `timeline_scale`.

**TimelineEvent refinements (AND when present on `scope`):**

| Refinement | Match rule |
|------------|------------|
| `timeline_event_ids` | `timelineEvent.timeline_event_id` ∈ array |
| `timeline_scale` | `timelineEvent.timeline_scale === scope.timeline_scale` |
| `fork_id` | `timelineEvent.fork_id === scope.fork_id` (events without `fork_id` do not match) |

Ignored on TimelineEvent: `entry_ids`, `entry_types`, `source_id`, `parent_fork_id`.

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
| `candidate.entry_id === stored.entry_id` | `INVALID_INPUT` on mismatch |
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
| `validateRelateRequest(relation, context)` | Shape + lifecycle rules before persist; context supplies `stored?: Relation` for create vs update | Pure |

**Rules:**

- `from_id` and `to_id` MUST be non-empty trimmed strings → else `RELATION_MISSING_ENDPOINT`.
- `from_id === to_id` → `RELATION_SELF_EDGE`.
- `relation_type` remains open string (no closed enum in library).

**Create** (`context.stored` absent):

| Rule | Reject |
|------|--------|
| `revision` absent, `undefined`, or `0` | accept |
| `revision` ≥ 1 on create | `INVALID_INPUT` |

**Update** (`context.stored` present):

| Rule | Reject |
|------|--------|
| `relation.relation_id === stored.relation_id` | `INVALID_INPUT` on mismatch |
| `relation.revision` present, integer ≥ 0 | `MISSING_REQUIRED_FIELD` if absent |
| `assertRevisionMatch(relation.revision, stored.revision ?? 0)` | OCC codes (`STORED_REVISION_STALE`, `REVISION_CONFLICT`) |

**Implicit path errors (caller wiring):**

| Situation | Code |
|-----------|------|
| Update path with no `stored` | `RELATION_NOT_FOUND` |
| Create path when `stored` provided | `RELATION_ALREADY_EXISTS` |

Reject codes reused: `INVALID_INPUT`, `MISSING_REQUIRED_FIELD`, `STORED_REVISION_STALE`, `REVISION_CONFLICT`.

**Tests must cover:** happy path create, happy path update with OCC, create with revision ≥ 1 reject, update null revision reject, self-edge, missing endpoint, stored relation_id mismatch.

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

### 12. Computable (`l2-computable`) — `computable/*`

| Export | Purpose | Purity |
|--------|---------|--------|
| `validateComputableFieldMap(value)` | Shape gate for `body.state` / `body.computable` and op payloads | Pure |
| `validateComputableLogEntry(entry)` | Shape gate for `TimelineEvent.computable_logs[]` items | Pure |
| `validateProjectRequest(request)` | Required-field gate for `project` op request | Pure |
| `validateComputeRequest(request)` | Required-field gate; `settle` flag sanity | Pure |

**Rules encoded (minimum):**

- `ComputableFieldMap` MUST be a non-null plain object when present.
- `validateProjectRequest`: requires `session_id`, `entry_id`, `state`.
- `validateComputeRequest`: requires `session_id`, `entry_id`, `computable`; `settle` when present MUST be boolean.
- **Out of scope:** running compute engines, WASM, merge algorithms, Session store I/O.

Wire shapes: [`spoke-data-model.md` §Computable body](spoke-data-model.md#computable-body-l2-computable-optional), [`spoke-ops.md` §Optional ops](spoke-ops.md#optional-ops-l2-computable).

---

### 13. Body attributes — `body/*`

| Export (TypeScript) | Export (Rust) | Purpose | Purity |
|---------------------|---------------|---------|--------|
| `listBodyAttributes(input)` | `list_body_attributes(input)` | List valid `body.attributes` traits in array order | Pure |
| `filterBodyAttributesByTraitType(input, traitType)` | `filter_body_attributes_by_trait_type(input, trait_type)` | Return all traits matching `trait_type` in original order | Pure |
| `findBodyAttribute(input, traitType)` | `find_body_attribute(input, trait_type)` | Return first trait matching `trait_type`, or absent | Pure |

**Input:** `KnowledgeEntry["body"]`, full `KnowledgeEntry`, or wire JSON (`null` / absent → empty). Rust: `BodyAttributesInput::Body`, `Entry`, or `Wire(Option<&Value>)`.

**Rules (normative):**

| Case | `list*` / `filter*` | `find*` |
|------|---------------------|---------|
| `body` / `attributes` absent | `[]` | absent (`undefined` / `None`) |
| `attributes: []` | `[]` | absent |
| Duplicate `trait_type` in array | all matches in order | first match in order |
| Malformed array element (not plain object; empty/missing `trait_type`; `value` not `string` \| `number` \| `boolean`) | skip element; never throw | skip element |
| `trait_type` match | exact string equality (case-sensitive) | same |

**Out of scope:** attribute upsert/merge, schema validation that rejects unknown traits, ranking, persistence.

Wire shape: [`spoke-data-model.md` §KnowledgeEntry body](spoke-data-model.md) — closed L2 payload with optional `attributes[]` (`BodyAttribute` items).

**Tests must cover:** absent input, omitted/empty attributes, duplicate `trait_type`, malformed skip, first-match find, case-sensitive filter.

---

### 14. Narrative sequence — `timeline/*`

Beat-assist pure helpers over caller-supplied `TimelineEvent[]` and `Relation[]`. Align with [`domain-profile-narrative-structure.md`](domain-profile-narrative-structure.md) §Relation endpoints and §Pure ops helpers. **No I/O.**

| Export (TypeScript) | Export (Rust) | Purpose | Purity |
|---------------------|---------------|---------|--------|
| `filterTimelineEventsByMomentScale(timelineEvents)` | `filter_timeline_events_by_moment_scale(timeline_events)` | Keep events where `timeline_scale === "moment"` | Pure |
| `orderTimelineEventsByIds(timelineEvents, orderedIds)` | `order_timeline_events_by_ids(timeline_events, ordered_ids)` | Order by explicit `timeline_event_id` list | Pure |
| `orderTimelineEventsByPrecedes(timelineEvents, relations, options?)` | `order_timeline_events_by_precedes(timeline_events, relations, options?)` | Topological order via `precedes` on dual KE ids | Pure |

**`filterTimelineEventsByMomentScale` rules:**

| Case | Result |
|------|--------|
| `timeline_scale === "moment"` (exact, case-sensitive) | included, **input order preserved** |
| absent `timeline_scale` or any other value | excluded |
| empty input | `[]` |

**`orderTimelineEventsByIds` rules:**

| Case | Result |
|------|--------|
| `orderedIds` sequence | output lists matching events in that order first |
| id in `orderedIds` not found in `timelineEvents` | `INVALID_INPUT`; `details: { unknown_timeline_event_ids: string[] }` |
| duplicate id in `orderedIds` | `INVALID_INPUT` |
| duplicate `timeline_event_id` in `timelineEvents` | `INVALID_INPUT`; `details: { duplicate_timeline_event_ids: string[] }` |
| events in input but not in `orderedIds` | append after ordered block in **input order** (stable tail) |

**`orderTimelineEventsByPrecedes` rules:**

| Step | Rule |
|------|------|
| Link map | For each input event, read `extensions.spoke.timeline_entry_id` (non-empty trimmed string) as linked KE `entry_id`; events without link are **unlinked** |
| Relation filter | `relation_type` exact match — default `"precedes"`; overridable via `options.relationType` |
| Endpoint filter | keep relations where both `from_id` and `to_id` resolve to linked KE ids present in the input link map |
| Self-loop | `from_id` and `to_id` resolve to the same linked event → `INVALID_INPUT`; `details: { precedes_cycle: true, entry_ids: string[] }` |
| Sort | Kahn topological sort on linked events; ready queue ordered by ascending `timeline_event_id` (UTF-8 lexicographic) before dequeue |
| Unlinked tail | append unlinked input events after linked ordered block in **input order** |
| Cycle | not all linked nodes sortable → `INVALID_INPUT`; `details: { precedes_cycle: true, entry_ids: string[] }` |
| Duplicate input ids | duplicate `timeline_event_id` in `timelineEvents` → `INVALID_INPUT`; `details: { duplicate_timeline_event_ids: string[] }` |

**Relation schema alignment:** `from_id` / `to_id` are KnowledgeEntry (or SourceAnchor) ids — helpers never accept `timeline_event_id` as Relation endpoints. Dual-concern pairing uses `extensions.spoke.timeline_entry_id` on `TimelineEvent` (toy-world convention).

**Reject codes:** `INVALID_INPUT` only for order helpers (cycles, self-loops, unknown ids, duplicate ids in `orderedIds` or `timelineEvents`).

**Tests must cover:** moment filter preserves order; order by ids happy path + unknown id + stable tail; precedes acyclic chain + cycle reject + unlinked tail + lexicographic tie-break; ignored relations with endpoints outside input link set.

---

## Adapter Interfaces (normative)

The operations packages define the **implementation protocol** for storage and query adapters. Port interfaces are capability-sliced and accept generated wire types directly. Adapter implementations own transport, persistence, transactions, and product DTO mapping; the operations package owns the port contracts and the injection orchestration below.

### Port policy

Port methods are asynchronous on the normative surface. TypeScript port methods return `Promise<SpokeResult<T>>`; Rust port traits are `#[async_trait] async fn …(&self, …) -> SpokeResult<T>` with `Send` futures (normative ports use the default `#[async_trait]`, not `#[async_trait(?Send)]`). All nine `orchestrate*` entrypoints are `async` — `export async function orchestrateX(…): Promise<SpokeResult<R>>` in TypeScript, `pub async fn orchestrate_x(…) -> SpokeResult<R>` in Rust — and await every injected port call. The library itself stays I/O-free: `await` appears only on injected port method calls, and pure helpers remain synchronous. Checker callbacks stay synchronous: `orchestrateCheck` / `orchestrateForkCheck` accept `(input: CheckRunInput) => SpokeResult<Finding[]>` (TypeScript) / `F: FnOnce(CheckRunInput) -> SpokeResult<Vec<Finding>>` (Rust) — pure product logic, not ports. Rust port traits use the `async-trait` crate, which keeps the dyn availability probes (`as_computable`, `as_fork_timeline`) object-safe. The async form is the only surface: methods never return `T | Promise<T>` unions, and no sync variants or compatibility shims exist.

All port methods resolve to `SpokeResult<T>` as the application outcome. Adapter-level failures map to stable `SpokeRejectCode` values; expected absence uses the relevant `*_NOT_FOUND` code. Ports do not throw for expected adapter outcomes.

**Remote adapter:** spoke-connect ships a `RemoteAdapter` that implements these port traits by proxying each call as a reserved `port.*` connect invoke over a message-oriented `Transport` (TS `./remote` subpath, Rust `remote-adapter` feature). It is a drop-in for the local adapters — the same `orchestrate*` entrypoints run against it unchanged, with all connect verification encapsulated. Port-method catalogue and error mapping: [spoke-remote-adapter.md](spoke-remote-adapter.md).

### Capability matrix

| Capability | Required interface families | Orchestration enabled |
|---|---|---|
| `spoke-baseline` | `KnowledgeEntryPort`, `RelationPort`, `ScopeQueryPort`, `FindingPort`, `RuleQueryPort`, **`HostManifestPort`** | `orchestrateUpsert`, `orchestratePromote`, `orchestrateRelate`, `orchestrateCheck`, `orchestrateAssemble` |
| `l2-computable` | `ComputablePort` (plus baseline) | `orchestrateProject`, `orchestrateCompute` |
| `l5-fork` | `ForkTimelineQueryPort` (plus baseline) | `orchestrateForkCheck`, `orchestrateForkAssemble` |

Unclaimed capabilities need no ports; their orchestrators are not callable for that product.

### Baseline port families

The following **six** families are required for `spoke-baseline`:

| Family | TypeScript interface | Rust trait | Methods |
|---|---|---|---|---|
| Knowledge entry persistence | `KnowledgeEntryPort` | `KnowledgeEntryPort` | `getKnowledgeEntry(entryId: string): Promise<SpokeResult<KnowledgeEntry>>` / `async fn get_knowledge_entry(&self, entry_id: &str) -> SpokeResult<KnowledgeEntry>`; `putKnowledgeEntry(entry: KnowledgeEntry, expectedBaseRevision: number \| null): Promise<SpokeResult<KnowledgeEntry>>` / `async fn put_knowledge_entry(&self, entry: KnowledgeEntry, expected_base_revision: Option<u64>) -> SpokeResult<KnowledgeEntry>` |
| Relation persistence | `RelationPort` | `RelationPort` | `getRelation(relationId: string): Promise<SpokeResult<Relation>>` / `async fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation>`; `putRelation(relation: Relation, expectedBaseRevision: number \| null): Promise<SpokeResult<Relation>>` / `async fn put_relation(&self, relation: Relation, expected_base_revision: Option<u64>) -> SpokeResult<Relation>` (`null`/`None` = create; non-null = conditional put — adapters MUST atomic compare-and-put, parity with `putKnowledgeEntry`) |
| Scope query | `ScopeQueryPort` | `ScopeQueryPort` | `listKnowledgeEntries(scope: Scope): Promise<SpokeResult<KnowledgeEntry[]>>` / `async fn list_knowledge_entries(&self, scope: &Scope) -> SpokeResult<Vec<KnowledgeEntry>>`; `listTimelineEvents(scope: Scope): Promise<SpokeResult<TimelineEvent[]>>` / `async fn list_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>>` |
| Finding persistence | `FindingPort` | `FindingPort` | `putFindings(findings: Finding[]): Promise<SpokeResult<Finding[]>>` / `async fn put_findings(&self, findings: Vec<Finding>) -> SpokeResult<Vec<Finding>>` |
| Rule query | `RuleQueryPort` | `RuleQueryPort` | `listRules(ruleRefs: string[]): Promise<SpokeResult<Rule[]>>` / `async fn list_rules(&self, rule_refs: &[String]) -> SpokeResult<Vec<Rule>>` |
| Host manifest | `HostManifestPort` | `HostManifestPort` | `getHostCapabilityManifest(): Promise<SpokeResult<HostCapabilityManifest>>` / `async fn get_host_capability_manifest(&self) -> SpokeResult<HostCapabilityManifest>`; `listPeerHostCapabilityManifests(): Promise<SpokeResult<HostCapabilityManifest[]>>` / `async fn list_peer_host_capability_manifests(&self) -> SpokeResult<Vec<HostCapabilityManifest>>` |

`ScopeQueryPort` is the query boundary for `check` and `assemble`. `RuleQueryPort` is used when a check request supplies rule references; embedded rules remain request data and do not require a port lookup.

`HostManifestPort` exposes in-process collaboration metadata. Integrators call it explicitly — existing `orchestrate*` entrypoints do **not** auto-fetch manifests. `listPeerHostCapabilityManifests` returns **peers only** (excludes self), deduped by `host_id`, sorted ascending by `host_id` (UTF-8 lexicographic); empty `[]` is valid. The library does not discover peers; product/adapter memory supplies the list.

**Persisted-entity OCC parity:** `putKnowledgeEntry` / `put_knowledge_entry` and `putRelation` / `put_relation` carry optimistic concurrency control structurally: adapters MUST treat `expectedBaseRevision` / `expected_base_revision` as the store's required current revision before accepting the write (`null`/`None` = absent entity for create). The orchestrated create-or-update entrypoints (upsert, promote, relate) deep-integrate load → validate → OCC → put. True concurrent safety requires atomic compare-and-put in the adapter; the library stays I/O-free. See [`spoke-data-model.md`](spoke-data-model.md) §Persisted-entity OCC parity for the structural guardrail and exemption list.

### Host collaboration (normative)

Wire shape: [`spoke-data-model.md`](spoke-data-model.md) §HostCapabilityManifest.

#### Roles ↔ ports map

| `roles[]` value | Typical port families / orchestrators | Write authority |
|-----------------|--------------------------------------|-----------------|
| `data-store` | `KnowledgeEntryPort`; `orchestrateUpsert`, `orchestratePromote` | **Yes** — single OCC authority per `entry_id` in collaboration context |
| `input-source` | Product ingest surface (no new port family) | No — proposes intent |
| `checker` | `RuleQueryPort`, `FindingPort`; `orchestrateCheck` | No — emits `Finding[]` only |
| `assembler` | `ScopeQueryPort`; `orchestrateAssemble` | No — emits `AssemblePacket` only |
| `computable-engine` | `ComputablePort` when `l2-computable` ∈ `capabilities` | No — Session I/O; settled state via data-store |

`assembler` is closed-loop core vocabulary. `computable-engine` is optional and MUST pair with `l2-computable` in `capabilities`.

#### Namespace exclusivity

Within one collaboration context, each namespace string in any manifest's `namespaces[]` MUST appear on **at most one** `host_id`. Integrators enforce exclusivity when building peer lists and attributing `KnowledgeEntry.extensions.<ns>` ownership product-side. Host roles and namespace ownership live on the manifest — not in KE `extensions`.

#### `authority` and OCC

Optional manifest `authority.scope_key` binds the data-store role to an opaque collaboration scope (aligns with `assertUniqueActiveKnowledgeEntry` `scope_key` folklore). When `authority` is absent and `data-store` ∈ `roles`, integrators treat this manifest's `host_id` as implicit write authority.

#### `HostManifestPort` contract

| Method (TS) | Method (Rust) | Returns | Notes |
|-------------|---------------|---------|-------|
| `getHostCapabilityManifest` | `get_host_capability_manifest` | `SpokeResult<HostCapabilityManifest>` | Self manifest |
| `listPeerHostCapabilityManifests` | `list_peer_host_capability_manifests` | `SpokeResult<HostCapabilityManifest[]>` | Peers only; exclude self; dedupe by `host_id`; stable ascending `host_id` sort; `[]` OK |

#### Baseline fold-in

`HostManifestPort` is the **sixth** required family folded into `BaselinePorts` and `BaselineAdapter` for `spoke-baseline` claims. Baseline orchestrators (`orchestrateUpsert` through `orchestrateAssemble`) do **not** auto-fetch manifests — integrators call `getHostCapabilityManifest` / `listPeerHostCapabilityManifests` explicitly when composing peer lists or attributing namespace ownership. A baseline adapter MUST implement both methods; absence is an adapter defect, not `CAPABILITY_PORT_MISSING`.

`HostManifestPort` is **baseline-required** — not gated behind `CAPABILITY_PORT_MISSING`.

### Optional port families

| Capability | Family | TypeScript interface | Rust trait | Methods |
|---|---|---|---|---|
| `l2-computable` | Computable session | `ComputablePort` | `ComputablePort` | `project(request: ProjectRequest): Promise<SpokeResult<ProjectResponse>>` / `async fn project(&self, request: ProjectRequest) -> SpokeResult<ProjectResponse>`; `compute(request: ComputeRequest): Promise<SpokeResult<ComputeResponse>>` / `async fn compute(&self, request: ComputeRequest) -> SpokeResult<ComputeResponse>` |
| `l5-fork` | Fork-aware timeline query | `ForkTimelineQueryPort` | `ForkTimelineQueryPort` | `listForkTimelineEvents(scope: Scope & { fork_id: ForkId }): Promise<SpokeResult<TimelineEvent[]>>` / `async fn list_fork_timeline_events(&self, scope: &Scope) -> SpokeResult<Vec<TimelineEvent>>` |

`ForkTimelineQueryPort` is a capability-specific refinement of `ScopeQueryPort`; one object MAY satisfy both.

### Capability composition and availability

```typescript
type BaselinePorts =
  KnowledgeEntryPort &
  RelationPort &
  ScopeQueryPort &
  FindingPort &
  RuleQueryPort &
  HostManifestPort;
type ComputablePorts = BaselinePorts & ComputablePort;
type ForkPorts = BaselinePorts & ForkTimelineQueryPort;
type FullPorts = BaselinePorts & ComputablePort & ForkTimelineQueryPort;
```

### Adapter aliases (normative)

Integrators implement **one** adapter type (class/struct) that satisfies the port families for the capabilities they claim. The operations packages export **Adapter** aliases as ergonomic names for the composed port intersections below. Individual capability families remain `*Port`; the composed `*Ports` names remain exported alongside the aliases.

| Adapter alias | Equivalent composed ports | Capabilities |
|---------------|---------------------------|--------------|
| `BaselineAdapter` | `BaselinePorts` | `spoke-baseline` |
| `ComputableAdapter` | `ComputablePorts` | baseline + `l2-computable` |
| `ForkAdapter` | `ForkPorts` | baseline + `l5-fork` |
| `FullAdapter` | `FullPorts` | baseline + computable + fork |

Rust exports the same names with TS/Rust parity per the **Alias implementation** table below.

**Adopter path:** implement the `*Port` families on one type, pass it to `orchestrate*` / `orchestrate_*`. Reference implementations: `fixtures/toy-world/` (TypeScript and Rust).

### Alias implementation (normative)

| Language | Shape | Location |
|----------|-------|----------|
| TypeScript | `export type BaselineAdapter = BaselinePorts` (and same for `ComputableAdapter`, `ForkAdapter`, `FullAdapter`) | `packages/spoke-operations/src/adapter/ports.ts` → flat-re-export `src/index.ts` |
| Rust | Marker traits `pub trait BaselineAdapter: BaselinePorts {}` with blanket `impl<T: BaselinePorts> BaselineAdapter for T {}` (same for `ComputableAdapter: ComputablePorts`, `ForkAdapter: ForkPorts`, `FullAdapter: FullPorts`) | `crates/spoke-operations/src/adapter/ports.rs` → flat-re-export `src/lib.rs` |

Orchestrator signatures keep `&impl BaselinePorts` (etc.); `&impl BaselineAdapter` is equivalent via the blanket impl. The `*Ports` names remain exported.

### Reference adapter stub policy (normative)

`fixtures/toy-world/` reference `ToyWorldAdapter` examples demonstrate **FullAdapter** composition:

| Port family | Reference behavior |
|-------------|-------------------|
| Six baseline families (incl. `HostManifestPort`) | Runnable in-memory OCC store; optional seed from committed `kb_tw_*` / `rel_tw_*` / `evt_tw_*` / `rule_tw_*` / `fnd_tw_*` JSON; self manifest + product-seeded peer list |
| `ComputablePort` | Minimal wire-valid `ProjectResponse` / `ComputeResponse` synthesized from committed `op_tw_project_response.json` / `op_tw_compute_settle_response.json` (echo `session_id`, `entry_id`, fixture-shaped `computable` / `state`) |
| `ForkTimelineQueryPort` | Returns seeded timeline events filtered by `scope.fork_id` from committed graph (e.g. `evt_tw_harbor_dawn.json` when fork scope matches) |

`CAPABILITY_PORT_MISSING` is **not** the default Full stub. Demonstrate it in **one** negative test per language using a baseline-only adapter at a dynamic optional boundary (parity with in-package `orchestrate` tests).

These are conceptual public types; Rust implementers satisfy the corresponding trait bounds. Optional orchestrators require the matching composed type at compile time. At JavaScript boundaries and for dynamically assembled Rust trait objects, an absent optional method returns `SpokeReject { code: "CAPABILITY_PORT_MISSING", ... }` rather than a `TypeError` or panic.

## Injection Orchestration (normative)

Orchestration is additive to the pure helper families. The public surface exposes one per-operation entrypoint per language — not a stateful facade. Each entrypoint receives injected ports and a request, calls the listed pure helpers, and performs all reads and writes through those ports. Check orchestrators additionally accept a caller-supplied checker callback — the library loads scoped data via ports, invokes the callback, and persists findings; checker engines remain product-owned.

### Check orchestration injectee

Check paths do not embed a checker engine. After loading scoped entries, timeline events, and rules via ports, the orchestrator invokes a caller-supplied callback and persists the returned findings.

```typescript
type CheckRunInput = {
  request: CheckRequest;
  entries: KnowledgeEntry[];
  events: TimelineEvent[];
  rules: Rule[];
};
```

Rust exports an equivalent `CheckRunInput` struct with snake_case fields. The callback type is `(input: CheckRunInput) => SpokeResult<Finding[]>` in TypeScript and `Fn(CheckRunInput) -> SpokeResult<Vec<Finding>>` (or equivalent trait object) in Rust.

| Operation | TypeScript entrypoint | Rust entrypoint | Required ports | Required sequence |
|---|---|---|---|---|
| upsert | `export async function orchestrateUpsert(ports: BaselinePorts, request: UpsertRequest): Promise<SpokeResult<UpsertResponse>>` | `pub async fn orchestrate_upsert(ports: &impl BaselinePorts, request: UpsertRequest) -> SpokeResult<UpsertResponse>` | `KnowledgeEntryPort` | Load update context; call `validateUpsertKnowledgeEntry`; call status/uniqueness helpers when applicable; call `putKnowledgeEntry(entry, expectedBaseRevision)` where `expectedBaseRevision` is `null`/`None` on create and the stored revision on update |
| promote | `export async function orchestratePromote(ports: BaselinePorts, request: PromoteRequest): Promise<SpokeResult<PromoteResponse>>` | `pub async fn orchestrate_promote(ports: &impl BaselinePorts, request: PromoteRequest) -> SpokeResult<PromoteResponse>` | `KnowledgeEntryPort` | Load stored entry; terminal and revision gates; call `validatePromoteRequest`; call `applyPromoteAcceptance`; call `putKnowledgeEntry(entry, expectedBaseRevision)` where `expectedBaseRevision` is `null`/`None` when absent and `stored.revision` (or `0`) when stored exists |
| relate | `export async function orchestrateRelate(ports: BaselinePorts, request: RelateRequest): Promise<SpokeResult<RelateResponse>>` | `pub async fn orchestrate_relate(ports: &impl BaselinePorts, request: RelateRequest) -> SpokeResult<RelateResponse>` | `RelationPort` | Load stored via `getRelation` (RELATION_NOT_FOUND ⇒ create path); call `validateRelateRequest(request.relation, { stored })` (create: revision absent/0 ok; update: id match, revision present ≥ 0, `assertRevisionMatch`); call `putRelation(relation, expectedBaseRevision)` (`null`/`None` on create, stored.revision on update) |
| check | `export async function orchestrateCheck(ports: BaselinePorts, request: CheckRequest, runChecker: (input: CheckRunInput) => SpokeResult<Finding[]>): Promise<SpokeResult<CheckResponse>>` | `pub async fn orchestrate_check(ports: &impl BaselinePorts, request: CheckRequest, run_checker: impl FnOnce(CheckRunInput) -> SpokeResult<Vec<Finding>>) -> SpokeResult<CheckResponse>` | `ScopeQueryPort`, `RuleQueryPort`, `FindingPort` | Resolve refs with `listRules`; query scoped entries/events via `ScopeQueryPort`; apply scope helpers; invoke `runChecker`; call `putFindings` |
| assemble | `export async function orchestrateAssemble(ports: BaselinePorts, request: AssembleRequest): Promise<SpokeResult<AssembleResponse>>` | `pub async fn orchestrate_assemble(ports: &impl BaselinePorts, request: AssembleRequest) -> SpokeResult<AssembleResponse>` | `ScopeQueryPort` | Query scoped entries/events; apply scope helpers; call `buildAssemblePacket`; return packet |
| project | `export async function orchestrateProject(ports: ComputablePorts, request: ProjectRequest): Promise<SpokeResult<ProjectResponse>>` | `pub async fn orchestrate_project(ports: &impl ComputablePorts, request: ProjectRequest) -> SpokeResult<ProjectResponse>` | `ComputablePort` | Call `validateProjectRequest`; call `project` |
| compute | `export async function orchestrateCompute(ports: ComputablePorts, request: ComputeRequest): Promise<SpokeResult<ComputeResponse>>` | `pub async fn orchestrate_compute(ports: &impl ComputablePorts, request: ComputeRequest) -> SpokeResult<ComputeResponse>` | `ComputablePort` | Call `validateComputeRequest`; call `compute`; any settled-state persistence is an explicit adapter step |
| fork check | `export async function orchestrateForkCheck(ports: ForkPorts, request: CheckRequest, runChecker: (input: CheckRunInput) => SpokeResult<Finding[]>): Promise<SpokeResult<CheckResponse>>` | `pub async fn orchestrate_fork_check(ports: &impl ForkPorts, request: CheckRequest, run_checker: impl FnOnce(CheckRunInput) -> SpokeResult<Vec<Finding>>) -> SpokeResult<CheckResponse>` | `ForkTimelineQueryPort` plus baseline check ports | Validate `scope.fork_id`; load knowledge entries via `ScopeQueryPort.listKnowledgeEntries`; load timeline events via `ForkTimelineQueryPort.listForkTimelineEvents`; resolve rules; apply scope helpers; invoke `runChecker`; call `putFindings` |
| fork assemble | `export async function orchestrateForkAssemble(ports: ForkPorts, request: AssembleRequest): Promise<SpokeResult<AssembleResponse>>` | `pub async fn orchestrate_fork_assemble(ports: &impl ForkPorts, request: AssembleRequest) -> SpokeResult<AssembleResponse>` | `ForkTimelineQueryPort` plus baseline assemble ports | Validate `scope.fork_id`; load knowledge entries via `ScopeQueryPort.listKnowledgeEntries`; load timeline events via `ForkTimelineQueryPort.listForkTimelineEvents`; apply scope helpers; call `buildAssemblePacket` |

Orchestrators compose pure helpers and port I/O only. Checker engines, compute engines, ranking, retrieval, transactions, and retries remain adapter- or product-owned. The adapter controls transaction boundaries.

### Public export and module paths

TypeScript places ports in `packages/spoke-operations/src/adapter/ports.ts` and entrypoints in `packages/spoke-operations/src/adapter/orchestrate.ts`; `src/index.ts` flat-re-exports both. Rust mirrors this at `crates/spoke-operations/src/adapter.rs` with `ports` and `orchestrate` submodules; `src/lib.rs` flat-re-exports the public traits and functions. Pure helper family paths remain unchanged.

### TS/Rust parity table

| TypeScript | Rust |
|---|---|
| `KnowledgeEntryPort` | `KnowledgeEntryPort` |
| `RelationPort` | `RelationPort` |
| `ScopeQueryPort` | `ScopeQueryPort` |
| `FindingPort` | `FindingPort` |
| `RuleQueryPort` | `RuleQueryPort` |
| `ComputablePort` | `ComputablePort` |
| `ForkTimelineQueryPort` | `ForkTimelineQueryPort` |
| `HostManifestPort` | `HostManifestPort` |
| `BaselineAdapter` | `BaselineAdapter` (marker trait; TS: `type BaselineAdapter = BaselinePorts`) |
| `ComputableAdapter` | `ComputableAdapter` (marker trait; TS: `type ComputableAdapter = ComputablePorts`) |
| `ForkAdapter` | `ForkAdapter` (marker trait; TS: `type ForkAdapter = ForkPorts`) |
| `FullAdapter` | `FullAdapter` (marker trait; TS: `type FullAdapter = FullPorts`) |
| `CheckRunInput` | `CheckRunInput` |
| `orchestrateUpsert` | `orchestrate_upsert` |
| `orchestratePromote` | `orchestrate_promote` |
| `orchestrateRelate` | `orchestrate_relate` |
| `orchestrateCheck` | `orchestrate_check` |
| `orchestrateAssemble` | `orchestrate_assemble` |
| `orchestrateProject` | `orchestrate_project` |
| `orchestrateCompute` | `orchestrate_compute` |
| `orchestrateForkCheck` | `orchestrate_fork_check` |
| `orchestrateForkAssemble` | `orchestrate_fork_assemble` |

## Package contract

### TypeScript

| Field | Value |
|-------|-------|
| Name | `@42ch/spoke-operations` |
| Path | `packages/spoke-operations/` |
| Dependency | `@42ch/spoke-schemas` (workspace) only |
| Publish | npm on stable tags (`@42ch/spoke-schemas` first, then this package) |
| Behavioral SSOT | This spec + TypeScript and Rust implementations at behavioral parity |

Public entry: `src/index.ts` re-exporting all families above plus `SpokeResult`, `SpokeReject`, `SpokeRejectCode` types/constants.

### Rust

| Field | Value |
|-------|-------|
| Name | `spoke-operations` |
| Path | `crates/spoke-operations/` |
| Dependency | `spoke-schemas` (workspace) only |
| Publish | crates.io on stable tags (`spoke-schemas` first, then this crate) |
| Parity rule | Behavioral parity with `@42ch/spoke-operations` — same normative helper families (baseline + deepen + computable + adapter orchestration), same `SpokeRejectCode` string literals, same In/Out tables |
| `SpokeResult` | Rust `enum SpokeResult<T> { Ok(T), Reject(SpokeReject) }` with `spoke_ok` / `spoke_reject` — code strings match TS; idiomatic Rust surface, not a second vocabulary |

Public entry: `src/lib.rs` flat re-exports (snake_case function names) covering **every** symbol in TS `src/index.ts`. Rust MAY also export additional typed/wire helpers not listed in TS `index.ts` — e.g. `KnowledgeEntryForAssemble`, `validate_promote_request_wire`, `UpsertMode`, `ExtensionMap`, `spoke_ok_unit` — without breaking parity.

**Module layout:** one source file per helper family (`result`, `extensions`, `finding`, `promote`, `assemble`, `body`, `occ`, `knowledge_entry`, `scope`, `timeline`, `upsert`, `relate`, `error`, `computable`); adapter ports/orchestrators use `adapter/ports.ts` and `adapter/orchestrate.ts` in TypeScript and `adapter.rs` submodules in Rust; private `util` is for typify field-access helpers only — no parallel wire DTOs.

**Wire types:** helpers accept `spoke_schemas` generated types directly (`KnowledgeEntry`, `Finding`, `ErrorEnvelope`, etc.).

---

## Acceptance (operations layer)

### First slice (TypeScript)

- [x] This spec + [`spoke-protocol.md`](spoke-protocol.md) cross-link (umbrella column 3)
- [x] Package exists with four helper families and unit tests per table above
- [x] `SpokeResult` / `SpokeRejectCode` exported and used on all reject paths
- [x] No I/O, LLM, ranking, retrieval, or storage imports in package dependency graph
- [x] CI typecheck + test + build includes `packages/spoke-operations/`

### Deepen + computable (TypeScript and Rust)

- [x] OCC, KnowledgeEntry status, uniqueness, Scope, upsert, relate, error-map, and computable validator families implemented per §Helper families (operations deepen + computable) in `@42ch/spoke-operations` and `spoke-operations`
- [x] `REVISION_CONFLICT` and `STORED_REVISION_STALE` emitted on documented paths in both packages
- [x] First-slice export behavior unchanged except additive OCC emit on new call sites

### Rust crate (shippable)

- [x] `spoke-operations` crate at `crates/spoke-operations/` re-exports all normative helper families and every TS `index.ts` symbol (first-slice + deepen + computable)
- [x] All 20 `SpokeRejectCode` strings exported from `result` module, including `CAPABILITY_PORT_MISSING`
- [x] `cargo test -p spoke-operations` in CI and release verify
- [x] crates.io publish after `spoke-schemas` on stable tags

### Computable slice (`l2-computable`)

- [x] `validateComputableFieldMap`, `validateComputableLogEntry`, `validateProjectRequest`, `validateComputeRequest` exported from `@42ch/spoke-operations` `src/index.ts` and `spoke-operations` `src/lib.rs`
- [x] No compute execution, WASM, or I/O in `packages/spoke-operations/` or `crates/spoke-operations/`

### Body attributes (`body.attributes` read)

- [x] `listBodyAttributes`, `filterBodyAttributesByTraitType`, `findBodyAttribute` exported from `@42ch/spoke-operations` `src/index.ts`
- [x] `list_body_attributes`, `filter_body_attributes_by_trait_type`, `find_body_attribute` re-exported from `spoke-operations` `src/lib.rs`
- [x] Read/filter semantics documented in §13 (missing → empty; duplicate `trait_type` → all matches; malformed wire skip)

### Narrative sequence (`timeline/*` beat assist)

- [x] `filterTimelineEventsByMomentScale`, `orderTimelineEventsByIds`, `orderTimelineEventsByPrecedes` exported from `@42ch/spoke-operations` `src/index.ts`
- [x] `filter_timeline_events_by_moment_scale`, `order_timeline_events_by_ids`, `order_timeline_events_by_precedes` re-exported from `spoke-operations` `src/lib.rs`
- [x] §14 documents filter/order semantics, dual KE link rule, cycle reject, stable sort

### Adapter interfaces + injection orchestration

- [x] Capability → interface family → methods matrix complete for `spoke-baseline` (six families incl. `HostManifestPort`), `l2-computable`, and `l5-fork` (no TBD cells)
- [x] `HostManifestPort` exported + required on `BaselinePorts` / `BaselineAdapter` (TS + Rust)
- [x] Host collaboration section documents roles↔ports map, namespace exclusivity, peer-list semantics, and `HostManifestPort` contract
- [x] Injection orchestration sequences documented for baseline five ops, `project`/`compute`, and fork-aware paths (no TBD cells)
- [x] Port interfaces and orchestration entrypoints exported from TS `src/index.ts` and Rust `src/lib.rs` per the parity table
- [x] Orchestrators call pure helpers and perform I/O only through injected ports
- [x] Missing optional port returns `CAPABILITY_PORT_MISSING` (not TypeError / panic)
- [x] In-package mock adapters cover baseline five ops plus at least one computable and one fork-aware path

## Non-goals (operations layer)

### Pure helpers and wire gates

- Product DTO conversion in consumer product repositories (outside this protocol repo)
- Conformance fixtures / golden files (owned by `fixtures/toy-world/`)
- `Rule` evaluation, checker engines, Guardian detectors
- HTTP/MCP binding or daemon routes
- Ranking / retrieval / token-budget helpers
- Storage fetch inside the library
- HTTP/MCP status code tables
- Compute engine execution, WASM, Session store I/O

Product adapter implementations satisfy the ports defined here; they ship in consumer repos when scheduled. Reference `ToyWorldAdapter` examples live under `fixtures/toy-world/` (TS `src/adapter/`; Rust `spoke-fixture-toy-world` in `rust/`).

## Related paths

| Path | Role |
|------|------|
| [`spoke-ops.md`](spoke-ops.md) | Ops **wire** request/response (column 2) |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | L0–L8 map; Check≠Assemble boundary framing |
| [`spoke-data-model.md`](spoke-data-model.md) | Data objects helpers operate on |
| [`.mstar/roadmap.md`](../roadmap.md) | Thrust A column 3 mandate |
| `packages/spoke-operations/` | TypeScript operations library (pure helpers + adapter ports/orchestration) |
| `crates/spoke-operations/` | Rust operations library — behavioral parity with `@42ch/spoke-operations` at lockstep SemVer |
| `crates/spoke-schemas/` | Generated Rust wire types |
| `fixtures/toy-world/src/adapter/` | TypeScript reference `ToyWorldAdapter` (private workspace package) |
| `fixtures/toy-world/rust/` | Rust `spoke-fixture-toy-world` crate (`publish = false`) |
