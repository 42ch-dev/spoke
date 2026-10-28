# SPOKE Memory (`spoke-memory`)

> **Status:** Normative
> **Document class:** Normative — optional capability family (wire + operations + reference engine)
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)
> **Wire SSOT:** `schemas/data/memory-*.schema.json`, `schemas/ops/memory-*-{request,response}.schema.json`
> **Operations library:** `packages/spoke-operations/src/memory/`, `crates/spoke-operations/src/memory/`; port `MemoryPort` in `packages/spoke-operations/src/adapter/ports.ts`, `crates/spoke-operations/src/adapter/ports.rs`
> **Reference engine:** `crates/spoke-memory/` (`MemoryEngine`, `ENGINE_POLICY_VERSION`)
> **Capability decision:** [`spoke-memory-capability-adr.md`](spoke-memory-capability-adr.md)

## Problem & user value

A narrative product needs memory that is **subjective** — what one subject experienced, believed, or felt — alongside its canonical KnowledgeEntry graph. Canonical knowledge is shared, reviewed, and stable; memory is per-subject, provisional until reviewed, and deletable on request.

| Without a memory family | With `spoke-memory` |
|-------------------------|---------------------|
| Each product invents its own memory tables and semantics | One wire contract for spaces, subjects, capture, review, records, recall, profiles, reflections, grants, and transfer |
| "What does this character remember" is unanswerable across hosts | Per-subject records with placement (`scope`) and provenance (`source_context_ids` / `anchors`) as independent axes |
| A provider swap means a bespoke migration script | Versioned, digested export/import with an operator-timed cutover sequence |
| Memory quietly leaks into KnowledgeEntry canon | A separate optional family with its own authorization model and no baseline expansion |

The key words **MUST**, **MUST NOT**, **SHOULD**, and **MAY** in this document are to be interpreted as described in RFC 2119. They apply to implementations that claim the `spoke-memory` capability.

## User value summary

| Need | Where this spec answers it |
|------|----------------------------|
| Which capability flags a provider must declare | §4.1 |
| Which invariants a conforming provider must never violate | §5 (`MEM-R001`–`MEM-R027`) |
| Exact data shapes | §6 (one section per committed schema) |
| Exact op set, grant per op, and error per op | §7 |
| Lifecycle and state machines | §8 |
| Recall ordering, budgets, and truncation | §9 |
| Trust boundary between authored profiles and captured text | §10, §16 |
| Multi-client, revoke, freeze, delete races | §11 |
| Provider replacement and rollback | §12 |
| How ops ride Connect, and the chunked-export header | §13 |
| Error codes and retry classes | §14 |
| Locked protocol semantics (subjectivity, forgetting, affect, flat subjects) | §15 |
| ProfileDocument substance | §16 |
| What a conformance scenario must cite | §17 |
| What exists in this repository today | §18 |

---

## 1. Scope and non-goals

### 1.1 In scope

- A complete wire and service contract for per-subject long-term memory: spaces, subjects, capture, candidate review, retained records, recall, profiles, reflections, grants, write fencing, and versioned export/import.
- The single public replacement boundary `MemoryPort` (in-process) and `RemoteMemoryService` (Connect responder seam) through which a product embeds or serves a memory provider.
- A separately reusable reference algorithm engine (`spoke-memory` crate, type `MemoryEngine`) holding deterministic, I/O-free policy: candidate classification, recall ranking and budgeting, reflection assembly and staleness, and transfer partitioning/verification/translation. The engine receives already-authorized data as pure function inputs; it holds **no storage abstraction** and enforces no authorization, CAS, idempotency, fencing, or transaction semantics. Those belong to the provider behind `MemoryPort`, in the provider's own transaction layer. The engine is not a protocol participant.
- Concurrency, revocation, freeze, delete, and provider-switching semantics that hold across multiple clients of one authoritative provider.

### 1.2 Non-goals

- Distributed exactly-once delivery, cross-provider or cross-space atomic transactions, CRDT, offline multi-master replication, federated recall, and silent local fallback. One `MemorySpace` has exactly one authoritative provider (`MEM-R001`).
- Transport definition beyond mapping onto the existing Connect envelope (§13). No HTTP routes, headers, or methods appear in this family.
- Mandated identical ranking, summarization, or reflection algorithms across providers (§9, §17). Algorithm policy is engine-versioned, not protocol-versioned.
- A space-creation or provider-bootstrap op: spaces and initial administrator grants are provider-operator provisioned out of band (§7.0.6).
- A product identity model: `Subject` is a stable opaque ID with optional namespaced `kind` metadata; equal subject IDs across systems never auto-link identities (`MEM-R002`).
- Changes to `BaselinePorts`, the six baseline port families, the core op table (`upsert` / `promote` / `relate` / `check` / `assemble` / `project` / `compute` / `extract`), or the `port.*` catalogue, except the additive extension points in §13.
- Storage shape. Physical storage of records, profiles, reflections, and staged imports is provider-owned; the protocol prescribes wire shapes only.

## 2. Terminology

| Term | Definition |
|------|-----------|
| **MemorySpace** | One logical memory store with exactly one authoritative provider. The unit of fencing, export, and import activation. |
| **Subject** | The entity memory is about. Stable opaque `subject_id` chosen by the integrating product; optional namespaced `kind` metadata. |
| **Principal** | An authenticated caller identity established by the transport/session or in-process admission context — never asserted by the requester through payload fields (`MEM-R003`). Payloads may *select* a target principal (e.g. a grantee), never impersonate the requester (§7.0.2, §6.4). |
| **MemoryGrant** | A provider-enforced authorization record binding (principal, space, optional subject, capability set, optional context restriction, shared-access flag). |
| **RetainedRecord** | A reviewed or directly written durable memory record. Untrusted content; trusted only as to provenance and scope metadata. |
| **MemoryCandidate** | A provisional, not-yet-retained capture product with an immutable admitted `capture_scope`. Never visible to `memory.recall` before review (`MEM-R004`). |
| **ProfileDocument** | Author-maintained, trusted subject document (authored content kinds; core: `personality`, `preferences`). §16. |
| **Reflection** | A derived, generated synthesis over a pinned basis (selection + provenance filter + authorized source set + revisions + policy). |
| **Context** | A placement/visibility partition within a subject (product-meaningful ID, e.g. a binding). Placement (where a record lives), provenance (where material came from), and read selection (what a view includes) are independent concepts. |
| **ProvenanceFilter** | A read/projection filter over `provenance.source_context_ids` — e.g. a per-scope projection. Never a placement statement. |
| **MemoryReceipt** | Durable, payload-bound, content-free operation receipt; the single async/outcome model of this family (§7.0.3). |
| **MemoryExportBundle / MemoryExportChunk** | Versioned, digested logical export artifact and its bounded wire chunks (§6.11, §6.13). |

## 3. Versioning

Three independent version axes; conforming implementations MUST NOT conflate them.

1. **`memory_protocol_version`** (integer ≥ 1; this document defines version `1`). Wire contract. Carried in `memory.describe` responses and `MemoryExportBundle`. Breaking wire changes increment it; cross-version interop rules are out of scope for version 1 (single-version deployments).
2. **Package versions** (SPOKE repository lockstep SemVer; see [`spoke-version-release.md`](spoke-version-release.md)). Orthogonal to protocol versions.
3. **`engine_policy_version`** (opaque string). Reference-engine algorithm policy (candidate classification, recall scoring, reflection assembly). Surfaced by `memory.describe` and recorded on each `Reflection.generator`. Two providers with equal `memory_protocol_version` and different `engine_policy_version` are wire-compatible and algorithm-variable (§9, §17).

`bundle_format_version` (§6.11) is a fourth, independent axis for the export artifact and starts at `1`.

## 4. Capability and profile compatibility

### 4.1 Capability flags

| Capability ID | Gates | Required for conformance |
|---------------|-------|--------------------------|
| `spoke-memory` | All `memory.*` ops except those gated by the two sub-flags below | Yes (family core) |
| `spoke-memory-reflection` | `memory.reflections.state`, `memory.reflections.list`, `memory.reflections.generate` | Optional |
| `spoke-memory-transfer` | `memory.exports.create`, `memory.exports.get`, `memory.imports.stage`, `memory.imports.activate`, `memory.imports.discard` | Optional |

Rules:

- A provider declaring `spoke-memory-reflection` or `spoke-memory-transfer` MUST also declare `spoke-memory`.
- A provider declaring `spoke-memory` without a sub-flag MUST reject that sub-flag's ops with `capability_missing` at the dispatch/admission boundary, with zero side effects.
- Hosts advertise these flags in `HostCapabilityManifest.capabilities[]` (open string vocabulary, same carrier as `spoke-baseline` / `l2-computable` / `ke-extraction`; see [`spoke-data-model.md`](spoke-data-model.md)). Connect peers negotiate them in the hello exchange; the dispatch gate evaluates **negotiated** capabilities only (§13).
- Baseline compliance MUST NOT require any memory flag (§18.6, `MEM-R019`).

### 4.2 Profile compatibility

`ProfileDocument.profile_kind` is an open string vocabulary of **authored content kinds**. Core vocabulary (documented, not schema-enforced): `personality`, `preferences`. Profile kinds are never capability concepts — provider features are expressed exclusively through the §4.1 flags, never through the presence or absence of profile documents.

- Each provider declares `required_profile_kinds` in `memory.describe` — the authored content kinds a subject must present before reflection generation is admitted. Admission fails with `PROFILE_INCOMPLETE` if any required kind is absent or empty for that subject; there is no empty success (`MEM-R005`).
- Absence of a *non-required* profile kind is honest-empty: `memory.profile.get` returns fewer documents, never an error.
- Provider **selection** failure (missing capability → `capability_missing`) and profile **admission** failure (missing authored content → `PROFILE_INCOMPLETE`) are distinct outcomes and MUST NOT be collapsed (§16.4).

## 5. Normative invariants

Anchors `MEM-R###` are the conformance reference points referenced by conformance scenarios (§17), the operations library, and the reference engine.

| Anchor | Invariant |
|--------|-----------|
| MEM-R001 | One authoritative provider per `MemorySpace`. A deployment MUST route every write for a space through that provider, and multi-client write access MUST go through it under `revision` CAS and grant evaluation. Clients MUST NOT write locally and reconcile later. |
| MEM-R002 | Equal IDs across systems MUST NOT be treated as the same identity. Identity mapping is explicit product policy — an importing provider MUST translate identities only through the staged maps of `memory.imports.stage` (`subject_id_map`, `record_id_map`, `candidate_id_map`) or an explicit `rename_subject` collision decision. |
| MEM-R003 | Principal identity, ownership, consuming subject, and grants MUST derive from the authenticated admission context and provider state. Model- or payload-supplied type, owner, or grant JSON MUST NOT be treated as authority. A payload principal field MUST be treated only as a target selector for an admin operation, never as requester identity. |
| MEM-R004 | `memory.recall` MUST NOT return candidates and MUST NOT count them in `total_authorized`. A candidate MUST become visible memory only through `memory.candidates.review` (`MEM-R009`). |
| MEM-R005 | Missing required authored profile content MUST fail `memory.reflections.generate` with `PROFILE_INCOMPLETE`; a provider that does not serve a requested sub-capability MUST reject the op with `capability_missing` before any handler runs; absent optional data MUST return an honest-empty success. Implementations MUST NOT substitute an empty success for a fail-closed rejection. |
| MEM-R006 | Providers MUST evaluate grant coverage — including `context_ids` restriction, `shared_access`, and the selection mode (`subject_all` / `shared_only` / `selected_contexts`) — before ranking and pagination. Returned records, `total_authorized` counts, pages, and cursors MUST reflect only authorized data. |
| MEM-R007 | Every mutable memory object MUST carry `revision` (integer ≥ 1); a conditional write MUST require `expected_revision` and MUST fail `REVISION_CONFLICT` on mismatch. A genuine no-op MUST succeed without incrementing `revision`; content changes are never no-ops. |
| MEM-R008 | Every mutating op except `memory.subjects.ensure` MUST require `idempotency_key`. The provider MUST durably bind `(space_id, requester_principal_id, op, idempotency_key)` to the `payload_digest` and to the produced receipt. Replay of the same key with the same digest MUST return the original receipt without re-executing; the same key with a different digest MUST fail `IDEMPOTENCY_CONFLICT`. |
| MEM-R009 | A review or dismissal MUST commit candidate consumption, the §6.7 content scrub, and every resulting record and receipt in one provider transaction. A partial outcome MUST NOT be observable. |
| MEM-R010 | After a timeout, transport failure, or uncertain outcome, the client MUST retry with the same `idempotency_key` and/or poll `memory.receipts.get`; the provider MUST NOT double-apply an effect. |
| MEM-R011 | The read-only ops (`memory.describe`, `memory.spaces.list`, `memory.subjects.get`, `memory.grants.list`, `memory.candidates.list`, `memory.candidates.get`, `memory.records.get`, `memory.records.list`, `memory.recall`, `memory.profile.get`, `memory.reflections.state`, `memory.reflections.list`, `memory.receipts.get`, `memory.exports.get`) MUST NOT mutate persistent state, MUST NOT trigger reflection generation, and MUST NOT create candidates or records. |
| MEM-R012 | Reflection generation MUST be requested through `memory.reflections.generate`, MUST be authorized by a `reflect` grant covering the pinned selection and filter, and MUST record cost. It MUST NOT be a side effect of a read or of capture. |
| MEM-R013 | Any movement of a reflection's pinned basis — new, edited, or deleted records in scope, profile updates, and placement promotions moving records into or out of scope — MUST make the reflection read `stale: true` at read time. A stored staleness flag is a write-time cache only. |
| MEM-R014 | `memory.records.delete` MUST remove record content, MUST exclude the record from recall and from default export, and MUST retain a tombstone plus all receipts (`MEM-R022`) for audit. |
| MEM-R015 | Every collection MUST be finite and paginated, and every request, stored-material, and response size MUST have a declared limit surfaced by `memory.describe.limits`. Exceeding a limit MUST fail `INVALID_INPUT` with `details.limit` naming it. |
| MEM-R016 | Every error MUST use a stable machine code from §14.1 and MUST carry an explicit `details.retry_class` of `retryable`, `terminal`, or `uncertain`. |
| MEM-R017 | Space freeze and subject/context fences MUST be enforced by the provider at admission for all clients of the authority. Per-process locks MUST NOT be treated as satisfying this invariant. |
| MEM-R018 | Grant revocation MUST take effect at admission of subsequent ops; ops that already passed admission MUST complete and issue receipts. A revoked principal MUST NOT read receipts. |
| MEM-R019 | When `spoke-memory` is not negotiated or not served, a client MUST NOT emulate memory semantics through `KnowledgeEntry`, `modules.*`, `Scope.viewpoint`, or `port.*` ops; it MUST report the missing capability. |
| MEM-R020 | The memory grant model MUST NOT inherit, imply, or consult `ke-ownership` / `Scope.viewpoint` disclosure semantics, and memory ops MUST NOT accept a `Scope` object as authorization input. |
| MEM-R021 | Replay MUST NOT re-evaluate superseded CAS or fence preconditions of an already-committed effect; receipt disclosure MUST still require current authorization. Dedup tombstones MUST outlive receipt bodies (`dedup_retention_seconds`); replay against an evicted receipt MUST fail `RECEIPT_EXPIRED` and MUST NOT silently re-execute. |
| MEM-R022 | `MemoryReceipt.result` MUST contain only identifiers, revisions, counts, outcome enums, digests, and cost facts. Record, candidate, profile, and reflection bodies MUST NOT appear in a receipt. |
| MEM-R023 | A frozen space MUST admit exactly the §8.2 allowlist. Quiescent export and cutover MUST require the drain barrier (`pending_receipts == 0`). |
| MEM-R024 | A context-scoped fence MUST reject writes that place into the fenced context across all clients while sibling contexts remain usable. |
| MEM-R025 | Export/import MUST preserve every candidate: a consumed candidate transfers as its content-scrubbed shell carrying `content_digest`, and a pending candidate transfers in full with its `proposed` content, so unreviewed material survives transfer. Content-free receipt and dedup summaries and all reference maps MUST be preserved. Grants MUST NOT be activated on import except those listed in `adopt_grants`. Imported reflections MUST enter `stale: true`. |
| MEM-R026 | Every cursor MUST bind principal, space, subject, selection/filter, snapshot marker, and `engine_policy_version`; any state or grant change MUST invalidate it with `CURSOR_INVALID`. Every by-id read MUST recheck current authorization. |
| MEM-R027 | Exactly one authority MUST exist through cutover and rollback: the old provider MUST stay frozen and reject writes. Once the target has accepted writes, rollback MUST require verified reverse transfer (delta reconciliation) before the source reopens. Two providers MUST NOT accept writes at once, and target-written history MUST NOT be deleted. |

## 6. Data models

Thirteen committed schemas under `schemas/data/` carry this family, generated into `packages/spoke-schemas/src/generated/data/` (TypeScript) and `crates/spoke-schemas/src/generated/data/` (Rust). They follow the repository schema rules: draft-07, closed objects (`additionalProperties: false`), `schema_version` + `extensions` required on durable objects, open string vocabularies documented in descriptions, absolute `$ref` URIs.

Conditional field rules that draft-07 `if`/`then`/`else` cannot express reachable from codegen are encoded as closed union variants (`oneOf`): `memory-grant.schema.json` (`RestrictedGrant` / `UnrestrictedGrant`), `retained-record.schema.json` (`LiveRecord` / `TombstonedRecord`), `memory-candidate.schema.json` (`PendingCandidate` / `ConsumedCandidate`), `memory-receipt.schema.json` (`CommittedReceipt` / `PendingReceipt` / `FailedReceipt`), `read-selection.schema.json` (one variant per mode), and `memory-export-chunk.schema.json` (`HeaderChunk` / `TailChunk`). Generated types mirror those variants.

Field tables below are normative; "—" in the Null/Omission column means the field is required and non-null.

### 6.1 Common primitives

| Name | Type | Semantics |
|------|------|-----------|
| `schema_version` | integer ≥ 1 | Per-object wire version (existing `SchemaVersion` def in [`common.schema.json`](../../schemas/common/common.schema.json)). |
| `*_id` | string, minLength 1 | Opaque. `subject_id` is product-chosen and stable; all other IDs are provider-assigned unless stated. |
| `revision` | integer ≥ 1 | Per-object optimistic-concurrency counter; increments on each accepted state-changing write (`MEM-R007`). |
| timestamps | string RFC 3339 UTC | Existing `Timestamp` def. |
| `extensions` | `ExtensionMap` | Existing def; adapters round-trip unknown namespaces verbatim. |
| `idempotency_key` | string, 1–128 chars | Caller-chosen; unique within the dedup namespace (§7.0.5). Declared per request schema with the scalar title `IdempotencyKey` (memory-local title; `common.schema.json` gains no new definition). |
| `payload_digest` | string, 64 lowercase hex | SHA-256 over the RFC 8785 JCS canonicalization of the request payload object. Binding to the op comes from the dedup namespace (§7.0.5), which includes the op name. Declared per receipt/request schema with the scalar title `PayloadDigest`. |
| `cursor` | string, opaque | Paginates one bound query (`MEM-R026`). |

Memory schemas reuse `SchemaVersion`, `Timestamp`, `ExtensionMap`, and `OpaqueJson` from `common.schema.json`; they add no shared definitions there.

### 6.2 `MemorySpace`

Schema: [`memory-space.schema.json`](../../schemas/data/memory-space.schema.json). Generated: `MemorySpace`.

| Field | Type | Null/Omission | Semantics |
|-------|------|---------------|-----------|
| `schema_version` | int | — | |
| `space_id` | string | — | Provider-assigned at operator provisioning. |
| `display_name` | string | optional | Human label; absence = unnamed. |
| `authority_host_id` | string | — | `host_id` of the single authoritative provider (`MEM-R001`). |
| `authority_generation` | int ≥ 1 | — | Increments on each cutover (import activation into the space) and on each operator re-open after rollback. Client bindings pin `(authority_host_id, authority_generation)` (§12.5). |
| `status` | string | — | `active` \| `frozen`. `frozen` admits only the §8.2 allowlist. |
| `pending_receipts` | int | — | Computed at read time: count of receipts with status `pending`. The drain barrier (`MEM-R023`). |
| `created_at` | Timestamp | — | |
| `revision` | int | — | Bumped by freeze/unfreeze and cutover. |
| `extensions` | ExtensionMap | — | |

### 6.3 `Subject`

Schema: [`subject.schema.json`](../../schemas/data/subject.schema.json). Generated: `Subject`.

| Field | Type | Null/Omission | Semantics |
|-------|------|---------------|-----------|
| `schema_version` | int | — | |
| `space_id` | string | — | |
| `subject_id` | string | — | Product-chosen stable opaque ID. |
| `kind` | object | optional | Namespaced kind metadata: `{ "ns": string (ExtensionMap key pattern `^[a-z][a-z0-9_-]*$`), "id": string }`. Absence = kind unspecified. Never a closed enum; never authorization input. |
| `status` | string | — | `active` \| `fenced` \| `archived` (§8.3). `archived` is terminal decommission; products with reversible archive semantics map them to fence/unfence (§7.7). |
| `fenced_contexts` | string[] | — | Currently context-fenced placement IDs (may be empty; `MEM-R024`). |
| `created_at` | Timestamp | — | |
| `revision` | int | — | Bumped by any fence/unfence/archive transition. |
| `extensions` | ExtensionMap | — | |

### 6.4 `Principal`

Schema: [`principal.schema.json`](../../schemas/data/principal.schema.json). Generated: `Principal`.

| Field | Type | Null/Omission | Semantics |
|-------|------|---------------|-----------|
| `principal_id` | string | — | Authenticated identity as established by the admission context. |
| `display_name` | string | optional | |
| `extensions` | ExtensionMap | — | |

Principals appear **only** inside provider state (grants, receipts, audit fields) and as *target selectors* in admin ops. The requester's principal always comes from the authenticated context (`MEM-R003`): over Connect, the hello/capability-token peer identity is mapped to a `principal_id` by provider-local configuration; in-process, the embedding product supplies it. A payload `principal_id` (e.g. in `memory.grants.grant`) names the *target grantee* and never reassigns the requester.

### 6.5 `MemoryGrant`

Schema: [`memory-grant.schema.json`](../../schemas/data/memory-grant.schema.json). Generated: `MemoryGrant` (`RestrictedGrant` / `UnrestrictedGrant` variants).

| Field | Type | Null/Omission | Semantics |
|-------|------|---------------|-----------|
| `schema_version` | int | — | |
| `grant_id` | string | — | Provider-assigned. |
| `space_id` | string | — | |
| `subject_id` | string | optional | Absence = space-wide grant. |
| `principal_id` | string | — | Grantee (target). |
| `capabilities` | string[] | — | minItems 1, uniqueItems. Open vocabulary; core: `capture`, `review`, `records_read`, `records_write`, `recall`, `profile_write`, `reflect`, `promote`, `transfer`, `admin`. `admin` implies grant management and all other capabilities within the grant's coverage. |
| `context_ids` | string[] | conditional | Context restriction over **placement** contexts; minItems 1, and present in the `RestrictedGrant` variant only. **Omitted** = unrestricted within the grant's subject/space scope (includes shared visibility and `subject_all` selection). **Present** = only the listed placement contexts, plus shared visibility iff `shared_access` is true. An empty array is invalid. |
| `shared_access` | bool | optional | Default `false`. Meaningful only with `context_ids`; the schema forbids it on `UnrestrictedGrant`. When true, the grant additionally covers shared-visibility data and `shared_only` / `include_shared` selections — still never `subject_all`. |
| `created_at` | Timestamp | — | |
| `expires_at` | Timestamp | optional | Absence = no expiry. Expired grants are treated as revoked. |
| `revoked_at` | Timestamp | optional | Set by `memory.grants.revoke`; terminal. |
| `revision` | int | — | |
| `extensions` | ExtensionMap | — | |

**Grant evaluation** (`MEM-R006`, `MEM-R017`). An op is admitted iff the authenticated principal holds at least one grant that is unexpired, unrevoked, matches the space, matches the subject (or is space-wide), includes the op's required capability (§7 tables), and covers the op's data access:

- Ops naming placement context IDs (capture placement, record write/read by placement, context fence): grant `context_ids` omitted, or covers every named context.
- Reads selecting shared visibility (`shared_only`, `include_shared: true`, by-id read of a shared record, profile reads, whole-subject exports): grant `context_ids` omitted, or `shared_access: true`.
- `subject_all` selection: grant `context_ids` MUST be omitted (unrestricted subject or space-wide grant). A context-limited grant never obtains `subject_all`, and never obtains shared access implicitly by naming no contexts on an op — the selection mode itself is checked against coverage.
- Reflections: the grant must cover the reflection's pinned selection and filter exactly as above (§7.25–§7.27).

**Non-amplification** (§7.9): a granter may only grant a subset of its own effective coverage — capabilities, subject scope, `context_ids`, `shared_access`, and `expires_at` (a granter whose grants expire cannot issue longer-lived or non-expiring grants).

### 6.6 `RetainedRecord`

Schema: [`retained-record.schema.json`](../../schemas/data/retained-record.schema.json). Generated: `RetainedRecord` (`LiveRecord` / `TombstonedRecord` variants; `Content`, `Provenance` local definitions).

| Field | Type | Null/Omission | Semantics |
|-------|------|---------------|-----------|
| `schema_version` | int | — | |
| `record_id` | string | — | Provider-assigned. |
| `space_id`, `subject_id` | string | — | |
| `kind` | string | — | Open vocabulary. Core (documented, not enforced): `event`, `fact`, `note`. Product-specific kinds are product policy, never protocol vocabulary. |
| `content` | object | — | `{ "text": string, "structured": OpaqueJson (optional) }`. **Untrusted** content (§10). |
| `scope` | object | — | **Write placement**: `{ "visibility": "shared" \| "context", "context_id": string }`. `context_id` is required iff `visibility = "context"`, and omitted when `visibility = "shared"`. Changed only by `memory.records.promote`. |
| `provenance` | object | — | `{ "source_context_ids": string[] (may be empty), "anchors": SourceAnchor[] (may be empty), "captured_by": principal_id, "captured_at": Timestamp }`. `captured_by` / `captured_at` are always provider-populated; `source_context_ids` / `anchors` are recorded client claims (§10). Provenance is independent of placement. Reuses the existing `SourceAnchor` wire object. |
| `import_ref` | object | optional | Present iff created by import activation: `{ "import_id": string, "source_record_id": string, "source_revision": int }`. Historical attribution; never CAS input. |
| `occurred_at` | Timestamp | optional | When the remembered event happened, when the source supplied it; independent of `created_at` / `updated_at` (record lifecycle). |
| `created_at`, `updated_at` | Timestamp | — | |
| `revision` | int | — | |
| `tombstone` | object | conditional | Present in the `TombstonedRecord` variant only: `{ "deleted_at": Timestamp, "deleted_by": principal_id, "reason": string (optional) }`. A tombstoned record has `content` removed (empty text, no `structured`) and is excluded from recall and default export (`MEM-R014`). |
| `extensions` | ExtensionMap | — | |

### 6.7 `MemoryCandidate`

Schema: [`memory-candidate.schema.json`](../../schemas/data/memory-candidate.schema.json). Generated: `MemoryCandidate` (`PendingCandidate` / `ConsumedCandidate` variants).

| Field | Type | Null/Omission | Semantics |
|-------|------|---------------|-----------|
| `schema_version` | int | — | |
| `candidate_id` | string | — | Provider-assigned. |
| `space_id`, `subject_id` | string | — | |
| `capture_scope` | object | — | **Immutable admitted placement** from `memory.capture` (§7.11): `{ "visibility": "shared" \| "context", "context_id": string (conditional as §6.6) }`. The authorization fence for review outcomes. |
| `proposed` | object | conditional | `{ "kind": string, "content": Content (as §6.6) }`. Required while `pending`; removed from the shell on consumption (§6.7 scrub). |
| `provenance` | object | — | Same shape as §6.6; claims recorded, `captured_by` provider-set. |
| `classification` | object | — | Advisory deterministic classification: `{ "disposition": "promote" \| "fragment" \| "drop", "rationale_codes": string[] (may be empty), "confidence": number 0..1 (optional) }`. Advisory only; the reviewer decision is authoritative. |
| `status` | string | — | `pending` \| `consumed`. |
| `consumed_by` | object | conditional | Present in the `ConsumedCandidate` variant: `{ "op": "memory.candidates.review" \| "memory.candidates.dismiss", "receipt_id": string, "outcome": "promoted" \| "fragmented" \| "dropped" \| "dismissed", "record_ids": string[] (may be empty) }`. |
| `content_digest` | string | conditional | 64-hex SHA-256 over the JCS-canonical `proposed.content`; required in the `ConsumedCandidate` variant, where content is scrubbed and the digest is retained for dedup/audit (`MEM-R025`). |
| `occurred_at` | Timestamp | optional | When the captured material happened; echoed from the capture request. |
| `created_at` | Timestamp | — | |
| `expires_at` | Timestamp | optional | Provider policy; expired pending candidates are excluded from lists (not consumed). |
| `revision` | int | — | |
| `extensions` | ExtensionMap | — | |

**Consumption scrub:** when a candidate is consumed, `proposed.content` is removed from the candidate shell (promoted/fragmented content now lives in the created records; dropped/dismissed content is discarded) and `content_digest` is set. The shell (IDs, `capture_scope`, classification, provenance, outcome) is retained for audit until `expires_at`.

### 6.8 `ProfileDocument`

Schema: [`profile-document.schema.json`](../../schemas/data/profile-document.schema.json). Generated: `ProfileDocument` (`Document` local definition).

| Field | Type | Null/Omission | Semantics |
|-------|------|---------------|-----------|
| `schema_version` | int | — | |
| `space_id`, `subject_id` | string | — | Unique per `(space, subject, profile_kind)`. |
| `profile_kind` | string | — | Open vocabulary of authored content kinds; core `personality` \| `preferences` (§4.2). |
| `document` | object | — | `{ "format": "markdown" \| "json", "body": string }`. **Trusted** author-maintained content (§10, §16). |
| `updated_by` | principal_id | — | Provider-populated from the authenticated context. |
| `created_at`, `updated_at` | Timestamp | — | |
| `revision` | int | — | |
| `extensions` | ExtensionMap | — | |

Substance — definition, use cases, three-state rule, lifecycle, and the soul landing point — is normative in §16.

### 6.9 `Reflection`

Schema: [`reflection.schema.json`](../../schemas/data/reflection.schema.json). Generated: `Reflection`.

| Field | Type | Null/Omission | Semantics |
|-------|------|---------------|-----------|
| `schema_version` | int | — | |
| `reflection_id` | string | — | Provider-assigned. |
| `space_id`, `subject_id` | string | — | |
| `selection` | ReadSelection | — | Pinned placement selection used for generation. |
| `filter` | ProvenanceFilter | optional | Pinned provenance filter (absent = none). |
| `basis` | object | — | `{ "source_set_digest": string (64 hex over the authorized source record ID+revision set), "record_count": int, "profile_revisions": {profile_kind: revision}, "state_digest": string (64 hex, §7.25) }`. Pins the exact inputs and coverage. |
| `body` | object | — | `{ "format": "markdown" \| "json", "body": string }`. Derived content; trusted only as to its pinned basis and generator. |
| `stale` | bool | — | Authoritative value is computed at read time (§7.26): true iff the current authorized basis differs from the pinned basis (`MEM-R013`). The stored flag is a write-time cache. |
| `generator` | object | — | `{ "engine_policy_version": string, "model": string (optional) }`. |
| `cost` | object | optional | `{ "units": number, "unit_kind": string }`. Content-free cost fact; survives failed generation without exposing input text (§14.3). |
| `created_at` | Timestamp | — | |
| `revision` | int | — | |
| `extensions` | ExtensionMap | — | |

**Privacy exclusion:** a reflection whose pinned basis references any tombstoned record is excluded from `memory.reflections.list` and from export — deleted content must not remain reachable through derived text (§11).

### 6.10 `MemoryReceipt`

Schema: [`memory-receipt.schema.json`](../../schemas/data/memory-receipt.schema.json). Generated: `MemoryReceipt` (`CommittedReceipt` / `PendingReceipt` / `FailedReceipt`, plus `MemoryErrorEnvelope`).

| Field | Type | Null/Omission | Semantics |
|-------|------|---------------|-----------|
| `schema_version` | int | — | |
| `receipt_id` | string | — | Provider-assigned; returned by every mutating op. |
| `op` | string | — | The op string that produced it. |
| `space_id` | string | — | |
| `requester_principal_id` | string | — | For receipt-read authorization (§7.28). |
| `idempotency_key` | string | optional | Echo when supplied (always, except `memory.subjects.ensure`). |
| `payload_digest` | string | — | `MEM-R008` binding. |
| `status` | string | — | `committed` \| `pending` \| `failed`. |
| `result` | OpaqueJson | conditional | **Content-free** op summary (`MEM-R022`): IDs, revisions, counts, outcome enums, digests, cost facts. Required in `CommittedReceipt`, absent otherwise. Never contains record/candidate/profile/reflection bodies. |
| `error` | ErrorEnvelope | conditional | Required in `FailedReceipt`; absent otherwise. Reuses the shared envelope shape with the memory subclass below. |
| `created_at` | Timestamp | — | |
| `completed_at` | Timestamp | conditional | Required when status ≠ `pending`. |
| `extensions` | ExtensionMap | — | |

**`MemoryErrorEnvelope`** extends the shared `error-envelope.schema.json` shape: `code` + `message` + `details` are required, and `details.retry_class` is required (`MEM-R016`, §14.2). All `memory.*-response` schemas reference this memory envelope on their failure branch.

Receipt bodies are retained for `receipt_retention_seconds`; afterwards a content-free dedup tombstone (namespace key, `payload_digest`, `receipt_id`, terminal status) is retained for `dedup_retention_seconds` (`MEM-R021`).

### 6.11 `MemoryExportBundle` (logical artifact)

Schema: [`memory-export-bundle.schema.json`](../../schemas/data/memory-export-bundle.schema.json). Generated: `MemoryExportBundle` (`Source`, `ReceiptSummary`, `GrantSummary` local definitions). The bundle is the logical whole reassembled from chunks (§6.13).

| Field | Type | Null/Omission | Semantics |
|-------|------|---------------|-----------|
| `schema_version` | int | — | |
| `bundle_format_version` | int ≥ 1 | — | Independent of `memory_protocol_version`. |
| `memory_protocol_version` | int | — | Protocol version of the exporting provider. |
| `export_id` | string | — | |
| `space_id` | string | — | Source space. |
| `created_at` | Timestamp | — | |
| `source` | object | — | `{ "authority_host_id": string, "authority_generation": int, "engine_policy_version": string }`. |
| `subjects` | Subject[] | — | May be empty only if the exported scope has no subjects. |
| `records` | RetainedRecord[] | — | Tombstoned records excluded unless `include_tombstoned: true` was requested. |
| `candidates` | MemoryCandidate[] | — | Shells (content-scrubbed, `content_digest` present) for consumed candidates; full pending candidates. Preserves consumed-operation history (`MEM-R025`). |
| `profiles` | ProfileDocument[] | — | May be empty. |
| `reflections` | Reflection[] | optional | Present iff `include_reflections: true`. Excludes tombstone-referencing reflections (§6.9). |
| `receipt_summaries` | object[] | — | Content-free dedup provenance: `{ "receipt_id", "op", "requester_principal_ref": string (opaque label), "idempotency_key" (optional), "payload_digest", "status", "created_at" }`. Enables reimport/replay duplicate suppression without transferring live receipt state. |
| `grants` | object[] | — | **Redacted** grant summaries `{ "grant_id", "subject_id" (optional), "principal_ref": string (opaque correlation label — never credentials or tokens), "capabilities", "context_ids" (optional), "shared_access" (optional), "revision": int }`. Sufficient for explicit administrator adoption via `adopt_grants`; never auto-activated (`MEM-R025`). |
| `digest` | string | — | SHA-256 hex over JCS of the bundle object with the `digest` field removed. |
| `extensions` | ExtensionMap | — | Required (durable-object convention). |

Secrets, credentials, and capability tokens MUST NOT appear anywhere in a bundle.

### 6.12 `ReadSelection`

Schema: [`read-selection.schema.json`](../../schemas/data/read-selection.schema.json). Generated: `ReadSelection` (one variant per mode).

| Field | Type | Null/Omission | Semantics |
|-------|------|---------------|-----------|
| `mode` | string | — | `subject_all` \| `shared_only` \| `selected_contexts`. |
| `context_ids` | string[] | conditional | Required, minItems 1 iff `mode = "selected_contexts"`; omitted otherwise. |
| `include_shared` | bool | conditional | Allowed only with `mode = "selected_contexts"`; omission there means `false`. Omitted for other modes. |

Semantics: `subject_all` = every record of the subject regardless of placement or provenance (requires an unrestricted grant, §6.5). `shared_only` = only `scope.visibility = "shared"` (requires shared access). `selected_contexts` = records whose placement `context_id` ∈ `context_ids`, plus shared records iff `include_shared = true` (requires coverage of every listed context; shared access when `include_shared`). Whole-subject management views use `subject_all`; ordinary execution views use `shared_only` or `selected_contexts`. No mode is expressed by null/empty-array overloading.

### 6.13 `ProvenanceFilter` and `MemoryExportChunk`

`ProvenanceFilter` ([`provenance-filter.schema.json`](../../schemas/data/provenance-filter.schema.json)): `{ "source_context_ids": string[] (minItems 1) }` — matches records/candidates having at least one provenance source context in the list. It is a **filter**, never a placement statement: a per-scope projection filters provenance while placement stays untouched; a binding is placement. Selection (§6.12) and filter combine by AND.

`MemoryExportChunk` ([`memory-export-chunk.schema.json`](../../schemas/data/memory-export-chunk.schema.json), generated `MemoryExportChunk` union `HeaderChunk` / `TailChunk`):

| Field | Type | Null/Omission | Semantics |
|-------|------|---------------|-----------|
| `export_id` | string | — | |
| `chunk_ordinal` | int ≥ 0 | — | Strict order. |
| `chunk_count` | int ≥ 1 | — | |
| `bundle_digest` | string | — | Digest of the logical bundle (§6.11) this chunk belongs to. |
| `chunk_digest` | string | — | SHA-256 over JCS of this chunk object minus `chunk_digest`. |
| `header` | object | conditional | Required in `HeaderChunk` (present iff `chunk_ordinal = 0`), absent in `TailChunk`: the scalar bundle fields `schema_version`, `bundle_format_version`, `memory_protocol_version`, `export_id`, `space_id`, `created_at`, `source`, `digest`, plus optional `extensions`. |
| `subjects` / `records` / `profiles` / `reflections` / `candidates` / `receipt_summaries` / `grants` | arrays | optional | Slices of the bundle arrays. Partitioning rule: arrays are concatenated in that fixed field order and split into consecutive element runs; one element is never split across chunks. |
| `bytes` | int | — | Serialized chunk size. ≤ `export_chunk_bytes`, except a single element larger than the limit occupies its own chunk (flagged by `bytes` exceeding the limit; never split, never dropped). |

**Header `extensions` (lossless chunking).** `header.extensions` is an optional `ExtensionMap` carried only on `chunk_ordinal = 0`. Reassembly MUST write it back to the reassembled `MemoryExportBundle.extensions` before verifying `bundle_digest`, because the bundle requires `extensions` while the header enumeration is the only carrier that can transport it. Omitting the field and carrying `{}` are both valid and produce the same reassembled bundle when the source bundle's extensions were empty.

Reassembly concatenates the slices in ordinal order and field order; verification recomputes `chunk_digest` per chunk and `bundle_digest` / `digest` over the reassembled bundle (`crates/spoke-memory/src/transfer.rs`: `partition_bundle`, `reassemble`, `verify_reassembly`).

## 7. Operations

The family defines 33 ops: 25 core (`spoke-memory`), 3 reflection (`spoke-memory-reflection`), and 5 transfer (`spoke-memory-transfer`). Each op has a committed request and response schema pair under `schemas/ops/`, named `memory-<segments>-{request,response}.schema.json` (dots in the op name become dashes).

| Op | Capability | Required grant | Schema stem | Mutates | `idempotency_key` |
|----|-----------|----------------|-------------|---------|-------------------|
| `memory.describe` | `spoke-memory` | none (authenticated principal) | `memory-describe` | no | — |
| `memory.spaces.list` | `spoke-memory` | any valid grant in the space, or provider-configured visibility | `memory-spaces-list` | no | — |
| `memory.spaces.freeze` | `spoke-memory` | `admin` | `memory-spaces-freeze` | yes | required |
| `memory.spaces.unfreeze` | `spoke-memory` | `admin` | `memory-spaces-unfreeze` | yes | required |
| `memory.subjects.ensure` | `spoke-memory` | `capture` (or `admin` with coverage) | `memory-subjects-ensure` | yes | exempt |
| `memory.subjects.get` | `spoke-memory` | any capability on the subject | `memory-subjects-get` | no | — |
| `memory.subjects.fence` | `spoke-memory` | `admin` | `memory-subjects-fence` | yes | required |
| `memory.grants.list` | `spoke-memory` | `admin` (full view) or self-view | `memory-grants-list` | no | — |
| `memory.grants.grant` | `spoke-memory` | `admin` + non-amplification | `memory-grants-grant` | yes | required |
| `memory.grants.revoke` | `spoke-memory` | `admin` | `memory-grants-revoke` | yes | required |
| `memory.capture` | `spoke-memory` | `capture` covering `capture_scope` | `memory-capture` | yes | required |
| `memory.candidates.list` | `spoke-memory` | `review` covering `capture_scope` | `memory-candidates-list` | no | — |
| `memory.candidates.get` | `spoke-memory` | `review` covering `capture_scope` | `memory-candidates-get` | no | — |
| `memory.candidates.dismiss` | `spoke-memory` | `review` covering `capture_scope` | `memory-candidates-dismiss` | yes | required |
| `memory.candidates.review` | `spoke-memory` | `review` + `records_write`, both covering `capture_scope` | `memory-candidates-review` | yes | required |
| `memory.records.create` | `spoke-memory` | `records_write` covering placement | `memory-records-create` | yes | required |
| `memory.records.get` | `spoke-memory` | `records_read` or `recall` covering placement; tombstoned reads need `admin` | `memory-records-get` | no | — |
| `memory.records.update` | `spoke-memory` | `records_write` covering current placement | `memory-records-update` | yes | required |
| `memory.records.delete` | `spoke-memory` | `records_write` covering placement | `memory-records-delete` | yes | required |
| `memory.records.list` | `spoke-memory` | `records_read` or `recall` covering selection/filter; `include_tombstoned` needs `admin` | `memory-records-list` | no | — |
| `memory.records.promote` | `spoke-memory` | `promote` + `records_write` covering source and target placements | `memory-records-promote` | yes | required |
| `memory.recall` | `spoke-memory` | `recall` covering selection/filter | `memory-recall` | no | — |
| `memory.profile.get` | `spoke-memory` | any capability on the subject with shared access or unrestricted coverage | `memory-profile-get` | no | — |
| `memory.profile.update` | `spoke-memory` | `profile_write` | `memory-profile-update` | yes | required |
| `memory.reflections.state` | `spoke-memory-reflection` | `reflect` or `admin` covering selection/filter | `memory-reflections-state` | no | — |
| `memory.reflections.list` | `spoke-memory-reflection` | `reflect` or `admin` covering selection/filter | `memory-reflections-list` | no | — |
| `memory.reflections.generate` | `spoke-memory-reflection` | `reflect` covering selection/filter | `memory-reflections-generate` | yes (async possible) | required |
| `memory.receipts.get` | `spoke-memory` | receipt's `requester_principal_id`, or `admin` | `memory-receipts-get` | no | — |
| `memory.exports.create` | `spoke-memory-transfer` | `transfer` covering export scope | `memory-exports-create` | yes (async possible) | required |
| `memory.exports.get` | `spoke-memory-transfer` | `transfer` covering export scope | `memory-exports-get` | no | — |
| `memory.imports.stage` | `spoke-memory-transfer` | `transfer` + `admin` on target space | `memory-imports-stage` | yes (async possible) | required |
| `memory.imports.activate` | `spoke-memory-transfer` | `transfer` + `admin` | `memory-imports-activate` | yes (async possible) | required |
| `memory.imports.discard` | `spoke-memory-transfer` | `transfer` + `admin` | `memory-imports-discard` | yes | required |

### 7.0 Common operation contract

**7.0.1 Requests.** Requests are closed JSON objects (`additionalProperties: false`); each carries optional top-level `extensions` for transport metadata. The requester's principal never appears in the payload (§6.4). Two op classes exist:

- **Space-scoped ops** name `space_id`; subject-scoped ops also name `subject_id`.
- **Unscoped ops** carry no space or subject selector: `memory.describe` sends `{}` plus optional `extensions` (it describes the provider itself, §7.1), and `memory.spaces.list` sends only `cursor` / `limit` / `extensions` (it discovers spaces, §7.2). A client MUST NOT add `space_id` to either request — both schemas are closed and would reject it.

**7.0.2 Responses.** Read ops return their payload directly. Mutating ops return a `MemoryReceipt` (§6.10) — `CommittedReceipt` or `PendingReceipt` — whose `result` is a content-free summary (`MEM-R022`); callers obtain live objects through read ops, which always re-check current authorization (`MEM-R026`). Read responses are snapshots: they are not CAS tokens and carry no freshness guarantee beyond their creation instant. Every response schema also admits the failure branch `{ "error": MemoryErrorEnvelope, "extensions"?: ExtensionMap }`.

**7.0.3 Async model (single).** All mutating ops either commit synchronously (`status: "committed"`) or — only `memory.reflections.generate`, `memory.exports.create`, the final chunk of `memory.imports.stage`, and `memory.imports.activate` — return `status: "pending"` and complete asynchronously. Completion is observed exclusively through `memory.receipts.get`. No webhooks, streams, or callbacks exist.

**7.0.4 Revisions and no-ops.** A mutating op that changes observable state increments the target object's `revision`. A genuine no-op (e.g. freezing an already-frozen space with matching `expected_revision`, unfreezing an active space) succeeds without incrementing. Content updates are never no-ops.

**7.0.5 Idempotency and replay (`MEM-R008` / `MEM-R021`).** Every mutating op except `memory.subjects.ensure` REQUIRES `idempotency_key` (absent → `INVALID_INPUT`). `memory.subjects.ensure` is the sole exception: get-or-create is naturally idempotent and its outcome is recoverable through `memory.subjects.get`. The dedup namespace is `(space_id, requester_principal_id, op, idempotency_key)`; reconnects and retries share the authenticated principal, so logical identity is stable across connections. Replay (same namespace + same `payload_digest`) returns the stored receipt without re-executing and without re-evaluating the original CAS/fence preconditions. Replay with the same key and a different digest fails `IDEMPOTENCY_CONFLICT`. Replay against an evicted receipt fails `RECEIPT_EXPIRED`, never a silent re-execution.

**7.0.6 Bootstrap.** Spaces and the initial administrator grants are created by provider-operator provisioning (host-local configuration), not by any protocol op. There is no uncallable grant cycle: the operator assigns the first `admin` grant directly in provider storage; all further grants flow through `memory.grants.grant` under the §6.5 non-amplification rule. Clients discover provisioned spaces through `memory.describe` / `memory.spaces.list`.

**7.0.7 Pagination.** List and recall ops accept `cursor` + `limit` (≤ `max_page_size`) and return `next_cursor` (absent = end). Cursors bind principal + space + subject + selection/filter + snapshot marker + `engine_policy_version`; any state or grant change invalidates them with `CURSOR_INVALID`, so pages never silently re-filter mid-pagination (`MEM-R026`).

Limits (`memory.describe.limits`) apply to every op; exceeding a limit fails `INVALID_INPUT` with `details.limit` naming the exceeded limit. Each op below states: purpose — request fields — grant — result — errors.

### 7.1 `memory.describe`

Provider self-description. **Grant:** none (authenticated principal only). Read-only (`MEM-R011`).

- Request: `{}` plus optional `extensions`.
- Result: `{ "memory_protocol_version": int, "capabilities": string[] (subset of §4.1 actually served), "limits": Limits, "required_profile_kinds": string[] (authored content kinds, §4.2), "engine_policy_version": string }`.
- `Limits` (all integers > 0): `max_content_bytes` (request field and stored content), `max_candidates_per_capture` (request), `max_page_size` (response page), `export_chunk_bytes` (response chunk, §6.13), `idempotency_retention_seconds` (stored receipt bodies), `receipt_retention_seconds` (stored receipts), `dedup_retention_seconds` (stored dedup tombstones; ≥ `idempotency_retention_seconds`), `staged_import_ttl_seconds` (stored staged imports), `export_retention_seconds` (stored export chunks).
- Space discovery is exclusively `memory.spaces.list` (paginated); `describe` carries no space catalogue.
- Errors: `INTERNAL_ERROR`.

### 7.2 `memory.spaces.list`

List spaces visible to the principal; also the drain-barrier read (`MEM-R023`). **Grant:** any valid grant in the space, or provider-configured visibility. Read-only.

- Request: `{ "cursor"?: string, "limit"?: int }`.
- Result: `{ "spaces": MemorySpace[], "next_cursor"?: string }`. Each entry includes the computed `pending_receipts`.
- Pagination invariant: no page may contain a space the principal cannot see.

### 7.3 `memory.spaces.freeze`

Freeze a space for maintenance or migration. **Grant:** `admin`.

- Request: `{ "space_id", "expected_revision": int, "idempotency_key": string }`.
- Result: `{ "status": "frozen", "pending_receipts": int, "revision": int }`.
- Admission effect (one transaction with the status flip, `MEM-R017`): subsequent non-allowlist ops fail `SPACE_FROZEN`; already-admitted ops complete (`MEM-R018`). Freezing an already-frozen space with matching revision is a no-op success (§7.0.4).
- Errors: `REVISION_CONFLICT`, `NOT_AUTHORIZED`, `NOT_FOUND`.

### 7.4 `memory.spaces.unfreeze`

Reverse of §7.3; always allowed on a frozen space (maintenance allowlist, §8.2). Re-opening a decommissioned source after cutover is an **operator** action that also bumps `authority_generation` (§12.5) — protocol ops never auto-unfreeze a cutover source (`MEM-R027`). Request and result mirror §7.3.

### 7.5 `memory.subjects.ensure`

Idempotent get-or-create of a subject. **Grant:** `capture` or `admin`. Sole `idempotency_key` exemption (§7.0.5).

- Request: `{ "space_id", "subject_id", "kind"?: Kind }`. `kind` is set only on creation; on an existing subject the op returns it unchanged with `created: false` (a kind mismatch is not an error — kind is metadata, not authority).
- Result: `{ "subject_id", "created": bool, "status": string, "revision": int }`.
- Atomic: single conditional insert.
- Errors: `SPACE_FROZEN`, `SUBJECT_FENCED`, `NOT_AUTHORIZED`, `INVALID_INPUT` (bad kind shape).

### 7.6 `memory.subjects.get`

**Grant:** any capability on the subject. Read-only; rechecks current authorization.

- Request: `{ "space_id", "subject_id" }`. Result: `{ "subject": Subject }`.
- Errors: `NOT_FOUND`, `NOT_AUTHORIZED`.

### 7.7 `memory.subjects.fence`

Provider-enforced subject- or context-scoped lifecycle fencing across all clients (`MEM-R017`, `MEM-R024`). **Grant:** `admin`.

- Request: `{ "space_id", "subject_id", "target": Target, "action": "fence" | "unfence" | "archive", "expected_revision": int, "idempotency_key": string }`, where `Target` = `{ "type": "subject" }` or `{ "type": "context", "context_id": string }`.
- Subject target: `active → fenced` (fence), `fenced → active` (unfence), `active|fenced → archived` (archive; **terminal** decommission). Products with reversible archive semantics map reversible archive/restore to fence/unfence; protocol `archive` is never the mapping target of a reversible product flow. Fence state is durable across provider restarts.
- Context target (`fence` / `unfence` only; `archive` → `INVALID_INPUT`): adds or removes `context_id` in `fenced_contexts`. Writes placing into a fenced context fail `SUBJECT_FENCED` with `details.scope: "context"`; sibling contexts remain fully usable (`MEM-R024`).
- Effects: a `fenced` subject rejects all writes for the subject (reads allowed). `archived` additionally excludes the subject's records from recall and from default candidate listing; records and receipts are retained and export stays allowed.
- Result: `{ "subject_id", "status": string, "fenced_contexts": string[], "revision": int }`.
- Stale in-flight writes: writes admitted before the fence commit complete; writes admitted after fail — never half-applied (§11).
- Errors: `INVALID_STATUS_TRANSITION` (archive→anything, unfence of an active subject, unfence of a non-fenced context), `INVALID_INPUT` (archive on a context target), `REVISION_CONFLICT`, `NOT_FOUND`, `NOT_AUTHORIZED`, `SPACE_FROZEN`.

### 7.8 `memory.grants.list`

**Grant:** `admin` (full view; the optional `principal_id` filter is admin-only) or self-view (a principal lists exactly its own grants). Read-only.

- Request: `{ "space_id", "subject_id"?, "principal_id"? (admin-only target selector, §6.4), "cursor"?, "limit"? }`.
- Result: `{ "grants": MemoryGrant[], "next_cursor"?: string }`.

### 7.9 `memory.grants.grant`

**Grant:** `admin` whose coverage subsumes the grant issued (non-amplification, §6.5).

- Request: `{ "space_id", "subject_id"?, "principal_id": string (target grantee — a selector, never requester identity), "capabilities": string[] (minItems 1), "context_ids"?: string[] (minItems 1), "shared_access"?: bool (forbidden without `context_ids`), "expires_at"?: Timestamp, "idempotency_key": string }`. The `context_ids` presenter and the unrestricted presenter are separate closed variants, mirroring §6.5.
- Non-amplification checks (all fail `NOT_AUTHORIZED`): granted capabilities ⊆ granter's effective capabilities; granted subject scope ⊆ granter's (a space-wide granter may issue subject-scoped grants; a subject-scoped granter only the same subject); granted `context_ids` ⊆ granter's (or granter unrestricted); `shared_access` only if the granter has it; `expires_at` not later than the granter's own earliest expiring applicable grant.
- Result: `{ "grant_id", "revision": int }`.
- Errors: `NOT_AUTHORIZED` (amplification), `INVALID_INPUT`, `SPACE_FROZEN`, `SUBJECT_FENCED`.

### 7.10 `memory.grants.revoke`

**Grant:** `admin`. Allowed during freeze (incident response is on the §8.2 allowlist).

- Request: `{ "space_id", "grant_id", "expected_revision", "idempotency_key" }`. Result: `{ "grant_id", "revoked": true, "revision": int }`.
- Terminal; applies to subsequent admissions (`MEM-R018`). Errors: `REVISION_CONFLICT`, `NOT_FOUND`, `NOT_AUTHORIZED`.

### 7.11 `memory.capture`

Capture raw material for one subject; produces zero or more pending candidates. **Grant:** `capture` covering the declared `capture_scope` (§6.5: context coverage for `context` placement; shared access for `shared` placement).

- Request: `{ "space_id", "subject_id", "content": Content, "capture_scope": Scope (required, immutable placement), "source_context_ids"?: string[] (provenance claims), "anchors"?: SourceAnchor[] (provenance claims), "kind_hint"?: string (advisory), "occurred_at"?: Timestamp, "idempotency_key": string }`.
- `capture_scope` (placement, admission-checked, immutable on the candidate) and `source_context_ids` / `anchors` (recorded provenance claims) are distinct and independent.
- Result: `{ "candidate_ids": string[], "classified": int }`. Zero candidates is a valid committed outcome (provider policy, e.g. dedup); the receipt records the count — never an empty success without a receipt.
- Atomic: admission check + candidate inserts + receipt commit in one transaction. The deterministic classifier runs inside the transaction boundary or its output is committed with the candidates; classification is advisory (§6.7).
- Capture NEVER writes retained records, profiles, or reflections, and never triggers generation (`MEM-R012`).
- Errors: `NOT_AUTHORIZED`, `SPACE_FROZEN`, `SUBJECT_FENCED`, `INVALID_INPUT` (oversize content, missing key), `IDEMPOTENCY_CONFLICT`, `OUTCOME_UNCERTAIN` (§14.3).

### 7.12 `memory.candidates.list`

**Grant:** `review`. Read-only. Coverage: a context-restricted review grant sees only candidates whose `capture_scope` it covers (plus shared-placement candidates iff `shared_access`).

- Request: `{ "space_id", "subject_id", "status"?: "pending" | "consumed" (omitted = both), "cursor"?, "limit"? }`.
- Result: `{ "candidates": MemoryCandidate[], "next_cursor"?: string }`. Expired pending candidates are excluded.

### 7.13 `memory.candidates.get`

**Grant:** `review` covering the candidate's `capture_scope`. Read-only; rechecks current authorization.

- Request: `{ "space_id", "candidate_id" }`. Result: `{ "candidate": MemoryCandidate }`. Errors: `NOT_FOUND`, `NOT_AUTHORIZED`.

### 7.14 `memory.candidates.dismiss`

Lightweight consumption without a review decision (queue removal). **Grant:** `review` covering the candidate's `capture_scope`.

- Request: `{ "space_id", "candidate_id", "expected_revision", "reason"?, "idempotency_key" }`.
- Result: `{ "candidate_id", "outcome": "dismissed" }`.
- Atomic per `MEM-R009`, including the §6.7 consumption scrub. Dismissal is replayable per §7.0.5.
- Errors: `CANDIDATE_CONSUMED`, `REVISION_CONFLICT`, `NOT_FOUND`, `NOT_AUTHORIZED`.

### 7.15 `memory.candidates.review`

Formal review decision; the only path from candidate to retained records. **Grant:** `review` plus `records_write`, both covering the candidate's `capture_scope`.

- Request: `{ "space_id", "candidate_id", "expected_revision", "decision": Decision, "idempotency_key" }`, where `Decision` is exactly one of:
  - `{ "action": "promote", "kind"?: string (default from candidate), "content_override"?: Content }`
  - `{ "action": "fragment", "fragments": [ { "kind": string, "content": Content } ] (minItems 1) }`
  - `{ "action": "drop", "rationale"?: string }`
- **Placement rule:** created records inherit the candidate's `capture_scope` verbatim. A review decision carries no placement fields and cannot cross the admitted capture scope. Any later visibility change is a separate authorized `memory.records.promote` (§7.21).
- Result: `{ "outcome": "promoted" | "fragmented" | "dropped", "record_ids": string[] (empty for drop) }`.
- Atomic (`MEM-R009`): candidate consumption + scrub, all record inserts, and the receipt commit in one transaction. Partial success is impossible by construction.
- No merge-into-existing action exists in version 1; products compose `records.update` after promote if needed.
- Errors: `CANDIDATE_CONSUMED`, `REVISION_CONFLICT`, `NOT_FOUND`, `NOT_AUTHORIZED`, `INVALID_INPUT`, `SPACE_FROZEN`, `SUBJECT_FENCED`.

### 7.16 `memory.records.create`

Direct authored write, bypassing capture/review (management surfaces). **Grant:** `records_write` covering the declared placement.

- Request: `{ "space_id", "subject_id", "kind", "content": Content, "scope": Scope, "source_context_ids"?: string[], "anchors"?: SourceAnchor[], "idempotency_key" }`. `captured_by` / `captured_at` are always provider-populated from the authenticated context — never client-supplied.
- Result: `{ "record_id", "revision": int }`. Atomic single insert. The live record is read through `memory.records.get`.
- Errors: `NOT_AUTHORIZED`, `INVALID_INPUT`, `SPACE_FROZEN`, `SUBJECT_FENCED`, `IDEMPOTENCY_CONFLICT`.

### 7.17 `memory.records.get`

**Grant:** `records_read` or `recall` covering the record's placement (shared records require shared access). Read-only; rechecks current authorization (`MEM-R026`).

- Request: `{ "space_id", "record_id" }`. Result: `{ "record": RetainedRecord }`. A tombstoned record is returned with its tombstone (audit visibility) only to `admin`; others receive `NOT_FOUND`.
- Errors: `NOT_FOUND`, `NOT_AUTHORIZED`.

### 7.18 `memory.records.update`

**Grant:** `records_write` covering the current placement.

- Request: `{ "space_id", "record_id", "expected_revision", "content"?: Content, "kind"?: string, "idempotency_key" }`. At least one of `content` / `kind` is required; the two presenters are separate closed variants. Placement changes are NOT an update — use `memory.records.promote`.
- Result: `{ "record_id", "revision": int }`.
- Same-transaction reflection invalidation per `MEM-R013`.
- Errors: `REVISION_CONFLICT`, `NOT_FOUND`, `NOT_AUTHORIZED`, `INVALID_INPUT`, `SUBJECT_FENCED`, `SPACE_FROZEN`.

### 7.19 `memory.records.delete`

**Grant:** `records_write` covering the placement.

- Request: `{ "space_id", "record_id", "expected_revision", "reason"?, "idempotency_key" }`.
- Result: `{ "record_id", "tombstoned": true }`.
- Semantics (`MEM-R014`, `MEM-R022`): content removed, tombstone retained, receipts unaffected (they are content-free), reflections referencing the record excluded from list/export (§6.9), remaining reflections invalidated per `MEM-R013`. Privacy-erasure deployments MAY additionally scrub tombstone provenance claims by product policy; the receipt trail itself is never rewritten.
- Errors: `REVISION_CONFLICT`, `NOT_FOUND`, `NOT_AUTHORIZED`, `SPACE_FROZEN`.

### 7.20 `memory.records.list`

**Grant:** `records_read` or `recall` with coverage of the selection (and filter coverage per §6.5). Read-only.

- Request: `{ "space_id", "subject_id", "selection": ReadSelection, "filter"?: ProvenanceFilter, "kinds"?: string[], "include_tombstoned"?: bool (default false, admin only), "cursor"?, "limit"? }`.
- Result: `{ "records": RetainedRecord[], "total_authorized": int, "next_cursor"?: string }`. Authorization filters before pagination; `total_authorized` counts only records the principal may see (`MEM-R006`).

### 7.21 `memory.records.promote`

Explicit placement change of a retained record (context ↔ shared, or context → other context). **Grant:** `promote` plus `records_write` covering **both** source and target placements (target `shared` requires shared access).

- Request: `{ "space_id", "record_id", "expected_revision", "target_scope": Scope, "idempotency_key" }`.
- Result: `{ "record_id", "revision": int, "scope": Scope }`.
- Promotion is always explicit; no capture, review, or recall path ever changes placement implicitly. Provenance is unchanged. Promotion moves the record into or out of reflection scopes and therefore invalidates per `MEM-R013`.
- Errors: `REVISION_CONFLICT`, `NOT_FOUND`, `NOT_AUTHORIZED`, `INVALID_INPUT`, `SPACE_FROZEN`, `SUBJECT_FENCED`.

### 7.22 `memory.recall`

Authorized, ranked, budgeted read for execution-time context. **Grant:** `recall` covering the selection (§6.5). Read-only (`MEM-R011`).

- Request: `{ "space_id", "subject_id", "selection": ReadSelection, "filter"?: ProvenanceFilter, "query"?: string (ranking hint), "budget": { "max_items": int (required, ≥1, ≤ max_page_size), "max_bytes"?: int (hard UTF-8 budget, §9) }, "cursor"?: string }`.
- Result: `{ "records": RetainedRecord[], "total_authorized": int, "truncated": bool, "overflow_record_id"?: string (§9), "next_cursor"?: string, "engine_policy_version": string }`.
- Semantics: authorization filter first (`MEM-R006`); then provider ranking (algorithm-variable, §9 item 2); then budget. `truncated: true` whenever the budget cut the authorized set. Candidates never appear (`MEM-R004`); tombstoned records never appear. Ordering MUST be stable for identical provider state, request, and `engine_policy_version`; it MAY differ across providers or policy versions.
- Errors: `NOT_AUTHORIZED`, `INVALID_INPUT`, `NOT_FOUND`, `CURSOR_INVALID`.

### 7.23 `memory.profile.get`

**Grant:** any capability on the subject with shared access or unrestricted coverage (profiles are subject-level trusted documents, §6.5). Read-only.

- Request: `{ "space_id", "subject_id", "profile_kinds"?: string[] (omitted = all present kinds) }`.
- Result: `{ "profiles": ProfileDocument[] }`. Absent kinds are honest-empty, never an error (§4.2).

### 7.24 `memory.profile.update`

**Grant:** `profile_write`.

- Request: `{ "space_id", "subject_id", "profile_kind", "expected_revision": int | null (null = create-only), "document": Document, "idempotency_key" }`.
- Result: `{ "profile_kind", "revision": int }`.
- Same-transaction reflection invalidation per `MEM-R013`.
- Errors: `REVISION_CONFLICT`, `NOT_AUTHORIZED`, `INVALID_INPUT`, `SPACE_FROZEN`, `SUBJECT_FENCED`.

### 7.25 `memory.reflections.state`

Capability `spoke-memory-reflection`. **Grant:** `reflect` or `admin` covering the selection/filter. Read-only.

- Request: `{ "space_id", "subject_id", "selection": ReadSelection, "filter"?: ProvenanceFilter }`.
- Result: `{ "state_digest": string (64 hex), "basis": { "record_count": int, "profile_kinds": string[] }, "stale": bool, "last_reflection_id"?: string, "missing_required_profile_kinds": string[], "data_sufficiency": "sufficient" | "insufficient" }`.
- `state_digest` commits to: selection, filter, the authorized source record ID+revision set, profile kinds and revisions, and `engine_policy_version`. It is the CAS token for generation. `data_sufficiency` surfaces engine/provider policy (minimum-material heuristics); the protocol defines no numeric thresholds.

### 7.26 `memory.reflections.list`

Capability `spoke-memory-reflection`. **Grant:** `reflect` or `admin` covering each returned reflection's pinned selection/filter (context-restricted grants never read all-subject derived text). Read-only.

- Request: `{ "space_id", "subject_id", "selection"?: ReadSelection (restricts listing), "filter"?: ProvenanceFilter, "cursor"?, "limit"? }`.
- Result: `{ "reflections": Reflection[], "next_cursor"?: string }`, newest-first. `stale` values are computed at read time (§6.9). Reflections referencing tombstoned records are excluded (§6.9).

### 7.27 `memory.reflections.generate`

Capability `spoke-memory-reflection`. **Grant:** `reflect` covering the selection/filter. Explicit, cost-bearing generation (`MEM-R012`).

- Request: `{ "space_id", "subject_id", "selection": ReadSelection, "filter"?: ProvenanceFilter, "expected_state_digest": string, "idempotency_key": string }`.
- Admission failures (fail-closed): `missing_required_profile_kinds` non-empty → `PROFILE_INCOMPLETE` with `details.missing` (`MEM-R005`); `data_sufficiency: "insufficient"` → `INSUFFICIENT_DATA` (provider/engine policy gate, §7.25); `expected_state_digest` ≠ current digest → `REVISION_CONFLICT`.
- Result: pending `MemoryReceipt`; on completion `result = { "reflection_id": string, "cost"?: Cost }` (content-free).
- **Concurrency rule (publish-pinned):** generation completes against the pinned basis from admission. If the basis moved meanwhile, the reflection is still published with its pinned basis and immediately reads `stale: true` (`MEM-R013`) — never a false-current result, never a silently dropped paid generation.
- Errors: `PROFILE_INCOMPLETE`, `INSUFFICIENT_DATA`, `REVISION_CONFLICT`, `NOT_AUTHORIZED`, `SPACE_FROZEN`, `SUBJECT_FENCED`, `OUTCOME_UNCERTAIN`.

### 7.28 `memory.receipts.get`

Read-only (`MEM-R011`). **Grant:** current authorization as the receipt's `requester_principal_id`, or `admin` in the space (`MEM-R018`, `MEM-R021`).

- Request: `{ "space_id", "receipt_id" }`. Result: `{ "receipt": MemoryReceipt }`.
- Sole uncertainty-resolution channel (`MEM-R010`, §14.3) and the polling mechanism for pending receipts.
- Errors: `NOT_FOUND` (unknown), `RECEIPT_EXPIRED` (evicted body; `details.last_known_status` from the dedup tombstone), `NOT_AUTHORIZED`.

### 7.29 `memory.exports.create`

Capability `spoke-memory-transfer`. **Grant:** `transfer` with coverage of the requested export scope: a whole-space export requires space-wide coverage; a subject export requires coverage of the optional `selection` (shared access where applicable).

- Request: `{ "space_id", "subject_id"?: string (omitted = whole space), "selection"?: ReadSelection (subject exports only), "include_reflections"?: bool (default false), "include_tombstoned"?: bool (default false, admin only), "require_quiescent"?: bool (default false), "idempotency_key": string }`. The whole-space and subject presenters are separate closed variants, so a whole-space request cannot carry a subject selection.
- Quiescence: with `require_quiescent: true`, the op fails `INVALID_INPUT` (`details.reason: "space_not_drained"`) unless the space is frozen AND `pending_receipts == 0` (drain barrier, `MEM-R023`) — the provider's snapshot-consistency guarantee for migration-grade exports. With `false`, the snapshot is provider-consistent under the single-authority property but may race ongoing writes; for backups only.
- Result: pending receipt; completion `result = { "export_id": string, "chunk_count": int, "total_bytes": int, "bundle_digest": string, "counts": { "subjects": int, "records": int, "candidates": int, "profiles": int, "reflections": int, "receipt_summaries": int } }` (all content-free).
- Errors: `NOT_AUTHORIZED`, `NOT_FOUND`, `INVALID_INPUT`, `IDEMPOTENCY_CONFLICT`. Export is on the freeze allowlist — `SPACE_FROZEN` never applies to it.

### 7.30 `memory.exports.get`

Capability `spoke-memory-transfer`. **Grant:** `transfer` (same coverage as creation). Read-only.

- Request: `{ "space_id", "export_id", "chunk_ordinal": int ≥ 0 }`. Result: `{ "chunk": MemoryExportChunk }` (§6.13).
- Callers fetch chunks `0..chunk_count-1` and verify `chunk_digest` per chunk and `bundle_digest` on reassembly.
- Errors: `NOT_FOUND`, `EXPORT_EXPIRED` (past `export_retention_seconds`), `INVALID_INPUT` (ordinal out of range), `NOT_AUTHORIZED`.

### 7.31 `memory.imports.stage`

Capability `spoke-memory-transfer`. **Grant:** `transfer` + `admin` on the target space.

- Request (per chunk, strict ordinal order): `{ "space_id", "chunk": MemoryExportChunk, "conflict_policy": "fail" | "rename_subject" (required iff chunk_ordinal = 0, forbidden otherwise), "adopt_grants"?: [ { "grant_id": string, "principal_id": string (target-space principal) } ] (chunk_ordinal = 0 only), "idempotency_key": string }`. The idempotency namespace makes per-chunk replay safe (§7.0.5).
- Chunk accumulation is durable (survives provider crash). Out-of-order chunks fail `INVALID_INPUT`. Verification runs when the final chunk arrives (all failures reject `IMPORT_CONFLICT` with `details.errors[]`, nothing staged): per-chunk `chunk_digest`, reassembled `bundle_digest` / `digest`, `bundle_format_version` support, `memory_protocol_version` compatibility, schema validation of every element, and a secret-shape scan of `grants` summaries.
- Subject collision: the same `subject_id` already exists in the target → `fail` rejects; `rename_subject` assigns a fresh `subject_id` and records the mapping (`MEM-R002`: no auto-link).
- Grant adoption: only grants listed in `adopt_grants` are created on the target, bound to the named target principal, and only if the staging principal's own coverage subsumes them (non-amplification, §6.5). Unlisted grants are dropped. No grant is ever auto-activated (`MEM-R025`).
- Reference translation (computed at staging, applied at activation): `subject_id_map`, `record_id_map`, `candidate_id_map`; candidate `consumed_by.record_ids`, reflection basis record IDs, and receipt-summary references are remapped through them; imported reflections are marked `stale: true` unconditionally (`MEM-R025`); provenance and `import_ref` are adopted verbatim as historical attribution (§6.6); target revisions are fresh (source revisions preserved only in the maps).
- Result: pending receipt per chunk (`result = { "import_id": string, "chunks_received": int }`); final chunk completion adds `"staged_report": { "counts": Counts (incl. candidates, receipt_summaries, reflections), "subject_id_map": {source: target}, "record_id_map": {source: target}, "candidate_id_map": {source: target}, "grants_adopted": int, "grants_dropped": int, "verification": { "bundle_digest_ok": bool, "errors": string[] } }`.
- Staged data is invisible to all non-transfer ops. Staged imports expire after `staged_import_ttl_seconds`.

### 7.32 `memory.imports.activate`

Capability `spoke-memory-transfer`. **Grant:** `transfer` + `admin`.

- Request: `{ "space_id", "import_id", "idempotency_key" }`.
- Result: pending receipt; completion `result = { "import_id", "activated": Counts, "authority_generation": int }`.
- Atomic per space: all staged subjects, records, candidates, profiles, reflections, and receipt summaries become visible (or recorded) with freshly assigned target revisions in one transaction, or none do. Activation increments the space `authority_generation` (§12.5). Replay per §7.0.5. Expired or discarded imports fail `IMPORT_CONFLICT`.
- Recommended practice (non-normative): activate into a frozen, drained target space, verify, then unfreeze — the sequence of §12.

### 7.33 `memory.imports.discard`

Capability `spoke-memory-transfer`. **Grant:** `transfer` + `admin`.

- Request: `{ "space_id", "import_id", "idempotency_key" }`. Result: `{ "import_id", "discarded": true }`.
- Explicit pre-activation rollback: removes all staged data. Valid only before activation; after activation it fails `IMPORT_CONFLICT` — post-activation rollback is §12.4, never compensating-only deletion.
- Errors: `IMPORT_CONFLICT`, `NOT_FOUND`, `NOT_AUTHORIZED`.

## 8. Lifecycle and state transitions

### 8.1 Candidate FSM

`pending` → `consumed` through exactly one of `memory.candidates.review` (outcomes `promoted` / `fragmented` / `dropped`) or `memory.candidates.dismiss` (outcome `dismissed`). `consumed` is terminal; further mutation fails `CANDIDATE_CONSUMED`. Consumption scrubs content and sets `content_digest` (§6.7). Expiry removes a `pending` candidate from lists without consuming it.

### 8.2 Space FSM and freeze maintenance allowlist

`active ⇄ frozen` through `memory.spaces.freeze` / `memory.spaces.unfreeze`. While `frozen`, the space admits **exactly**: all read-only ops (`MEM-R011` list), `memory.spaces.unfreeze`, `memory.grants.revoke`, and the transfer ops `memory.exports.create`, `memory.exports.get`, `memory.imports.stage`, `memory.imports.activate`, `memory.imports.discard`. All other mutations fail `SPACE_FROZEN`. Already-admitted ops complete (`MEM-R018`); the drain barrier is `pending_receipts == 0`, observable through `memory.spaces.list` (`MEM-R023`). Freeze is durable across provider restart.

### 8.3 Subject and context fence FSM

Subject target: `active ⇄ fenced`, `{active, fenced} → archived` (terminal decommission; a reversible product archive maps to `fence`, §7.7). Context target: membership in `fenced_contexts` toggles per context (`MEM-R024`). All transitions bump the subject `revision` and are provider-enforced at admission across all clients.

### 8.4 Import FSM

`staging (chunks 0..n)` → `staged` → `activated` (terminal, visible) or `staged` / `staging` → `discarded` (terminal, removed) or → `expired` (TTL, removed). All transitions are receipt-bound.

### 8.5 Receipt FSM

`pending → committed | failed`. Ops that complete synchronously commit directly as `committed`. Only `memory.reflections.generate`, `memory.exports.create`, the final chunk of `memory.imports.stage`, and `memory.imports.activate` may return `pending`. Bodies retire to dedup tombstones per §6.10.

## 9. Recall, ranking, truncation, budget

1. **Filter (normative):** authorization + selection + provenance filter + tombstone/candidate exclusion (`MEM-R004`, `MEM-R006`).
2. **Rank (non-normative):** provider algorithm; `query` is a hint, never a filter. Identical-algorithm behavior across providers is explicitly not mandated.
3. **Budget (normative):** `max_items` is a hard item cap. `max_bytes` is a **hard** budget measured as the UTF-8 byte length of the serialized `content` fields of returned records. Progress rule: if the next ranked item would exceed `max_bytes` and the page is empty, the provider returns that single item whole and sets `overflow_record_id` to its ID — an oversized first item is surfaced, never silently dropped or silently skipped; if the page is non-empty, the item is deferred and `truncated: true`. `truncated` MUST be true iff the authorized filtered set exceeds the returned page under the applied budget.
4. **Pagination:** `next_cursor` per §7.0.7 (`MEM-R026`): bound, snapshot-consistent, invalidated by state/grant change with `CURSOR_INVALID`; no hidden-row leaks.

The reference engine implements this policy set in `crates/spoke-memory/src/recall.rs` (`recall`), which receives already-authorized records.

## 10. Trusted profiles vs untrusted captured text

- `ProfileDocument.document.body` is trusted author-maintained content; it may drive execution conditioning per product policy.
- `RetainedRecord.content` and `MemoryCandidate.proposed.content` are untrusted data: conforming providers and consumers MUST treat them as data, never as instructions, and MUST NOT let record content alter grant, scope, or principal interpretation (`MEM-R003`).
- Provenance `source_context_ids` / `anchors` on capture and create are recorded **claims**: stored verbatim, attributed to the provider-populated `captured_by`, and never validated as authorization. Imported provenance is adopted verbatim as historical attribution linked through `import_ref` (§6.6); adoption never re-stamps `captured_by`.
- Reflection bodies are derived data, trusted only as to basis and generator metadata.

## 11. Concurrent clients, revoke, freeze, delete races

- Single authority (`MEM-R001`) serializes conflicting writes; CAS (`MEM-R007`) detects the rest. No client-side locking assumption exists.
- **Revoke vs in-flight:** admission-time evaluation (`MEM-R018`). Receipt disclosure rechecks current authorization (§7.0.5).
- **Freeze/fence vs write:** the fence commit and the admission-gate update are one transaction; a write either passes admission before the fence commit (and completes) or is rejected after — never half-applied. This applies to space freeze and context fence alike (`MEM-R017`, `MEM-R024`).
- **Delete vs recall:** recall sees either pre- or post-delete state per record, never partial content. **Delete vs reflections:** tombstone-referencing reflections drop out of list/export (§6.9); others read stale per `MEM-R013`.
- **Delete vs receipts:** receipts are content-free (`MEM-R022`), so erasure and immutable audit coexist.
- **Grant narrowing vs existing records:** narrowing `context_ids` / `shared_access` does not rewrite records; it filters future reads and writes and invalidates outstanding cursors (`MEM-R026`).
- **Generation vs source change:** publish-pinned (§7.27).
- **Archive vs export:** export includes archived subjects; recall excludes them.

## 12. Provider switching, export/import, recovery

### 12.1 Cutover sequence (normative steps, operator-timed)

1. `memory.spaces.freeze` on the source space.
2. Drain barrier: poll `memory.spaces.list` until `pending_receipts == 0` (`MEM-R023`).
3. `memory.exports.create` with `require_quiescent: true`; fetch all chunks through `memory.exports.get`; verify chunk and bundle digests.
4. `memory.imports.stage` on the target (recommended into a frozen, drained target space), chunk by chunk, with explicit `adopt_grants`.
5. Verify the staged report (counts, all ID maps, `verification.errors` empty).
6. `memory.imports.activate` (bumps the target `authority_generation`).
7. Operator re-points every participating client binding, pinned to the new `(authority_host_id, authority_generation)`; unfreeze the target.
8. Keep the source frozen until the rollback window closes; then decommission per product policy.

### 12.2 Crash recovery

Freeze/fence state, staged import chunks, receipts, and dedup tombstones are durable. After a crash: pending receipts resolve through `memory.receipts.get` or key replay (`MEM-R010`, `MEM-R021`); half-activated imports are impossible (atomic activation, §7.32); staging resumes at the first unacknowledged chunk ordinal; a staged import resumes to activation or discard.

### 12.3 Identity and reference preservation/translation

Subject IDs are preserved by default (`rename_subject` on collision). Record and candidate IDs and all revisions are re-assigned by the target; source values survive in the staged report maps. Candidate consumption outcomes, reflection basis references, profile/subject/fence revisions, and provenance translate through those maps (§7.31); imported reflections enter `stale: true` (`MEM-R025`). Receipt bodies and live idempotency state do NOT transfer as active state — `receipt_summaries` carry content-free dedup provenance so the target can recognize reimported or replayed capture digests; clients use fresh idempotency keys against the new provider. Grants transfer only through explicit `adopt_grants` (`MEM-R025`).

### 12.4 Rollback

- **Before the target accepts any writes** (target still frozen): `memory.imports.discard` (or operator abandonment of the frozen target space), then controlled binding rollback — clients re-pin to the source and the operator unfreezes the source with a bumped `authority_generation`.
- **After the target has accepted writes:** freeze and drain the target; perform a verified **reverse transfer** — `memory.exports.create(require_quiescent: true)` on the target, stage and activate into the still-frozen source (delta reconciliation through the same staging verification) — and only then reopen the source. Until reverse reconciliation verifies, the source remains frozen and clients observe `SPACE_FROZEN` (explicit blocked state, `MEM-R027`). Default behavior never deletes target-written history and never opens both authorities.

### 12.5 Cutover coordination

Cutover is an operator-coordinated procedure, not a local configuration edit: all participating clients re-pin bindings to the new `(authority_host_id, authority_generation)`. The old provider remains frozen and rejects writes after cutover; stale clients fail with `SPACE_FROZEN` against it. Nothing auto-unfreezes a decommissioned source except an explicit operator re-open, which bumps `authority_generation` so stale pins are detectable (`MEM-R027`).

## 13. Wire/transport mapping

- Memory ops ride the existing Connect envelope: `ConnectInvokeRequest.op` carries the exact op string (e.g. `memory.capture`) and `payload` carries the op's request object — the same wrapping rule as the core `extract` op.
- **Payload sizing (limits apply to values, not to envelopes).** `memory.describe.limits` entries bound specific values:
  - `max_content_bytes` — a **byte** bound on a serialized `content` value (request field and stored content, §7.11, §7.16).
  - `max_candidates_per_capture` — an **item-count** bound on the candidates one capture may produce/accept. It is not a byte bound.
  - `export_chunk_bytes` — a byte bound on the **serialized `MemoryExportChunk` value itself** (the size `Chunk.bytes` reports, §6.13). It does not bound the enclosing `memory.imports.stage` request or `memory.exports.get` response, which wrap the chunk plus other fields (`space_id`, `conflict_policy`, `adopt_grants`, `idempotency_key`, pagination or `extensions`); those enclosing payloads are accordingly larger than the chunk they carry.
  - `max_page_size` — an item-count bound on a response page (§7.0.7).
- **Oversized single element (§6.13 exception).** An element larger than `export_chunk_bytes` occupies its own chunk; that chunk's `bytes` — and therefore the serialized chunk — exceeds the limit, and the element is never split or dropped.
- The Connect invoke envelope wrapping any payload (op string, correlation identity, and other envelope fields) is outside every limit above, and transfer correctness does not depend on an envelope size ceiling: each chunk digest and the reassembled `bundle_digest` are verified on receipt (§6.13, `crates/spoke-memory/src/transfer.rs`).
- **Dispatch gate:** every `memory.*` op has a `required_capability` row mapping it to `spoke-memory` or the sub-flag per §4.1 (`crates/spoke-connect/src/core/dispatch.rs`, mirrored in the TypeScript dispatch tests). Unknown ops keep failing closed with `op_unsupported`; unnegotiated memory ops fail `capability_missing` before any handler runs, with zero side effects.
- **Serving seam:** `RemoteMemoryService` in `crates/spoke-connect/src/remote/serve_ports.rs`; `MemoryPortService` adapts a provider `MemoryPort` to it, mapping the authenticated connect peer to a `Principal` through an admission context (`MemoryAdmission`). One callable serves every `memory.*` op — the typed per-op contract already lives on `MemoryPort`, so no second 33-method serving trait exists.
- **Client seam:** remote adapters map each `MemoryPort` method to its `memory.*` op — Rust `crates/spoke-connect/src/remote/remote_adapter.rs`, TypeScript `packages/spoke-connect-ts/src/remote/remote-adapter.ts` (method → op table exported for parity tests).
- **Chunk header carrier:** a chunked transfer transmits the logical bundle's `extensions` through `MemoryExportChunk.header.extensions` on `chunk_ordinal = 0` (§6.13); reassembly writes it back before digest verification, so a valid bundle round-trips losslessly.
- **Fallback prohibition (`MEM-R019`):** absent the negotiated capability, there is no emulation path through baseline objects — including `KnowledgeEntry`, `modules.*`, `Scope.viewpoint`, or `port.*`. `Scope.viewpoint` / `ke-ownership` semantics are never consulted for memory ops (`MEM-R020`).
- In-process embedding uses `MemoryPort` directly with a product-supplied admission context producing the authenticated `Principal`. `MemoryPorts.as_memory()` (TS `asMemory`) is the optional availability probe: adapters that do not serve memory report absence, and the library never falls back.

## 14. Errors and unsupported handling

### 14.1 Code vocabulary (wire strings)

Reused ops-layer codes (`SCREAMING_SNAKE`, `SpokeRejectCode`): `INVALID_INPUT`, `INVALID_STATUS_TRANSITION`, `REVISION_CONFLICT`, `INTERNAL_ERROR`. Reused dispatch-layer codes (`snake_case`, Connect boundary): `op_unsupported`, `capability_missing`. Memory-specific codes: `NOT_FOUND`, `NOT_AUTHORIZED`, `GRANT_REVOKED`, `SPACE_FROZEN`, `SUBJECT_FENCED`, `CANDIDATE_CONSUMED`, `PROFILE_INCOMPLETE`, `INSUFFICIENT_DATA`, `IDEMPOTENCY_CONFLICT`, `OUTCOME_UNCERTAIN`, `EXPORT_EXPIRED`, `IMPORT_CONFLICT`, `RECEIPT_EXPIRED`, `CURSOR_INVALID`.

The fourteen memory-specific codes are first-class `SpokeRejectCode` variants in both languages (`crates/spoke-operations/src/result.rs`, `packages/spoke-operations/src/result.ts`) and round-trip through their wire strings.

`GRANT_REVOKED` vs `NOT_AUTHORIZED`: `GRANT_REVOKED` applies when the principal presents a specific grant that exists but is revoked or expired; `NOT_AUTHORIZED` applies to absence of coverage (and is also the correct code when the provider declines to reveal existence).

### 14.2 Retry classes

Every error envelope carries `details.retry_class` (required by `MemoryErrorEnvelope`, `MEM-R016`):

| Class | Meaning | Client action |
|-------|---------|---------------|
| `retryable` | No effect committed; safe to retry (same key if mutating). | Retry with backoff. |
| `terminal` | No effect; retrying unchanged will fail again. | Fix the request or authorization. |
| `uncertain` | Commit state unknown (timeout or transport failure after admission). | Poll `memory.receipts.get` or replay with the same `idempotency_key` (`MEM-R010`). |

Mapping: `INVALID_INPUT`, `NOT_AUTHORIZED`, `GRANT_REVOKED`, `CANDIDATE_CONSUMED`, `PROFILE_INCOMPLETE`, `INSUFFICIENT_DATA`, `IDEMPOTENCY_CONFLICT`, `EXPORT_EXPIRED`, `IMPORT_CONFLICT`, `RECEIPT_EXPIRED`, `CURSOR_INVALID`, `REVISION_CONFLICT`, `INVALID_STATUS_TRANSITION`, `SPACE_FROZEN`, `SUBJECT_FENCED`, `NOT_FOUND`, `op_unsupported`, `capability_missing` → `terminal`. `OUTCOME_UNCERTAIN` → `uncertain`.

`INTERNAL_ERROR` is `retryable` **only when the provider asserts no effect committed** (storage or runtime failure before commit). If effects may have committed or cost was charged, the provider MUST NOT claim a clean no-effect failure: it returns `OUTCOME_UNCERTAIN`, or completes the receipt as `failed` with content-free cost facts (§14.3).

### 14.3 Uncertain outcomes and paid generation

A provider that cannot determine whether a mutating op committed returns `OUTCOME_UNCERTAIN`, or the client observes a transport timeout. Resolution is always: same-key replay (returns the original receipt, `MEM-R008` / `MEM-R021`) or `memory.receipts.get`. There is no other recovery channel.

For async pending receipts (§7.0.3): the pending receipt is durable; after a provider crash the work is resumed or the receipt is completed as `failed`. Duplicate paid generation is impossible because replay returns the same pending or committed receipt (`MEM-R008`). A `failed` generation receipt MAY record content-free cost facts (`cost.units` / `unit_kind`) actually charged; cost facts never include input text.

## 15. Semantic locks

Four semantics are locked into the wire and the operations contract. Changing any of them is a breaking wire change (§3).

### 15.1 Subjective memory, not knowledge

Memory records are per-subject impressions captured and reviewed by that subject's product flows; they are never KnowledgeEntry canon. The family ships its own envelopes, authorization model, and lifecycle, and shares only the common primitives (`SchemaVersion`, `Timestamp`, `ExtensionMap`, `OpaqueJson`, `SourceAnchor`) with the KB surface. A memory record never becomes a KnowledgeEntry by promotion; a product that also wants KB canon writes KnowledgeEntries through `upsert` / `promote`.

### 15.2 Forgetting is explicit only

Every erasure path is an operation: candidate `drop` (review) or `dismiss`, `memory.records.delete` (tombstone), `memory.subjects.fence` with lifecycle effect, subject `archive`, and `memory.imports.discard` for staged data. Records carry no expiry field and no time-based fading: providers MUST NOT delete or hide a retained record because time passed. `expires_at` exists only on `MemoryCandidate` (queue hygiene for unreviewed captures) and on `MemoryGrant` (authorization expiry) — never on `RetainedRecord`.

Erasure with audit: receipts are content-free (`MEM-R022`), so deletion of content and an immutable operation trail coexist.

### 15.3 Affect rides `content.structured`

Emotion is expressed through the opaque `content.structured` object and the open `kind` vocabulary (e.g. `{ "text": "…", "structured": { "affect": { "valence": 0.6 } } }`). There is no first-class `affect` wire field, no affect enum, and no affect index. The convention is documented, never schema-enforced: mood taxonomies are product policy, round-tripped verbatim through `content.structured` and `extensions`.

### 15.4 Flat subjects

`Subject` is flat. A memory record names exactly one `subject_id` (who the memory is about) and carries no `about_subject_ids[]` or other cross-subject reference field. Cross-subject references live inside `content.structured` or `extensions.<namespace>` under product policy.

Per-viewer differences are expressed only through the two independent read axes — placement (`scope` visibility + `context_id`) and read selection (`ReadSelection` modes plus `ProvenanceFilter`) — together with context-scoped grants and context fences. Placement, provenance, and read selection never overload null/empty arrays, and no hidden graph semantics exist.

## 16. ProfileDocument — the trusted core of personality memory

Profile documents are the refinement landing point of personality memory and the only trusted content in the family. Schema and field table: §6.8.

### 16.1 Definition (three lines)

- **What it is** — author-maintained, per-subject refined personality documents: the distilled, hand-written baseline of who a subject is. The only trusted content: records and candidates are untrusted capture material, profiles are authored material.
- **Trust boundary** — the content itself is trusted (author-written), but a profile's existence or content constitutes **no authorization whatsoever**. Authorization flows only from `MemoryGrant` (`MEM-R003`, `MEM-R020`); profile presence never implies capability; provider features are expressed exclusively through the §4.1 flags.
- **Reflection boundary** — ProfileDocument is **authored** (hand-written static basis); Reflection is **derived** (generated synthesis over a pinned basis). The two are never conflated.

### 16.2 Four use cases

| Use case | Who writes / when | Readers | Invalidation |
|----------|-------------------|---------|--------------|
| **1 · Personality basis** — `profile_kind = "personality"`: stable traits (voice, values, long-term dispositions) | Author; rarely changes — long-lived | Execution conditioning per product policy; reflection admission | Author edits only; the provider never rewrites authored content |
| **2 · Preferences basis** — `profile_kind = "preferences"`: the author's stable preference settings about the subject | Author | Downstream consumers | Author edits only |
| **3 · Reflection admission gate** (load-bearing) — the provider declares `required_profile_kinds` in `memory.describe`; generation runs profile admission first; any missing or empty required kind fails `PROFILE_INCOMPLETE`, fail closed, no empty success. A missing *non-required* kind is honest-empty: fewer documents returned, never an error | Provider configuration declares; admission evaluated at `memory.reflections.generate` | Callers of reflection generation | Authoring the missing kind unblocks generation |
| **4 · Three states never collapse** (hard rule) — capability missing (`capability_missing`, provider *selection* failure) ≠ profile incomplete (`PROFILE_INCOMPLETE`, profile *admission* failure) ≠ optional data absent (honest-empty *success*) | — | All clients | — |

### 16.3 Content-kind discipline

`profile_kind` is an open string vocabulary of authored content kinds; the core pair `personality` / `preferences` is documented, not schema-enforced. `ProfileDocument` carries content kinds only. Provider policy (binding, retention, review policy) and transfer manifests are configuration and state — they are never expressed as fake profile documents to steer the provider. A profile kind is never a capability concept (§4.2).

### 16.4 Three-state rule (normative)

| Outcome | Trigger | Wire result | Retry class |
|---------|---------|-------------|-------------|
| `capability_missing` | The provider does not serve the requested sub-capability | Rejection at the dispatch/admission boundary, zero side effects | `terminal` |
| `PROFILE_INCOMPLETE` | A required authored kind is absent or empty for the subject | Rejection with `details.missing` naming the kinds | `terminal` |
| honest-empty success | A non-required kind is absent | Success carrying fewer documents | — |

Collapsing any two of these into one observable outcome is a conformance violation (`MEM-R005`).

### 16.5 Lifecycle

- **Profile update ⇒ reflection goes stale.** Profile revisions are an explicit basis-movement source: the pinned basis records `profile_revisions`, and staleness is computed at read time (`MEM-R013`).
- **Revision and CAS** per §7.24: `expected_revision` (null = create-only); mismatch fails `REVISION_CONFLICT`; a genuine no-op does not increment the revision (`MEM-R007`).
- **Idempotency:** profile updates carry `idempotency_key` (`MEM-R008` exempts only `memory.subjects.ensure`).
- **Uniqueness:** one document per `(space, subject, profile_kind)`; a create with an existing kind is a CAS conflict, not an implicit overwrite.

### 16.6 Where "soul" lands

An authored soul document (`SOUL.md` in products that keep one) lands as `ProfileDocument.profile_kind = "personality"` — author-maintained and trusted. That projection is soul's only protocol-level landing point: the protocol has no concept named "soul" and knows only profile kinds.

Soul narrative and experience aggregation land as **Reflection** (derived). A reflection's scope is `ReadSelection` AND `ProvenanceFilter{source_context_ids}` — two independent axes (a per-scope projection is a provenance filter; a binding scope is placement/visibility). The basis pins selection + filter + authorized source-set digest + source revisions + `engine_policy_version`; `state_digest` commits to all of it.

Physical storage (soul files, narrative tables) stays product-side; the protocol never prescribes storage shape.

`TriggerSoulExperienceOnly` is the upstream product action that projects soul narrative and experience material into memory. It is a product-side action — currently runtime-skipped upstream, so it is unsupported in practice — and it is neither a protocol capability nor implemented in this repository. This family ships only the access surfaces: `memory.profile.*` and `memory.reflections.*`, with reflection generation gated exclusively by §4.2 profile admission and `MEM-R005`.

### 16.7 Personality memory is three layers

| Layer | Object | Trust | Role |
|-------|--------|-------|------|
| Refined authored basis | `ProfileDocument` | Trusted (author-written) | Who the subject is, per authored content kind |
| Concrete experiences | `RetainedRecord` (via `MemoryCandidate` → review) | Untrusted | What the subject did, saw, or felt |
| Cross-record synthesis | `Reflection` | Derived, trusted only as to basis | What the record set amounts to, pinned to a basis |

Personality = basis + experiences + synthesis. No layer is authority for another; no layer is a second source of truth for a fact owned by a different layer.

## 17. Conformance anchors

Conformance scenarios cite at least one `MEM-R###` anchor from §5 or the precise §6/§7 contract they exercise (section-level citations name the exact op or model section). Identical-outcome expectations apply to: admission/authorization results (including `shared_access` and `subject_all` coverage), CAS conflicts, idempotent replays and dedup-tombstone behavior, candidate consumption atomicity and placement inheritance, fencing rejections (space, subject, context), freeze allowlist and drain barrier, cursor invalidation, export digest verification, staged-report contents, profile admission three-state distinguishability, profile-update ⇒ reflection-stale, and grant non-amplification.

Algorithm-variable expectations apply to: candidate classification dispositions, recall ranking order, reflection body text, and any provider scoring signal. Two conforming providers MAY disagree on those and still be conformant under equal `memory_protocol_version`.

Both the reference-policy loopback provider and a repository-local persistent reference provider exercise the same scenario set over Connect; a scenario passes only when both providers produce the identical outcome required by the anchor it cites. Provider selection for conformance proof is decided in [`spoke-memory-capability-adr.md`](spoke-memory-capability-adr.md) §1.5.

## 18. Current state

This section states what this repository contains today for the family. Normative sections above define the contract; this section maps it to committed artifacts.

### 18.1 Wire and schemas

- **13 data schemas:** `schemas/data/` — `memory-space`, `subject`, `principal`, `memory-grant`, `retained-record`, `memory-candidate`, `profile-document`, `reflection`, `memory-receipt`, `read-selection`, `provenance-filter`, `memory-export-bundle`, `memory-export-chunk`.
- **66 op schemas:** 33 request + 33 response pairs named `memory-<segments>-{request,response}.schema.json` under `schemas/ops/`.
- **79 family schema files**, counted by `EXPECTED_SCHEMA_COUNT` in [`tooling/codegen/assert-schema-count.mjs`](../../tooling/codegen/assert-schema-count.mjs) alongside the rest of the inventory (113 committed `*.schema.json` files in total). [`schemas/README.md`](../../schemas/README.md) lists the family.
- **Generated packages:** TypeScript `packages/spoke-schemas/src/generated/{data,ops}/` and Rust `crates/spoke-schemas/src/generated/{data,ops}/`; drift fails `pnpm run verify-codegen`.

### 18.2 Operations library

- **`MemoryPort`** (TS interface, Rust `#[async_trait]` trait) in the adapter ports modules — standalone like `ExtractionPort`, not part of `BaselinePorts` / `FullPorts` / `ComputablePorts` / `ForkPorts` or their aliases. Availability probe: `MemoryPorts.as_memory()` / `asMemory()`.
- **Pure helpers** in `packages/spoke-operations/src/memory/` and `crates/spoke-operations/src/memory/` — per-op request validators for the conditional rules the codegen toolchain cannot derive, `ReadSelection` / `ProvenanceFilter` invariants, grant-coverage evaluation (`grantCovers` / `grant_covers`, `validateGrantIssuance` / `validate_grant_issuance`, `GrantRequirement`), the freeze-allowlist predicate (`is_freeze_allowlisted` / `isFreezeAllowlisted`), and RFC 8785 JCS + SHA-256 digest helpers (`jcsSha256Hex` / `jcs_sha256_hex`).
- **Reject codes:** the 14 memory codes of §14.1 are `SpokeRejectCode` variants with wire-string round-trip.

### 18.3 Reference engine

`crates/spoke-memory/` (`spoke-memory`, `ENGINE_POLICY_VERSION = "spoke-memory/1.0.0-deterministic"`, façade `MemoryEngine`) with four policy modules: `classify` (candidate disposition), `recall` (rank + budget + truncation), `reflect` (basis digests, staleness, assembly), `transfer` (canonical JSON, bundle/chunk digests, partitioning, reassembly and verification, translation maps, grant adoption, staged report). No storage, I/O, or enforcement duties.

### 18.4 Connect surface

- Dispatch rows for all 33 ops under the three flags (`crates/spoke-connect/src/core/dispatch.rs`), with TypeScript parity (`packages/spoke-connect-ts/tests/dispatch.test.ts`).
- `RemoteMemoryService` + `MemoryPortService` + `MemoryAdmission` (`crates/spoke-connect/src/remote/serve_ports.rs`), optionally composed through `RemoteServePortsComposite::with_memory`.
- Client method → op mapping in the Rust remote adapter and `packages/spoke-connect-ts/src/remote/remote-adapter.ts`; deny classifications and uncertain-timeout semantics follow §14.

### 18.5 Manifest and gates

- Hosts advertise the flags through `HostCapabilityManifest.capabilities[]` (open string vocabulary, no new manifest field).
- Unknown ops fail closed with `op_unsupported`; unnegotiated memory ops fail `capability_missing` before a handler runs.

### 18.6 Boundary statement

Author Memory is standardized as the optional `spoke-memory` capability family defined by this document. A product that does not implement the family keeps memory product-local and MUST NOT emulate memory semantics through baseline objects (`MEM-R019`). World KB and mental-state surfaces (`modules.mental`, `modules.belief`, `MindState`) remain independent and referenceable; nothing is copied into memory.

## See also

| Doc | Topic |
|-----|-------|
| [`spoke-protocol.md`](spoke-protocol.md) | Umbrella — data / ops / operations-library columns; capability flags |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | L0–L8 layer model; optional capability families |
| [`spoke-ops.md`](spoke-ops.md) | Baseline and optional op wire families; error envelope attachment |
| [`spoke-operations.md`](spoke-operations.md) | Operations-library conventions; [adapter interfaces](spoke-operations.md#adapter-interfaces-normative); [injection orchestration](spoke-operations.md#injection-orchestration-normative) |
| [`spoke-data-model.md`](spoke-data-model.md) | Shared primitives reused by memory models (`SourceAnchor`, `Scope`, `ExtensionMap`) |
| [`spoke-memory-capability-adr.md`](spoke-memory-capability-adr.md) | Capability decision: optional family, single authority, receipt model, engine/provider split, conformance providers |
| [`spoke-connect.md`](spoke-connect.md) | Connect envelope family — invoke request/response carrying `memory.*` payloads |
| [`ke-extraction-adr.md`](ke-extraction-adr.md) | Precedent for a standalone optional port outside `BaselinePorts` |
| [`l5-mind-capability-adr.md`](l5-mind-capability-adr.md) | Mental-state capability boundary; authority vs derivative split |
| [`schemas/README.md`](../../schemas/README.md) | Schema inventory and authoring rules |
| [`CONCEPTS.md`](../../CONCEPTS.md) | Protocol vocabulary; memory family boundary |
