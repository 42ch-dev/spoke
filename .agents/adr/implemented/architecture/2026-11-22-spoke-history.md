# ADR: `spoke-history` revision replay and pruning

Status: implemented — history uses a standalone port, open entity kinds, checkpoint-derived floors, entity write authorization, and stable refusal semantics

Date: 2026-11-22

## Problem

KnowledgeEntry and Relation revisions give consumers optimistic-concurrency identity, while historical access also requires a portable snapshot, bounded checkpoint retention, and replay between retained checkpoints. A history contract must preserve the complete stored entity and distinguish discarded history from an implementation-specific retention gap.

The composition choices affect every host. Coupling history to an entity port would make an optional retention capability part of baseline entity storage. A closed entity-kind enum would make each product-defined kind a protocol revision. An ambiguous floor or refusal vocabulary would make providers disagree about revisions that remain reconstructable and would leave consumers unable to select a recovery path.

## Decision

`spoke-history` is an optional capability for exact-revision snapshots and explicit checkpoint pruning. `RevisionSnapshot` carries the selected KnowledgeEntry or Relation with its stored fields and extensions intact. Replay returns entity data as stored; cognition and other derived state retain their existing owners.

`HistoryPort` is standalone and composed separately from `KnowledgeEntryPort` and `RelationPort`. Its two asynchronous operations are `get_history_snapshot(scope, entity_kind, entity_id, revision)` and `prune_history(scope, entity_kind, entity_id, keep)`. Hosts advertise and compose `spoke-history` independently of `spoke-baseline` and `spoke-knowledge`.

`entity_kind` is an open string with core values `knowledge_entry` and `relation`. Products can add kinds while retaining the same capability and snapshot envelope. After Connect admits an operation through negotiated `spoke-history`, a host that does not serve the requested kind returns `HISTORY_ENTITY_UNSUPPORTED` with the exact value in `details.entity_kind`.

Pruning is explicit. `keep` counts retained checkpoints and excludes the live row. The operation retains the newest `keep` checkpoints and deletes older checkpoints. When deletion occurs, `floor_revision` equals one plus the newest deleted checkpoint revision. A non-deleting prune returns `deleted = 0` and `floor_revision = null`.

Prune authorization is exactly the selected entity's write permission in the same scope. History reads use the selected entity's read and disclosure rules. The authorization surface therefore remains the entity's existing permission model, with no separate `history_admin` grant.

## Retention and replay rationale

A checkpoint is a retained preimage; the live row is outside the checkpoint collection. The floor follows the newest discarded preimage rather than the oldest surviving checkpoint because a later checkpoint can reconstruct an earlier revision between checkpoints. This preserves every revision that the retained data can still answer.

For checkpoints `{1,3,5}` and `keep = 2`, the retained set is `{3,5}`, the deleted set is `{1}`, and the floor is `2`. Revision 1 is truncated, while revision 2 remains answerable from checkpoint 3. Setting the floor to 3 would incorrectly discard a reconstructable revision; counting the live row would retain one fewer checkpoint than the caller requested.

A revision below the floor returns `HISTORY_TRUNCATED` with `details.floor_revision`. A revision at or above the floor that an implementation cannot reconstruct because of a declared retention gap returns `HISTORY_NOT_RETAINED` with `details.retention_reason`. These outcomes distinguish protocol retention from provider-specific reconstruction limits.

## Authorization and refusal rationale

Pruning and entity writes are mutations of the same scoped entity, so entity write permission supplies one coherent grant boundary. This choice gives every authorized entity writer explicit control over that entity's retained checkpoints and avoids a second permission to negotiate, delegate, and audit.

The history-specific codes preserve three distinct consumer actions:

| Code | Meaning preserved by a distinct code |
|---|---|
| `HISTORY_TRUNCATED` | Retention has discarded the requested below-floor history; `floor_revision` identifies the current boundary. |
| `HISTORY_ENTITY_UNSUPPORTED` | Connect admitted the capability and the host lacks the requested entity kind; `entity_kind` identifies the unsupported selector. |
| `HISTORY_NOT_RETAINED` | The revision is above the truncation boundary but unavailable for a provider-declared retention reason. |

Existing codes retain their established purposes: `NOT_FOUND` covers missing or non-disclosable entities, existing entity authorization outcomes cover denied reads and writes, `REVISION_CONFLICT` covers conditional entity writes, and `INVALID_INPUT` covers malformed requests. Reusing those codes for the three history outcomes would conflate entity existence, authorization, optimistic concurrency, request validity, capability admission, and retention.

## Alternatives considered

- **A floor at the oldest retained checkpoint, or a `keep` count that includes the live row:** these formulations either reject derived revisions that retained checkpoints can answer or make retention counts depend on an object outside the checkpoint collection. The newest-deleted-plus-one formula preserves replay reachability and gives `keep` one unit.
- **A closed `entity_kind` enum:** every additional product kind would require a shared schema change and closed-enum upgrades. Open strings keep extension additive, while `HISTORY_ENTITY_UNSUPPORTED` gives unsupported selectors an explicit outcome instead of routing them to a core entity port.
- **History methods inherited from `KnowledgeEntryPort` or `RelationPort`, or supplied by baseline composition:** inheritance would expand every entity provider with an optional history obligation and duplicate the same contract across entity ports. A standalone port gives knowledge-only, relation-serving, and baseline hosts the same opt-in composition boundary.
- **A dedicated `history_admin` grant:** a second grant would add permission and negotiation surface without expressing a distinct product requirement. Reusing entity write permission keeps mutation authority in one place; its cost is that every entity writer is also authorized to prune that entity's history.
- **Existing generic refusals for all history failures:** `NOT_FOUND`, `REVISION_CONFLICT`, or `INVALID_INPUT` would misstate the cause and hide the recovery information carried by the floor, entity kind, and retention reason. Dedicated codes preserve those semantics while Connect continues to handle capability admission before history dispatch.

## Consequences

- Providers own checkpoint storage and reconstruction behind one standalone port, and consumers can depend on the same snapshot and operation contracts across entity kinds.
- Products can introduce entity kinds without changing the core envelope. Every host must preserve exact-kind dispatch and return the unsupported-kind refusal rather than approximate a match.
- Checkpoint retention has a deterministic cross-provider meaning. Derived revisions at or above the floor remain eligible, and a provider must distinguish a declared reconstruction gap from truncation.
- Every authorized entity writer can explicitly prune that entity's history. The protocol applies the same existing write-authorization outcome to denied prune calls and adds no parallel authorization vocabulary.
- Consumers can choose recovery from stable codes and structured details: adjust the requested revision using `floor_revision`, select a supported entity kind, or surface the provider's `retention_reason`.
- Entity writes retain their existing optimistic-concurrency behavior. History replay and pruning do not create a second write-conflict mechanism, and checkpoint deletion occurs only through the explicit prune operation.
