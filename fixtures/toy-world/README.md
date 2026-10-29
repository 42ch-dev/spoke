# Mira at Harbor — protocol toy-world fixtures

Protocol-owned JSON graph, AJV/Vitest conformance harness, and reference **`ToyWorldAdapter`** examples (TypeScript and Rust) for integrators validating parsers, codegen, and operations port orchestration against committed `schemas/`.

**Story:** Cartographer Mira arrives at Harbor Town at dawn; a consistency rule flags an open finding; an AssemblePacket scopes context for the scene. A dual-concern pair links ontology `entry_type: "event"` KnowledgeEntry `kb_tw_harbor_dawn_event` to TimelineEvent `evt_tw_harbor_dawn`. Three additional moment-scale beats extend the Harbor mainline in order — market square inquiry, customs gate inspection (profile `entry_type: "beat"`), berth confirmation — linked by `precedes` Relations on dual KE ids and `extensions.spoke.timeline_entry_id` on each TimelineEvent. Harbor Town carries optional l2-computable `body.state` / `body.computable` (tide and cargo); the dawn moment records `computable_logs` for those field changes.

**l5-mind sample:** a false-belief box/basket story pair. `mind_tw_bo_pre_transfer` / `mind_tw_bo_post_transfer` are `MindState` when-axis records for holder `kb_tw_bo`; the snapshots are identical across the hidden transfer (Bo did not observe it — the false-belief structure as data, no inference engine). `evt_tw_hidden_transfer` is the world-changing `TimelineEvent` carrying `modules.observation` (`observers: ["kb_tw_ana"]`, `access` with line-of-sight / hearing / modality) — Bo's absence from `observers` is the absence mechanic that leaves his belief stale.

## Host capability manifests (in-process collaboration)

Committed `HostCapabilityManifest` JSON describes four toy-world hosts with **pairwise disjoint** `namespaces[]`:

| Fixture | `host_id` | Owned namespaces | Roles (summary) |
|---------|-----------|------------------|-----------------|
| `host_tw_primary.json` | `host_tw_primary` | `toy_world` | `data-store`, `checker`, `assembler`, `input-source` |
| `host_tw_peer.json` | `host_tw_peer` | `peer_demo` | `checker`, `input-source` |
| `host_tw_tools.json` | `host_tw_tools` | `tool_demo` | `checker`, `input-source` |
| `host_tw_extractor.json` | `host_tw_extractor` | `extract_demo` | `input-source`, capability `ke-extraction` |

The three collaboration hosts declare `spoke-baseline`; the extraction host claims only `ke-extraction` with the existing `input-source` role, so it declares no baseline adapter. The primary host provides closed-loop `assembler`, write authority via `data-store`, and the broader collaboration role set (`checker`, `input-source`); the peer host contributes `checker` and `input-source` for a narrower in-process peer. The primary host also declares a `tools[]` surface — the copyable tool-provider reference: `tools.toy_world.roll_dice` (deterministic seeded dice) and `tools.toy_world.lore_lookup` (read-only store lookup), both listed in `capabilities[]` with the owned `toy_world` namespace; `validateManifestTools` passes on the manifest. The tools host additionally declares a `tools[]` entry (`tools.tool_demo.lookup`) whose `capability_id` also appears in `capabilities[]` and whose namespace (`tool_demo`) is owned by the manifest. Integrators map each manifest's `namespaces[]` to the owning `host_id` when attributing `KnowledgeEntry.extensions.<ns>` in a collaboration context. Host metadata lives on the `HostCapabilityManifest` wire object. Reference adapters compose these manifests in-process from committed fi…

## Extraction samples (optional `ke-extraction`)

Committed `op_tw_extract_*` / `host_tw_extractor.json` JSON demonstrates the optional extraction op against the same manuscript references:

| Fixture | Envelope | Story |
|---------|----------|-------|
| `op_tw_extract_request.json` | ExtractRequest | `run_id: run_tw_extract_0001`; two `SourceAnchor` references over `manuscript:tw-ch1` (one carrying a `span`, one referencing the artifact whole) plus an `entry_types: ["info_point"]` hint |
| `op_tw_extract_response.json` | ExtractResponse (success branch) | Two provisional `info_point` candidates keyed back to the anchor they came from, plus `run` echoing the request `run_id` with an open `method` and an object `coverage_hint` |
| `op_tw_extract_error_response.json` | ExtractResponse (error branch) | `CANDIDATE_NOT_PROVISIONAL` for a candidate flipped to `confirmed` — no partial candidate list and no second run record |
| `host_tw_extractor.json` | HostCapabilityManifest | `input-source` host claiming `ke-extraction` with the disjoint `extract_demo` namespace |

`tests/toy-world-extract.test.ts` validates all four samples through the AJV loader, runs `orchestrateExtract` with a host-local source loader and an async extractor double (the loaded content stays in-process and never crosses the wire), and pins the schema boundaries: the success and error branches are mutually exclusive, and an inline source-text field is rejected. The success/error pair is the library's own wire shape at the documented boundary, not a hand-written approximation.

## Connect envelope samples (opt-in `spoke-connect`)

Committed `conn_tw_*` JSON demonstrates a two-host connect exchange between the toy-world hosts under the optional `spoke-connect` capability:

| Fixture | Envelope | Story |
|---------|----------|-------|
| `conn_tw_hello_primary_to_peer.json` | ConnectHello | Primary → peer handshake; `peer_id: peer_tw_primary` |
| `conn_tw_hello_peer_to_primary.json` | ConnectHello | Peer → primary handshake; `peer_id: peer_tw_peer` |
| `conn_tw_session.json` | ConnectSession | Established session snapshot (`initial_sequence: 0`, negotiated `spoke-connect`) |
| `conn_tw_invoke_check_request.json` | ConnectInvokeRequest | Remote `check` call wrapping the real `op_tw_check_request` payload |
| `conn_tw_invoke_check_response.json` | ConnectInvokeResponse | Success branch wrapping a real check-response envelope (embedded `fnd_tw_open`) |
| `conn_tw_invoke_error_response.json` | ConnectInvokeResponse | Error branch reusing the shared `error-envelope` (`INVALID_INPUT`) |

Dual identity: each hello carries an opaque `peer_id` (network identity, trust root) distinct from the embedded manifest's `host_id` (application identity). Embedded manifests are adapted from the committed `host_tw_*` hosts with the optional `spoke-connect` capability added — the standalone manifests stay baseline. Hello `signature` fields are **structural test vectors only**: valid-shaped base64url strings (no padding, non-cryptographic) for schema conformance; JCS canonicalization and cryptographic verification belong to the reference stack. Invoke responses echo `session_id` / `sequence` / `request_id` from the request; the error branch echoes a second, malformed invoke (`req_tw_check_0002`).

## Memory protocol samples (optional `spoke-memory`)

Committed `mem_tw_*` JSON traces one Harbor Town memory slice in space `spc_tw_harbor` (authority `host_tw_primary`) for subjects `toy:character:mira` and `toy:character:bo`:

| Fixture | Envelope | Story |
|---------|----------|-------|
| `mem_tw_space_harbor.json` | MemorySpace | Active space, generation 1, drained (`pending_receipts: 0`) |
| `mem_tw_subject_mira.json` | Subject | Mira, `kind: { ns: "toy", id: "character" }`, no fenced contexts |
| `mem_tw_subject_bo.json` | Subject | Bo, same kind, no fenced contexts |
| `mem_tw_grant_mira_context.json` | MemoryGrant (RestrictedGrant) | `recall`/`capture`/`review`/`records_write` narrowed to `ctx_tw_customs_gate` with `shared_access: true` |
| `mem_tw_grant_operator_admin.json` | MemoryGrant (UnrestrictedGrant) | Space-wide operator `admin`; `context_ids`/`shared_access` deliberately absent |
| `mem_tw_capture_request.json` | MemoryCaptureRequest | Placement `ctx_tw_customs_gate` + provenance claims (anchor over `manuscript:tw-ch1`) |
| `mem_tw_capture_response.json` | MemoryCaptureResponse (committed) | Receipt whose `payload_digest` is the JCS SHA-256 of the capture request |
| `mem_tw_candidate_pending_bo.json` | MemoryCandidate (PendingCandidate) | Bo's belief in `ctx_tw_market_square`, awaiting review |
| `mem_tw_candidate_consumed_gate_key.json` | MemoryCandidate (ConsumedCandidate) | Gate-key candidate after fragment review: content scrubbed, `content_digest` kept |
| `mem_tw_review_promote_request.json` | MemoryCandidatesReviewRequest | `promote` decision on the pending Bo candidate |
| `mem_tw_review_fragment_request.json` | MemoryCandidatesReviewRequest | `fragment` decision producing the fact + event records |
| `mem_tw_recall_request.json` | MemoryRecallRequest | `selected_contexts` over `ctx_tw_customs_gate` with `include_shared`, budget `max_items: 2` |
| `mem_tw_recall_response.json` | MemoryRecallResponse | `max_items`-cut page (`truncated: true`, `total_authorized: 3`, no `overflow_record_id`) |
| `mem_tw_recall_overflow_request.json` | MemoryRecallRequest | Same selection with an 8-byte `max_bytes` budget |
| `mem_tw_recall_overflow_response.json` | MemoryRecallResponse | §9 oversized-first-item page: one record returned whole with `overflow_record_id` |
| `mem_tw_export_bundle.json` | MemoryExportBundle | Whole-space export: subjects, records, candidate shells, receipt summaries, redacted grant summaries, JCS digest |

Digests are real: `payload_digest`, `content_digest` and the bundle `digest` are SHA-256 over RFC 8785 JCS canonicalization (§6.1) and are pinned by `tests/toy-world-memory.test.ts`, which also drives the P2 helpers (`validateMemoryCapture`, `validateMemoryCandidatesReview`, `validateMemoryRecall`, `grantCovers`, `validateGrantIssuance`) over the same samples.

## Integrator path (TypeScript)

One adapter type implements the port families, then call `orchestrate*` from `@42ch/spoke-operations`:

1. Open `src/adapter/` — `ToyWorldAdapter` (`toy-world-adapter.ts`), in-memory OCC store (`memory-store.ts`), barrel (`index.ts`).
2. Construct the adapter: `ToyWorldAdapter.withCommittedFixtures()` (seeded `kb_tw_*` / `rel_tw_*` / `evt_tw_*` / `rule_tw_*` / `fnd_tw_*`) or `new ToyWorldAdapter()` for an empty store.
3. Pass the same adapter instance into baseline orchestrators: `orchestrateUpsert`, `orchestratePromote`, `orchestrateRelate`, `orchestrateCheck`, `orchestrateAssemble`.
4. For Full composition, the same type also implements `project` / `compute` / `listForkTimelineEvents` — call `orchestrateProject`, `orchestrateCompute`, and fork-aware orchestrators against that instance.
5. For the tool-provider surface, the adapter serves `tools.toy_world.roll_dice` and `tools.toy_world.lore_lookup` out of the box: `adapter.toolDescriptors()` lists them (defensive copy of manifest `tools[]`), `adapter.registerToolHandler(id, handler)` overrides a handler (last-wins; a non-`tools.` id throws), and `await adapter.invokeTool(id, args)` runs the grammar gate → descriptor lookup → argument gate → handler dispatch. `validateManifestTools` passes on the adapter's manifest.

Vitest demos live in `tests/toy-world-adapter.test.ts` and `tests/toy-world-tools.test.ts` (import adapter source directly).

## Integrator path (Rust)

One adapter type implements the port traits, then call `orchestrate_*` from `spoke-operations`:

1. Open `rust/` — crate `spoke-fixture-toy-world` (`src/toy_world_adapter.rs`, `src/memory_store.rs`, `src/lib.rs`).
2. Construct the adapter: `ToyWorldAdapter::with_committed_fixtures()` (seeded `kb_tw_*` / `rel_tw_*` / `evt_tw_*` / `rule_tw_*` / `fnd_tw_*`) or `ToyWorldAdapter::default()` for an empty store.
3. Pass the same adapter into baseline orchestrators: `orchestrate_upsert`, `orchestrate_promote`, `orchestrate_relate`, `orchestrate_check`, `orchestrate_assemble`.
4. For Full composition, the same type also implements `project` / `compute` / `list_fork_timeline_events` — call `orchestrate_project`, `orchestrate_compute`, and fork-aware orchestrators against that instance.
5. For a baseline-only dynamic boundary, wrap with `as_baseline_only(adapter)` so optional ports surface `CAPABILITY_PORT_MISSING`.
6. For the tool-provider surface (mirror of the TypeScript surface), the adapter serves the same two tools out of the box: `adapter.tool_descriptors()`, `adapter.register_tool_handler(id, handler)` (last-wins; a non-`tools.` id panics), and `adapter.invoke_tool(id, args).await`. The `ToolHandler` type in `toy_world_tools` mirrors `spoke-connect`'s handler shape so copied provider code adapts directly.

Cargo demos live in `rust/tests/toy_world_adapter.rs` and `rust/tests/toy_world_tools.rs`.

### Full stub policy (reference behavior)

| Port family | ToyWorldAdapter behavior |
|-------------|--------------------------|
| Baseline OCC families (five port families) | Runnable in-memory OCC; optional seed from committed fixture JSON |
| `HostManifestPort` | Self manifest from `host_tw_primary.json`; peer list from in-memory `host_tw_peer.json` (exclude self; dedupe by `host_id`; ascending `host_id` sort) |
| `ComputablePort` | Wire-valid `ProjectResponse` / `ComputeResponse` synthesized from `op_tw_project_response.json` / `op_tw_compute_settle_response.json` (echo request `session_id` / `entry_id`) |
| `ForkTimelineQueryPort` | Seeded timeline events filtered by `scope.fork_id` (e.g. `evt_tw_harbor_storm_delay.json` for `fork_tw_storm_branch`) |
| Tool surface (`tools.*`) | `toolDescriptors()` / `tool_descriptors()` list the manifest `tools[]`; `registerToolHandler` / `register_tool_handler` registers a handler (grammar-asserted, last-wins); `invokeTool` / `invoke_tool` runs the grammar gate → descriptor lookup → argument gate → handler. Both frozen tools are pre-registered: `roll_dice` (seeded deterministic) and `lore_lookup` (read-only store lookup) |

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
| `kb_tw_owner_private.json` | KnowledgeEntry (`ke-ownership` `owner` / `disclosure: "owner-private"`) | `kb_tw_mira_private_shoal` |
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
| `evt_tw_hidden_transfer.json` | TimelineEvent (`modules.observation` on the false-belief transfer) | `evt_tw_hidden_transfer` |
| `mind_tw_bo_pre_transfer.json` | MindState (pre-transfer snapshot for `kb_tw_bo`) | `mind_tw_bo_pre_transfer` |
| `mind_tw_bo_post_transfer.json` | MindState (post-transfer snapshot for `kb_tw_bo`) | `mind_tw_bo_post_transfer` |
| `rule_tw_consistency.json` | Rule | `rule_tw_consistency` |
| `fnd_tw_open.json` | Finding | `fnd_tw_open` |
| `pkt_tw_scope.json` | AssemblePacket | `pkt_tw_scope` |
| `op_tw_ownership_check_request.json` | CheckRequest (shared `Scope.viewpoint` reader selector) | `toy-scope-ownership` / `kb_tw_mira` |
| `op_tw_project_request.json` | ProjectRequest (optional `l2-computable` op) | `sess_tw_dawn_arrival` / `kb_tw_harbor` |
| `op_tw_project_response.json` | ProjectResponse (success branch) | `sess_tw_dawn_arrival` / `kb_tw_harbor` |
| `op_tw_compute_request.json` | ComputeRequest (mid-Session apply) | `sess_tw_dawn_arrival` / `kb_tw_harbor` |
| `op_tw_compute_settle_request.json` | ComputeRequest (`settle: true`) | `sess_tw_dawn_arrival` / `kb_tw_harbor` |
| `op_tw_compute_settle_response.json` | ComputeResponse (success + merged `state`) | `sess_tw_dawn_arrival` / `kb_tw_harbor` |
| `op_tw_extract_request.json` | ExtractRequest (optional `ke-extraction` op) | `run_tw_extract_0001` |
| `op_tw_extract_response.json` | ExtractResponse (success branch, provisional candidates + run metadata) | `run_tw_extract_0001` |
| `op_tw_extract_error_response.json` | ExtractResponse (error branch, `CANDIDATE_NOT_PROVISIONAL` for `kb_tw_extract_2`) | `kb_tw_extract_2` |
| `host_tw_primary.json` | HostCapabilityManifest (primary collaboration host, `tools[]` declared: `tools.toy_world.roll_dice` / `tools.toy_world.lore_lookup`) | `host_tw_primary` |
| `host_tw_peer.json` | HostCapabilityManifest (peer checker/input host) | `host_tw_peer` |
| `host_tw_tools.json` | HostCapabilityManifest (tool provider host, `tools[]` declared) | `host_tw_tools` |
| `host_tw_extractor.json` | HostCapabilityManifest (extraction host: `input-source` + `ke-extraction`) | `host_tw_extractor` |
| `tool_tw_minimal.json` | ToolDescriptor (unconstrained `{}` input/output ABI) | `tools.tool_demo.lookup` |
| `conn_tw_hello_primary_to_peer.json` | ConnectHello (opt-in `spoke-connect` handshake) | `peer_tw_primary` / `host_tw_primary` |
| `conn_tw_hello_peer_to_primary.json` | ConnectHello (opt-in `spoke-connect` handshake) | `peer_tw_peer` / `host_tw_peer` |
| `conn_tw_session.json` | ConnectSession (established session snapshot) | `sess_tw_connect_0001` |
| `conn_tw_invoke_check_request.json` | ConnectInvokeRequest (remote `check`) | `sess_tw_connect_0001` / `req_tw_check_0001` |
| `conn_tw_invoke_check_response.json` | ConnectInvokeResponse (success branch) | `sess_tw_connect_0001` / `req_tw_check_0001` |
| `conn_tw_invoke_error_response.json` | ConnectInvokeResponse (error branch, `INVALID_INPUT`) | `sess_tw_connect_0001` / `req_tw_check_0002` |
| `mem_tw_space_harbor.json` | MemorySpace | `spc_tw_harbor` |
| `mem_tw_subject_mira.json` | Subject | `toy:character:mira` |
| `mem_tw_subject_bo.json` | Subject | `toy:character:bo` |
| `mem_tw_grant_mira_context.json` | MemoryGrant (RestrictedGrant, `shared_access: true`) | `grt_tw_mira_context_0001` |
| `mem_tw_grant_operator_admin.json` | MemoryGrant (UnrestrictedGrant, `admin`) | `grt_tw_operator_admin_0002` |
| `mem_tw_capture_request.json` | MemoryCaptureRequest | `cap-tw-gate-key-0001` |
| `mem_tw_capture_response.json` | MemoryCaptureResponse (committed receipt) | `rcpt_tw_capture_0001` / `cand_tw_gate_key_0001` |
| `mem_tw_candidate_pending_bo.json` | MemoryCandidate (PendingCandidate) | `cand_tw_bo_belief_0002` |
| `mem_tw_candidate_consumed_gate_key.json` | MemoryCandidate (ConsumedCandidate, content scrubbed) | `cand_tw_gate_key_0001` |
| `mem_tw_review_promote_request.json` | MemoryCandidatesReviewRequest (`promote`) | `cand_tw_bo_belief_0002` |
| `mem_tw_review_fragment_request.json` | MemoryCandidatesReviewRequest (`fragment`) | `cand_tw_gate_key_0001` |
| `mem_tw_recall_request.json` | MemoryRecallRequest (budgeted `selected_contexts`) | `spc_tw_harbor` / `toy:character:mira` |
| `mem_tw_recall_response.json` | MemoryRecallResponse (`max_items`-cut page) | `rec_tw_gate_key_fact` / `rec_tw_bo_promise` |
| `mem_tw_recall_overflow_request.json` | MemoryRecallRequest (8-byte `max_bytes`) | `spc_tw_harbor` / `toy:character:mira` |
| `mem_tw_recall_overflow_response.json` | MemoryRecallResponse (§9 oversized first item) | `rec_tw_gate_key_fact` |
| `mem_tw_export_bundle.json` | MemoryExportBundle (JCS `digest`) | `exp_tw_harbor_0001` |

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
