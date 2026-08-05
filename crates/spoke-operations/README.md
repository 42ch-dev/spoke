# spoke-operations

Hand-written Rust lifecycle helpers for [SPOKE](https://github.com/42ch-dev/spoke): extension merge/preserve, Finding status transitions, promote acceptance gates, Scope/upsert/relate validators, `body.attributes` read helpers, timeline sequence helpers, `AssemblePacket` builders, **capability-sliced adapter port traits**, and **injection orchestration**.

Depends on [`spoke-schemas`](https://crates.io/crates/spoke-schemas) for wire types. Behavioral parity with [`@42ch/spoke-operations`](https://www.npmjs.com/package/@42ch/spoke-operations) (TypeScript).

## Install

```bash
cargo add spoke-schemas spoke-operations
# Pin both to the same lockstep SemVer (e.g. X.Y.Z)
```

```toml
[dependencies]
spoke-schemas = "X.Y.Z"
spoke-operations = "X.Y.Z"
```

## Usage — pure helpers

```rust
use spoke_operations::{
    apply_promote_acceptance, validate_promote_request, SpokeResult,
};
use spoke_operations::spoke_schemas::PromoteRequest;

let gate = validate_promote_request(&request);
if let SpokeResult::Ok(_) = gate {
    let _accepted = apply_promote_acceptance(&request);
}
```

## Usage — adapter ports and orchestration

Implement the port traits (`KnowledgeEntryPort`, `RelationPort`, `ScopeQueryPort`, `FindingPort`, `RuleQueryPort`, `HostManifestPort`, plus optional `ComputablePort` / `ForkTimelineQueryPort`) on one adapter type. Marker traits `BaselineAdapter`, `ComputableAdapter`, `ForkAdapter`, and `FullAdapter` name the same compositions as `BaselinePorts`, `ComputablePorts`, `ForkPorts`, and `FullPorts` (blanket impl for any matching port set). Then call the matching orchestrator:

```rust
use spoke_operations::{
    orchestrate_check, orchestrate_upsert, BaselinePorts, CheckRunInput, SpokeResult,
};
use spoke_operations::spoke_schemas::{CheckRequest, UpsertRequest};

async fn run_baseline(ports: &impl BaselinePorts, upsert: UpsertRequest, check: CheckRequest) {
    let _ = orchestrate_upsert(ports, upsert).await;
    let _ = orchestrate_check(ports, check, |_input: CheckRunInput| SpokeResult::Ok(Vec::new())).await;
}
```

The orchestrators are `async fn`s — call them from an async context and drive the returned future with your runtime (test code may use `pollster::block_on`).

Optional capabilities use `ComputablePorts` / `ForkPorts` with `orchestrate_project` / `orchestrate_compute` and `orchestrate_fork_check` / `orchestrate_fork_assemble`.

**Integrator notes**

- Adapters own **transaction boundaries** for multi-entry upsert and other multi-write sequences.
- Active-uniqueness helpers take **caller-supplied peer sets**. Orchestration supplies batch-local peers; pass a store-wide snapshot when product uniqueness must span the whole store.
- Absent optional ports at a dynamic boundary surface `SpokeRejectCode::CapabilityPortMissing` (`CAPABILITY_PORT_MISSING`). `HostManifestPort` is baseline-required — not gated behind that code.

## Helper families

- `SpokeResult` / `SpokeReject` / `SpokeRejectCode` — unified reject envelope (stable code strings shared with TypeScript)
- `merge_extension_maps`, `preserve_extension_maps`
- `is_valid_finding_status_transition`, `transition_finding_status`
- `validate_promote_request`, `apply_promote_acceptance`, `validate_promote_request_wire`
- `knowledge_entry_to_assemble_entry`, `build_assemble_packet`
- Scope matchers, OCC revision assert, KnowledgeEntry status/uniqueness, upsert/relate gates, computable validators
- `list_body_attributes`, `filter_body_attributes_by_trait_type`, `find_body_attribute` — read/filter `body.attributes` by `trait_type`
- `filter_timeline_events_by_moment_scale`, `order_timeline_events_by_ids`, `order_timeline_events_by_precedes` — filter and order moment-scale TimelineEvents from caller-supplied sets
- Adapter ports + orchestration: `KnowledgeEntryPort` … `HostManifestPort` … `FullAdapter`, `CheckRunInput`, `orchestrate_upsert` … `orchestrate_fork_assemble`

Reference **FullAdapter** implementation: [`fixtures/toy-world/`](https://github.com/42ch-dev/spoke/tree/main/fixtures/toy-world) (`ToyWorldAdapter` in crate `spoke-fixture-toy-world` under `rust/`).

Pure functions and port-injected orchestrators over wire types. Normative behavior: [spoke-operations.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-operations.md).
