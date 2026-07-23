# SPOKE codegen pipeline (v0.1)

**Category:** architecture-patterns  
**Source:** iteration:v0.1 (core-bootstrap + QC fix wave)  
**Status:** durable

## Problem

Hand-authored JSON Schema must produce both TypeScript (`@42ch/spoke-schema`) and Rust (`spoke-schema`) without drift. Two generators (jstt + typify) have different `$ref` / naming constraints; a soft-fail Rust path can exit 0 with a partial tree.

## Decision

1. **SSOT** — only `schemas/**/*.schema.json` are hand-authored; generated trees are committed.
2. **Orchestrator** — `tooling/codegen` walks schemas, localizes `$ref`s for typify, emits mirrored `generated/{common,data,ops}/` in both packages.
3. **Verify** — `pnpm run verify-codegen` = regenerate + `git diff --exit-code` on generated dirs.
4. **Rust fail-fast** — `rust-gen` returns non-zero on per-schema failure and asserts exactly **17** output files.
5. **Closed ops responses** — mutually exclusive success/error shapes use draft-07 `oneOf` (see `assemble-response.schema.json`).

## What not to do

- Do not invent a custom codegen engine.
- Do not warn-and-skip failed schemas (CI would go green on partial output).
- Do not treat typify-inlined types as interchangeable across modules (known residual: nominal duplication after dereference).

## Related

- Specs: `.mstar/specs/spoke-protocol.md`
- Workflow: `.github/workflows/ci.yml` (`verify-codegen` job)
- Residual R1: Rust nominal type duplication (deferred)
