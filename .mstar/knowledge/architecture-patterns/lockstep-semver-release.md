---
module: release / lockstep versioning
date: 2026-07-25
last_updated: 2026-11-08
problem_type: architecture_pattern
category: architecture-patterns
severity: medium
tags:
  - semver
  - lockstep
  - release
  - oidc
  - cargo
  - changelog
applies_when:
  - Preparing a coordinated release
  - Adding a workspace member or version-bearing surface
---

# Lockstep SemVer release

## Context

Root `package.json` supplies the repository's package version. TypeScript packages, Cargo workspace members and native-binding manifests share that release identity. Wire `schema_version` and the C ABI revision are separate compatibility axes.

## Guidance

### One derived version surface

`tooling/release/lockstep-surfaces.mjs` owns the version-bearing JSON surfaces and derives Cargo members from root `Cargo.toml`. Every workspace member uses `version.workspace = true`; its `Cargo.lock` package entry participates in assertion and bumping, including workspace-private fixtures and carriers.

Every pinned inter-crate path dependency participates, including optional dependencies and aliases. Bump and assertion use the same member-derived set. A stale optional pin can prevent Cargo resolution after an otherwise coordinated bump.

`pnpm run verify:version` invokes the existing lockstep assertion. The required `verify-version` CI job checks the same contract. README release badges resolve GitHub Releases dynamically; they are independent of manifest version writes.

### Strictly increasing bump

```bash
pnpm run release:bump -- X.Y.Z
```

The requested SemVer must be strictly greater than the current root version. Equal and lower requests fail before manifest, lockfile, changelog or Git-ref mutation. The CLI updates all version surfaces and dependency pins, updates the lockfile and changelog, then runs the shared assertion.

`CHANGELOG.md` is generated through git-cliff and Conventional Commits. If the target section already exists, bumping promotes that section to the top instead of adding a duplicate. `extract-changelog-notes.mjs` reads the resulting target section for release notes. Version-dependent tests use relative fixture versions and isolated repositories.

### Signed release workflow

Maintainers request **New release** with the desired greater SemVer. `.github/workflows/new-release.yml` creates a GitHub-verified bump on a release branch and opens the labeled release PR. The request also checks that the target tag is absent. Merging that PR lets the top-level `.github/workflows/release.yml` create the annotated tag and publish; closing the PR aborts the cut.

**Release** is also triggered by matching tag pushes. Stable `vX.Y.Z` tags and prerelease tags outside the `-rc.` family publish the coordinated packages. Tags containing `-rc.` create GitHub pre-releases.

The top-level `release.yml` filename is part of the Trusted Publishing identity. npm publishing uses Node24, the configured registry and stock npm; crates.io publishing obtains a short-lived credential through its OIDC authentication action. The published core packages are `@42ch/spoke-schemas`, `@42ch/spoke-operations`, `@42ch/spoke-connect`, and the corresponding `spoke-schemas`, `spoke-operations`, `spoke-connect` crates. Workspace-private fixtures and codegen remain in the coordinated version assertion.

Native bindings use their configured channels at the same release identity: NuGet and Maven through GitHub Packages, Python through PyPI, and Swift/Go/C/C++ through matching repository tags. Their owning contract is `connect-binding-channels.md`.

### Partial-publication recovery

Re-run the failed publish job after a sibling registry succeeds. Existing npm and crate publish helpers skip versions already present and retry supported transient upload failures. Registry confirmations determine each channel's result; the version remains the same throughout that recovery.

## Why This Matters

A shared version resolver keeps optional dependencies, private workspace members and consumer manifests consistent. Strict pre-mutation rejection preserves a safe target for the signed release workflow. A single changelog section keeps extracted publication notes aligned with that target.

## When to Apply

- Preparing a coordinated SemVer release through the maintainer workflow.
- Adding a Cargo workspace member, pinned path dependency or version-bearing manifest.
- Updating release-tool fixtures or recovering a partially published release.

## References

- [Normative version and release policy](../../specs/spoke-version-release.md)
- [Native-binding channels](../../specs/connect-binding-channels.md)
- [Version-agnostic release test fixtures](../testing-patterns/release-bump-tests-version-agnostic.md)
- [Maintainer procedures](../../../CONTRIBUTING.md)
- Source: `tooling/release/lockstep-surfaces.mjs`, `assert-lockstep-version.mjs`, `bump-version.mjs`, `extract-changelog-notes.mjs`.
