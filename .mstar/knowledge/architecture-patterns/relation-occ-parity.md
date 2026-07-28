# Relation OCC parity

> **Category:** architecture-patterns  
> **Source:** compound 2026-07-29 (relation OCC port)  
> **Packages:** `@42ch/spoke-operations` (npm), `spoke-operations` (crates.io), `spoke-fixture-toy-world` (reference adapter)

## Problem

`KnowledgeEntry` persistence is guarded by optimistic concurrency control (OCC): the orchestrator loads the stored revision, validates, then issues a compare-and-put so concurrent writers cannot silently overwrite each other. `Relation` persistence needs the same guarantee — relations are mutable graph edges whose `revision` must advance on every accepted write. Without OCC parity, the `relate` op would blind-put and lose concurrent updates, diverging from the `upsert`/`promote` lifecycle.

## Pattern

`RelationPort` mirrors `KnowledgeEntryPort` exactly — a `get` reader plus an OCC `put`:

```ts
interface RelationPort {
  getRelation(relationId: string): SpokeResult<Relation>;
  putRelation(
    relation: Relation,
    expectedBaseRevision: number | null,
  ): SpokeResult<Relation>;
}
```

```rust
trait RelationPort {
    fn get_relation(&self, relation_id: &str) -> SpokeResult<Relation>;
    fn put_relation(
        &self,
        relation: Relation,
        expected_base_revision: Option<u64>,
    ) -> SpokeResult<Relation>;
}
```

`orchestrateRelate` / `orchestrate_relate` run the same sequence as `orchestrateUpsert` / `orchestrate_upsert`:

1. **Load** — `getRelation(relation_id)`; a `RELATION_NOT_FOUND` reject collapses to “no stored entity” (create path).
2. **Validate** — `validateRelateRequest` / `validate_relate_request` infers create vs update from stored presence:
   - create: `revision` must be absent, `0`, or a non-negative integer; `revision ≥ 1` is rejected as `INVALID_INPUT`.
   - update: `revision` is required (`MISSING_REQUIRED_FIELD`), must be a non-negative integer, and must match the stored revision via `assertRevisionMatch`.
3. **OCC** — the orchestrator passes `expectedBaseRevision = stored.revision ?? 0` on update, or `null`/`None` on create.
4. **Put** — the adapter enforces compare-and-put and owns **revision assignment**: seed `1` on create, bump `current + 1` on accepted update. The persisted (returned) relation carries the assigned revision.

## Error codes

| Condition | Code |
|-----------|------|
| Create finds id already present | `RELATION_ALREADY_EXISTS` |
| Update finds no stored relation | `STORED_REVISION_STALE` |
| Update revision does not match stored | `STORED_REVISION_STALE` (store ahead) / `REVISION_CONFLICT` (candidate ahead) |
| Update omits `revision` | `MISSING_REQUIRED_FIELD` |
| Create carries `revision ≥ 1` | `INVALID_INPUT` |
| `from_id`/`to_id` empty or whitespace | `RELATION_MISSING_ENDPOINT` |
| `from_id == to_id` | `RELATION_SELF_EDGE` |
| No stored relation on read | `RELATION_NOT_FOUND` |

`RELATION_NOT_FOUND`, `RELATION_ALREADY_EXISTS`, `REVISION_CONFLICT`, and `STORED_REVISION_STALE` round-trip through the error envelope (`toErrorEnvelope` / `fromErrorEnvelope`) like every other `SpokeRejectCode`.

## Dual-language parity

TypeScript (`@42ch/spoke-operations`) and Rust (`spoke-operations`) expose the same `RelationPort` shape, the same `validateRelateRequest` / `validate_relate_request` rules, and the same `orchestrateRelate` / `orchestrate_relate` load → validate → OCC → put sequence. The reference `ToyWorldAdapter` (`fixtures/toy-world/`) implements the OCC contract identically in both languages: the in-memory store rejects create-when-exists with `RELATION_ALREADY_EXISTS`, enforces revision match on update, and assigns `revision` (seed 1 / bump +1).

## Gotchas

- **Revision ownership differs from KnowledgeEntry.** The Relation adapter assigns `revision` (seed on create, bump on update) and returns the relation with the assigned revision; the orchestrator passes the candidate as-is. `KnowledgeEntry` upsert stores the caller-supplied revision; only `orchestratePromote` bumps it before put. Consumers must not assume the two ports share the same revision-assignment rule.
- **Create stays permissive.** `revision` is optional or `0` on create so first-write callers and fixtures without a revision still pass. The `revision ≥ 1` guard only rejects pre-seeded revisions on first write.
- **Concurrent safety is adapter-owned.** The library performs no storage I/O; true CAS requires an atomic compare-and-put in the adapter (database conditional update, lock, or single-writer). The in-memory toy-world store is a reference, not a concurrency primitive.
- **0.x breaking port change.** `putRelation(relation)` became `putRelation(relation, expectedBaseRevision)` and `getRelation` is now required. Every in-repo implementer updated; external adopters add `null`/`None` on create and the stored revision on update. No dual-signature shim.

## See also

- [`adapter-injection-orchestration.md`](adapter-injection-orchestration.md) — ports, capability composition, injection orchestration, persisted-entity OCC note
- [`spoke-operations-pure-actions.md`](spoke-operations-pure-actions.md) — pure helper families (validate / OCC / scope)
- [`rust-spoke-operations-parity.md`](rust-spoke-operations-parity.md) — Rust crate layout and reject-code parity
- `.mstar/specs/spoke-operations.md` — Adapter Interfaces + Injection Orchestration

## Adopter path

1. Implement `RelationPort.getRelation` (read) and `putRelation(relation, expectedBaseRevision)` (compare-and-put) on the adapter.
2. On create (`expectedBaseRevision` is `null`/`None`): reject `RELATION_ALREADY_EXISTS` if the id exists; otherwise persist with `revision = 1`.
3. On update (non-null): require the stored revision to match; reject `STORED_REVISION_STALE` on mismatch; persist with `revision = current + 1`.
4. Return the persisted relation (with the assigned revision) so `orchestrateRelate` can surface it.
5. Copy the OCC store/revision pattern from `fixtures/toy-world/` (`MemoryStore.putRelation` TS + `MemoryStore::put_relation` Rust).
