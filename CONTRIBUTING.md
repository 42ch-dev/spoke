# Contributing to SPOKE

Maintainer and local-development guide. Integrators consuming published packages should start at [`README.md`](README.md) / [`README_CN.md`](README_CN.md).

Agent and harness invariants live in [`AGENTS.md`](AGENTS.md). Normative release policy: [`.mstar/specs/spoke-version-release.md`](.mstar/specs/spoke-version-release.md).

## Prerequisites

- Node.js ≥ 20 and pnpm ≥ 8
- Rust toolchain (stable) for `spoke-schemas` / `spoke-operations` crates

```bash
git clone https://github.com/42ch-dev/spoke.git
cd spoke
pnpm install
```

## Local development

Useful scripts (repo root):

| Script | Purpose |
|--------|---------|
| `pnpm run codegen` | Regenerate TypeScript and Rust types from `schemas/` |
| `pnpm run verify-codegen` | Codegen + schema-count assert + `git diff` on generated trees |
| `pnpm run typecheck` | Typecheck `@42ch/spoke-schemas` and `@42ch/spoke-operations` |
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

PRs must pass GitHub Actions jobs `verify-codegen`, `typescript`, `rust`, and `verify-version` ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)).

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

If `publish-crates` fails after npm succeeded, re-run the failed job (or push the annotated tag with a non-`GITHUB_TOKEN` credential so `push.tags` starts **Release** again).

## Further reading

| Doc | Audience |
|-----|----------|
| [`README.md`](README.md) | Package consumers |
| [`CONCEPTS.md`](CONCEPTS.md) | Protocol vocabulary |
| [`STRATEGY.md`](STRATEGY.md) | Product direction |
| [`.mstar/specs/`](.mstar/specs/) | Normative protocol and release specs |
| [`AGENTS.md`](AGENTS.md) | Agent / harness boundaries |
