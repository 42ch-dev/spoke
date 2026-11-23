---
title: History reference
---

# History reference (`spoke-history`)

`spoke-history` is the optional capability for exact-revision replay of stored entities and explicit pruning of retained checkpoints. It covers `KnowledgeEntry` and `Relation` data; checkpoint storage and reconstruction stay provider-owned.

## Capability

| Aspect | Contract |
|--------|----------|
| Capability string | `spoke-history` — an optional member of `HostCapabilityManifest.capabilities` |
| Manifest declaration | The host serves the revision-snapshot and history operation contract |
| Dependency | No co-capability; `spoke-baseline` and `spoke-knowledge` neither imply nor replace it |
| Negotiation | Both Connect peers advertise it for the string to enter `negotiated_capabilities`; dispatch requires exact membership in that negotiated set |
| Connect surface | Gates `history.revisions.get` and `history.revisions.prune` |

`capabilities` is an open string array with unique members, so declaring `spoke-history` is additive to the existing capability vocabulary.

## Operations

| `op` | Request | Success value | Idempotency |
|------|---------|---------------|-------------|
| `history.revisions.get` | `HistoryRevisionsGetRequest` — `entity_kind`, `entity_id`, `revision`; optional `scope`, `extensions` | `RevisionSnapshot` | Read-only; no idempotency key |
| `history.revisions.prune` | `HistoryRevisionsPruneRequest` — `entity_kind`, `entity_id`, `keep`, `idempotency_key`; optional `scope`, `extensions` | `HistoryPruneResult` with `deleted` and `floor_revision` | Required |

Both operations travel the existing opaque Connect payload and response envelopes. `idempotency_key` contains 1–128 Unicode code points; the length is counted in code points, not UTF-16 code units or UTF-8 bytes.

### `RevisionSnapshot`

A closed object with these required fields:

| Field | Type | Contract |
|-------|------|----------|
| `schema_version` | shared `SchemaVersion` | Version of the `RevisionSnapshot` wire schema |
| `entity_kind` | string | Entity-kind selector; core values are `knowledge_entry` and `relation` |
| `entity_id` | string | Opaque stable entity id; equals `snapshot.entry_id` for a knowledge entry and `snapshot.relation_id` for a relation |
| `revision` | integer ≥ 0 | Revision represented by the snapshot; equals the selected entity snapshot's own `revision` |
| `snapshot` | `oneOf`: `KnowledgeEntry` \| `Relation` | The stored entity as it stood at `revision`, with its stored fields intact |
| `extensions` | shared `ExtensionMap` | Product-namespace extensions; unknown namespaces and keys round-trip unchanged |

The `snapshot` variant agrees with `entity_kind`. A knowledge entry carries its stored `body`, `modules`, `owner`, `disclosure`, `source_anchor`, `status`, and `extensions` when present; a relation carries its existing relation envelope. Two providers returning the same snapshot at the same revision produce identical bytes under RFC 8785 canonical JSON.

### `HistoryPruneResult`

| Field | Type | Contract |
|-------|------|----------|
| `deleted` | integer ≥ 0 | Number of checkpoints deleted by the call; excludes the live row and derived revisions |
| `floor_revision` | integer ≥ 0 \| null | Current retained floor: the newest deleted checkpoint revision plus one, or `null` when nothing was deleted |

### Retained floor

A checkpoint is a retained preimage used to reconstruct historical revisions. The live row is not a checkpoint and never consumes `keep`.

For one `(scope, entity_kind, entity_id)` target, `prune_history(..., keep)` retains the newest `keep` checkpoints and deletes every older checkpoint:

- With checkpoints `{1, 3, 5}` and `keep = 2`, the retained set is `{3, 5}` and the deleted set is `{1}`, so `deleted = 1` and `floor_revision = 2`.
- A replay of revision 1 is below the floor and refuses `HISTORY_TRUNCATED` with `details.floor_revision = 2`.
- A replay of revision 2 is at the floor and is answered from retained checkpoint 3.
- A prune that deletes nothing returns `{ "deleted": 0, "floor_revision": null }`.

Replay answers a revision at or above the floor from the nearest retained checkpoint at or above the requested revision. A revision at or above the floor that cannot be reconstructed because of an implementation-declared retention gap refuses `HISTORY_NOT_RETAINED`.

### `HistoryPort`

`HistoryPort` is standalone and composes separately from `KnowledgeEntryPort` and `RelationPort`. The asynchronous signatures are:

```text
get_history_snapshot(principal, scope, entity_kind, entity_id, revision)
  -> SpokeResult<RevisionSnapshot>

prune_history(principal, scope, entity_kind, entity_id, keep)
  -> SpokeResult<HistoryPruneResult>
```

Every method takes the authenticated `Principal` as its first admission-context parameter and the required `Scope` second, so entity read and write authorization bind to the caller rather than to payload-asserted identity. TypeScript uses camelCase names and `Promise<SpokeResult<T>>`; Rust uses `#[async_trait] async fn` with the snake_case names above. Expected outcomes are returned as `SpokeResult`, not thrown.

The Connect serving face is `RemoteHistoryService` over a provider `HistoryPort` via `HistoryPortService`, mirroring `RemoteMemoryService` / `MemoryPortService`. The service owns peer admission, effective-scope resolution, and prune deduplication.

## Refusal vocabulary

| Code | Condition | Required `details` |
|------|-----------|--------------------|
| `HISTORY_TRUNCATED` | The requested revision is below the current retained floor and its checkpoint history was discarded | `{ "floor_revision": integer ≥ 0 }` |
| `HISTORY_ENTITY_UNSUPPORTED` | Connect admitted the operation but the host does not serve the requested `entity_kind` | `{ "entity_kind": string }` with the exact requested kind |
| `HISTORY_NOT_RETAINED` | The requested revision is at or above the floor but cannot be reconstructed because of an implementation-declared retention gap | `{ "retention_reason": string }` — provider-supplied, human-readable; the stable code remains authoritative |
| `op_unsupported` / `capability_missing` | Connect dispatch denies the row because `spoke-history` is absent from the negotiated set (no capability row, or row present but not negotiated) | None; the refusal precedes the handler with no side effects |
| `INVALID_INPUT` | Malformed request; an absent `idempotency_key` on prune; a `revision` or `keep` value outside the receiving runtime's exact-integer range | Field-level detail as for any malformed request |
| `NOT_FOUND` | The entity is missing or non-disclosable, or the request `scope` differs from the peer's admitted scope on `history.revisions.get` | None beyond the shared envelope |
| Entity authorization outcome | A prune caller lacks the selected entity's write permission, or the request `scope` differs from the admitted scope on prune | The entity's existing write-authorization refusal |
| `STORED_REVISION_STALE` | A conditional entity put targets a revision below the floor and the caller's `expected_revision` lags the stored revision | The entity's existing stale-revision details |
| `IDEMPOTENCY_CONFLICT` | A prune retry reuses the same key with a different payload digest | None |
| `RECEIPT_EXPIRED` | A prune retry replays a key whose recorded outcome was evicted | None |

`HISTORY_TRUNCATED`, `HISTORY_ENTITY_UNSUPPORTED`, and `HISTORY_NOT_RETAINED` are new `SpokeRejectCode` members; the remaining codes keep their established meanings.

## Usage constraints

- **Replay is entity data.** A snapshot returns the stored entity exactly as it stood at `revision`; replay does not re-derive cognition or other derived state. A stored `modules.cognition` value travels as entity data, and derived views such as `MindState` keep their own owner.
- **Byte presence survives.** Present-and-empty stored members, including `body.tags: []`, `body.state: {}`, `modules: {}`, `body.attributes: []`, and map-shaped `body.computable`, survive replay and RFC 8785 canonicalization with their member names and empty values byte-identical.
- **Entity kinds are open.** Core values are `knowledge_entry` and `relation`. A provider serving an additional kind declares the core shape (`KnowledgeEntry` or `Relation`) under which it encodes the stored entity and uses that `snapshot` branch consistently; otherwise it answers `HISTORY_ENTITY_UNSUPPORTED`. An unknown kind is never routed to a core entity port.
- **Pruning is explicit.** `history.revisions.prune` is the only path that deletes checkpoints; no write path prunes implicitly. `keep` counts retained checkpoints and excludes the live row.
- **Prune uses entity write permission.** Pruning requires exactly the same permission as a put to the selected entity in the same scope. Reads follow the selected entity's read and disclosure rules. There is no separate `history_admin` grant and no history-specific authorization code.
- **Idempotency is connect-owned.** The service records `{payload_digest, outcome}` under `(effective_scope, requester_principal_id, op, idempotency_key)`, where `payload_digest` is SHA-256 over RFC 8785 canonical JSON of the request payload. Same-key, same-digest replay returns the recorded outcome without re-executing the port call; a recorded response-validation failure replays as that error; a different digest answers `IDEMPOTENCY_CONFLICT`; an evicted record answers `RECEIPT_EXPIRED` and never re-executes. Dedup records outlive a Connect session, and a tombstone from an evicted record is retained for the store's lifetime.
- **OCC is unchanged.** A conditional entity put below the floor keeps the entity's existing optimistic-concurrency behavior; history adds no parallel write-conflict refusal and the write path does not consult history floor state.
- **Integer range is exact.** `revision` and `keep` have `u64` semantics with no schema maximum. A runtime that cannot represent a received value exactly refuses the request with `INVALID_INPUT` at decode and never rounds, truncates, or coerces it.
- **Scope is admitted, not asserted.** The request `scope` is optional; when present it equals the admitted scope. A mismatch answers `NOT_FOUND` for get and the selected entity's write-authorization denial for prune, without invoking the port.
- **Success values validate at both decode boundaries.** The serving face validates provider output before emitting success and each `RemoteAdapter` validates the response before returning `Ok`; a malformed success value produces a transport decode error, never `Ok`.

## Related

- [Protocol reference](/reference/protocol) — capability flags and the schema inventory.
- [Ops wire reference](/reference/ops) — request/response envelopes, `Scope`, and `ErrorEnvelope`.
- [Connect reference](/reference/connect) — the envelopes that carry `history.*` payloads and the dispatch gate.
- [Data model reference](/reference/data-model) — `KnowledgeEntry`, `Relation`, and `ExtensionMap` field tables.
