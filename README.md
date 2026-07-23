# SPOKE

[中文](README_CN.md)

**Standardized Programmable Ontology Keyblock Engine** — a protocol repository of JSON Schema wire contracts for narrative **Keyblock** data and **ops**. Products such as Nexus and Creader use these shapes for consistency-check and context-assembly I/O across independent stacks.

**Includes:** data-layer schemas (Keyblock, Relation, SourceAnchor, Finding, AssemblePacket); ops-layer schemas (`upsert`, extract→promote, `relate`, `check`, `assemble`); generated TypeScript (`@42ch/spoke-schema`) and Rust (`spoke-schema`); pure lifecycle helpers (`@42ch/spoke-operations`).

## Packages

| Package | Role |
|---------|------|
| [`@42ch/spoke-schema`](packages/spoke-schema/) | Generated TypeScript types from JSON Schema — **what** crosses the wire |
| [`@42ch/spoke-operations`](packages/spoke-operations/) | Hand-written pure helpers — promote gates, Finding transitions, extension merge, AssemblePacket construction |
| `spoke-schema` (Rust crate) | Generated Rust types in [`crates/spoke-schema/`](crates/spoke-schema/) |

Product-specific payloads live under `extensions.<namespace>` (e.g. `extensions.nexus`, `extensions.creader`).

## Install and consume

Packages are **workspace-local** (private). In a pnpm monorepo:

```json
{
  "dependencies": {
    "@42ch/spoke-schema": "workspace:*",
    "@42ch/spoke-operations": "workspace:*"
  }
}
```

From another repo, depend on a local checkout:

```json
{
  "dependencies": {
    "@42ch/spoke-schema": "file:../spoke/packages/spoke-schema",
    "@42ch/spoke-operations": "file:../spoke/packages/spoke-operations"
  }
}
```

Then build (`pnpm install` at the SPOKE root, then `pnpm --filter @42ch/spoke-schema build` and `pnpm --filter @42ch/spoke-operations build`).

## Core concepts

| Term | In SPOKE |
|------|----------|
| **Keyblock** | Atomic narrative knowledge unit on the wire (`keyblock_id`, `block_type`, `status`, `body`, `extensions`) |
| **Relation** | Directed edge between Keyblocks (or Keyblock ↔ source) |
| **SourceAnchor** | Provenance pointer to a manuscript span or external locator |
| **Finding** | Checker output for consistency, style, or analysis |
| **AssemblePacket** | Wire context-assembly payload (slim entries for downstream LLM prompts) |
| **Extensions** | Product-specific bag on every data object (`extensions.<namespace>`) |

Vocabulary and positioning: [`CONCEPTS.md`](CONCEPTS.md), [`STRATEGY.md`](STRATEGY.md).

## Quick start

```typescript
import type { Keyblock, PromoteRequest } from "@42ch/spoke-schema";
import { validatePromoteRequest } from "@42ch/spoke-operations";

const candidate: Keyblock = {
  schema_version: 1,
  keyblock_id: "kb_01",
  block_type: "character",
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
- AssemblePacket builders from Keyblocks
- Unified `SpokeResult` / `SpokeRejectCode` on reject paths

Normative detail: [`.mstar/specs/spoke-operations.md`](.mstar/specs/spoke-operations.md).

## Specs and schemas

| Path | Topic |
|------|-------|
| [`schemas/`](schemas/) | JSON Schema SSOT (Draft-07) — source for codegen |
| [`.mstar/specs/spoke-protocol.md`](.mstar/specs/spoke-protocol.md) | Umbrella protocol spec |
| [`.mstar/specs/spoke-data-model.md`](.mstar/specs/spoke-data-model.md) | Data objects and open vocabulary |
| [`.mstar/specs/spoke-ops.md`](.mstar/specs/spoke-ops.md) | Ops wire request/response envelopes |
| [`.mstar/specs/spoke-operations.md`](.mstar/specs/spoke-operations.md) | Operations library behavior |

## Contributing and CI

Pull requests must pass GitHub Actions jobs `verify-codegen`, `typescript`, and `rust` ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)). Schema changes require regenerated output in the same commit (`pnpm run codegen`).
