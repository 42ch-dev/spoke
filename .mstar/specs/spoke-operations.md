# SPOKE Operations Library

> **Status:** Normative (v0-iter002)  
> **Document class:** Detail — hand-written behavior layer (column 3)  
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)  
> **Package:** `@42ch/spoke-operations` under `packages/spoke-operations/`

## Problem & user value

Wire schemas (`@42ch/spoke-schema`) tell integrators **what** crosses the boundary. They do not encode cross-product **lifecycle invariants** — promote gates, Finding status rules, extension round-trip preservation, or wire-valid `AssemblePacket` construction.

Without a shared operations library, every product (Nexus, Creader, future adapters) reimplements the same pure rules and drifts. **`@42ch/spoke-operations`** is the single hand-written place for those invariants: callable from adapters and product code, with **no** I/O, storage, LLM, ranking, or retrieval.

**Integrator outcome:** import types from `@42ch/spoke-schema`, import lifecycle helpers from `@42ch/spoke-operations`, bind transport locally — no shared daemon required.

---

## Three-way boundary (normative)

| Layer | Authored how | Owns | Does not own |
|-------|--------------|------|--------------|
| **Wire schemas** | Hand-written JSON Schema in `schemas/` → generated `@42ch/spoke-schema` | Object shapes, ops request/response envelopes, `extensions` bag presence | Lifecycle transitions, merge semantics, promote gates |
| **Operations library** | Hand-written TypeScript in `packages/spoke-operations/` | Pure functions / small state machines over generated types | HTTP/MCP, persistence, LLM, ranking, retrieval, product-specific detectors |
| **Adapters** | Hand-written per product in `adapters/*` | Product DTO ↔ SPOKE mapping, transport binding | Reimplementing operations invariants (MUST call library instead) |

### Hard In / Out

| **In (library MUST provide)** | **Out (library MUST NOT)** |
|---------------------------------|----------------------------|
| Extension map merge + round-trip preserve | Storage read/write |
| Finding `status` transition validation + apply | HTTP routes, MCP tools, message queues |
| Promote acceptance checks (pure gate before persist) | LLM calls, checker engines, Guardian logic |
| AssemblePacket builders from Keyblocks (structure only) | Ranking, scoring, vector retrieval, token budgeting |
| Unified `SpokeResult` / `SpokeRejectCode` on every reject path | Silent auto-promote bypassing human review semantics |
| Revision bump on promote apply (see §Promote acceptance) | OCC against stored revision (deferred — codes reserved only) |

### Per-family In / Out

| Family | **In** | **Out** |
|--------|--------|---------|
| **Extensions** | Deep merge; overlay wins scalars; preserve unknown namespaces/keys | Dropping empty `{}` namespaces; mutating inputs |
| **Finding** | Transition table enforcement; no-op same-status; structured reject | Product-specific workflow beyond cross-product minimum |
| **Promote** | Provisional gate; terminal-status reject; revision bump; merge-target id guard | Persist; fetch stored Keyblock; OCC compare (deferred) |
| **Assemble** | Wire-valid `AssemblePacket`; `snippet` from `body.summary` rule; order-preserving `maxEntries` truncate | Sort, rank, dedupe, token count, embedding search |

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

### `SpokeRejectCode` (v0-iter002)

Stable string literals exported from `@42ch/spoke-operations` (e.g. `as const` object + union type). Implementers MUST NOT invent parallel code strings.

| Code | Family | Emitted in v0-iter002 | Meaning |
|------|--------|----------------------|---------|
| `INVALID_INPUT` | shared | yes | Argument fails shape/null checks before domain rules |
| `INVALID_STATUS` | finding | yes | `to` (or current `finding.status`) not in core vocabulary |
| `INVALID_STATUS_TRANSITION` | finding | yes | Disallowed `from` → `to` (see transition table) |
| `CANDIDATE_NOT_PROVISIONAL` | promote | yes | `candidate.status` ≠ `provisional` (default gate) |
| `CANDIDATE_TERMINAL_STATUS` | promote | yes | `candidate.status` is `merged` or `deleted` |
| `EMPTY_CANONICAL_NAME` | promote | yes | `canonical_name` missing or whitespace-only |
| `MERGE_TARGET_SELF` | promote | yes | `target_keyblock_id` equals `candidate.keyblock_id` |
| `MISSING_REQUIRED_FIELD` | promote | yes | Required Keyblock field absent (schema-aligned check) |
| `INVALID_PACKET_INPUT` | assemble | yes | e.g. empty `packetId`, negative `maxEntries` |
| `REVISION_CONFLICT` | promote (OCC) | **no** — reserved | Stored revision ≠ expected (full OCC deferred) |
| `STORED_REVISION_STALE` | promote (OCC) | **no** — reserved | Caller-supplied base revision behind store (deferred) |

**OCC deferral (v0-iter002):** export `REVISION_CONFLICT` and `STORED_REVISION_STALE` in the public code enum for forward compatibility; **no helper emits them** until a later slice adds stored-state compare.

---

## First-slice helper inventory (v0-iter002)

Four families. Export names below are **normative** for v0-iter002; module layout under `src/` may group them, but public `src/index.ts` MUST expose these symbols.

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

**Allowed transitions (v0-iter002):**

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
| `applyPromoteAcceptance(request)` | On success, return `SpokeResult<Keyblock>` — promoted view (`status: confirmed`, revision bump per below); does **not** persist | Pure |

**Rules encoded (minimum):**

- `candidate` MUST satisfy Keyblock required fields (delegate to schema-shaped checks, not a parallel DTO).
- `candidate.status` MUST be `provisional` unless product documents an explicit override path (default: reject non-provisional → `CANDIDATE_NOT_PROVISIONAL`).
- `candidate.canonical_name` MUST be non-empty (`minLength` semantics → `EMPTY_CANONICAL_NAME`).
- If `target_keyblock_id` present: MUST NOT equal `candidate.keyblock_id` → `MERGE_TARGET_SELF`; merge semantics are structural only (no storage fetch).
- Reject `candidate` in terminal Keyblock statuses (`merged`, `deleted`) → `CANDIDATE_TERMINAL_STATUS`.
- **Human-in-loop invariant:** library never silently upgrades provisional → confirmed without caller explicitly invoking promote acceptance (no hidden side effects).

**Revision bump on apply (normative):**

| `candidate.revision` before apply | `revision` on returned Keyblock |
|-----------------------------------|---------------------------------|
| absent / `undefined` | `1` |
| integer ≥ 0 | `candidate.revision + 1` |

Returned Keyblock also sets `status: "confirmed"`. Other fields are shallow-copied from `candidate` unless promote rules explicitly transform them. Library does **not** set `updated_at` unless a later slice adds an optional clock parameter — v0-iter002 leaves timestamps to the caller/adapter.

**Reject codes:** `CANDIDATE_NOT_PROVISIONAL`, `CANDIDATE_TERMINAL_STATUS`, `EMPTY_CANONICAL_NAME`, `MERGE_TARGET_SELF`, `MISSING_REQUIRED_FIELD`, `INVALID_INPUT`.

**Tests must cover:** happy path provisional→confirmed, reject deleted/merged candidate, reject empty name, merge-target id collision, revision `undefined`→`1`, revision `2`→`3`.

**Deferred to later slice (not v0-iter002):** full OCC conflict resolution against stored revision — `REVISION_CONFLICT` / `STORED_REVISION_STALE` reserved in enum only (§Result / reject envelope).

---

### 4. AssemblePacket builder — `assemble/*`

| Export | Purpose | Purity |
|-------------------|---------|--------|
| `keyblockToAssembleEntry(keyblock)` | Map `Keyblock` → slim `AssembleEntry` per rules below | Pure |
| `buildAssemblePacket({ packetId, keyblocks, extensions?, maxEntries? })` | Build valid `AssemblePacket`; `maxEntries` truncates **input order** only (no sort/rank) | Pure |

**`keyblockToAssembleEntry` mapping (normative):**

| Output field | Source |
|--------------|--------|
| `keyblock_id` | `keyblock.keyblock_id` |
| `block_type` | `keyblock.block_type` |
| `canonical_name` | `keyblock.canonical_name` |
| `snippet` | See rule below — **omit key** when rule does not apply |

**`snippet` from `body.summary`:**

1. Read `keyblock.body` as a record; if `body.summary` is **not** a string, omit `snippet`.
2. If it is a string, `trim()` it; if trimmed length is `0`, omit `snippet`.
3. Otherwise set `snippet` to the trimmed string.

Do **not** coerce non-strings, fall back to other `body` keys, or emit `snippet: ""`.

**`buildAssemblePacket`:** maps each input Keyblock via `keyblockToAssembleEntry`; when `maxEntries` is a positive integer, keep the first *n* entries in input order; when omitted, include all. Reject invalid args via `INVALID_PACKET_INPUT`.

**Explicitly out:** scoring, embedding search, deduplication by relevance, token counting.

**Tests must cover:** empty keyblock list, snippet present/absent/whitespace-only, non-string `body.summary`, `maxEntries` truncation preserves order, `extensions` passthrough.

---

## Package contract

| Field | Value |
|-------|-------|
| Name | `@42ch/spoke-operations` |
| Dependency | `@42ch/spoke-schema` (workspace) only |
| Publish | Private workspace package; no npm publish job in CI |
| Rust | Deferred (`spoke-operations` crate not in v0-iter002) |

Public entry: `src/index.ts` re-exporting the four families above plus `SpokeResult`, `SpokeReject`, `SpokeRejectCode` types/constants.

---

## Acceptance (operations layer — iteration)

- [x] This spec + [`spoke-protocol.md`](spoke-protocol.md) cross-link (umbrella column 3)
- [ ] Package exists with four helper families and unit tests per table above
- [ ] `SpokeResult` / `SpokeRejectCode` exported and used on all reject paths
- [ ] No I/O, LLM, ranking, retrieval, or storage imports in package dependency graph
- [ ] CI typecheck + test includes `packages/spoke-operations/`

## Non-goals (operations layer — v0-iter002)

- Adapter conversion code
- Rust operations crate
- Conformance fixtures / golden files
- `Rule` evaluation, checker engines, Guardian detectors
- HTTP/MCP binding or daemon routes
- Ranking / retrieval / token-budget helpers

---

## Related paths

| Path | Role |
|------|------|
| [`spoke-ops.md`](spoke-ops.md) | Ops **wire** request/response (column 2) |
| [`spoke-data-model.md`](spoke-data-model.md) | Data objects helpers operate on |
| [`.mstar/roadmap.md`](../roadmap.md) | Thrust A column 3 mandate |
| `packages/spoke-operations/` | Implementation (v0-iter002) |
