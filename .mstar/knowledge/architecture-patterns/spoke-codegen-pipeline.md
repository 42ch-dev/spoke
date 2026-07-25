# SPOKE codegen pipeline (v0.1)

**Category:** architecture-patterns  
**Source:** compound 2026-07-23 (bootstrap); inventory + Rust dup strategy 2026-07-25  
**Status:** durable

## Problem

Hand-authored JSON Schema must produce both TypeScript (`@42ch/spoke-schemas`) and Rust (`spoke-schemas`) without drift. Two generators (jstt + typify) have different `$ref` / naming constraints; a soft-fail Rust path can exit 0 with a partial tree.

## Decision

1. **SSOT** — only `schemas/**/*.schema.json` are hand-authored; generated trees are committed.
2. **Orchestrator** — `tooling/codegen` walks schemas, localizes `$ref`s for typify, emits mirrored `generated/{common,data,ops}/` in both packages.
3. **Verify** — `pnpm run verify-codegen` = regenerate → `node tooling/codegen/assert-schema-count.mjs` (`EXPECTED_SCHEMA_COUNT = 23`) → `git diff --exit-code` on generated dirs. Bump the constant when adding/removing `schemas/**/*.schema.json` (same commit as schema + generated output).
4. **Rust fail-fast** — `rust-gen` returns non-zero on per-schema failure and asserts exactly **23** output files (keep in sync with the TS assert constant when the schema inventory changes).
5. **Closed ops responses** — mutually exclusive success/error shapes use draft-07 `oneOf` (see `assemble-response.schema.json`).
6. **Opaque JSON** — `#/definitions/OpaqueJson` (empty schema `{}`) with `$ref` on consuming properties (e.g. `ComputableLogChange.previous` / `.next`). Generators emit any-JSON types (`unknown` / `OpaqueJson` in TS; `serde_json::Value` in Rust).
7. **Duplicate generated types** — typify and jstt may emit duplicate nominal structs when the same definition is inlined into `common/` and `data/` modules after `$ref` dereference. Integrators import canonical types from `generated/common` (TS barrel or `spoke_schemas::generated::common` / crate root re-exports). Duplicates are generator output, not separate wire shapes.

## What not to do

- Do not invent a custom codegen engine.
- Do not warn-and-skip failed schemas (CI would go green on partial output).
- Do not `as`-cast between duplicate generated structs; convert via JSON round-trip if needed.

## Related

- Specs: `.mstar/specs/spoke-protocol.md`
- Workflow: `.github/workflows/ci.yml` (`verify-codegen` job)
