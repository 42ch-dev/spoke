# SPOKE

[![CI](https://github.com/42ch-dev/spoke/actions/workflows/ci.yml/badge.svg)](https://github.com/42ch-dev/spoke/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Node](https://img.shields.io/badge/node-%3E%3D20-brightgreen.svg?logo=nodedotjs&logoColor=white)](package.json)
[![pnpm](https://img.shields.io/badge/pnpm-%3E%3D8-F69220.svg?logo=pnpm&logoColor=white)](package.json)
[![TypeScript](https://img.shields.io/badge/TypeScript-generated-3178C6.svg?logo=typescript&logoColor=white)](packages/spoke-schemas)
[![Rust](https://img.shields.io/badge/Rust-generated-DEA584.svg?logo=rust&logoColor=black)](crates/spoke-schemas)
[![Schema](https://img.shields.io/badge/JSON%20Schema-SSOT-0B7285.svg)](schemas)
[![Version](https://img.shields.io/badge/version-0.1.0-alpha.2-informational.svg)](package.json)
[![Last commit](https://img.shields.io/github/last-commit/42ch-dev/spoke)](https://github.com/42ch-dev/spoke/commits/main)
[![Greptile: The War on Bugs](https://www.greptile.com/badge.svg)](https://www.greptile.com/?utm_source=oss_badge&utm_medium=readme&utm_campaign=greptile_for_open_source)

[中文](README_CN.md) · [Concepts](CONCEPTS.md) · [Strategy](STRATEGY.md)

**Standardized Programmable Ontology Knowledge Engine** — a protocol repository of JSON Schema wire contracts for narrative **KnowledgeEntry** data and **ops**. Independent products exchange consistency-check and context-assembly I/O through these shapes.

**Includes:**

- Data-layer schemas: KnowledgeEntry, Relation, SourceAnchor, Finding, AssemblePacket, Rule, TimelineEvent
- Ops-layer schemas: `upsert`, extract→promote, `relate`, `check`, `assemble`; optional **`project` / `compute`** under `l2-computable`
- Generated TypeScript (`@42ch/spoke-schemas`) and Rust (`spoke-schemas`, `spoke-operations`)
- Pure lifecycle helpers (`@42ch/spoke-operations` / `spoke-operations`)
- Protocol conformance fixtures ([`fixtures/toy-world/`](fixtures/toy-world/))

## Packages

| Package | Role |
|---------|------|
| [`@42ch/spoke-schemas`](packages/spoke-schemas/) | Generated TypeScript types from JSON Schema — **what** crosses the wire |
| [`@42ch/spoke-operations`](packages/spoke-operations/) | Hand-written pure helpers — promote gates, Finding transitions, extension merge, AssemblePacket construction |
| `spoke-schemas` (Rust crate) | Generated Rust types in [`crates/spoke-schemas/`](crates/spoke-schemas/) |
| `spoke-operations` (Rust crate) | Hand-written pure helpers in [`crates/spoke-operations/`](crates/spoke-operations/) — parity with `@42ch/spoke-operations` |

Product-specific payloads live under `extensions.<namespace>` (namespace keys are product-chosen ids).

## Install and consume

Install from npm and crates.io at the lockstep SemVer `X.Y.Z`:

```bash
pnpm add @42ch/spoke-schemas@X.Y.Z @42ch/spoke-operations@X.Y.Z
```

```toml
# Cargo.toml
spoke-schemas = "X.Y.Z"
spoke-operations = "X.Y.Z"
```

In a pnpm monorepo that vendors SPOKE as a workspace member:

```json
{
  "dependencies": {
    "@42ch/spoke-schemas": "workspace:*",
    "@42ch/spoke-operations": "workspace:*"
  }
}
```

From another repo, depend on a local checkout:

```json
{
  "dependencies": {
    "@42ch/spoke-schemas": "file:../spoke/packages/spoke-schemas",
    "@42ch/spoke-operations": "file:../spoke/packages/spoke-operations"
  }
}
```

Then build (`pnpm install` at the SPOKE root, then `pnpm --filter @42ch/spoke-schemas build` and `pnpm --filter @42ch/spoke-operations build`).

## Version and pinning

SPOKE publishes a **single lockstep SemVer** (`X.Y.Z`) across all workspace packages and Rust consumer crates (`spoke-schemas`, `spoke-operations`). Pin a consumer repo to an annotated git tag:

```bash
git checkout vX.Y.Z
```

Or download the source archive from the matching [GitHub Release](https://github.com/42ch-dev/spoke/releases). Release notes live in [`CHANGELOG.md`](CHANGELOG.md) at the tagged commit. Then depend via `file:` path (above) or a git dependency:

```json
{
  "dependencies": {
    "@42ch/spoke-schemas": "github:42ch-dev/spoke#vX.Y.Z",
    "@42ch/spoke-operations": "github:42ch-dev/spoke#vX.Y.Z"
  }
}
```

The shields.io **Version** badge at the top of this README reflects the canonical version on the checked-out commit. Normative policy: [`.mstar/specs/spoke-version-release.md`](.mstar/specs/spoke-version-release.md).

## Release (maintainers)

Primary path — **New release** on GitHub Actions, then merge the PR:

1. Open [Actions → New release](https://github.com/42ch-dev/spoke/actions/workflows/new-release.yml) → **Run workflow**.
2. Set **version** to the lockstep SemVer (e.g. `0.1.0-alpha.3`). Optional summary fills the PR body.
3. Merge the opened PR (keep the `release` label). CI creates annotated tag `vX.Y.Z` and runs [**Release**](https://github.com/42ch-dev/spoke/actions/workflows/release.yml): verify gates, GitHub Release from [`CHANGELOG.md`](CHANGELOG.md), and on stable / alpha tags (no `-rc.` suffix) publish npm + crates.io.

[`CHANGELOG.md`](CHANGELOG.md) is the release-notes source (Keep a Changelog via [git-cliff](https://git-cliff.org)). Preview locally: `pnpm run release:changelog -- --unreleased`.

Pre-release tags `vX.Y.Z-rc.N` create a GitHub pre-release. Tags without `-rc.` publish to npm and crates.io.

### Registry auth (maintainers)

| Auth | Use |
|------|-----|
| npm **Trusted Publisher** | OIDC from `release.yml` → `publish-npm` (each package: org `42ch-dev`, repo `spoke`, workflow `release.yml`) |
| `CARGO_REGISTRY_TOKEN` | crates.io auth for `cargo publish` (GitHub repository secret) |

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

Other exports include `buildAssemblePacket`, `transitionFindingStatus`, and `mergeExtensionMaps` — see [`spoke-operations.md`](.mstar/specs/spoke-operations.md).

## Operations

`@42ch/spoke-operations` provides pure, cross-product lifecycle helpers:

- Extension map merge and round-trip preservation
- Finding `status` transition validation and apply
- Promote acceptance checks (gate before persist)
- AssemblePacket builders from KnowledgeEntries
- Unified `SpokeResult` / `SpokeRejectCode` on reject paths

Normative detail: [`.mstar/specs/spoke-operations.md`](.mstar/specs/spoke-operations.md).

## Specs and schemas

| Path | Topic |
|------|-------|
| [`schemas/`](schemas/) | JSON Schema SSOT (Draft-07) — source for codegen |
| [`fixtures/toy-world/`](fixtures/toy-world/) | Protocol conformance JSON graph ("Mira at Harbor") — CI schema-validated |
| [`.mstar/specs/spoke-protocol.md`](.mstar/specs/spoke-protocol.md) | Umbrella protocol spec |
| [`.mstar/specs/spoke-protocol-layers.md`](.mstar/specs/spoke-protocol-layers.md) | Nine layers (L0–L8), capability levels, Timeline tiers |
| [`.mstar/specs/spoke-data-model.md`](.mstar/specs/spoke-data-model.md) | Data objects and open vocabulary |
| [`.mstar/specs/spoke-ops.md`](.mstar/specs/spoke-ops.md) | Ops wire request/response envelopes |
| [`.mstar/specs/spoke-operations.md`](.mstar/specs/spoke-operations.md) | Operations library behavior |

## Contributing and CI

Pull requests must pass GitHub Actions jobs `verify-codegen`, `typescript`, `rust`, and `verify-version` ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)). Schema changes require regenerated output in the same commit (`pnpm run codegen`).
