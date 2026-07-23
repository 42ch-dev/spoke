# Fixture AJV harness outside package dist

> **Category:** architecture-patterns  
> **Source:** iteration:v0-iter004  
> **Context:** `fixtures/toy-world` + `@42ch/spoke-operations` Vitest

## Problem

Protocol conformance fixtures need AJV validation in CI. Putting the validator under `packages/spoke-operations/src/` made Vitest green but broke `tsc`/`build` (missing Node types, AJV ESM imports) and risked shipping test I/O into `dist/`.

## Pattern

1. Keep protocol JSON at repo-root `fixtures/toy-world/`.
2. Colocate AJV harness under `src/fixtures/**` for Vitest convenience.
3. **Exclude** `src/fixtures/**` from `tsconfig.build.json` so dist stays pure library.
4. Add `@types/node` when using `node:fs` in the harness.
5. Use Ajv v8 ESM imports (`import { Ajv } from "ajv"`) with `ajv-formats` interop.
6. Wire `pnpm run ci:typescript` to run the package tests (already includes fixture suite); optional `test:fixtures` script that points at the single conformance file.

## What failed

- Relying on `pnpm test` alone without `ci:typescript` — typecheck/build still failed in GitHub `typescript` job.
- Shipping harness modules in the published package graph.

## See also

- `packages/spoke-operations/src/fixtures/`
- `fixtures/toy-world/README.md`
