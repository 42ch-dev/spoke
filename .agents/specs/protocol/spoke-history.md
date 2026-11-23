# SPOKE History

> **Status:** Normative (`spoke-history` optional capability)  
> **Document class:** Detail — revision replay and retention capability layer  
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)  
> **Schema home:** `schemas/data/revision-snapshot.schema.json` and `schemas/ops/history-revisions-*.schema.json`

## Scope

The `spoke-history` optional capability: exact-revision replay through `RevisionSnapshot`, explicit
checkpoint pruning through `HistoryPruneResult`, the `HistoryPort` composition boundary, the
`history.revisions.get` and `history.revisions.prune` Connect operations, and their admission,
authorization, retention, idempotency, and refusal contracts.

Core entity kinds are `knowledge_entry` and `relation`. The capability does not define a storage
engine, checkpoint format, reconstruction algorithm, cognition derivation, or implicit retention
policy. Connect request/response envelopes and general dispatch rules remain owned by
[`spoke-ops.md`](spoke-ops.md).

## Current state

### Capability and composition

A host serving revision snapshots and history operations advertises `spoke-history` in
`HostCapabilityManifest.capabilities`. Both Connect peers must advertise it for it to enter
`negotiated_capabilities`; dispatch checks exact membership in that set. Neither `spoke-baseline`
nor `spoke-knowledge` implies or replaces it.

`HistoryPort` is standalone and composed separately from `KnowledgeEntryPort` and `RelationPort`.
Its asynchronous methods are `get_history_snapshot` and `prune_history` (TypeScript
`getHistorySnapshot` and `pruneHistory`). Each receives the authenticated `Principal` first and the
required effective `Scope` second, followed by `entity_kind` and `entity_id`, then `revision` or
`keep`. Expected outcomes use `SpokeResult`; the port has no idempotency-key argument.

`RemoteHistoryService` is the Connect serving face and `HistoryPortService` adapts a provider port
to it. The service owns peer admission, effective-scope resolution, and prune deduplication. An
optional request `scope` must equal the admitted scope; an omitted scope selects the admitted
scope. The port receives the authenticated principal and effective scope, never payload-asserted
identity or scope.

### `RevisionSnapshot`

`RevisionSnapshot` is a closed object with required members:

| Field | Type | Contract |
|---|---|---|
| `schema_version` | shared `SchemaVersion` | Version of the snapshot wire schema. |
| `entity_kind` | non-empty open string | Selects the stored entity shape. Core values are `knowledge_entry` and `relation`. |
| `entity_id` | non-empty string | Opaque stable id within scope; equals the nested entry or relation id. |
| `revision` | integer `>= 0` | Revision represented by this snapshot; equals the nested entity revision. |
| `snapshot` | `KnowledgeEntry` or `Relation` | Complete stored entity at that revision, with no stored fields projected away. |
| `extensions` | shared `ExtensionMap` | Product namespaces and unknown keys round-trip unchanged. |

The `snapshot` branch must agree with `entity_kind`. Products may add kind selectors, but must
encode each served kind consistently as one of the two core shapes; the protocol defines no third
snapshot body. An unsupported kind is never routed approximately to a core entity port.

Replay returns stored entity data without re-deriving cognition or other derived state. A stored
`modules.cognition` value remains entity data, not a replayed `MindState`. Stored fields, including
present empty arrays, objects, and maps, retain their member presence and values through replay and
RFC 8785 canonicalization. Two providers returning the same snapshot at the same revision produce
identical RFC 8785 canonical bytes.

### Connect operations and gates

| Operation | Request members | Success value | Idempotency |
|---|---|---|---|
| `history.revisions.get` | Required `entity_kind`, `entity_id`, `revision`; optional `scope`, `extensions` | A response `snapshot` containing `RevisionSnapshot` | Read-only; no idempotency key. |
| `history.revisions.prune` | Required `entity_kind`, `entity_id`, `keep`, `idempotency_key`; optional `scope`, `extensions` | `HistoryPruneResult` with required `deleted` and nullable `floor_revision` | Required. |

Both operations require negotiated `spoke-history`. Without it, Connect refuses with
`op_unsupported` or `capability_missing` before invoking the handler and without side effects.
`HISTORY_ENTITY_UNSUPPORTED` applies only after that gate admits the operation.

`revision` and `keep` are non-negative integers with no schema maximum. A receiver that cannot
represent either value exactly refuses with `INVALID_INPUT`; it never rounds, truncates, or coerces
it. The prune `idempotency_key` contains 1–128 Unicode code points inclusive, not UTF-16 code units
or UTF-8 bytes.

Both Connect ends validate success values. Get responses must contain a snapshot whose envelope
selectors match the request and whose nested entity id and revision satisfy the snapshot equality
rules. Prune responses must contain an integer `deleted >= 0` and a present, nullable
`floor_revision`. A malformed success value is a transport decode error, never `Ok`.

### Replay and retained floor

A checkpoint is a retained preimage used to reconstruct historical revisions. The live row is not
a checkpoint. For one `(scope, entity_kind, entity_id)` target, let `C` be its checkpoints ordered
oldest to newest. Replay returns the stored entity at the requested revision, including revisions
without checkpoints of their own when a retained checkpoint can reconstruct them.

A requested revision below the current retained floor is truncated. A revision at or above the
floor is not truncated merely because it has no checkpoint: when reconstructable, it is answered
from the nearest retained checkpoint at or above that revision. An implementation-declared
retention gap at or above the floor uses `HISTORY_NOT_RETAINED` instead.

For `prune_history(..., keep)`, retain the newest `keep` members of `C` and delete every older
checkpoint. `deleted` counts only checkpoints deleted by that call, excluding the live row and
derived revisions. The returned floor is:

- `floor_revision = newest_deleted_checkpoint_revision + 1` when at least one checkpoint is deleted;
- `floor_revision = null` and `deleted = 0` when none is deleted, including `keep >= |C|`.

For checkpoints `{1,3,5}` and `keep=2`, the live row is excluded, the retained checkpoints are
`{3,5}`, and the deleted set is `{1}`. Therefore `deleted=1` and `floor_revision=1+1=2`. Revision 1
returns `HISTORY_TRUNCATED` with `details.floor_revision=2`; revision 2 remains answerable from
retained checkpoint 3 and is not truncated.

A conditional entity `put` below the floor retains the entity's existing optimistic-concurrency
outcome. History does not add a write-floor check or a parallel stale-revision refusal.

### Authorization and pruning

History reads follow the selected entity's existing read and disclosure rules. Pruning requires
exactly the same permission as `put` to the selected entity in the same scope. There is no
`history_admin` grant or history-specific authorization refusal. A request scope different from the
admitted scope produces `NOT_FOUND` for get and the entity's existing write-authorization denial
for prune, without invoking the port.

Pruning occurs only through the explicit prune operation. No entity write path prunes implicitly.
Every authorized entity writer may explicitly prune that entity's checkpoints.

### Prune idempotency

Prune uses the existing `memory.*` idempotency model. The deduplication key is
`(effective_scope, requester_principal_id, op, idempotency_key)`, and the payload digest is SHA-256
over RFC 8785 canonical JSON of the request payload object.

A same-namespace, same-key, same-digest replay returns the recorded outcome without re-executing
the port or re-evaluating its preconditions. This includes a recorded transport decode error caused
by malformed provider success output. A same-key request with a different digest returns
`IDEMPOTENCY_CONFLICT`; replay of an evicted recorded outcome returns `RECEIPT_EXPIRED` and never
re-executes. An absent wire `idempotency_key` returns `INVALID_INPUT`.

The deduplication store is host-injectable for durable persistence. Entries outlive a Connect
session and retain their principal-bound namespace across reconnects. Capacity eviction removes
records but leaves tombstones for the store lifetime, so an evicted key cannot become unseen.

### Refusal vocabulary

| Code | Condition | Required `details` contract |
|---|---|---|
| `HISTORY_TRUNCATED` | The requested revision is below the current retained floor because its checkpoint history was discarded. | `floor_revision`: integer `>= 0`, containing the current retained floor. |
| `HISTORY_ENTITY_UNSUPPORTED` | The capability gate admitted the operation, but the host does not serve the requested kind. | `entity_kind`: string containing the exact requested kind. |
| `HISTORY_NOT_RETAINED` | The requested revision is at or above the floor but cannot be reconstructed because of a declared retention gap. | `retention_reason`: provider-supplied human-readable string; the stable code remains authoritative. |
| `NOT_FOUND` | The selected entity is missing or is not disclosable to the caller; a get scope mismatch also uses this absence rule. | The existing absence contract applies; history does not reveal existence or retained history through additional details. |

Malformed requests use `INVALID_INPUT`; denied reads and writes use the selected entity's existing
authorization outcomes. Prune additionally uses `IDEMPOTENCY_CONFLICT` and `RECEIPT_EXPIRED` under
the deduplication rules above. Capability denials remain Connect-layer `op_unsupported` or
`capability_missing`, distinct from post-admission history refusals.

## Invariants

1. `spoke-history` is optional and independently negotiated; only exact capability membership gates
   either history operation, and denied dispatch has no handler side effects.
2. `RevisionSnapshot` preserves the complete selected entity, matching envelope and nested ids and
   revisions, stored field presence, extensions, and RFC 8785 canonical bytes.
3. Replay never re-derives cognition or other derived state. Derived revisions at or above the
   retained floor remain answerable when retained checkpoints can reconstruct them.
4. The live row never counts as a checkpoint or consumes `keep`; `deleted` counts deleted
   checkpoints only, never derived revisions or the live row.
5. Pruning is explicit only. The retained floor is one plus the newest deleted checkpoint revision,
   or `null` when no checkpoint was deleted.
6. Prune authorization is exactly the selected entity's write permission in the effective scope;
   history adds no separate grant or authorization code.
7. Missing and non-disclosable entities retain generic `NOT_FOUND` absence behavior. Unsupported
   kinds, truncation, and provider retention gaps remain distinguishable through their stable codes
   and details.
8. Prune deduplication is bound to effective scope, requester principal, operation, and key; replay
   never silently re-executes, including after outcome eviction.
9. A runtime never rounds or truncates an inexact `revision` or `keep`, and malformed success values
   never cross either Connect decode boundary as `Ok`.

## Where it lives

| Artifact | Path |
|---|---|
| Snapshot wire schema | `schemas/data/revision-snapshot.schema.json` |
| Operation wire schemas | `schemas/ops/history-revisions-get-request.schema.json`, `schemas/ops/history-revisions-get-response.schema.json`, `schemas/ops/history-revisions-prune-request.schema.json`, `schemas/ops/history-revisions-prune-response.schema.json` |
| Rust port contract | `crates/spoke-operations/src/history.rs` |
| TypeScript port contract and tests | `packages/spoke-operations/src/history.ts`, `packages/spoke-operations/src/history.test.ts` |
| Connect serving and deduplication | `crates/spoke-connect/src/remote/serve_ports.rs` |
| Connect conformance anchors | `crates/spoke-connect/tests/history_conformance.rs` |
| Provider-neutral vectors | `fixtures/history-conformance/` |
| Decision record | [`2026-11-22-spoke-history.md`](../../adr/implemented/architecture/2026-11-22-spoke-history.md) |

## Change obligations

- Change the snapshot or operation schemas → regenerate both type packages and update this page in
  the same change.
- Change replay, checkpoint, floor, authorization, idempotency, or refusal behavior → update this
  page, the affected implementations, and the conformance vectors together.
- Change capability negotiation or Connect dispatch gates → update § Capability and composition and
  § Connect operations and gates here.
- Add a spec or ADR that references history replay or pruning → link this page as the normative
  source; do not restate its wire and retention contracts elsewhere.
