# spoke-schemas

Generated Rust wire types for [SPOKE](https://github.com/42ch-dev/spoke) — the Standardized Programmable Ontology Knowledge Engine.

Types are produced from the repository JSON Schema SSOT (`schemas/`). Pair with [`spoke-operations`](https://crates.io/crates/spoke-operations) for pure lifecycle helpers.

## Install

```bash
cargo add spoke-schemas
# Prefer the same SemVer as spoke-operations (e.g. X.Y.Z)
```

```toml
[dependencies]
spoke-schemas = "X.Y.Z"
```

Lockstep with `spoke-operations` at the same SemVer when using both.

## Usage

Import wire types from the crate root — re-exports resolve to the canonical generated module for each schema:

```rust
use spoke_schemas::{
    AssemblePacket, ComputableLogChange, KnowledgeEntry, PromoteRequest, Scope, TimelineEvent,
};
```

typify may emit duplicate nominal structs when it inlines shared definitions into multiple generated files (for example `SourceAnchor` nested inside `timeline_event.rs`). Those copies serialize the same JSON but are distinct Rust types. Use crate-root or `generated::common` imports for shared defs; convert across duplicates with `serde_json` round-trip when needed.

Protocol docs and the lockstep SemVer release policy live in the [SPOKE repository](https://github.com/42ch-dev/spoke).
