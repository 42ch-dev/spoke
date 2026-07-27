# SPOKE codegen pipeline (v0.1)

**Category:** architecture-patterns  
**Source:** compound 2026-07-23 (bootstrap); inventory + Rust dup strategy 2026-07-25  
**Status:** durable

## Problem

Hand-authored JSON Schema must produce both TypeScript (`@42ch/spoke-schemas`) and Rust (`spoke-schemas`) without drift. Two generators (jstt + typify) have different `$ref` / naming constraints; a soft-fail Rust path can exit 0 with a partial tree.

## Decision

1. **SSOT** — only `schemas/**/*.schema.json` are hand-authored; generated trees are committed.
2. **Orchestrator** — `tooling/codegen` walks schemas, localizes `$ref`s for typify, emits mirrored `generated/{common,data,ops}/` in both packages.
3. **Verify** — `pnpm run verify-codegen` = regenerate → `node tooling/codegen/assert-schema-count.mjs` (`EXPECTED_SCHEMA_COUNT = 24`) → `git diff --exit-code` on generated dirs. Bump the constant when adding/removing `schemas/**/*.schema.json` (same commit as schema + generated output).
4. **Rust fail-fast** — `rust-gen` returns non-zero on per-schema failure and asserts exactly **24** output files (keep in sync with the TS assert constant when the schema inventory changes).
5. **Closed ops responses** — mutually exclusive success/error shapes use draft-07 `oneOf` (see `assemble-response.schema.json`).
6. **Opaque JSON** — `#/definitions/OpaqueJson` must be an **empty schema object `{}`** (optional `description` alone is insufficient for jstt — it still emits `{ [k: string]: unknown }` object-index maps). Consuming properties `$ref` that definition (e.g. `ComputableLogChange.previous` / `.next`). Generators then emit any-JSON types (`unknown` / `OpaqueJson` in TS; `serde_json::Value` in Rust).
7. **Duplicate generated types (strategy A)** — document typify nominal duplication as known generator behavior; integrators use canonical `common/` imports (see below). No orchestrator dedupe.
8. **Release tooling tests** — `pnpm run test:release` runs pure unit tests for lockstep assert/bump scripts (temp fixtures; optional `SPOKE_REPO_ROOT` for harness isolation). Wired into the CI `typescript` job.

## Schema inventory

The repository maintains **24** hand-authored `schemas/**/*.schema.json` files (includes `HostCapabilityManifest`). `assert-schema-count.mjs` and `rust-gen` both assert exactly **24** generated output files — bump both constants in the same commit when the inventory changes.

## Rust typify nominal duplication (strategy A)

typify dereferences `$ref` into each per-schema output module. When a shared definition from `common/` is inlined into a `data/` or `ops/` module, typify emits a **second nominal struct** with the same field layout (for example `SourceAnchor` inside `generated/data/timeline_event.rs` alongside the canonical `generated/data/source_anchor.rs`).

Facts for Rust integrators:

1. typify emits duplicate nominal structs when the same definition is inlined into `common/` and `data/` (or `ops/`) modules after dereference.
2. Duplicates are generator artifacts — they serialize the same JSON shape and are **not** separate wire types.
3. Import canonical shared types from `spoke_schemas::generated::common` or crate-root re-exports (for example `use spoke_schemas::Scope`, `ComputableLogChange`). Do **not** use same-named structs from nested inlines inside other `generated::data::*` or `generated::ops::*` files.
4. Duplicate nominal structs are not interchangeable in Rust even when fields match. Convert via `serde_json` round-trip when bridging values; do not `as`-cast between them.

TypeScript (`jstt`) may emit parallel duplicate interfaces; import shared defs from `generated/common` the same way.

## What not to do

- Do not invent a custom codegen engine.
- Do not warn-and-skip failed schemas (CI would go green on partial output).
- Do not `as`-cast between duplicate generated structs; convert via JSON round-trip if needed.

## Related

- Specs: `.mstar/specs/spoke-protocol.md`
- Workflow: `.github/workflows/ci.yml` (`verify-codegen` job)
