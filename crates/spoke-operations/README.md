# spoke-operations

Hand-written Rust lifecycle helpers for [SPOKE](https://github.com/42ch-dev/spoke): extension merge/preserve, Finding status transitions, promote acceptance gates, Scope/upsert/relate validators, `body.attributes` read helpers, and `AssemblePacket` builders.

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

## Usage

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

## Helper families

- `SpokeResult` / `SpokeReject` / `SpokeRejectCode` — unified reject envelope (stable code strings shared with TypeScript)
- `merge_extension_maps`, `preserve_extension_maps`
- `is_valid_finding_status_transition`, `transition_finding_status`
- `validate_promote_request`, `apply_promote_acceptance`, `validate_promote_request_wire`
- `knowledge_entry_to_assemble_entry`, `build_assemble_packet`
- Scope matchers, OCC revision assert, KnowledgeEntry status/uniqueness, upsert/relate gates, computable validators
- `list_body_attributes`, `filter_body_attributes_by_trait_type`, `find_body_attribute` — read/filter `body.attributes` by `trait_type`

Pure functions over wire types. Normative behavior: [spoke-operations.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-operations.md).
