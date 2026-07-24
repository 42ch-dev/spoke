# Optional l5-fork TimelineEvent wire

## Problem

Products that fork immutable world-history branches need interoperable branch metadata on the L5 when-axis. Without protocol fields, adapters invent private shapes or stash hints in `extensions` (for example `fork_hint`), which does not interoperate. Baseline consumers must remain able to omit Fork entirely.

## Decision

| Concern | Wire / rule |
|---------|-------------|
| Capability flag | Optional **`l5-fork`** (not required for `spoke-baseline`) |
| Shared type | `ForkId` — `string`, `minLength: 1` — `common.schema.json#/definitions/ForkId` |
| Branch identity | Optional `TimelineEvent.fork_id` → `ForkId` |
| Lineage | Optional `TimelineEvent.parent_fork_id` → `ForkId` (parent/base branch); when present, `fork_id` SHOULD be present; MUST NOT equal `fork_id` (prose, not schema-enforced) |
| Scope filter | Optional `Scope.fork_id` — strict `timelineEvent.fork_id === scope.fork_id`; events **without** `fork_id` do **not** match |
| Top-level Fork entity | **None** — optional fields on TimelineEvent only |
| Schema-count | In-place edits; no new top-level schema files |
| Engines | Product-owned (merge, rebase, world-history store); SPOKE owns wire + pure Scope match helpers |

## Orthogonality

| Concept | Relation to Fork |
|---------|------------------|
| `timeline_scale` (`brief` / `narrative` / `moment`) | Projection **tier** — not branch identity |
| Domain Profile / open vocabulary | Adapters MUST NOT fork core schemas for profile types |
| Session / `l2-computable` | Orthogonal lifecycle; not branch identity |
| Finding | Checker output — not Fork metadata |
| `extensions.*.fork_hint` | Product folklore — not the interoperable `l5-fork` contract |

## Ops helper

`timelineEventMatchesScope` / `filterTimelineEventsByScope` honor `Scope.fork_id` with the semantics above. `parent_fork_id` is never used as a Scope filter.

## Dual-concern

KnowledgeEntry ontology labels (including `entry_type: "event"`) remain distinct from TimelineEvent. Fork fields live only on TimelineEvent / Scope under `l5-fork`.

## See also

- Normative: `.mstar/specs/spoke-data-model.md` §Fork, `spoke-protocol-layers.md` capability `l5-fork`, `spoke-ops.md` Scope
- Related: [`timeline-projection-tiers.md`](timeline-projection-tiers.md), [`l2-computable-session-model.md`](l2-computable-session-model.md)
