---
title: Walk the ToyWorld reference adapter
---

# Walk the ToyWorld reference adapter

[`fixtures/toy-world/`](https://github.com/42ch-dev/spoke/tree/main/fixtures/toy-world) is the protocol-owned worked example: a committed JSON graph ("Mira at Harbor"), reference `ToyWorldAdapter` implementations in TypeScript and Rust, and a conformance harness that validates every fixture against the committed schemas. It is the starting skeleton for your own adapter — the pattern to copy, not the product.

## The story behind the graph

Cartographer Mira arrives at Harbor Town at dawn; a consistency rule flags an open finding; an `AssemblePacket` scopes context for the scene. The graph exercises the wire surface deliberately:

- A **dual-concern pair** links ontology `entry_type: "event"` KnowledgeEntry `kb_tw_harbor_dawn_event` to TimelineEvent `evt_tw_harbor_dawn`.
- Three moment-scale beats extend the Harbor mainline in order — market square inquiry, customs gate inspection (profile `entry_type: "beat"`), berth confirmation — linked by `precedes` Relations and `extensions.spoke.timeline_entry_id` on each TimelineEvent.
- Harbor Town carries optional `l2-computable` `body.state` / `body.computable` (tide and cargo); the dawn moment records `computable_logs`.
- Two `HostCapabilityManifest` hosts (`host_tw_primary` / `host_tw_peer`) declare pairwise-disjoint `namespaces[]` for in-process collaboration.
- Six `conn_tw_*` fixtures demonstrate a two-host connect exchange under the optional `spoke-connect` capability: signed hellos in both directions, a session snapshot, an invoke request wrapping the real `op_tw_check_request` payload, plus success and error invoke responses.

## What the adapter implements

`ToyWorldAdapter` implements **`FullAdapter`** — baseline ports plus `l2-computable` and `l5-fork`:

| Port family | ToyWorldAdapter behavior |
|-------------|--------------------------|
| Baseline OCC families (five port families) | Runnable in-memory OCC; optional seed from committed fixture JSON |
| `HostManifestPort` | Self manifest from `host_tw_primary.json`; peer list from in-memory `host_tw_peer.json` (exclude self, dedupe by `host_id`, ascending sort) |
| `ComputablePort` | Wire-valid `ProjectResponse` / `ComputeResponse` synthesized from committed op fixtures (echo request `session_id` / `entry_id`) |
| `ForkTimelineQueryPort` | Seeded timeline events filtered by `scope.fork_id` |

## TypeScript side

Open `src/adapter/`:

- `memory-store.ts` — `MemoryStore`: the in-memory OCC store (get/put with revision checks) that the adapter delegates to. This is the pattern for your own storage bridge.
- `toy-world-adapter.ts` — `ToyWorldAdapter` + `asBaselineOnly()`: one type implements all ports; the baseline-only projection omits the optional methods so dynamic orchestrators surface `CAPABILITY_PORT_MISSING`.
- `index.ts` — the barrel exporting both.

```ts
import { ToyWorldAdapter, asBaselineOnly } from "@42ch/spoke-fixture-toy-world";
import { orchestrateUpsert, orchestrateCheck } from "@42ch/spoke-operations";

const adapter = ToyWorldAdapter.withCommittedFixtures(); // seeded kb/rel/evt/rule/fnd fixtures
// or: new ToyWorldAdapter() for an empty store

const baseline = asBaselineOnly(adapter); // baseline-only dynamic boundary
```

The conformance harness (`tests/`) drives the same adapter source:

- `toy-world-conformance.test.ts` — validates every committed fixture against the schemas (AJV).
- `toy-world-adapter.test.ts` — exercises port orchestration over the seeded graph (Vitest).
- `toy-world-ops-exercise.test.ts` — walks the op families against the adapter.

## Rust side

The crate `spoke-fixture-toy-world` under `rust/` mirrors the TypeScript adapter (`src/toy_world_adapter.rs`, `src/memory_store.rs`, `src/lib.rs`):

```rust
use spoke_fixture_toy_world::{as_baseline_only, ToyWorldAdapter};

let adapter = ToyWorldAdapter::with_committed_fixtures();
// or: ToyWorldAdapter::default() for an empty store
let baseline = as_baseline_only(adapter);
```

`rust/tests/toy_world_adapter.rs` runs the same port-orchestration demos in cargo. The crate is `publish = false` — a workspace-local reference, exactly like the TypeScript fixture package.

## Connect fixtures

The `conn_tw_*` JSON shows a full two-host connect exchange between the toy-world hosts: `ConnectHello` in each direction (`peer_tw_primary` ↔ `peer_tw_peer`), a `ConnectSession` snapshot (`initial_sequence: 0`), a `ConnectInvokeRequest` wrapping the real check payload, and both invoke response branches (success with an embedded `fnd_tw_open`; error reusing the shared `error-envelope` with `INVALID_INPUT`). The hello `signature` fields are structural test vectors — JCS canonicalization and cryptographic verification belong to the reference stack.

## Copy the skeleton

1. Clone the adapter shape: one type implementing the port families you claim (start from `BaselineAdapter`, grow to `FullAdapter`).
2. Swap `MemoryStore` for your storage (same method surface: get/put with expected-base-revision checks).
3. Call the matching `orchestrate*` — see [Orchestrate operations](/how-to/orchestrate-ops).
4. Validate your fixtures against the committed schemas with the same AJV/Vitest harness pattern (`pnpm run test:fixtures` in the repo, or `cargo test -p spoke-fixture-toy-world` for the Rust side).

## Next steps

- [Implement an adapter](/how-to/implement-adapter) — the port contract behind `ToyWorldAdapter`.
- [Orchestrate operations](/how-to/orchestrate-ops) — the `orchestrate*` calls the harness exercises.
- [Data model reference](/reference/data-model) — the wire objects the fixtures validate against.
