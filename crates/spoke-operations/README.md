# spoke-operations

Hand-written Rust lifecycle helpers for [SPOKE](https://github.com/42ch-dev/spoke): extension merge/preserve, Finding status transitions, promote acceptance gates, and `AssemblePacket` builders.

Depends on [`spoke-schemas`](https://crates.io/crates/spoke-schemas) for wire types. Behavioral parity with `@42ch/spoke-operations` (TypeScript).

## Usage

```toml
[dependencies]
spoke-schemas = "0.1.0"
spoke-operations = "0.1.0"
```

```rust
use spoke_operations::{
    apply_promote_acceptance, merge_extension_maps, spoke_schemas::PromoteRequest,
};

let result = apply_promote_acceptance(&request);
```

## First-slice helpers

- `SpokeResult` / `SpokeReject` / `SpokeRejectCode` — unified reject envelope (19 stable code strings)
- `merge_extension_maps`, `preserve_extension_maps`
- `is_valid_finding_status_transition`, `transition_finding_status`
- `validate_promote_request`, `apply_promote_acceptance`
- `knowledge_entry_to_assemble_entry`, `build_assemble_packet`

Pure functions only: no I/O, storage, HTTP, LLM, ranking, or retrieval.
