# SPOKE Version Release

> **Status:** Normative  
> **Document class:** Policy — monorepo packaging SemVer, git tags, CI-gated GitHub Release  
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)  
> **Strategy:** [`STRATEGY.md`](../../STRATEGY.md)

---

## Purpose

SPOKE publishes a **single lockstep SemVer** for workspace packages and Rust consumer crates so integrators can pin **one version** across TypeScript and Rust artifacts. Releases are cut with **annotated git tags**, materialized as **GitHub Releases**, and published to **npm** (`@42ch/spoke-schemas`, `@42ch/spoke-operations`) and **crates.io** (`spoke-schemas`, `spoke-operations`) when CI passes on stable tags.

**Integrator value:** install from npm/crates.io at `X.Y.Z`, or pin sibling repos to `vX.Y.Z` (git tag or GitHub Release source archive) via `file:` path or git dependency.

**Maintainer value:** one bump path updates every SSOT surface; CI blocks drift; tagging a green commit produces a Release and registry artifacts after gates re-run.

## Version SSOT (lockstep)

All of the following MUST share the same `X.Y.Z` string (no independent channels):

| # | Surface | Path | Assert method |
|---|---------|------|---------------|
| 1 | Workspace root (canonical) | `package.json` → `version` | JSON `version` field |
| 2 | TypeScript schemas | `packages/spoke-schemas/package.json` → `version` | JSON `version` field |
| 3 | TypeScript operations | `packages/spoke-operations/package.json` → `version` | JSON `version` field |
| 4 | Fixture harness | `fixtures/toy-world/package.json` → `version` | JSON `version` field |
| 5 | Codegen runner | `tooling/codegen/package.json` → `version` | JSON `version` field |
| 6 | TypeScript connect library | `packages/spoke-connect-ts/package.json` → `version` | JSON `version` field (published as `@42ch/spoke-connect`; asserted by `verify:version`, bumped by `release:bump`; built via `tsup` → `dist/` for the npm tarball) |
| 7 | Rust workspace | `Cargo.toml` → `[workspace.package].version` | TOML parse |
| 8 | Rust schema crate | `crates/spoke-schemas/Cargo.toml` | MUST declare `version.workspace = true`; effective version equals row 7 |
| 9 | Rust operations crate | `crates/spoke-operations/Cargo.toml` | MUST declare `version.workspace = true`; effective version equals row 7 |
| 10 | Rust connect crate | `crates/spoke-connect/Cargo.toml` | MUST declare `version.workspace = true`; effective version equals row 7 (published as `spoke-connect`) |
| 11 | Cargo lockfile | `Cargo.lock` → `[[package]]` for `spoke-schemas`, `spoke-operations`, `spoke-fixture-toy-world`, `spoke-connect` | TOML package version entries |
| 12 | README EN badge | `README.md` | Dynamic shields.io GitHub Releases badge (presence) |
| 13 | README CN badge | `README_CN.md` | Same as row 12 |

**Canonical version source:** row 1 (`package.json` → `version`). The assert script compares every other row to that string.

**Excluded from lockstep** (not in assert script):

| Path | Reason |
|------|--------|
| `tooling/codegen/rust-gen/Cargo.toml` | Standalone `[workspace]` bin crate (`spoke-rust-gen`); not a consumer pin surface; version is local to the codegen tool |
| `pnpm-lock.yaml` | Workspace packages use `link:` protocol; lockfile does not embed package SemVer |

CI **lockstep assert** MUST cover rows 1–13. Drift on any row MUST fail the build (no warn-only path). `release:bump` MUST rewrite `Cargo.lock` member versions when bumping (Node-only; no `cargo` required).

## SemVer usage (monorepo)

| Bump | When |
|------|------|
| **PATCH** (`0.1.0` → `0.1.1`) | Bugfix-only packaging release; no wire contract change |
| **MINOR** (`0.1.z` → `0.2.0`) | Backward-compatible wire or ops library additions |
| **MAJOR** (`0.x.y` → `1.0.0`) | Breaking wire or public API change |

Pre-1.0: breaking wire or public API changes remain allowed without a deprecation period; still use SemVer strings for packaging identity. Release notes MUST call out wire-impacting changes even when package and `schema_version` move independently.

## Tag convention

| Rule | Value |
|------|-------|
| Form | Annotated tag `vX.Y.Z` (required leading `v`) |
| Target | Commit on `main` (or `iteration/**` integration branch) that already passes CI |
| Annotation | Optional human-readable summary; used only when `CHANGELOG.md` has no matching section |
| Notes extraction | Primary: `tooling/release/extract-changelog-notes.mjs` on `CHANGELOG.md` for `vX.Y.Z`; fallback: `git tag -l --format='%(contents)' <tag>`; final fallback: `Release vX.Y.Z` |
| Pre-release | `vX.Y.Z-rc.N` — CI MUST create a GitHub **pre-release**; MUST NOT publish to npm or crates.io |
| Stable | `vX.Y.Z` without prerelease suffix — GitHub Release is **not** marked pre-release |

Tags SHOULD be annotated. Release notes come from `CHANGELOG.md` first; tag annotation is a fallback only.

## What a release is

A SPOKE release is:

1. Lockstep manifests and `CHANGELOG.md` bumped to `X.Y.Z` on `main` (via **New release** PR or equivalent maintainer bump). README Version badges track the latest GitHub Release dynamically.
2. Annotated tag `vX.Y.Z` (or `vX.Y.Z-rc.N`) points at that commit (created by **Release** after a `release`-labeled PR merges, or by a maintainer).
3. CI **Release** workflow (`release.yml` — top-level only: tag push or PR-merge tag path) re-validates verify-equivalent gates.
4. On success, workflow creates a **GitHub Release** for that tag with notes from the matching `CHANGELOG.md` section; tag annotation and a one-line fallback apply when the section is missing.
5. When the tag name does not contain `-rc.`, CI publishes `@42ch/spoke-schemas`, then `@42ch/spoke-operations`, then crate `spoke-schemas`, then crate `spoke-operations` to npm and crates.io.
6. Consumers install from registries at `X.Y.Z` or pin the repo at that tag.

**GitHub Release contents (minimum):** tag name, release notes body, automatic source archive (GitHub default). Registry artifacts: `@42ch/spoke-schemas`, `@42ch/spoke-operations`, `spoke-schemas`, `spoke-operations` at the same SemVer.

## Who may cut a release

| Actor | Rule |
|-------|------|
| Maintainers | MAY run **New release** (`workflow_dispatch`) and merge or close the labeled PR; MAY still bump/tag manually |
| CI | **New release** MAY open the bump PR; **Release** MAY create the annotated tag and publish when a `release`-labeled PR merges to `main` |
| CI | MUST NOT auto-bump or auto-tag on ordinary (non-release) merges to `main` |
| Forks | Release workflow MAY no-op or fail without `contents: write`; document in operator guide |

## CI requirements

| Trigger | Workflow | Requirement |
|---------|----------|-------------|
| `pull_request` / push to `main` / `iteration/**` | `.github/workflows/ci.yml` | Existing verify jobs **plus** dedicated `verify-version` job |
| `workflow_dispatch` (version input) | `.github/workflows/new-release.yml` | Opens lockstep bump PR with label `release` (GitHub-signed commit); MUST refuse when version ≤ `package.json` on `main` or when `vX.Y.Z` already exists; MUST NOT duplicate `CHANGELOG` version headings |
| `pull_request` closed (merged + label `release`) or push of tag `v*` | `.github/workflows/release.yml` | Top-level only (no `workflow_call`, no `workflow_dispatch`). Tag-on-merge when applicable → parallel verify → GitHub Release → `publish-npm` + `publish-crates` when tag has no `-rc.` (fail-closed). Trusted Publishing OIDC requires this filename (`release.yml`). |

Release workflows publish `@42ch/spoke-schemas`, `@42ch/spoke-operations`, `@42ch/spoke-connect`, `spoke-schemas`, `spoke-operations`, and `spoke-connect`. Fixture and codegen packages remain private. Third-party Actions MUST pin by commit SHA (same policy as `ci.yml`).

### Lockstep assert (PR / main)

| Item | Value |
|------|-------|
| Script | `tooling/release/assert-lockstep-version.mjs` |
| SSOT manifest | `tooling/release/lockstep-surfaces.mjs` (shared with bump script) |
| Root script | `pnpm run verify:version` → `node tooling/release/assert-lockstep-version.mjs` |
| CI job | `verify-version` in `ci.yml` — checkout + Node 20 only; runs in parallel with other jobs |
| On failure | Non-zero exit; prints expected (canonical) vs actual per surface |

On tag push, `release.yml` `verify-version` MUST assert `github.ref_name` via `SPOKE_RELEASE_TAG`: stable tags (`vX.Y.Z`) require exact match to canonical `package.json` version; RC tags (`vX.Y.Z-rc.N`) compare base `X.Y.Z` only (prerelease suffix is not part of lockstep manifests).

### Tag release workflow (fail-closed)

| Item | Value |
|------|-------|
| File | `.github/workflows/release.yml` |
| Trigger | `on.push.tags: ['v*']`; `pull_request` closed (merged + label `release`). No `workflow_call` / no `workflow_dispatch` (OIDC Trusted Publishing requires top-level `release.yml`). |
| Concurrency | `group: release-${{ github.event_name == 'pull_request' && format('pr-{0}', github.event.pull_request.number) || github.ref }}`, `cancel-in-progress: true` |
| Permissions | Workflow-level `contents: write`, `id-token: write`, `pull-requests: read` |
| Job layout | Optional `tag` → `resolve-tag` → **four parallel verify jobs** → **`release`** → **`publish-npm`** then **`publish-crates`** (tags without `-rc.` only) |
| Fail-closed | If any verify job fails, `release` and registry publish jobs MUST NOT run |
| Pre-release | Tag name contains `-rc.` → `prerelease: true` on GitHub Release; **skip** `publish-npm` and `publish-crates` |
| Release action | `softprops/action-gh-release` pinned by commit SHA |
| Notes body | `extract-changelog-notes.mjs` on `CHANGELOG.md`; fallback tag annotation; fallback one-liner |
| Registry publish | `publish-npm`: Node 24 + `registry-url`, stock npm (no global npm upgrade), pack with pnpm then `npm publish` tarball; `publish-crates`: `rust-lang/crates-io-auth-action` then `cargo publish` schemas then ops |
| Registry auth | npm and crates.io: Trusted Publishing only (org `42ch-dev`, repo `spoke`, workflow **`release.yml`** as top-level filename) |
| Operator cut | `new-release.yml` opens labeled PR with GraphQL-signed bump; merge triggers this workflow’s `tag` job |

**Verify-equivalent gates** (minimum, shared by `ci.yml` and `release.yml`): `pnpm run verify-codegen`, TypeScript typecheck/build/test for `@42ch/spoke-schemas` and `@42ch/spoke-operations`, `pnpm run test:fixtures`, `pnpm run test:release` (lockstep assert/bump unit tests), `cargo check -p spoke-schemas`, `cargo test -p spoke-operations`, `cargo test -p spoke-fixture-toy-world` (private fixture crate; excluded from `publish-crates`), `cargo test -p spoke-connect` (private spike crate; excluded from `publish-crates`), `pnpm run verify:version` (lockstep assert via `tooling/release/assert-lockstep-version.mjs`).

### README version badge assert

Both `README.md` and `README_CN.md` MUST contain a dynamic shields.io GitHub Releases Version badge (not a static `badge/version-<SemVer>` URL):

| Item | Value |
|------|-------|
| Line marker | `[![Version]` |
| URL marker | `img.shields.io/github/v/release/42ch-dev/spoke` (include prereleases + SemVer sort) |
| Link target | `https://github.com/42ch-dev/spoke/releases` |
| Bump script | Does **not** rewrite README badges — badge tracks published GitHub Releases |

## Operator tooling

| Script | Path | Role |
|--------|------|------|
| SSOT manifest | `tooling/release/lockstep-surfaces.mjs` | Exports `CANONICAL_PATH`, `JSON_VERSION_PATHS[]`, `CARGO_WORKSPACE_PATH`, `CARGO_SCHEMA_CRATE_PATH`, `CARGO_OPS_CRATE_PATH`, `CARGO_LOCK_PATH`, `CARGO_LOCK_PACKAGE_NAMES[]`, `README_BADGE_PATHS[]`, `README_RELEASE_BADGE_MARKER` |
| Assert | `tooling/release/assert-lockstep-version.mjs` | Reads manifest (incl. `Cargo.lock` member versions); exits 0/1 |
| Bump | `tooling/release/bump-version.mjs` | Updates all lockstep manifests + `Cargo.lock` member versions; regenerates `CHANGELOG.md` via git-cliff; invokes assert before exit 0 |
| Changelog runner | `tooling/release/run-git-cliff.mjs` | Resolves `git-cliff` (PATH → `pnpm dlx` → `npx`); used by bump and `release:changelog` |
| Notes extractor | `tooling/release/extract-changelog-notes.mjs` | Prints `CHANGELOG.md` section body for `vX.Y.Z` / `X.Y.Z` (CI + local) |
| Config | `cliff.toml` | git-cliff Conventional Commits grouping (Keep a Changelog sections) |

| Root script | Command |
|-------------|---------|
| `verify:version` | `node tooling/release/assert-lockstep-version.mjs` |
| `release:bump` | `node tooling/release/bump-version.mjs` |
| `release:changelog` | `node tooling/release/run-git-cliff.mjs` (pass git-cliff flags after `--`) |

**Changelog SSOT:** root `CHANGELOG.md`. Maintainers edit only when correcting entries; routine updates happen via `release:bump`, which prepends the section for the target version using git-cliff.

**Bump → assert contract:** `bump-version.mjs` writes all surfaces from `lockstep-surfaces.mjs`, updates `CHANGELOG.md` for the target tag via git-cliff (`cliff.toml`), then spawns assert (same entrypoint as CI). Exit non-zero if assert fails; no success message on drift.

**CLI:** `node tooling/release/bump-version.mjs <X.Y.Z> [--tag [message]]` (root: `pnpm run release:bump -- X.Y.Z [--tag [message]]` — pass `--` before arguments when using pnpm).

| Mode | Behavior |
|------|----------|
| Bump (`current ≠ target`) | Writes all lockstep surfaces, updates `CHANGELOG.md`, runs assert, prints commit + tag next steps. |
| Already at target (`current = target`) | Re-runs assert only; prints tag push instructions. |
| `--tag` on clean tree at target | Creates **local annotated tag** `vX.Y.Z` (optional message; default `Release vX.Y.Z`). Script never pushes. |
| `--tag` during bump or on dirty tree | **Refused** (non-zero exit) with printed commit/tag instructions — commit first, then re-run same version with `--tag`. |

## Consumer pinning

| Method | Pattern |
|--------|---------|
| npm | `pnpm add @42ch/spoke-schemas@X.Y.Z @42ch/spoke-operations@X.Y.Z` |
| crates.io | `spoke-schemas = "X.Y.Z"` and `spoke-operations = "X.Y.Z"` in `Cargo.toml` |
| Git tag | `git checkout vX.Y.Z` |
| GitHub Release | Download source archive for tag `vX.Y.Z` |
| pnpm workspace | `"@42ch/spoke-schemas": "file:../spoke/packages/spoke-schemas"` at checked-out tag |
| Git dependency | `"@42ch/spoke-schemas": "github:42ch-dev/spoke#vX.Y.Z"` (org/repo as applicable) |

Package names: `@42ch/spoke-schemas`, `@42ch/spoke-operations`, `@42ch/spoke-fixture-toy-world`; Rust crates `spoke-schemas`, `spoke-operations` (published), `spoke-fixture-toy-world` (workspace-private fixture; lockstep version asserted, never published) and `spoke-connect` (workspace-private spike; lockstep version asserted, not published).

## Orthogonality — package SemVer vs wire `schema_version`

| Axis | Owner | Notes |
|------|-------|-------|
| Package / monorepo SemVer | This document | Bumps when packaging identity changes for consumers |
| Wire `schema_version` | Data/ops schemas (`common.schema.json`) | Integer on durable objects; independent of package SemVer |

A package SemVer bump does **not** require a wire `schema_version` bump, and vice versa, unless release notes explicitly couple them.

## Non-goals

- Publishing fixture or codegen packages
- Independent per-package SemVer
- Adapter package releases
- Auto-release on every merge to `main`
- Changing Morning Star harness process paths

## Related

| Doc | Role |
|-----|------|
| [`spoke-protocol.md`](spoke-protocol.md) | Protocol umbrella |
| [`STRATEGY.md`](../../STRATEGY.md) | Registry publish on tagged stable releases |
| [`.mstar/roadmap.md`](../roadmap.md) | Product scheduling |
| Root `README.md` / `README_CN.md` | Version badge and consumer pinning |
| [`CONTRIBUTING.md`](../../CONTRIBUTING.md) | Maintainer local development, CI, and release how-to |
