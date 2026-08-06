# Lockstep SemVer release (git tag + GitHub Release)

**Category:** architecture-patterns  
**Source:** compound 2026-07-25 (version-release)  
**Status:** durable

## Problem

Integrators pin SPOKE across TypeScript packages, Rust crates, and human docs. Without a single version identity, a tag-driven release path, and registry artifacts at the same SemVer, sibling repos cannot install or pin reproducibly.

## Decision

1. **Lockstep SemVer** — one `X.Y.Z` across ten consumer pin surfaces. Canonical source: root `package.json` → `version`. SSOT manifest: `tooling/release/lockstep-surfaces.mjs`; assert: `tooling/release/assert-lockstep-version.mjs` (`pnpm run verify:version`).
2. **CI drift gate** — `verify-version` job in `.github/workflows/ci.yml` on PR/push to `main` and `iteration/**`. Any lockstep manifest mismatch fails the build (no warn-only path). README Version badges are dynamic GitHub Releases shields (presence-checked, not version-locked).
3. **Tag-triggered release** — `.github/workflows/release.yml` is top-level only (`push.tags: ['v*']` or `pull_request` closed + label `release`). No `workflow_call` / no `workflow_dispatch` — Trusted Publishing OIDC binds to filename `release.yml`. Verify jobs → GitHub Release → npm (Node 24 + `registry-url`, stock npm) → crates.io (`crates-io-auth-action`); schemas before ops in each registry.
4. **Annotated tags** — form `vX.Y.Z` (leading `v` required). Pre-release: `vX.Y.Z-rc.N` → GitHub pre-release. Release notes extraction order: matching `CHANGELOG.md` section first (`extract-changelog-notes.mjs`); tag annotation fallback (`git tag -l --format='%(contents)'`); one-line `Release vX.Y.Z` when both are empty.
5. **Operator bump** — `pnpm run release:bump -- X.Y.Z` writes all lockstep surfaces + README shields.io badges, regenerates `CHANGELOG.md` via git-cliff, then runs assert. Optional `--tag [message]` creates a **local** annotated tag only when the tree is clean and already at target version; script never pushes.
6. **`--tag` deferral** — if bumping or working tree is dirty, `--tag` is refused (non-zero exit) with printed instructions: commit first, then re-run the same version with `--tag`.
7. **Package SemVer vs wire `schema_version`** — independent axes. Package bumps track packaging identity; wire `schema_version` is an integer on durable JSON objects (`common.schema.json`). Couple only when release notes say so.
8. **Consumer pinning** — npm/crates.io at `X.Y.Z`, git tag checkout, GitHub Release source archive, pnpm `file:` path, or git dependency at tag (e.g. `github:42ch-dev/spoke#vX.Y.Z`). Packages: `@42ch/spoke-schemas`, `@42ch/spoke-operations`, `@42ch/spoke-fixture-toy-world`; Rust crate `spoke-schemas`.

### Lockstep surfaces (assert rows 1–10)

| # | Surface |
|---|---------|
| 1 | Root `package.json` → `version` (canonical) |
| 2–5 | `packages/spoke-schemas`, `packages/spoke-operations`, `fixtures/toy-world`, `tooling/codegen` → `package.json` `version` |
| 6 | `Cargo.toml` `[workspace.package].version` |
| 7 | `crates/spoke-schemas/Cargo.toml` with `version.workspace = true` |
| 8 | `crates/spoke-operations/Cargo.toml` with `version.workspace = true` |
| 9–10 | `README.md` and `README_CN.md` shields.io `version-X.Y.Z` badge URLs |

**Ops crate dependency pin (row 8 manifest):** `spoke-operations` MUST declare `spoke-schemas = { version = "X.Y.Z", path = "../spoke-schemas" }` — not path-only. Bump script rewrites the `version` key via `formatOpsSpokeSchemasDependency` / `replaceOpsSpokeSchemasDependencyVersion` in `lockstep-surfaces.mjs`.

**Excluded:** `tooling/codegen/rust-gen/Cargo.toml` (standalone bin workspace; not a pin surface).

### Maintainer happy path

```bash
pnpm run release:bump -- 0.1.1
git add -A && git commit -m "chore(release): bump to 0.1.1"
git push origin main
pnpm run release:bump -- 0.1.1 --tag "Release 0.1.1"
git push origin v0.1.1
```

Tag push re-runs verify-equivalent gates; on success, workflow creates the GitHub Release and publishes registry artifacts (stable tags only).

## What not to do

- Do not use independent per-package SemVer channels.
- Do not auto-bump or auto-tag on every merge to `main`.
- Do not create lightweight-only tags (annotation required for release notes).
- Do not conflate package SemVer with wire `schema_version` in docs or tooling.
- Do not hardcode live lockstep SemVer in `bump-version` unit tests that seed fixtures from `package.json` — see `testing-patterns/release-bump-tests-version-agnostic.md`.

## Workspace crates in the lockstep surface

All workspace crates participate in lockstep version assertions and bumps alongside published crates. The private fixture crate `spoke-fixture-toy-world` (`publish = false`) and the published crate `spoke-connect` are both registered in `tooling/release/lockstep-surfaces.mjs` `CARGO_LOCK_PACKAGE_NAMES` and `assert-lockstep-version.mjs` manifest coverage; `bump-version.mjs` generalizes path-dependency version rewriting so a crate's `spoke-schemas` path-dep requirement advances with the workspace bump. Release tests include a fixture containing a workspace crate and assert both bump and drift detection. Rationale: a crate drifting from the workspace version silently breaks path-dep resolution on a future minor bump and leaves the lockfile stale; the same locks…

**Every pinned inter-crate path dependency is a lockstep surface, including optional deps.** The v0.9.0 release broke in the Release `rust` job because the lockstep bump advanced `spoke-schemas` path-dep pins but left `spoke-connect`'s optional `spoke-operations = { version = "0.8.2", path = … }` behind: Cargo could not resolve `^0.8.2` against the 0.9.0 workspace (`failed to select a version for the requirement`). `tooling/release/lockstep-surfaces.mjs` now enumerates every inter-crate path-dependency pin (manifest-driven, optional deps included) so `bump-version` rewrites and `assert-lockstep-version` verifies all of them; release tests cover the generalized rewrite. Two companion hardenings shipped with the repair: `tooling/release/publish-npm.sh` is idempotent across job re-runs (skips versions already on the registry), and the published `spoke-connect` crate no longer carries an optional path dep on the unpublished fixture crate (registry resolution cannot fail on a package that never publishes). When a Release publish job partially fails, re-run only the failed jobs — the publish scripts skip versions already on the registry, so re-runs are safe.

## Related

- Normative policy: `.mstar/specs/spoke-version-release.md`
- Rust ops parity + typify helpers: `architecture-patterns/rust-spoke-operations-parity.md`
- Release test pattern: `testing-patterns/release-bump-tests-version-agnostic.md`
- Workflows: `.github/workflows/ci.yml`, `.github/workflows/release.yml`
- Tooling: `tooling/release/lockstep-surfaces.mjs`, `assert-lockstep-version.mjs`, `bump-version.mjs`
- Consumer twin READMEs: `README.md`, `README_CN.md`
