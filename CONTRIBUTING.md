# Contributing to SPOKE

Maintainer and local-development guide. Integrators consuming published packages should start at [`README.md`](README.md) / [`README_CN.md`](README_CN.md).

Agent and harness invariants live in [`AGENTS.md`](AGENTS.md). Normative release policy: [`.mstar/specs/spoke-version-release.md`](.mstar/specs/spoke-version-release.md).

## Prerequisites

- Node.js ≥ 20 and pnpm ≥ 11.21 (pinned via the root packageManager field)
- Rust toolchain (stable) for `spoke-schemas` / `spoke-operations` crates
- git-lfs — the committed Swift xcframework static libraries and C carrier dynamic libraries are LFS objects

```bash
git clone https://github.com/42ch-dev/spoke.git
cd spoke
git lfs install
pnpm install
```

## Local development

Useful scripts (repo root):

| Script | Purpose |
|--------|---------|
| `pnpm run codegen` | Regenerate TypeScript and Rust types from `schemas/` |
| `pnpm run verify-codegen` | Codegen + schema-count assert + `git diff` on generated trees |
| `pnpm run typecheck` | Typecheck all workspace TS packages (`@42ch/spoke-schemas`, `@42ch/spoke-operations`, `@42ch/spoke-connect`, `@42ch/spoke-demo-server`, `@42ch/spoke-demo-client`) — warm-tree helper; the cold-clone gate is `ci:typescript` |
| `pnpm run ci:typescript` | Cold-clone gate mirroring the ci.yml `typescript` job stage chain in dependency order, then the demo typechecks + `test:demo` |
| `pnpm run build:schema` / `build:operations` | Build published npm packages |
| `pnpm run test` / `test:fixtures` | Operations unit tests / toy-world fixture harness |
| `pnpm run verify:version` | Lockstep SemVer assert across publish surfaces |
| `pnpm run release:changelog -- --unreleased` | Preview CHANGELOG section |

Schema changes and generated output belong in the **same** commit.

Rust crates (from repo root):

```bash
cargo check -p spoke-schemas
cargo test -p spoke-operations
cargo test -p spoke-fixture-toy-world
cargo test -p spoke-connect
```

If `~/.cargo/config.toml` sets `-Zno-embed-metadata` (an unstable flag
incompatible with stable rustc), prefix cargo commands with `RUSTFLAGS=""`:

```bash
RUSTFLAGS="" cargo test -p spoke-connect
```

CI does not need this workaround.

### Workspace / path dependencies

When this repo is a pnpm workspace member of another monorepo:

```json
{
  "dependencies": {
    "@42ch/spoke-schemas": "workspace:*",
    "@42ch/spoke-operations": "workspace:*"
  }
}
```

From another repo during development:

```json
{
  "dependencies": {
    "@42ch/spoke-schemas": "file:../spoke/packages/spoke-schemas",
    "@42ch/spoke-operations": "file:../spoke/packages/spoke-operations"
  }
}
```

Then at the SPOKE root: `pnpm install`, `pnpm --filter @42ch/spoke-schemas build`, `pnpm --filter @42ch/spoke-operations build`.

Published consumers should prefer npm / crates.io at lockstep SemVer (see root README).

### Workspace-private packages

These stay private to the monorepo (not published to registries):

| Package | Path |
|---------|------|
| `@42ch/spoke-fixture-toy-world` | `fixtures/toy-world/` |
| `@42ch/spoke-codegen` | `tooling/codegen/` |

## Pull requests and CI

Every pull request must pass the required checks bound to the `main` ruleset: the PR validation contexts produced by [`.github/workflows/ci.yml`](.github/workflows/ci.yml) (`verify-codegen`, `typescript`, `rust`, `verify-version`, `connect-identity`, `golden-vector-sync`, `csharp-connect`, `kotlin-connect-smoke`), `windows-smoke` and `macos-smoke` ([`cpp-connect.yml`](.github/workflows/cpp-connect.yml)), `xcframework` ([`xcframework.yml`](.github/workflows/xcframework.yml)), `Build docs site` ([`docs.yml`](.github/workflows/docs.yml)), the managed `Analyze (…)` / `CodeQL` analysis, and the Cursor Approval Agent and Greptile review checks. The pull-request triggers of those workflows are deliberately unfiltered — a path filter would leave a required context unscheduled, and an unscheduled required check can never be satisfied.

Two maintenance rules follow. When a new PR validation check is added, register it in the `main` ruleset required checks in the same round; an unregistered check gates nothing. And a PR is ready only once every required check has finished successfully — a pending CodeQL or Greptile run is not a pass, and "no review comments" is not the same as a green review.

## Integrator docs site

The integrator-facing VitePress site lives in `docs/` (config: `docs/.vitepress/config.mts`).

| Command | Purpose |
|---------|---------|
| `pnpm docs:dev` | Local dev server with hot reload |
| `pnpm docs:build` | Static build to `docs/.vitepress/dist` (the CI gate) |
| `pnpm docs:preview` | Serve the built site locally |

Pages summarize each topic and link the normative body in `.mstar/specs/` — the specs remain the single source of truth for normative detail. The build gate runs on PRs and the Pages deploy runs on `main` via [`.github/workflows/docs.yml`](.github/workflows/docs.yml); the Pages source must be **GitHub Actions** (repo Settings → Pages → Source).

## Refreshing the Swift xcframework

The three-slice `spoke_connectFFI.xcframework` is assembled in CI by the `xcframework` job ([`.github/workflows/xcframework.yml`](.github/workflows/xcframework.yml)) on `macos-14` from the checkout's Rust sources — pinned toolchain 1.96.0, four Apple targets, `--locked` build via `tooling/connect/build-swift-xcframework.sh`. Every pull request runs it; pushes run it when the FFI surface changes. `tooling/connect/verify-xcframework-drift.sh` compares per-file SHA-256 hashes of the CI build against the committed (LFS) artifact and fails the job on drift. The built xcframework and its hash manifest upload on every run.

Refresh the committed artifact from a run in one command (requires the `gh` CLI and `rsync`):

```bash
./tooling/connect/apply-xcframework-artifact.sh <run-id>
```

The script first verifies the run's provenance — the `Xcframework` workflow, a `success` conclusion (or a `failure` caused only by the drift gate, which still ships a complete artifact), and a `headSha` matching the current checkout (override with `--allow-sha <sha>` only to deliberately pin an older build). It then requires the artifact's hash manifest and checksum-verifies every file against it before rsyncing the built tree over the committed one and staging the LFS pointers. Commit with the suggested line — `build(connect): refresh xcframework from CI artifact <run-id>` — using normal maintainer credentials (token pushes don't re-trigger workflows and can't carry LFS objects).

A refresh-only push does not re-trigger the `xcframework` job (the committed xcframework path is off the workflow's push filter — a pull request touching it always re-runs), so after committing a refreshed artifact, confirm the drift gate is still green before pushing — re-run the producing job (`gh run rerun <run-id>`), or compare the artifact's built tree against the committed one locally:

```bash
gh run download <run-id> --name spoke-connect-xcframework --dir /tmp/xcf-stage
./tooling/connect/verify-xcframework-drift.sh \
  crates/spoke-connect/bindings/swift/xcframework/spoke_connectFFI.xcframework \
  /tmp/xcf-stage/spoke_connectFFI.xcframework
```

## Refreshing the C carrier natives

`crates/spoke-connect/bindings/cpp/native/` holds the two committed C carriers (`osx-arm64/libspoke_connect_capi.dylib`, `win-x64/spoke_connect_capi.dll`) with the Windows import library and `provenance.json`. The dynamic libraries are git-lfs objects; `spoke_connect_capi.dll.lib` and `provenance.json` stay ordinary Git objects, so a checkout can read and diff the record without an LFS fetch. A converted or refreshed carrier needs `git add --renormalize <path>` — a plain `git add` sees unchanged bytes and stages nothing.

The Windows lane of [`cpp-connect.yml`](.github/workflows/cpp-connect.yml) checks out with `lfs: true`, builds the carrier (`node tooling/connect/cpp-build.mjs --target x86_64-pc-windows-msvc --toolchain 1.96.0`) and uploads `spoke-connect-win-x64`: the DLL, the import library and the refreshed `provenance.json`. Refresh the committed carrier by copying that artifact over `native/win-x64/` and committing the binaries together with the record that describes them — the recorded `headerSha256` / artifact hashes must match the committed bytes, never the other way round.

`node tooling/connect/cpp-build.mjs --verify --target <triple>` re-reads the committed `provenance.json` and re-hashes the committed header and native without building; it fails on a mismatch, and both carrier lanes run it before they build anything. The C contract header is pinned to LF (`text eol=lf` in `.gitattributes`) so its recorded `headerSha256` is identical on every platform.

## Release

Primary path — **New release** on GitHub Actions, then merge the PR:

1. Open [Actions → New release](https://github.com/42ch-dev/spoke/actions/workflows/new-release.yml) → **Run workflow**.
2. Set **version** to the lockstep SemVer (e.g. `0.1.0-alpha.3`). Optional summary fills the PR body.
3. Merge the opened PR (keep the `release` label), or close it to abort. CI runs [**Release**](https://github.com/42ch-dev/spoke/actions/workflows/release.yml): creates annotated tag `vX.Y.Z`, verify gates, GitHub Release from [`CHANGELOG.md`](CHANGELOG.md), and on stable / alpha tags (no `-rc.` suffix) publishes npm + crates.io.

[`CHANGELOG.md`](CHANGELOG.md) is the release-notes source (Keep a Changelog via [git-cliff](https://git-cliff.org)). Preview locally: `pnpm run release:changelog -- --unreleased`.

Pre-release tags `vX.Y.Z-rc.N` create a GitHub pre-release. Tags without `-rc.` publish to npm and crates.io.

### Registry auth

| Auth | Use |
|------|-----|
| npm **Trusted Publisher** | OIDC from `release.yml` → `publish-npm` (each package: org `42ch-dev`, repo `spoke`, workflow `release.yml`) |
| crates.io **Trusted Publishing** | OIDC from `release.yml` → `publish-crates` via `rust-lang/crates-io-auth-action` (each crate: org `42ch-dev`, repo `spoke`, workflow `release.yml`) |
| PyPI **Trusted Publishing** | OIDC from `release.yml` → `publish-pypi` (publisher registered to org `42ch-dev`, repo `spoke`, workflow `release.yml`) — no long-lived `PYPI_TOKEN` |
| GitHub Packages **Maven** | `GITHUB_TOKEN` with `packages: write` on `release.yml` → `publish-maven` (registry `maven.pkg.github.com/42ch-dev/spoke`) |

If `publish-crates` fails after npm succeeded, re-run the failed job (or push the annotated tag with a non-`GITHUB_TOKEN` credential so `push.tags` starts **Release** again).

Re-running Release at an already-published tag is safe for **PyPI** and **Maven**: both jobs pre-check the registry for the full expected artifact set and skip build + publish when it is complete. PyPI's pre-check queries the `https://pypi.org/pypi/spoke-connect/<version>/json` JSON API for the three platform wheels (no sdist); Maven's pre-check performs authenticated GETs for the `dev.42ch:spoke-connect` POM, Gradle module metadata, and jar (JNA natives verified inside the jar) on GitHub Packages. Partial uploads are attempted — `skip-existing` lets PyPI resume, `gradle publish` re-attempts Maven — and an unconditional re-probe closes each job: green requires the verified full set. Any registry doubt (network error, HTTP 401/403/5xx, malformed payload) fails the job loud instead of skipping.

## Further reading

| Doc | Audience |
|-----|----------|
| [`README.md`](README.md) | Package consumers |
| [`CONCEPTS.md`](CONCEPTS.md) | Protocol vocabulary |
| [`STRATEGY.md`](STRATEGY.md) | Product direction |
| [`.mstar/specs/`](.mstar/specs/) | Normative protocol and release specs |
| [`AGENTS.md`](AGENTS.md) | Agent / harness boundaries |
