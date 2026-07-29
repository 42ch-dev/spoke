# Mira at Harbor — protocol toy-world fixtures

Protocol-owned JSON graph, AJV/Vitest conformance harness, and reference **`ToyWorldAdapter`** examples (TypeScript and Rust) for integrators validating parsers, codegen, and operations port orchestration against committed `schemas/`.

**Story:** Cartographer Mira arrives at Harbor Town at dawn; a consistency rule flags an open finding; an AssemblePacket scopes context for the scene. A dual-concern pair links ontology `entry_type: "event"` KnowledgeEntry `kb_tw_harbor_dawn_event` to TimelineEvent `evt_tw_harbor_dawn`. Three additional moment-scale beats extend the Harbor mainline in order — market square inquiry, customs gate inspection (profile `entry_type: "beat"`), berth confirmation — linked by `precedes` Relations on dual KE ids and `extensions.spoke.timeline_entry_id` on each TimelineEvent. Harbor Town carries optional l2-computable `body.state` / `body.computable` (tide and cargo); the dawn moment records `computable_logs` for those field changes.

## Host capability manifests (in-process collaboration)

Committed `HostCapabilityManifest` JSON describes two toy-world hosts with **pairwise disjoint** `namespaces[]`:

| Fixture | `host_id` | Owned namespaces | Roles (summary) |
|---------|-----------|------------------|-----------------|
| `host_tw_primary.json` | `host_tw_primary` | `toy_world` | `data-store`, `checker`, `assembler`, `input-source` |
| `host_tw_peer.json` | `host_tw_peer` | `peer_demo` | `checker`, `input-source` |

Both declare `spoke-baseline`. The primary host provides closed-loop `assembler`, write authority via `data-store`, and the broader collaboration role set (`checker`, `input-source`); the peer host contributes `checker` and `input-source` for a narrower in-process peer. Integrators map each manifest's `namespaces[]` to the owning `host_id` when attributing `KnowledgeEntry.extensions.<ns>` in a collaboration context. Host metadata lives on the `HostCapabilityManifest` wire object. Reference adapters compose these manifests in-process from committed fixture JSON; peer hosts resolve through product-supplied in-memory listing.

## Integrator path (TypeScript)

One adapter type implements the port families, then call `orchestrate*` from `@42ch/spoke-operations`:

1. Open `src/adapter/` — `ToyWorldAdapter` (`toy-world-adapter.ts`), in-memory OCC store (`memory-store.ts`), barrel (`index.ts`).
2. Construct the adapter: `ToyWorldAdapter.withCommittedFixtures()` (seeded `kb_tw_*` / `rel_tw_*` / `evt_tw_*` / `rule_tw_*` / `fnd_tw_*`) or `new ToyWorldAdapter()` for an empty store.
3. Pass the same adapter instance into baseline orchestrators: `orchestrateUpsert`, `orchestratePromote`, `orchestrateRelate`, `orchestrateCheck`, `orchestrateAssemble`.
4. For Full composition, the same type also implements `project` / `compute` / `listForkTimelineEvents` — call `orchestrateProject`, `orchestrateCompute`, and fork-aware orchestrators against that instance.

Vitest demos live in `tests/toy-world-adapter.test.ts` (imports adapter source directly).

## Integrator path (Rust)

One adapter type implements the port traits, then call `orchestrate_*` from `spoke-operations`:

1. Open `rust/` — crate `spoke-fixture-toy-world` (`src/toy_world_adapter.rs`, `src/memory_store.rs`, `src/lib.rs`).
2. Construct the adapter: `ToyWorldAdapter::with_committed_fixtures()` (seeded `kb_tw_*` / `rel_tw_*` / `evt_tw_*` / `rule_tw_*` / `fnd_tw_*`) or `ToyWorldAdapter::default()` for an empty store.
3. Pass the same adapter into baseline orchestrators: `orchestrate_upsert`, `orchestrate_promote`, `orchestrate_relate`, `orchestrate_check`, `orchestrate_assemble`.
4. For Full composition, the same type also implements `project` / `compute` / `list_fork_timeline_events` — call `orchestrate_project`, `orchestrate_compute`, and fork-aware orchestrators against that instance.
5. For a baseline-only dynamic boundary, wrap with `as_baseline_only(adapter)` so optional ports surface `CAPABILITY_PORT_MISSING`.

Cargo demos live in `rust/tests/toy_world_adapter.rs`.

### Full stub policy (reference behavior)

| Port family | ToyWorldAdapter behavior |
|-------------|--------------------------|
| Baseline OCC families (five port families) | Runnable in-memory OCC; optional seed from committed fixture JSON |
| `HostManifestPort` | Self manifest from `host_tw_primary.json`; peer list from in-memory `host_tw_peer.json` (exclude self; dedupe by `host_id`; ascending `host_id` sort) |
| `ComputablePort` | Wire-valid `ProjectResponse` / `ComputeResponse` synthesized from `op_tw_project_response.json` / `op_tw_compute_settle_response.json` (echo request `session_id` / `entry_id`) |
| `ForkTimelineQueryPort` | Seeded timeline events filtered by `scope.fork_id` (e.g. `evt_tw_harbor_storm_delay.json` for `fork_tw_storm_branch`) |

Normative detail: [`.mstar/specs/spoke-operations.md`](../../.mstar/specs/spoke-operations.md) § Reference adapter stub policy.

## Reference adapters

| Language | Path | Package / crate |
|----------|------|-----------------|
| TypeScript | `src/adapter/` | `@42ch/spoke-fixture-toy-world` (workspace-private) |
| Rust | `rust/` | `spoke-fixture-toy-world` (`publish = false`) |

`ToyWorldAdapter` implements **FullAdapter** (baseline + `l2-computable` + `l5-fork`).

## Files

| File | Schema | Id |
|------|--------|-----|
| `kb_tw_mira.json` | KnowledgeEntry | `kb_tw_mira` |
| `kb_tw_harbor.json` | KnowledgeEntry (l2-computable `body.state` / `body.computable`) | `kb_tw_harbor` |
| `kb_tw_harbor_dawn_event.json` | KnowledgeEntry (`entry_type: "event"`) | `kb_tw_harbor_dawn_event` |
| `kb_tw_harbor_market_square_event.json` | KnowledgeEntry (`entry_type: "event"`) | `kb_tw_harbor_market_square_event` |
| `kb_tw_harbor_customs_gate_beat.json` | KnowledgeEntry (profile `entry_type: "beat"`, `structural_role`) | `kb_tw_harbor_customs_gate_beat` |
| `kb_tw_harbor_berth_confirm_event.json` | KnowledgeEntry (`entry_type: "event"`) | `kb_tw_harbor_berth_confirm_event` |
| `anchor_tw_manuscript.json` | SourceAnchor | (provenance example) |
| `rel_tw_mira_harbor.json` | Relation | `rel_tw_mira_harbor` |
| `rel_tw_harbor_precedes_dawn_to_market.json` | Relation (`precedes` on KE ids) | `rel_tw_harbor_precedes_dawn_to_market` |
| `rel_tw_harbor_precedes_market_to_customs.json` | Relation (`precedes` on KE ids) | `rel_tw_harbor_precedes_market_to_customs` |
| `rel_tw_harbor_precedes_customs_to_berth.json` | Relation (`precedes` on KE ids) | `rel_tw_harbor_precedes_customs_to_berth` |
| `evt_tw_harbor_dawn.json` | TimelineEvent (`timeline_scale: "moment"`, `computable_logs`) | `evt_tw_harbor_dawn` |
| `evt_tw_harbor_market_square.json` | TimelineEvent (`timeline_scale: "moment"`, beat-sheet sample) | `evt_tw_harbor_market_square` |
| `evt_tw_harbor_customs_gate.json` | TimelineEvent (`timeline_scale: "moment"`, beat-sheet sample) | `evt_tw_harbor_customs_gate` |
| `evt_tw_harbor_berth_confirm.json` | TimelineEvent (`timeline_scale: "moment"`, beat-sheet sample) | `evt_tw_harbor_berth_confirm` |
| `evt_tw_harbor_storm_delay.json` | TimelineEvent (`fork_id: fork_tw_storm_branch`) | `evt_tw_harbor_storm_delay` |
| `rule_tw_consistency.json` | Rule | `rule_tw_consistency` |
| `fnd_tw_open.json` | Finding | `fnd_tw_open` |
| `pkt_tw_scope.json` | AssemblePacket | `pkt_tw_scope` |
| `op_tw_project_request.json` | ProjectRequest (optional `l2-computable` op) | `sess_tw_dawn_arrival` / `kb_tw_harbor` |
| `op_tw_project_response.json` | ProjectResponse (success branch) | `sess_tw_dawn_arrival` / `kb_tw_harbor` |
| `op_tw_compute_request.json` | ComputeRequest (mid-Session apply) | `sess_tw_dawn_arrival` / `kb_tw_harbor` |
| `op_tw_compute_settle_request.json` | ComputeRequest (`settle: true`) | `sess_tw_dawn_arrival` / `kb_tw_harbor` |
| `op_tw_compute_settle_response.json` | ComputeResponse (success + merged `state`) | `sess_tw_dawn_arrival` / `kb_tw_harbor` |
| `host_tw_primary.json` | HostCapabilityManifest (primary collaboration host) | `host_tw_primary` |
| `host_tw_peer.json` | HostCapabilityManifest (peer checker/input host) | `host_tw_peer` |

`proposed/pack_tw_harbor_companion.json` — companion sample showing **proposed** `modules.pack` / `modules.activation` shape (not validated by harness; baseline atoms within are valid).

`kb_tw_mira` carries two distinct `extensions.<namespace>` bags with preserve-unknown keys (fixture namespaces are illustrative only).

## Validate locally

TypeScript (AJV/Vitest harness):

```bash
pnpm run test:fixtures
```

Or from this directory:

```bash
pnpm test
```

Rust (workspace member `spoke-fixture-toy-world`, from repo root):

```bash
cargo test -p spoke-fixture-toy-world
```

CI runs the AJV/Vitest harness via `@42ch/spoke-fixture-toy-world` (`fixtures/toy-world/tests/`) and `cargo test -p spoke-fixture-toy-world` in the Rust job.
