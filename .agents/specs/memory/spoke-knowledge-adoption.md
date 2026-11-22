# SPOKE Knowledge Adoption (`spoke-knowledge-adoption`)

> **Status:** Normative
> **Document class:** Normative — optional capability family (wire + operations + connect serving)
> **Parent:** [`spoke-protocol.md`](../protocol/spoke-protocol.md)
> **Wire SSOT:** `schemas/data/adoption-anchor.schema.json`, `schemas/ops/knowledge-adopt-request.schema.json`, `schemas/ops/knowledge-adopt-response.schema.json`
> **Operations library:** `packages/spoke-operations/src/adoption.ts`, `crates/spoke-operations/src/adoption.rs`; port `AdoptionPort` in `packages/spoke-operations/src/adapter/ports.ts`, `crates/spoke-operations/src/adapter/ports.rs`; put-path anchor gate in `packages/spoke-operations/src/adapter/orchestrate.ts`, `crates/spoke-operations/src/adapter/orchestrate.rs`
> **Connect serving:** `RemoteAdoptionService` / `AdoptionPortService` in `crates/spoke-connect/src/remote/serve_ports.rs`, `packages/spoke-connect-ts/src/remote/serve_ports.ts`; dispatch rows in `crates/spoke-connect/src/core/dispatch.rs`, `packages/spoke-connect-ts/src/core/dispatch.ts`
> **Capability decision:** [`2026-11-23-spoke-knowledge-adoption.md`](../../adr/implemented/feature/2026-11-23-spoke-knowledge-adoption.md)

## Scope

This page specifies the optional `spoke-knowledge-adoption` capability: adopting a shared `KnowledgeEntry` into a destination scope. It fixes the `AdoptionAnchor` provenance shape and its immutability, the `knowledge.adopt` wire request and response, the standalone `AdoptionPort`, the copy and governance-reset rules, retirement-chain resolution, repeat and connect idempotency, the refusal vocabulary, atomicity, and the connect capability gate.

It does not define physical storage; how a `merged` entry names its successors (a storage-owned input to the resolver); transport beyond the existing Connect invoke envelope; baseline `port.*` ops or `ScopedKnowledgePort` serving (see [`spoke-operations.md`](../protocol/spoke-operations.md)); the semantics of product `extensions` namespaces; or any refresh or synchronization of previously adopted copies — an adopted copy is written once and is never refreshed or re-anchored by adoption.

## Current state

### Problem & user value

| Without an adoption contract | With `spoke-knowledge-adoption` |
|------------------------------|--------------------------------|
| Each product encodes adoption provenance in private fields no second host can parse | One protocol `adoption_anchor` on the destination entry, carried through get, put, and serve paths |
| Repeat calls, moved sources, and invisible sources behave differently per host | One repeat rule, one terminal-source resolution, one generic absence |
| A copy silently inherits the source's governance | Destination-owned `owner` and `disclosure` on every adopted copy |

The key words **MUST**, **MUST NOT**, **SHOULD**, and **MAY** are to be interpreted as described in RFC 2119. They apply to implementations that claim the `spoke-knowledge-adoption` capability.

### Capability flags

| Capability ID | Gates | Relation |
|---------------|-------|----------|
| `spoke-knowledge-adoption` | `knowledge.adopt` | Declaring it requires also declaring `spoke-knowledge` (mandatory co-capability) |
| `spoke-knowledge` | Remote scoped knowledge serving (see [`spoke-operations.md`](../protocol/spoke-operations.md)) | Never satisfied by `spoke-baseline`; serves as adoption's co-capability |

Rules:

- Both peers MUST advertise `spoke-knowledge-adoption` and `spoke-knowledge` in `HostCapabilityManifest.capabilities`, and both exact strings MUST be members of `negotiated_capabilities`. Neither `spoke-baseline` nor either capability alone satisfies the gate for `knowledge.adopt`.
- A host declaring `spoke-knowledge-adoption` also declares `spoke-knowledge`: adoption needs the scoped serving infrastructure to admit a caller's source visibility and destination authorization.
- If either capability is absent, the connect dispatch gate answers `op_unsupported` (no capability row) or `capability_missing` (row present but not negotiated) before the handler runs and with no side effects.
- `knowledge.*` is a reserved-product op prefix. Ops under it route to the adoption serving face; the family defines exactly one op, `knowledge.adopt`.

### Wire shapes

#### `AdoptionAnchor`

A closed object (`additionalProperties: false`) with exactly these required fields:

| Field | Type | Contract |
|---|---|---|
| `schema_version` | `common.schema.json#/definitions/SchemaVersion` via `$ref` | Version of the `AdoptionAnchor` wire schema. |
| `source_scope_id` | non-empty string | Scope id of the resolved terminal source. |
| `source_entry_id` | non-empty string | Entry id of the resolved terminal source. |
| `source_revision` | integer, minimum 0 | Revision of the resolved terminal source. |
| `extensions` | shared `ExtensionMap` | Product-namespace extensions; unknown namespaces and keys round-trip unchanged. |

`KnowledgeEntry.adoption_anchor` is an optional `$ref` to this schema. It carries adoption provenance as part of the entry payload through get, put, and serve paths — it is payload, not a side record. `source_anchor` and `extensions` keep their own contracts; adoption encoding does not live in them.

#### `KnowledgeAdoptRequest`

A closed object (`additionalProperties: false`) with this member set:

| Field | Presence | Type | Contract |
|---|---|---|---|
| `source_scope_id` | required | non-empty string | Requested source scope. Retirement resolution can change the terminal scope/id recorded by the anchor. |
| `source_entry_id` | required | non-empty string | Requested source entry. |
| `source_revision` | required | integer, minimum 0 | Requested source revision. |
| `destination_scope_id` | required | non-empty string | Destination scope admitted by the host. |
| `destination_entry_id` | optional | non-empty string | Destination id. When absent, the resolved terminal `source_entry_id` is the default; for a live source this equals the requested id. |
| `idempotency_key` | required | string, 1–128 Unicode code points inclusive | Connect retry key. Length is not counted in UTF-16 code units or UTF-8 bytes. |
| `extensions` | optional | shared `ExtensionMap` | Product-namespace request extensions, validated against the shared `ExtensionMap` constraints at request validation — keys match the shared key pattern and values are objects; a violation answers `INVALID_INPUT`. When present, the member survives byte-identically through decode and digest: an explicitly empty `extensions: {}` is a present member and stays a present member. |

#### `KnowledgeAdoptResponse`

The standard ops `oneOf` success/failure envelope. The success branch carries exactly `outcome` and `entry`, and the adoption response admits no further members — no response `extensions`. `entry` is the destination `KnowledgeEntry`, and a success entry always carries `adoption_anchor` as a well-formed `AdoptionAnchor` object — one satisfying the `AdoptionAnchor` schema: a provider result whose entry lacks the anchor or carries an anchor outside that schema is malformed and is answered as a transport/serving refusal rather than `Ok`. `outcome` is `"adopted"` for a newly written copy and `"already_present"` for an existing copy returned by the repeat rules. The failure branch carries the shared `ErrorEnvelope`.

### The `AdoptionPort`

`AdoptionPort` is standalone and composes separately from `ScopedKnowledgePort`. Neither port inherits the other, and `spoke-baseline` does not implicitly provide adoption. The language-neutral asynchronous signature is:

```text
adopt_knowledge_entry(principal, scope, request)
  -> SpokeResult<KnowledgeAdoptResponse>
```

| Name | Type | Contract |
|---|---|---|
| `principal` | shared `Principal` | Authenticated admission context supplied by the host; explicit first parameter. |
| `scope` | shared `Scope` | Admitted scope context for source visibility and destination authorization; explicit second parameter. |
| `request` | `KnowledgeAdoptRequest` | Adoption selectors, destination id override, retry key, and extensions. |

- Principal identity and scope derive from the host's authenticated admission context, never from payload fields (`MEM-R003` parity in [`spoke-memory.md`](spoke-memory.md); same principal-first, scope-at-port shape as `HistoryPort`).
- Source and destination scoped-serving dependencies are host-supplied port composition, selected by the host rather than by payload identity: the host decides where copies can land.
- TypeScript uses `adoptKnowledgeEntry(principal, scope, request)` returning `Promise<SpokeResult<KnowledgeAdoptResponse>>`. Rust uses `#[async_trait] async fn adopt_knowledge_entry(&self, principal, scope, request) -> SpokeResult<KnowledgeAdoptResponse>` with borrowed admission/scope/request arguments and a `Send` future. Expected refusals are returned as `SpokeResult` values, not thrown.
- The port receives the full `KnowledgeAdoptRequest`, including `idempotency_key` — both language servings pass the complete typed request object to the provider. Deduplication stays Connect-owned; `idempotency_key` is pass-through context for the provider.
- The availability probe mirrors the history family: types implementing `AdoptionPort` answer `AdoptionPorts::as_adoption()` / `asAdoption()` with themselves; dynamically assembled boundaries MAY answer `None` / `undefined`.

### Admission and owner-private absence

The caller MUST be admitted to both the requested source and the destination scope through the host's admission context; the request's scope ids are selectors validated against that admission, never authority. The visibility predicate applies to the entry adoption actually copies — the requested source when it is live, and the resolved terminal source after retirement-chain resolution: a requested source that fails to load, a resolved terminal outside the admitted scope, or one owner-private to another viewpoint produces the same generic `NOT_FOUND` refusal and no destination write occurs. The response carries no source-identifying `details` members and writes neither a destination copy nor an anchor. Repeated calls produce the same absence.

Destination `owner` and `disclosure` values govern the new copy. Source governance values are never inherited.

### Retirement-chain resolution

A live requested source is adopted directly. A source with status `merged` is resolved through its retirement chain to a terminal live source; adoption copies the terminal, never a retired intermediate. The anchor records the terminal live source's `source_scope_id`, `source_entry_id`, and `source_revision`, and the default destination id is that terminal `source_entry_id`.

Resolution follows at most 32 retirement edges after the requested entry (`RETIREMENT_CHAIN_MAX_EDGES`). Each visited entry id is tracked. Resolution refuses with `SOURCE_RETIRED` and `details.retirement_chain` when:

- a visited entry id repeats (a cycle);
- a `merged` entry has zero or multiple successors, or its successor resolves to a missing or unresolvable entry (an ambiguous chain);
- a terminal entry has status `deleted` rather than a live status;
- resolution requires a 33rd edge before reaching a terminal live source.

`details.retirement_chain` lists only the source entry ids that resolution actually visited, beginning with the requested id and ending at the last resolved node — the entry at which resolution failed. The over-limit refusal ends at the last resolved (still-`merged`) node; the successor beyond the rejected edge stays unlisted. A refusal raised by a missing or unresolvable successor likewise ends at the last resolved node; the unresolvable target stays unlisted. Successor lookup and node loading are storage-owned (`successors_of` / `load_entry`); how a `merged` entry names its successors is not defined by this protocol. A successful resolution copies only the terminal live source and writes only its provenance into `adoption_anchor`.

### Copy rules and create-only OCC

The destination copy follows these rules:

- `body`, `canonical_name`, and `entry_type` are preserved.
- `modules` is preserved except `modules.cognition`, which is not copied (derivative data, not identity).
- `source_anchor` is cleared; adoption provenance lives in `adoption_anchor`.
- `extensions` is copied verbatim except the legacy `mindcore_adoption` namespace, which is dropped. Every other namespace and key survives unchanged, including explicitly empty namespace objects.
- `owner` and `disclosure` are reset to the destination's own values.
- `entry_id` is destination-assigned: `destination_entry_id` when supplied, otherwise the resolved terminal source entry id.
- The destination write uses create-only OCC with `expectedBaseRevision: null` / `expected_base_revision: None`; the source revision is never reused as the destination's stored OCC base.

### Atomicity and concurrency

The complete adoption is one atomic transaction. A failed adoption leaves neither a destination copy nor a partial anchor. The create-only destination CAS is the concurrency guard. If another writer claims the destination slot before the write, the stored entry is classified by its `adoption_anchor` using the resolved source tuple `(source_entry_id, resolved_terminal_revision)` — the same tuple the create write records: an identical tuple returns that copy as `already_present`; a different resolved terminal revision refuses `REVISION_CONFLICT`; an unrelated occupant retains the existing create-only put refusal and is not overwritten.

### Repeat and idempotency

Adoption has both entry-anchor repeat semantics and connect idempotency:

- The repeat comparator is the resolved source tuple `(source_entry_id, resolved_terminal_revision)` — the same tuple the create write records in `adoption_anchor`. Repeating adoption of the same resolved tuple into the same `destination_scope_id` returns the existing copy with `outcome: "already_present"` and writes no second copy; a repeat issued at the requested revision after retirement-chain resolution classifies on the resolved tuple and returns `already_present`.
- Repeating adoption whose resolved terminal revision differs from the revision recorded in the existing copy's anchor refuses `REVISION_CONFLICT` with `details.adopted_revision` equal to the revision recorded in the existing copy's anchor. The existing copy is not refreshed or overwritten.
- Connect owns deduplication before the admitted port call, using the `MemoryPort` model. The retry identity is the RFC 8785 canonical JSON serialization of the ordered array `(op, effective_scope, requester_principal_id, idempotency_key)`. The member boundaries are structural, distinct tuples always derive distinct identities, and both language stacks and every reconnect derive the identical identity.
- The dedup record stored under the retry identity carries `payload_digest` — the SHA-256 digest over RFC 8785 canonical JSON of the complete request payload object, a payload form that preserves member presence per the request table. The same retry identity whose record carries the same `payload_digest` returns the recorded outcome with `outcome: "already_present"`; no second provider call or write occurs. The same retry identity whose record carries a different `payload_digest` refuses `IDEMPOTENCY_CONFLICT`, which has no adoption-specific `details` members.
- Single-flight is owned by the dedup store interface, not by a service instance: one store object shared by multiple service instances still yields at most one in-flight provider invocation per retry identity, through atomic probe-and-record or keyed single-flight at the store interface. Calls with unrelated retry identities proceed concurrently and stay independent of an unrelated in-flight call. A provider invocation that ends without a settled outcome releases its single-flight claim — recorded-outcome dedup covers settled outcomes only, and the next call with the same retry identity invokes the provider again. Cross-process atomicity is a store-implementation obligation.
- Dedup records outlive the Connect session (the store is host-injectable); an evicted record answers `RECEIPT_EXPIRED` and the adoption is not re-executed.

### Anchor immutability

Every spoke-operations knowledge write orchestration that stores a `KnowledgeEntry` preserves a stored `adoption_anchor` byte-identically or refuses `ADOPTION_ANCHOR_IMMUTABLE` with `details.entry_id` naming the destination entry before the write occurs. The obligation is owned by each write path by name: `orchestrateUpsert` / `orchestrate_upsert`, `orchestratePromote` / `orchestrate_promote` (which loads the stored entry before writing the accepted candidate), and the scoped put path serving `knowledge.put`. Preservation is a two-layer rule: orchestration checks the stored anchor, and the provider port is the atomic backstop. Each named write path loads the stored entry and compares its stored anchor before issuing the write, refusing when the write would change or remove it; the write then carries the loaded entry's revision as its compare-and-swap base, and the provider port's CAS on stored state accepts the write only against that revision. A write that carries the identical stored anchor leaves the anchor unchanged. Anchor equality is RFC 8785 canonical-JSON byte equality; two anchors that cannot be canonicalized compare equal across the language packages.

### Refusal vocabulary

| Code | Condition | Exact `details` contract |
|---|---|---|
| `ADOPTION_ANCHOR_IMMUTABLE` | A normal put attempts to change or remove an existing destination `adoption_anchor`. | Exactly `entry_id`: the destination entry id. |
| `SOURCE_RETIRED` | Retirement resolution encounters a cycle, ambiguity, a deleted terminal, or the 32-edge limit. | Exactly `retirement_chain`: the ordered visited source-entry-id array. |
| `REVISION_CONFLICT` | An existing destination copy anchors a different source revision. | Exactly `adopted_revision`: the integer revision in the existing anchor. |
| `NOT_FOUND` | The requested source is missing, or the requested or resolved terminal source is outside the admitted scope or owner-private to another viewpoint. | No source-identifying members; one generic absence. |
| `IDEMPOTENCY_CONFLICT` | A connect request reuses an idempotency key with a different request digest. | No adoption-specific members. |

`ADOPTION_ANCHOR_IMMUTABLE` and `SOURCE_RETIRED` are adoption-family `SpokeRejectCode` members; `REVISION_CONFLICT`, `NOT_FOUND`, and `IDEMPOTENCY_CONFLICT` reuse their existing codes and meanings. `INVALID_INPUT` answers malformed payloads at the serving boundary; `RECEIPT_EXPIRED` answers dedup-record eviction; `OUTCOME_UNCERTAIN` (`details.retry_class: "uncertain"`, `kind: "timeout"` / `"panic"`) answers a serve wait that exceeds the local budget or a service failure after invocation — the outcome is unknown, provider work is neither cancelled nor rolled back, and the caller recovers by replaying the same idempotency key.

### Connect operation and capability gate

The reserved-product operation table contains exactly this adoption row:

| `op` | Required capabilities | Request | Success value | Idempotency |
|---|---|---|---|---|
| `knowledge.adopt` | `spoke-knowledge-adoption` and mandatory co-capability `spoke-knowledge` | `KnowledgeAdoptRequest` | Success branch containing `outcome`, `entry` | Required, using the rules above |

- The operation uses the existing opaque connect request and response envelopes; no connect envelope fields change.
- The payload-dependent `ke-ownership` conjunct remains scoped to operations carrying a declared `Scope` in their payload. `KnowledgeAdoptRequest` carries scope ids rather than a declared `Scope`, so no separate op-level `ke-ownership` gate applies; owner-private visibility is enforced through the admitted port `Scope`.
- Serving: the responder routes the `knowledge.*` reserved prefix to the optional injected `RemoteAdoptionService` face, with the same gate → probe → serve/deny order as `memory.*` and `history.*`. `AdoptionPortService` adapts a provider `AdoptionPort` to that face: an `AdoptionAdmission` callback maps the authenticated peer to its `Principal` and admitted `Scope`, and the service owns decoding, the connect dedup, and served-response validation. A host that declares the capability without attaching the face denies the op. Provider success values are validated before emission — a malformed provider result answers an `INTERNAL_ERROR` transport refusal, never `Ok`.
- Client side: `RemoteAdapter` implements `AdoptionPort` over the session by invoking `knowledge.adopt` with the request object (`idempotency_key` included) and decoding the `KnowledgeAdoptResponse`.

## Invariants

- **Anchor immutability.** `adoption_anchor` is immutable after creation: every normal put preserves it byte-identically (RFC 8785 canonical equality) or refuses `ADOPTION_ANCHOR_IMMUTABLE`; no operation rewrites, refreshes, or removes an anchor.
- **Terminal-source provenance.** An anchor always names the resolved terminal live source, never a retired intermediate; an unresolvable chain (cycle, ambiguity, deleted terminal, 33rd edge) refuses `SOURCE_RETIRED` and writes nothing.
- **Destination governance.** `owner` and `disclosure` of a copy come from the destination and are never inherited from the source; `modules.cognition` never crosses; `source_anchor` is cleared; only the legacy `mindcore_adoption` namespace is dropped from `extensions`.
- **Create-only destination write.** The destination write carries `revision: null`; the source revision is never a destination OCC base, and an existing occupant is classified by its anchor (`already_present` / `REVISION_CONFLICT`) or left untouched by the create-only refusal.
- **Uniform absence.** Missing, out-of-scope, and owner-private-to-another-viewpoint sources answer one generic `NOT_FOUND` with no source-identifying details, on every call.
- **Atomicity.** An adoption is one transaction: no partial copy, no partial anchor, no partial governance reset is ever observable.
- **Exact conjunctive gating.** `knowledge.adopt` requires both exact capability strings in `negotiated_capabilities` on both peers; neither `spoke-baseline` nor either capability alone admits it, and gate denials precede the handler with zero side effects.
- **Never re-execute.** Both idempotency layers return the existing copy — the entry-anchor repeat by anchor classification, the connect dedup by recorded outcome bound to the retry identity `(op, effective_scope, requester_principal_id, idempotency_key)` plus `payload_digest`; a settled outcome is recorded before the response is emitted, and an unsettled invocation releases its single-flight claim.

## Where it lives

- Wire data: `schemas/data/adoption-anchor.schema.json`; the optional `adoption_anchor` member of `schemas/data/knowledge-entry.schema.json`.
- Operation wire: `schemas/ops/knowledge-adopt-request.schema.json`, `schemas/ops/knowledge-adopt-response.schema.json`.
- Generated types: `packages/spoke-schemas/src/generated/data/adoption-anchor.ts`, `packages/spoke-schemas/src/generated/ops/knowledge-adopt-{request,response}.ts` and the `crates/spoke-schemas/src/generated/` mirrors; inventory gate `tooling/codegen/assert-schema-count.mjs`, run via `pnpm run verify-codegen`.
- Port and helpers: `AdoptionPort` + `AdoptionPorts` probe in `packages/spoke-operations/src/adapter/ports.ts`, `crates/spoke-operations/src/adapter/ports.rs`; `assertAdoptionAnchorPreserved` / `assert_adoption_anchor_preserved`, `resolveRetirementChain` / `resolve_retirement_chain`, `RETIREMENT_CHAIN_MAX_EDGES`, `ADOPTION_LEGACY_EXTENSIONS_NAMESPACE` in `packages/spoke-operations/src/adoption.ts`, `crates/spoke-operations/src/adoption.rs`; package re-exports in `packages/spoke-operations/src/index.ts`, `crates/spoke-operations/src/lib.rs`.
- Put-path anchor gate: `packages/spoke-operations/src/adapter/orchestrate.ts`, `crates/spoke-operations/src/adapter/orchestrate.rs`, plus the scoped put path serving `knowledge.put` in `packages/spoke-connect-ts/src/remote/responder.ts` and `crates/spoke-connect/src/remote/responder.rs`.
- Reject codes: `ADOPTION_ANCHOR_IMMUTABLE`, `SOURCE_RETIRED` in `packages/spoke-operations/src/result.ts`, `crates/spoke-operations/src/result.rs`.
- Connect dispatch: capability constants and the compound co-capability check in `packages/spoke-connect-ts/src/core/dispatch.ts`, `crates/spoke-connect/src/core/dispatch.rs`.
- Connect serving: `RemoteAdoptionService`, `AdoptionPortService`, `AdoptionAdmission`, `AdoptionDedupStore`, `KNOWLEDGE_OP_ADOPT` in `packages/spoke-connect-ts/src/remote/serve_ports.ts`, `crates/spoke-connect/src/remote/serve_ports.rs`; responder routing and uncertain-outcome refusals in `packages/spoke-connect-ts/src/remote/responder.ts`, `crates/spoke-connect/src/remote/responder.rs`; client `AdoptionPort` in `packages/spoke-connect-ts/src/remote/remote-adapter.ts`, `crates/spoke-connect/src/remote/remote_adapter.rs`.
- Decision record: `.agents/adr/implemented/feature/2026-11-23-spoke-knowledge-adoption.md`.

## Change obligations

- Change the `AdoptionAnchor`, `KnowledgeAdoptRequest`, or `KnowledgeAdoptResponse` wire shape → update §Current state here and the owning `schemas/` file in the same change, regenerate both language packages (`pnpm run verify-codegen`), and update the schema inventory.
- Change the `KnowledgeEntry.adoption_anchor` field → update `schemas/data/knowledge-entry.schema.json`, this page, and the put-path anchor gate in the same change.
- Add an op under `knowledge.*` or change the capability gate → update the dispatch rows and parity tests in both connect packages, §Capability flags and §Connect operation here, and the reserved-prefix serving routing.
- Change a refusal code, condition, or `details` member → update §Refusal vocabulary here and the `SpokeRejectCode` surface in `packages/spoke-operations/src/result.ts` and `crates/spoke-operations/src/result.rs` in the same change.
- Change copy rules, governance reset, retirement resolution, repeat semantics, or idempotency → update the corresponding section here, the helpers in `spoke-operations` (`adoption.*`, `adapter/orchestrate.*`), the serving-face dedup in both connect packages, and the adoption conformance fixtures in the same change.
- Change anchor equality or immutability semantics → update §Anchor immutability here and both language implementations of the gate together; canonical-byte equality must stay language-agnostic.
