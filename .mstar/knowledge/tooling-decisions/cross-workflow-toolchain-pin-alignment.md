---
module: tooling / CI
date: 2026-08-14
problem_type: tooling-decision
category: tooling-decisions
severity: medium
plan_id: 2026-08-13-pnpm-lockfile-fix
tags: [pnpm, node, ci, workflow, pin, alignment, pull_request, base-branch]
---

# Cross-workflow toolchain pin alignment — pnpm + Node version consistency

## Context

A pnpm version bump (9 → 11) changed where `overrides` are read from (`package.json` → `pnpm-workspace.yaml`). Fixing the local dev environment (pnpm 11) broke CI (pnpm 9) because the CI workflow pinned an older pnpm that couldn't read the new config location.

## Guidance

**Every workflow that runs `pnpm install` needs its own node+pnpm pin aligned.** The CI workflow file for `pull_request` events loads from the **base branch**, not the PR head — so a workflow fix on the PR branch doesn't take effect until merged. This means:

1. When bumping a toolchain version (pnpm, Node, etc.), check **every** workflow file that invokes that tool (`ci.yml`, `docs.yml`, `release.yml`, `new-release.yml`, …).
2. The `pull_request` trigger loads the workflow from the base branch — a PR that changes the workflow won't see its own changes until merged. Use `push` trigger or merge-first-then-verify for workflow changes.
3. Prefer `packageManager` field in `package.json` + `corepack` over per-workflow `version:` pins — single-source the version.

## Why This Matters

The v0-iter036 pnpm fix initially only fixed local (pnpm 11) but broke CI (pnpm 9). The advisory caught the inversion: "fix local, break CI." The correct fix was aligning both environments to the same pnpm version, not dual-sourcing the config.

## When to Apply

- Bumping pnpm, Node, or any toolchain version used in CI
- Adding a new workflow that invokes a shared toolchain
- Debugging CI failures that don't reproduce locally

## See also

- [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) — CI workflow with aligned pnpm 11 + Node 24
- [`.github/workflows/docs.yml`](../../.github/workflows/docs.yml) — Docs workflow with aligned pins
- [`.github/workflows/release.yml`](../../.github/workflows/release.yml) — Release workflow with aligned pins
