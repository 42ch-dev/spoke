# SPOKE

[![CI](https://github.com/42ch-dev/spoke/actions/workflows/ci.yml/badge.svg)](https://github.com/42ch-dev/spoke/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Version](https://img.shields.io/github/v/release/42ch-dev/spoke?include_prereleases&sort=semver&label=version)](https://github.com/42ch-dev/spoke/releases)
[![Last commit](https://img.shields.io/github/last-commit/42ch-dev/spoke)](https://github.com/42ch-dev/spoke/commits/main)
[![Greptile: The War on Bugs](https://www.greptile.com/badge.svg)](https://www.greptile.com/?utm_source=oss_badge&utm_medium=readme&utm_campaign=greptile_for_open_source)

[中文](README_CN.md) · [Concepts](CONCEPTS.md) · [Strategy](STRATEGY.md) · [Contributing](CONTRIBUTING.md)

**Standardized Programmable Ontology Knowledge Engine** — a protocol of JSON Schema wire contracts for narrative **KnowledgeEntry** data and **ops**. Independent products exchange consistency-check and context-assembly I/O through these shapes.

**Includes:**

- Data-layer schemas: KnowledgeEntry, Relation, SourceAnchor, Finding, AssemblePacket, **HostCapabilityManifest**, Rule, TimelineEvent
- Ops-layer schemas: `upsert`, extract→promote, `relate`, `check`, `assemble`; optional **`project` / `compute`** under `l2-computable`
- Generated TypeScript (`@42ch/spoke-schemas`) and Rust (`spoke-schemas`, `spoke-operations`)
- Pure lifecycle helpers (`@42ch/spoke-operations` / `spoke-operations`)
- Protocol conformance fixtures ([`fixtures/toy-world/`](fixtures/toy-world/))

## Packages

Published consumer packages share one **lockstep SemVer**.

| Package | Registry | Role |
|---------|----------|------|
| [`@42ch/spoke-schemas`](https://www.npmjs.com/package/@42ch/spoke-schemas) | npm | Generated TypeScript wire types — **what** crosses the wire |
| [`@42ch/spoke-operations`](https://www.npmjs.com/package/@42ch/spoke-operations) | npm | Pure lifecycle helpers over those types |
| [`spoke-schemas`](https://crates.io/crates/spoke-schemas) | crates.io | Generated Rust wire types |
| [`spoke-operations`](https://crates.io/crates/spoke-operations) | crates.io | Pure lifecycle helpers — parity with `@42ch/spoke-operations` |

Product-specific payloads live under `extensions.<namespace>` (namespace keys are product-chosen ids).

## Install

### TypeScript (npm)

```bash
pnpm add @42ch/spoke-schemas @42ch/spoke-operations
# Pin both to the same lockstep SemVer, e.g. @X.Y.Z
```

**`@42ch/spoke-schemas`** — import generated wire types:

```ts
import type {
  KnowledgeEntry,
  TimelineEvent,
  PromoteRequest,
  AssemblePacket,
} from "@42ch/spoke-schemas";
```

**`@42ch/spoke-operations`** — call pure helpers (depends on `@42ch/spoke-schemas`):

```ts
import type { PromoteRequest } from "@42ch/spoke-schemas";
import {
  validatePromoteRequest,
  applyPromoteAcceptance,
  buildAssemblePacket,
  transitionFindingStatus,
  mergeExtensionMaps,
} from "@42ch/spoke-operations";

const request: PromoteRequest = { candidate /* KnowledgeEntry */ };
const gate = validatePromoteRequest(request);
if (gate.ok) {
  const accepted = applyPromoteAcceptance(request);
  // Persist via your product adapter
}
```

### Rust (crates.io)

```bash
cargo add spoke-schemas spoke-operations
# Pin both to the same lockstep SemVer, e.g. X.Y.Z
```

```toml
# Cargo.toml
[dependencies]
spoke-schemas = "X.Y.Z"
spoke-operations = "X.Y.Z"
```

**`spoke-schemas`** — wire types from the same JSON Schema SSOT:

```rust
use spoke_schemas::{KnowledgeEntry, PromoteRequest, TimelineEvent};
```

**`spoke-operations`** — helpers over those types (`spoke_schemas` is also re-exported):

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

## Version and pinning

Pin every consumer surface to the **same** SemVer (`X.Y.Z`) on npm and crates.io:

```bash
pnpm add @42ch/spoke-schemas@X.Y.Z @42ch/spoke-operations@X.Y.Z
cargo add spoke-schemas@X.Y.Z spoke-operations@X.Y.Z
```

Annotated git tags `vX.Y.Z` match that lockstep version. Release notes: [`CHANGELOG.md`](CHANGELOG.md) and [GitHub Releases](https://github.com/42ch-dev/spoke/releases).

## Quick start

```typescript
import type { KnowledgeEntry, PromoteRequest } from "@42ch/spoke-schemas";
import { validatePromoteRequest } from "@42ch/spoke-operations";

const candidate: KnowledgeEntry = {
  schema_version: 1,
  entry_id: "kb_01",
  entry_type: "character",
  canonical_name: "Aria",
  status: "provisional",
  body: { summary: "A reluctant scout." },
  extensions: {},
};

const request: PromoteRequest = { candidate };
const result = validatePromoteRequest(request);

if (result.ok) {
  // Gate passed — persist via your product adapter
} else {
  console.error(result.code, result.message);
}
```

## Core concepts

| Term | In SPOKE |
|------|----------|
| **KnowledgeEntry** | Atomic narrative knowledge unit on the wire (`entry_id`, `entry_type`, `status`, `body`, `extensions`) |
| **Relation** | Directed edge between KnowledgeEntries (or KnowledgeEntry ↔ source) |
| **SourceAnchor** | Provenance pointer to a manuscript span or external locator |
| **Finding** | Checker output for consistency, style, or analysis |
| **Rule** | Declarative constraint input to `check` (L6) |
| **TimelineEvent** | First-class temporal object on the when-axis (L5) |
| **AssemblePacket** | Wire context-assembly payload (slim entries for downstream LLM prompts) |
| **Extensions** | Product-specific bag on every data object (`extensions.<namespace>`) |

Vocabulary and positioning: [`CONCEPTS.md`](CONCEPTS.md), [`STRATEGY.md`](STRATEGY.md).

## Optional capabilities

Products that need programmable KnowledgeEntry body state may declare **`l2-computable`**:

- **`body.state`** — static durable computable values
- **`body.computable`** — dynamic Session-scoped projection
- **`TimelineEvent.computable_logs`** — Moment-scale field-change presentation
- **`project` / `compute` ops** — init/projection and apply/settle I/O envelopes

Baseline integrators use core schemas; `l2-computable` is opt-in. Normative detail: [`.mstar/specs/spoke-protocol-layers.md`](.mstar/specs/spoke-protocol-layers.md) §Capability levels.

## Operations

`@42ch/spoke-operations` / `spoke-operations` provide pure, cross-product lifecycle helpers:

- Extension map merge and round-trip preservation
- Finding `status` transition validation and apply
- Promote acceptance checks (gate before persist)
- AssemblePacket builders from KnowledgeEntries
- Unified `SpokeResult` / `SpokeRejectCode` on reject paths

Normative detail: [`.mstar/specs/spoke-operations.md`](.mstar/specs/spoke-operations.md).

## Specs and schemas

| Path | Topic |
|------|-------|
| [`schemas/`](schemas/) | JSON Schema SSOT (Draft-07) |
| [`fixtures/toy-world/`](fixtures/toy-world/) | Protocol conformance JSON graph ("Mira at Harbor") |
| [`.mstar/specs/spoke-protocol.md`](.mstar/specs/spoke-protocol.md) | Umbrella protocol spec |
| [`.mstar/specs/spoke-protocol-layers.md`](.mstar/specs/spoke-protocol-layers.md) | Nine layers (L0–L8), capability levels, Timeline tiers |
| [`.mstar/specs/spoke-data-model.md`](.mstar/specs/spoke-data-model.md) | Data objects and open vocabulary |
| [`.mstar/specs/spoke-ops.md`](.mstar/specs/spoke-ops.md) | Ops wire request/response envelopes |
| [`.mstar/specs/spoke-operations.md`](.mstar/specs/spoke-operations.md) | Operations library behavior |

## Contributing

Local development, CI gates, and release procedure: [`CONTRIBUTING.md`](CONTRIBUTING.md).
