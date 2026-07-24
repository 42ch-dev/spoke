# Lockstep SemVer release (git tag + GitHub Release)

**Category:** architecture-patterns  
**Source:** compound 2026-07-25 (version-release)  
**Status:** durable

## Problem

Integrators pin SPOKE across TypeScript packages, Rust crate, and human docs. Workspace packages are private (no npm/crates.io publish). Without a single version identity and a tag-driven release path, sibling repos cannot distinguish packaging cuts or pin reproducibly.

## Decision

1. **Lockstep SemVer** — one `X.Y.Z` across nine consumer pin surfaces. Canonical source: root `package.json` → `version`. SSOT manifest: `tooling/release/lockstep-surfaces.mjs`; assert: `tooling/release/assert-lockstep-version.mjs` (`pnpm run verify:version`).
2. **CI drift gate** — `verify-version` job in `.github/workflows/ci.yml` on PR/push to `main` and `iteration/**`. Any manifest or README badge mismatch fails the build (no warn-only path).
3. **Tag-triggered release** — `.github/workflows/release.yml` on `push.tags: ['v*']`. Four parallel verify jobs (including `verify-version`) → sequential `release` job with `needs:` (fail-closed). Creates GitHub Release from annotated tag; no registry publish steps.
4. **Annotated tags** — form `vX.Y.Z` (leading `v` required). Pre-release: `vX.Y.Z-rc.N` → GitHub pre-release. Release notes: tag annotation first (`git tag -l --format='%(contents)'`); one-line fallback when empty.
5. **Operator bump** — `pnpm run release:bump -- X.Y.Z` writes all lockstep surfaces + README shields.io badges, then runs assert. Optional `--tag [message]` creates a **local** annotated tag only when the tree is clean and already at target version; script never pushes.
6. **`--tag` deferral** — if bumping or working tree is dirty, `--tag` is refused (non-zero exit) with printed instructions: commit first, then re-run the same version with `--tag`.
7. **Package SemVer vs wire `schema_version`** — independent axes. Package bumps track packaging identity; wire `schema_version` is an integer on durable JSON objects (`common.schema.json`). Couple only when release notes say so.
8. **Consumer pinning** — git tag checkout, GitHub Release source archive, pnpm `file:` path, or git dependency at tag (e.g. `github:org/spoke#vX.Y.Z`). Packages: `@42ch/spoke-schemas`, `@42ch/spoke-operations`, `@42ch/spoke-fixture-toy-world`; Rust crate `spoke-schemas`.

### Lockstep surfaces (assert rows 1–9)

| # | Surface |
|---|---------|
| 1 | Root `package.json` → `version` (canonical) |
| 2–5 | `packages/spoke-schemas`, `packages/spoke-operations`, `fixtures/toy-world`, `tooling/codegen` → `package.json` `version` |
| 6–7 | `Cargo.toml` `[workspace.package].version`; `crates/spoke-schemas/Cargo.toml` with `version.workspace = true` |
| 8–9 | `README.md` and `README_CN.md` shields.io `version-X.Y.Z` badge URLs |

**Excluded:** `tooling/codegen/rust-gen/Cargo.toml` (standalone bin workspace; not a pin surface).

### Maintainer happy path

```bash
pnpm run release:bump -- 0.1.1
git add -A && git commit -m "chore(release): bump to 0.1.1"
git push origin main
pnpm run release:bump -- 0.1.1 --tag "Release 0.1.1"
git push origin v0.1.1
```

Tag push re-runs verify-equivalent gates; on success, workflow creates the GitHub Release.

## What not to do

- Do not add npm or crates.io publish jobs to CI.
- Do not use independent per-package SemVer channels.
- Do not auto-bump or auto-tag on every merge to `main`.
- Do not create lightweight-only tags (annotation required for release notes).
- Do not conflate package SemVer with wire `schema_version` in docs or tooling.

## Related

- Normative policy: `.mstar/specs/spoke-version-release.md`
- Strategy: `STRATEGY.md` (no registry publish)
- Workflows: `.github/workflows/ci.yml`, `.github/workflows/release.yml`
- Tooling: `tooling/release/lockstep-surfaces.mjs`, `assert-lockstep-version.mjs`, `bump-version.mjs`
- Consumer twin READMEs: `README.md`, `README_CN.md`
