# Fixture AJV harness outside package dist

> **Category:** architecture-patterns  
> **Source:** compound 2026-07-23 (fixtures-conformance); boundary correction 2026-07-23  
> **Context:** `fixtures/toy-world/` protocol JSON + `@42ch/spoke-fixture-toy-world` workspace package

## Problem

Protocol conformance fixtures need AJV validation in CI. Colocating the harness under `packages/spoke-operations/src/` made Vitest pass locally but broke `tsc`/`build` (missing Node types, AJV ESM imports) and risked shipping test I/O into `dist/`.

## Pattern

1. Keep protocol JSON at repo-root `fixtures/toy-world/` (committed graph only — no product DTO maps).
2. Own the AJV/Vitest harness in `fixtures/toy-world/tests/` as workspace package **`@42ch/spoke-fixture-toy-world`** (`fixtures/toy-world/package.json`).
3. **`@42ch/spoke-operations` stays pure library** — no `src/fixtures/**`, no AJV, no `node:fs` in the operations package graph.
4. Wire root `pnpm run test:fixtures` (or `pnpm -F @42ch/spoke-fixture-toy-world test`) into CI `typescript` job **before** `@42ch/spoke-operations` build.
5. Use Ajv v8 ESM imports (`import Ajv from "ajv"`) with `ajv-formats` interop; resolve schemas from repo-root `schemas/`.
6. Fixtures package MAY import `@42ch/spoke-operations` for optional helper smoke tests; operations MUST NOT import fixtures or host fixture validation I/O.

## Layout

```text
fixtures/toy-world/
├── *.json                 # protocol conformance graph
├── package.json           # @42ch/spoke-fixture-toy-world
├── vitest.config.ts
├── README.md
└── tests/                 # AJV harness (not under packages/spoke-operations/)
    └── validate-fixtures.test.ts
```

## What failed (pre-boundary)

- Relying on `pnpm test` inside `spoke-operations` without excluding harness from `tsconfig.build.json` — typecheck/build still failed in GitHub `typescript` job.
- Treating `packages/spoke-operations/src/fixtures/` as canonical — wrong ownership; harness belongs with fixture JSON.

## See also

- `fixtures/toy-world/README.md` — validate locally (`pnpm run test:fixtures`)
- [`spoke-protocol.md`](../../specs/spoke-protocol.md) — repository layout row (harness MUST NOT live under `packages/spoke-operations/`)
- [`architecture-patterns/spoke-operations-pure-actions.md`](spoke-operations-pure-actions.md) — pure helpers over wire types
