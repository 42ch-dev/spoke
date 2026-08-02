# SPOKE

[![CI](https://github.com/42ch-dev/spoke/actions/workflows/ci.yml/badge.svg)](https://github.com/42ch-dev/spoke/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Version](https://img.shields.io/github/v/release/42ch-dev/spoke?include_prereleases&sort=semver&label=version)](https://github.com/42ch-dev/spoke/releases)
[![Last commit](https://img.shields.io/github/last-commit/42ch-dev/spoke)](https://github.com/42ch-dev/spoke/commits/main)
[![Greptile: The War on Bugs](https://www.greptile.com/badge.svg)](https://www.greptile.com/?utm_source=oss_badge&utm_medium=readme&utm_campaign=greptile_for_open_source)

[中文](README_CN.md) · [Documentation](docs/) · [Concepts](CONCEPTS.md) · [Strategy](STRATEGY.md) · [Contributing](CONTRIBUTING.md)

Integrator documentation: browse [`docs/`](docs/) locally with `pnpm docs:dev`; the site is published to <https://42ch-dev.github.io/spoke/> once the Pages deploy workflow runs.

**Standardized Programmable Ontology Knowledge Engine** — a protocol of JSON Schema wire contracts for narrative **KnowledgeEntry** data and **ops**. Independent products exchange consistency-check and context-assembly I/O through these shapes.

**Includes:**

- Data-layer schemas: KnowledgeEntry, Relation, SourceAnchor, Finding, AssemblePacket, **HostCapabilityManifest**, Rule, TimelineEvent
- Ops-layer schemas: `upsert`, extract→promote, `relate`, `check`, `assemble`; optional **`project` / `compute`** under `l2-computable`
- Generated TypeScript (`@42ch/spoke-schemas`) and Rust (`spoke-schemas`, `spoke-operations`)
- Pure lifecycle helpers plus **adapter ports** and **injection orchestration** (`@42ch/spoke-operations` / `spoke-operations`)
- Protocol conformance fixtures and reference **`ToyWorldAdapter`** ([`fixtures/toy-world/`](fixtures/toy-world/))

## Packages

Published consumer packages share one **lockstep SemVer**.

| Package | Registry | Role |
|---------|----------|------|
| [`@42ch/spoke-schemas`](https://www.npmjs.com/package/@42ch/spoke-schemas) | npm | Generated TypeScript wire types — **what** crosses the wire |
| [`@42ch/spoke-operations`](https://www.npmjs.com/package/@42ch/spoke-operations) | npm | Pure helpers, adapter ports, and orchestration over those types |
| [`spoke-schemas`](https://crates.io/crates/spoke-schemas) | crates.io | Generated Rust wire types |
| [`spoke-operations`](https://crates.io/crates/spoke-operations) | crates.io | Pure helpers, adapter ports, and orchestration — parity with `@42ch/spoke-operations` |

Product-specific payloads live under `extensions.<namespace>` (namespace keys are product-chosen ids). Cross-product functional dialects shared by narrative hosts (lore activation, knowledge packs, assemble placement) live under `modules.*` — an optional, capability-flagged bag on KnowledgeEntry and AssemblePacket. See [`spoke-extension-modules.md`](.mstar/specs/spoke-extension-modules.md).

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
  HostCapabilityManifest,
} from "@42ch/spoke-schemas";
```

**`@42ch/spoke-operations`** — implement capability-sliced ports on one adapter, then call `orchestrate*`:

```ts
import type { PromoteRequest, UpsertRequest } from "@42ch/spoke-schemas";
import {
  orchestrateUpsert,
  orchestratePromote,
  orchestrateCheck,
  orchestrateAssemble,
  type BaselineAdapter,
} from "@42ch/spoke-operations";

declare const adapter: BaselineAdapter; // product implements BaselineAdapter / FullAdapter
declare const upsertRequest: UpsertRequest;
declare const promoteRequest: PromoteRequest;

const upserted = orchestrateUpsert(adapter, upsertRequest);
const promoted = orchestratePromote(adapter, promoteRequest);
```

Optional capabilities use `ComputableAdapter` / `ForkAdapter` (or `FullAdapter`) with `orchestrateProject`, `orchestrateCompute`, `orchestrateForkCheck`, and `orchestrateForkAssemble`. Pure helpers (`validatePromoteRequest`, `mergeExtensionMaps`, `buildAssemblePacket`, …) remain available for focused gates.

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
use spoke_schemas::{KnowledgeEntry, HostCapabilityManifest, PromoteRequest, TimelineEvent};
```

**`spoke-operations`** — port traits plus `orchestrate_*` (also re-exports `spoke_schemas`):

```rust
use spoke_operations::{
    orchestrate_promote, orchestrate_upsert, BaselineAdapter,
};
use spoke_operations::spoke_schemas::{PromoteRequest, UpsertRequest};

fn run_baseline(adapter: &impl BaselineAdapter, upsert: UpsertRequest, promote: PromoteRequest) {
    let _ = orchestrate_upsert(adapter, upsert);
    let _ = orchestrate_promote(adapter, promote);
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

Integrator path: one adapter type implements the port families, then call `orchestrate*` from `@42ch/spoke-operations` (same shape in Rust as `orchestrate_*`).

```typescript
import type { KnowledgeEntry, PromoteRequest } from "@42ch/spoke-schemas";
import {
  orchestratePromote,
  type BaselineAdapter,
} from "@42ch/spoke-operations";

// Product adapter implements BaselineAdapter.
// Reference FullAdapter: fixtures/toy-world ToyWorldAdapter
declare const adapter: BaselineAdapter;

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
const result = orchestratePromote(adapter, request);

if (result.ok) {
  // Confirmed entry persisted through adapter OCC ports
} else {
  console.error(result.code, result.message);
}
```

Walk the committed “Mira at Harbor” graph and Vitest/Cargo demos in [`fixtures/toy-world/`](fixtures/toy-world/):

```bash
pnpm run test:fixtures
cargo test -p spoke-fixture-toy-world
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
| **HostCapabilityManifest** | Host roles, capabilities, and owned `namespaces[]` for in-process collaboration |
| **Extensions** | Product-specific bag on every data object (`extensions.<namespace>`) |
| **Modules** | Optional cross-product functional-dialect bag on KnowledgeEntry + AssemblePacket (capability-flagged `narrative-modules`) |
| **Adapter ports** | Injected read/write surfaces (`KnowledgeEntryPort`, `HostManifestPort`, …) that own persistence |
| **Orchestration** | `orchestrate*` / `orchestrate_*` sequences that load scope, apply gates, and persist via ports |

Vocabulary and positioning: [`CONCEPTS.md`](CONCEPTS.md), [`STRATEGY.md`](STRATEGY.md).

## Optional capabilities

Products that need programmable KnowledgeEntry body state may declare **`l2-computable`**:

- **`body.state`** — static durable computable values
- **`body.computable`** — dynamic Session-scoped projection
- **`TimelineEvent.computable_logs`** — Moment-scale field-change presentation
- **`project` / `compute` ops** — init/projection and apply/settle I/O envelopes

Products that need fork-scoped timeline queries may declare **`l5-fork`**. Products that exchange cross-product functional dialects (lore activation, knowledge packs, assemble placement / activation trace) may declare **`narrative-modules`**: an optional `modules` (`ModuleMap`) bag on KnowledgeEntry and AssemblePacket carries these dialects, and adapters round-trip unknown module namespaces verbatim. Domain Profile handbooks define the inner shapes. Composed adapter aliases: `BaselineAdapter`, `ComputableAdapter`, `ForkAdapter`, `FullAdapter`.

Baseline integrators use core schemas; optional capabilities are opt-in. Normative detail: [`.mstar/specs/spoke-protocol-layers.md`](.mstar/specs/spoke-protocol-layers.md) §Capability levels.

## Operations

`@42ch/spoke-operations` / `spoke-operations` provide pure helpers and port-injected orchestration:

- **Baseline orchestrators:** `orchestrateUpsert`, `orchestratePromote`, `orchestrateRelate`, `orchestrateCheck`, `orchestrateAssemble`
- **Optional orchestrators:** `orchestrateProject`, `orchestrateCompute`, `orchestrateForkCheck`, `orchestrateForkAssemble`
- Capability-sliced ports and composed aliases (`BaselineAdapter` … `FullAdapter`)
- Extension and module map merge and round-trip preservation (`mergeExtensionMaps` / `mergeModuleMaps`, `preserveExtensionMaps` / `preserveModuleMaps`)
- Finding / KnowledgeEntry `status` transition helpers
- Promote acceptance and upsert/relate validators
- AssemblePacket builders from KnowledgeEntries
- Unified `SpokeResult` / `SpokeRejectCode` on reject paths

Reference **FullAdapter** (baseline + `l2-computable` + `l5-fork`, including `HostCapabilityManifest` peers): [`fixtures/toy-world/`](fixtures/toy-world/) — TypeScript `ToyWorldAdapter`, Rust crate `spoke-fixture-toy-world`.

Normative detail: [`.mstar/specs/spoke-operations.md`](.mstar/specs/spoke-operations.md).

## Specs and schemas

| Path | Topic |
|------|-------|
| [`schemas/`](schemas/) | JSON Schema SSOT (Draft-07) |
| [`fixtures/toy-world/`](fixtures/toy-world/) | Protocol conformance JSON graph (“Mira at Harbor”) + reference adapters |
| [`.mstar/specs/spoke-protocol.md`](.mstar/specs/spoke-protocol.md) | Umbrella protocol spec |
| [`.mstar/specs/spoke-protocol-layers.md`](.mstar/specs/spoke-protocol-layers.md) | Nine layers (L0–L8), capability levels, Timeline tiers |
| [`.mstar/specs/spoke-data-model.md`](.mstar/specs/spoke-data-model.md) | Data objects and open vocabulary |
| [`.mstar/specs/spoke-ops.md`](.mstar/specs/spoke-ops.md) | Ops wire request/response envelopes |
| [`.mstar/specs/spoke-operations.md`](.mstar/specs/spoke-operations.md) | Operations library behavior |
| [`.mstar/specs/spoke-extension-modules.md`](.mstar/specs/spoke-extension-modules.md) | Core / modules / extensions triad (bag placement) |

## Contributing

Local development, CI gates, and release procedure: [`CONTRIBUTING.md`](CONTRIBUTING.md).
