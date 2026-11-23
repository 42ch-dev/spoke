---
title: Knowledge adoption reference
---

# Knowledge adoption reference (`spoke-knowledge-adoption`)

`spoke-knowledge-adoption` is the optional capability for adopting a shared `KnowledgeEntry` into a destination scope: one atomic, write-once copy with destination governance and an immutable `adoption_anchor` recording the terminal source. It defines one Connect operation, `knowledge.adopt`.

## Capability

| Aspect | Contract |
|--------|----------|
| Capability string | `spoke-knowledge-adoption` — an optional member of `HostCapabilityManifest.capabilities` |
| Manifest declaration | The host serves the adoption operation and the `AdoptionAnchor` contract |
| Co-capability | Declaring `spoke-knowledge-adoption` requires also declaring `spoke-knowledge` |
| Negotiation | Both exact strings must be members of `negotiated_capabilities` on both peers; neither `spoke-baseline` nor either capability alone satisfies the gate |
| Connect surface | Gates `knowledge.adopt` together with `spoke-knowledge` |

`knowledge.*` is a reserved product prefix owned by the core Connect dispatch decision. `knowledge.adopt` is its single registered operation; every other `knowledge.*` string is an unknown reserved name.

## Operations

| `op` | Required negotiated capabilities | Request | Success value | Idempotency |
|------|----------------------------------|---------|---------------|-------------|
| `knowledge.adopt` | `spoke-knowledge-adoption` and `spoke-knowledge` | `KnowledgeAdoptRequest` | `KnowledgeAdoptResponse` success branch with `outcome` and `entry` | Required |

The operation travels the existing opaque Connect request and response envelopes. Serving routes the `knowledge.*` reserved prefix to the optional `RemoteAdoptionService` face, whose `AdoptionPortService` adapts a provider `AdoptionPort`; a missing capability denies the op before the handler runs and with no side effects.

### `KnowledgeAdoptRequest`

A closed object:

| Field | Presence | Type | Contract |
|-------|----------|------|----------|
| `source_scope_id` | required | non-empty string | Requested source scope; retirement resolution can change the terminal scope recorded by the anchor |
| `source_entry_id` | required | non-empty string | Requested source entry |
| `source_revision` | required | integer ≥ 0 | Requested source revision |
| `destination_scope_id` | required | non-empty string | Destination scope admitted by the host |
| `destination_entry_id` | optional | non-empty string | Destination id; when absent, the resolved terminal `source_entry_id` is the default |
| `idempotency_key` | required | string, 1–128 Unicode code points | Connect retry key; length is not counted in UTF-16 code units or UTF-8 bytes |
| `extensions` | optional | shared `ExtensionMap` | Product request extensions; a present-empty `extensions: {}` stays a present member through decode and digest |

### `KnowledgeAdoptResponse`

The standard ops `oneOf` success/failure envelope. The success branch carries `outcome` and `entry`; the error branch carries `error`; `extensions` is an optional member on either branch. No member beyond `outcome`, `entry`, `error`, and `extensions` is admitted.

| Member | Contract |
|--------|----------|
| `outcome` | `"adopted"` for a newly written copy; `"already_present"` for an existing copy returned by the repeat rules |
| `entry` | The destination `KnowledgeEntry`, always carrying a well-formed `adoption_anchor`; when `destination_entry_id` is absent, `entry.id` supplies the destination id |

A provider result whose entry lacks the anchor or carries an anchor outside the `AdoptionAnchor` schema is answered as a transport/serving refusal, never `Ok`.

### `AdoptionAnchor`

A closed object recording adoption provenance on the destination entry:

| Field | Type | Contract |
|-------|------|----------|
| `schema_version` | shared `SchemaVersion` | Version of the `AdoptionAnchor` wire schema |
| `source_scope_id` | non-empty string | Scope id of the resolved terminal source |
| `source_entry_id` | non-empty string | Entry id of the resolved terminal source |
| `source_revision` | integer ≥ 0 | Revision of the resolved terminal source |
| `extensions` | shared `ExtensionMap` | Product extensions; unknown namespaces and keys round-trip unchanged |

`KnowledgeEntry.adoption_anchor` is an optional `$ref` to this schema and travels as entry payload through get, put, and serve paths.

### Terminal-source provenance

A live requested source is adopted directly. A source with status `merged` is resolved through its retirement chain to a terminal live source, and adoption copies the terminal, never a retired intermediate. The anchor records the terminal live source's `source_scope_id`, `source_entry_id`, and `source_revision`; the default destination id is that terminal `source_entry_id`.

Resolution follows at most 32 retirement edges after the requested entry (`RETIREMENT_CHAIN_MAX_EDGES`) and tracks every visited entry id. A cycle, a `merged` entry with zero or multiple successors, a successor that resolves to a missing or unresolvable entry, a terminal entry whose status is `deleted`, or a required 33rd edge refuses `SOURCE_RETIRED` with `details.retirement_chain` listing exactly the visited source entry ids, beginning with the requested id and ending at the last resolved node.

### Copy rules

| Aspect | Rule |
|--------|------|
| Preserved | `body`, `canonical_name`, `entry_type` |
| Modules | `modules` is preserved except `modules.cognition`, which is not copied |
| Provenance fields | `source_anchor` is cleared; adoption provenance lives in `adoption_anchor` |
| Extensions | Copied verbatim except the legacy `mindcore_adoption` namespace, which is dropped; every other namespace and key survives, including explicitly empty namespace objects |
| Governance | `owner` and `disclosure` are reset to the destination's own values |
| Destination id | `destination_entry_id` when supplied, otherwise the resolved terminal source entry id |
| Destination write | Create-only OCC with `expected_base_revision: null`; the source revision is never reused as the destination's stored OCC base |

Port shape: `AdoptionPort.adoptKnowledgeEntry(principal, scope, request)` in TypeScript and `adopt_knowledge_entry(&self, principal, scope, request)` in Rust, returning `SpokeResult<KnowledgeAdoptResponse>`. The port receives the full typed request, including `idempotency_key` as pass-through context; deduplication stays Connect-owned.

### Repeat outcomes

The repeat comparator is the resolved source tuple `(source_entry_id, resolved_terminal_revision)` — the same tuple the create write records in `adoption_anchor`.

- Repeating adoption of the same resolved tuple into the same `destination_scope_id` returns the existing copy with `outcome: "already_present"` and writes no second copy.
- A repeat whose resolved terminal revision differs from the revision recorded in the existing copy's anchor refuses `REVISION_CONFLICT` with `details.adopted_revision` equal to the recorded revision. The existing copy is not refreshed or overwritten.
- If another writer claims the destination slot first, the stored entry is classified by its `adoption_anchor` using the same tuple: an identical tuple returns that copy as `already_present`, a different resolved terminal revision refuses `REVISION_CONFLICT`, and an unrelated occupant retains the create-only put refusal without being overwritten.

### Connect idempotency

Connect owns deduplication before the admitted port call, using the `MemoryPort` model:

- The retry identity is the RFC 8785 canonical JSON serialization of the ordered array `(op, effective_scope, requester_principal_id, idempotency_key)`; distinct tuples always derive distinct identities across both stacks and reconnects.
- The record under that identity carries `payload_digest` — the SHA-256 digest over RFC 8785 canonical JSON of the complete request payload object, preserving member presence.
- Same retry identity with the same `payload_digest` returns the recorded outcome with `outcome: "already_present"`; no second provider call or write occurs.
- Same retry identity with a different `payload_digest` refuses `IDEMPOTENCY_CONFLICT`.
- Single-flight is owned by the dedup store interface: one store shared by multiple service instances yields at most one in-flight provider invocation per retry identity, and a provider invocation that ends without a settled outcome releases its claim so the next call invokes the provider again.
- Dedup records outlive the Connect session; an evicted record answers `RECEIPT_EXPIRED` and the adoption is not re-executed.

## Refusal vocabulary

| Code | Condition | Exact `details` |
|------|-----------|-----------------|
| `ADOPTION_ANCHOR_IMMUTABLE` | A normal put attempts to change or remove an existing destination `adoption_anchor` | Exactly `entry_id`, the destination entry id |
| `SOURCE_RETIRED` | Retirement resolution meets a cycle, ambiguity, a deleted terminal, or the 32-edge limit | Exactly `retirement_chain`, the ordered visited source-entry-id array |
| `REVISION_CONFLICT` | An existing destination copy anchors a different source revision | Exactly `adopted_revision`, the integer revision in the existing anchor |
| `NOT_FOUND` | The requested source is missing, or the requested or resolved terminal source is outside the admitted scope or owner-private to another viewpoint | No source-identifying members; one generic absence |
| `NOT_AUTHORIZED` | The destination scope fails destination `space_id` authorization, checked before source-chain resolution | No adoption-specific members; the source stays untouched |
| `IDEMPOTENCY_CONFLICT` | A Connect request reuses an idempotency key with a different request digest | No adoption-specific members |
| `INVALID_INPUT` | A malformed payload, including invalid request `extensions` | Field-level detail as for any malformed request |
| `RECEIPT_EXPIRED` | A dedup record for the retry identity was evicted | None beyond the shared envelope |
| `OUTCOME_UNCERTAIN` | A serve wait exceeds the local budget or a service failure follows invocation; the provider outcome is unknown | `retry_class: "uncertain"`, `kind: "timeout"` \| `"panic"` |
| `op_unsupported` / `capability_missing` | Connect dispatch denies the row because the required capability strings are absent from the negotiated set | None; the refusal precedes the handler with no side effects |

`ADOPTION_ANCHOR_IMMUTABLE` and `SOURCE_RETIRED` are adoption-family `SpokeRejectCode` members; `REVISION_CONFLICT`, `NOT_FOUND`, `NOT_AUTHORIZED`, `IDEMPOTENCY_CONFLICT`, and `INVALID_INPUT` keep their existing codes and meanings. `OUTCOME_UNCERTAIN` reports an unknown outcome: provider work is neither cancelled nor rolled back, and the caller recovers by replaying the same idempotency key.

## Usage constraints

- **One atomic write.** A complete adoption is one transaction: a failed adoption leaves neither a destination copy nor a partial anchor, and no partial governance reset is observable. The create-only destination CAS is the concurrency guard.
- **Adoption is write-once.** An adopted copy is written once and is never refreshed, synchronized, or re-anchored by adoption. A later repeat returns the existing copy or refuses `REVISION_CONFLICT`.
- **The anchor is immutable.** Every spoke-operations knowledge write orchestration that stores a `KnowledgeEntry` preserves a stored `adoption_anchor` byte-identically or refuses `ADOPTION_ANCHOR_IMMUTABLE` before the write. The gates are `orchestrateUpsert` / `orchestrate_upsert`, `orchestratePromote` / `orchestrate_promote`, and the scoped put path serving `knowledge.put`; the provider port is the atomic backstop.
- **`source_anchor` is separate provenance.** On the destination entry, `source_anchor` is an independent provenance field and is cleared on the adopted copy. Product-specific source-anchor codecs are not adoption encoding; adoption provenance lives only in `adoption_anchor`.
- **Destination governance wins.** Destination `owner` and `disclosure` values govern the new copy; source governance values are never inherited. `modules.cognition` never crosses.
- **Absence is generic.** A requested source that fails to load, a resolved terminal outside the admitted scope, and an entry owner-private to another viewpoint produce the same `NOT_FOUND` with no source-identifying details and no destination write. Repeated calls produce the same absence.
- **Admission precedes resolution.** Destination `space_id` authorization is checked before source-chain resolution begins; an out-of-scope destination refuses `NOT_AUTHORIZED` without touching the source.
- **Reserved prefix.** `knowledge.adopt` is the only registered `knowledge.*` operation. Every other `knowledge.*` string is refused with `op_unsupported` before any product custom-capability map is consulted; `ConnectConfig.op_capability_requirements` and composed-responder equivalents cannot register or authorize an unknown reserved name.
- **Payload scope ids are selectors.** The caller must be admitted to both the requested source and the destination scope through the host's admission context; request scope ids are validated against that admission and are never authority.
- **Success values validate at the boundary.** Provider success values are validated before emission; a malformed provider result answers an `INTERNAL_ERROR` transport refusal, never `Ok`.

## Related

- [Protocol reference](/reference/protocol) — capability flags and the schema inventory.
- [Data model reference](/reference/data-model) — `KnowledgeEntry` fields, including `adoption_anchor`.
- [Ops wire reference](/reference/ops) — request/response envelopes, `Scope`, and `ErrorEnvelope`.
- [Connect reference](/reference/connect) — reserved dispatch, capability negotiation, and the invoke envelopes.
- [History reference](/reference/history) — the sibling optional family with the same principal-first port shape.
