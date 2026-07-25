# spoke-schemas

Generated Rust wire types for [SPOKE](https://github.com/42ch-dev/spoke) — the Standardized Programmable Ontology Knowledge Engine.

Types are produced from the repository JSON Schema SSOT (`schemas/`). Pair with [`spoke-operations`](https://crates.io/crates/spoke-operations) for pure lifecycle helpers.

## Install

```bash
cargo add spoke-schemas
# Prefer the same SemVer as spoke-operations (current lockstep: 0.1.0)
```

```toml
[dependencies]
spoke-schemas = "0.1.0"
```

Lockstep with `spoke-operations` at the same SemVer when using both.

## Usage

```rust
use spoke_schemas::{KnowledgeEntry, PromoteRequest, TimelineEvent, AssemblePacket};
```

Protocol docs and the lockstep SemVer release policy live in the [SPOKE repository](https://github.com/42ch-dev/spoke).
