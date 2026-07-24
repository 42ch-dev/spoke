# SPOKE Version Release

> **Status:** Normative  
> **Document class:** Policy — monorepo packaging SemVer, git tags, CI-gated GitHub Release  
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)  
> **Strategy:** [`STRATEGY.md`](../../STRATEGY.md)

---

## Purpose

SPOKE publishes a **single lockstep SemVer** for workspace packages and the Rust schema crate so integrators can pin **one version** across TypeScript and Rust artifacts. Releases are cut with **annotated git tags** and materialized as **GitHub Releases**. Registry publish (npm / crates.io) is out of scope and forbidden in CI.

**Integrator value:** pin sibling repos to `vX.Y.Z` (git tag or GitHub Release source archive) and consume workspace packages via `file:` path or git dependency at that tag — without a public registry.

**Maintainer value:** one bump path updates every SSOT surface; CI blocks drift; tagging a green commit produces a Release after gates re-run.

## Version SSOT (lockstep)

All of the following MUST share the same `X.Y.Z` string (no independent channels):

| # | Surface | Path | Assert method |
|---|---------|------|---------------|
| 1 | Workspace root (canonical) | `package.json` → `version` | JSON `version` field |
| 2 | TypeScript schemas | `packages/spoke-schemas/package.json` → `version` | JSON `version` field |
| 3 | TypeScript operations | `packages/spoke-operations/package.json` → `version` | JSON `version` field |
| 4 | Fixture harness | `fixtures/toy-world/package.json` → `version` | JSON `version` field |
| 5 | Codegen runner | `tooling/codegen/package.json` → `version` | JSON `version` field |
| 6 | Rust workspace | `Cargo.toml` → `[workspace.package].version` | TOML parse |
| 7 | Rust schema crate | `crates/spoke-schemas/Cargo.toml` | MUST declare `version.workspace = true`; effective version equals row 6 |
| 8 | README EN badge | `README.md` | shields.io `version-X.Y.Z` regex (see below) |
| 9 | README CN badge | `README_CN.md` | Same regex as row 8 |

**Canonical version source:** row 1 (`package.json` → `version`). The assert script compares every other row to that string.

**Excluded from lockstep** (not in assert script):

| Path | Reason |
|------|--------|
| `tooling/codegen/rust-gen/Cargo.toml` | Standalone `[workspace]` bin crate (`spoke-rust-gen`); not a consumer pin surface; version is local to the codegen tool |

CI **lockstep assert** MUST cover rows 1–9. Drift on any row MUST fail the build (no warn-only path).

## SemVer usage (monorepo)

| Bump | When |
|------|------|
| **PATCH** (`0.1.0` → `0.1.1`) | Bugfix-only packaging release; no wire contract change |
| **MINOR** (`0.1.z` → `0.2.0`) | Backward-compatible wire or ops library additions |
| **MAJOR** (`0.x.y` → `1.0.0`) | Breaking wire or public API change |

Pre-1.0: breaking changes remain allowed per root `AGENTS.md`; still use SemVer strings for packaging identity. Release notes MUST call out wire-impacting changes even when package and `schema_version` move independently.

## Tag convention

| Rule | Value |
|------|-------|
| Form | Annotated tag `vX.Y.Z` (required leading `v`) |
| Target | Commit on `main` (or `iteration/**` integration branch) that already passes CI |
| Annotation | SHOULD include human-readable release summary (primary Release notes source) |
| Notes extraction | `git tag -l --format='%(contents)' <tag>` on the pushed tag; if empty, workflow generates a one-line fallback body (`Release vX.Y.Z`) |
| Pre-release | `vX.Y.Z-rc.N` — CI MUST create a GitHub **pre-release**; MUST NOT imply registry publish |
| Stable | `vX.Y.Z` without prerelease suffix — GitHub Release is **not** marked pre-release |

Tags MUST NOT be lightweight-only; annotated tags are required so `git tag -l -n` and Release note extraction work.

## What a release is

A SPOKE release is **not** an npm or crates.io publish. A release is:

1. Lockstep manifests and README badges bumped to `X.Y.Z` on `main` (via maintainer PR).
2. Maintainer creates and pushes annotated tag `vX.Y.Z` (or `vX.Y.Z-rc.N`) pointing at that commit.
3. CI **release** workflow on tag push re-validates verify-equivalent gates.
4. On success, workflow creates a **GitHub Release** for that tag with notes from tag annotation first; generated fallback only when annotation is empty.
5. Consumers pin the repo at that tag.

**GitHub Release contents (minimum):** tag name, release notes body, automatic source archive (GitHub default). Optional: link to umbrella spec version. No registry artifacts.

## Who may cut a release

| Actor | Rule |
|-------|------|
| Maintainers | MAY bump, tag, and push after PR merge to `main` |
| CI | MUST NOT auto-bump version or auto-push tags on merge |
| Forks | Release workflow MAY no-op or fail without `contents: write`; document in operator guide |

## CI requirements

| Trigger | Workflow | Requirement |
|---------|----------|-------------|
| `pull_request` / push to `main` / `iteration/**` | `.github/workflows/ci.yml` | Existing verify jobs **plus** dedicated `verify-version` job |
| Push of tag matching `v*` | `.github/workflows/release.yml` | Parallel verify-equivalent jobs, then `release` job (fail-closed) |

Release workflows MUST NOT publish to npm or crates.io. Third-party Actions MUST pin by commit SHA (same policy as `ci.yml`).

### Lockstep assert (PR / main)

| Item | Value |
|------|-------|
| Script | `tooling/release/assert-lockstep-version.mjs` |
| SSOT manifest | `tooling/release/lockstep-surfaces.mjs` (shared with bump script) |
| Root script | `pnpm run verify:version` → `node tooling/release/assert-lockstep-version.mjs` |
| CI job | `verify-version` in `ci.yml` — checkout + Node 20 only; runs in parallel with other jobs |
| On failure | Non-zero exit; prints expected (canonical) vs actual per surface |

### Tag release workflow (fail-closed)

| Item | Value |
|------|-------|
| File | `.github/workflows/release.yml` |
| Trigger | `on.push.tags: ['v*']` |
| Concurrency | `group: release-${{ github.ref }}`, `cancel-in-progress: true` |
| Permissions | Workflow default `contents: read`; `release` job sets `contents: write` |
| Job layout | **Parallel verify** (`verify-codegen`, `typescript`, `rust` — same commands as `ci.yml`) → **sequential `release`** job with `needs: [verify-codegen, typescript, rust]` |
| Fail-closed | If any verify job fails, `release` MUST NOT run and MUST NOT create a GitHub Release |
| Pre-release | Tag name contains `-rc.` → `prerelease: true` on GitHub Release |
| Release action | `softprops/action-gh-release` pinned by commit SHA (same pin style as `ci.yml`) |
| Notes body | Tag annotation via `git tag -l --format='%(contents)'`; fallback one-liner when empty |

**Verify-equivalent gates** (minimum, shared by `ci.yml` and `release.yml`): `pnpm run verify-codegen`, TypeScript typecheck/build/test for `@42ch/spoke-schemas` and `@42ch/spoke-operations`, `pnpm run test:fixtures`, `cargo check -p spoke-schemas`.

### README version badge assert

Both `README.md` and `README_CN.md` MUST contain a shields.io Version badge whose URL segment matches the canonical version:

| Item | Value |
|------|-------|
| Line marker | `[![Version]` |
| URL pattern | `img.shields.io/badge/version-<X.Y.Z>` where `<X.Y.Z>` equals canonical `package.json` version |
| Regex (implement) | `/img\.shields\.io\/badge\/version-([0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.]+)?)-/` |
| Bump script | Rewrites badge URL in both README files when version changes |

## Operator tooling

| Script | Path | Role |
|--------|------|------|
| SSOT manifest | `tooling/release/lockstep-surfaces.mjs` | Exports `CANONICAL_PATH`, `JSON_VERSION_PATHS[]`, `CARGO_WORKSPACE_PATH`, `README_BADGE_PATHS[]` |
| Assert | `tooling/release/assert-lockstep-version.mjs` | Reads manifest; exits 0/1 |
| Bump | `tooling/release/bump-version.mjs` | Updates all manifest paths + badges; invokes assert before exit 0 |

| Root script | Command |
|-------------|---------|
| `verify:version` | `node tooling/release/assert-lockstep-version.mjs` |
| `release:bump` | `node tooling/release/bump-version.mjs` |

**Bump → assert contract:** `bump-version.mjs` writes all surfaces from `lockstep-surfaces.mjs`, then spawns assert (same entrypoint as CI). Exit non-zero if assert fails; no success message on drift.

**CLI:** `node tooling/release/bump-version.mjs <X.Y.Z> [--tag [message]]` — `--tag` creates a local annotated tag only (no push).

## Consumer pinning

| Method | Pattern |
|--------|---------|
| Git tag | `git checkout vX.Y.Z` |
| GitHub Release | Download source archive for tag `vX.Y.Z` |
| pnpm workspace | `"@42ch/spoke-schemas": "file:../spoke/packages/spoke-schemas"` at checked-out tag |
| Git dependency | `"@42ch/spoke-schemas": "github:42ch-dev/spoke#vX.Y.Z"` (org/repo as applicable) |

Package names: `@42ch/spoke-schemas`, `@42ch/spoke-operations`, `@42ch/spoke-fixture-toy-world`; Rust crate `spoke-schemas`.

## Orthogonality — package SemVer vs wire `schema_version`

| Axis | Owner | Notes |
|------|-------|-------|
| Package / monorepo SemVer | This document | Bumps when packaging identity changes for consumers |
| Wire `schema_version` | Data/ops schemas (`common.schema.json`) | Integer on durable objects; independent of package SemVer |

A package SemVer bump does **not** require a wire `schema_version` bump, and vice versa, unless release notes explicitly couple them.

## Non-goals

- npm or crates.io publish jobs
- Independent per-package SemVer
- Adapter package releases
- Auto-release on every merge to `main`
- Changing Morning Star harness process paths

## Related

| Doc | Role |
|-----|------|
| [`spoke-protocol.md`](spoke-protocol.md) | Protocol umbrella |
| [`STRATEGY.md`](../../STRATEGY.md) | No registry publish from CI |
| [`.mstar/roadmap.md`](../roadmap.md) | Product scheduling |
| Root `README.md` / `README_CN.md` | Consumer pinning and maintainer release workflow |
