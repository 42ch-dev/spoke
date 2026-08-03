---
module: spoke-connect
date: 2026-08-03
problem_type: tooling_decision
category: tooling-decisions
severity: medium
applies_when: ["publishing a new connect surface from the protocol repo", "staging a registry publish of a connect artifact", "choosing npm/crates.io vs GitHub Packages vs SPM vs Go modules vs PyPI for a connect binding"]
tags: [spoke-connect, publish, npm, nuget, maven, github-packages, spm, go-modules, pypi, staging, lockstep-semver, trusted-publishing, bindings]
---

# Connect publish staging (registry split)

## Context

The connect embedding model ships several surfaces with different publishing costs: a pure-TS client, a Rust crate (libp2p + optional `ffi` cdylib), per-language uniffi bindings, and an integrator docs site. The connect publish strategy (`.mstar/specs/connect-publish-strategy.md`) stages registries so primary TS/Rust surfaces use npm/crates.io Trusted Publishing while Path B bindings use **four channel types** across five languages (packaging contract: `.mstar/specs/connect-binding-channels.md`).

## Guidance

### Registry split: npm/crates.io for primary; four channels for bindings

| Surface | Registry / mechanism | Rule |
|---|---|---|
| TS connect client (`@42ch/spoke-connect`) | **npm** | Lockstep publish via top-level `release.yml` Trusted Publishing |
| Rust connect crate (`spoke-connect`) | **crates.io** | Lockstep publish via the same release workflow |
| C# NuGet (`42ch.Spoke.Connect`) | **GitHub Packages NuGet** | Generated C# + multi-RID `ffi` natives; `PackageReference` DX; `publish-nuget` on the tag gate |
| Kotlin binding | **GitHub Packages Maven** | `io.github.42ch-dev:spoke-connect` on `maven.pkg.github.com/42ch-dev/spoke`; `publish-maven` sibling job |
| Swift binding | **GitHub repo + SPM** | Root `Package.swift` + `vX.Y.Z` tags; consumers `.package(url:from:)` |
| Go binding | **GitHub repo + Go modules** | Root `go.mod` + `vX.Y.Z` tags; consumers `go get …@vX.Y.Z` |
| Python binding | **PyPI** | `publish-pypi` via Trusted Publishing OIDC; `pip install <registered-name>` |
| Docs site | **GitHub Pages** | Companion site on main |

### Lockstep SemVer continuation

Published connect surfaces — including binding packages — stay on the monorepo lockstep `X.Y.Z` (`.mstar/specs/spoke-version-release.md`): one version across schemas + operations + connect-ts + binding manifests, covered by `release:bump` / `verify:version`. Tag-resolved channels (SPM, Go modules) take the version from git tag `vX.Y.Z` itself. Revisit triggers for an independent channel: a package gains a substantially different release cadence, or a public-registry mirror (nuget.org, Maven Central) is required.

### Binding package shape (C# landed reference)

- Session **core only** in `42ch.Spoke.Connect` (peer_id, hello, allowlist, nonce, sequence, correlation, dispatch) — transport stays product-owned.
- Native libs under NuGet `runtimes/<rid>/native/` (`win-x64`, `linux-x64`, `osx-arm64` minimum).
- Consumers add `https://nuget.pkg.github.com/42ch-dev/index.json` once, then `PackageReference Include="42ch.Spoke.Connect"`.
- Maintainers regenerate with the vendored bindgen fork until upstream tags uniffi 0.32+; consumers never run bindgen.
- Other languages follow the per-channel layouts in `connect-binding-channels.md` (Maven JNA resources, SPM xcframework, Go `native/<goos>_<goarch>/`, PyPI platform wheels).

### When a gate blocks the pipeline: defer with a record

C# bindings hit a toolchain gap (`uniffi-bindgen-cs` targets uniffi 0.31 vs the repo pin 0.32) and were recorded with a decision record + revisit trigger ([`connect-csharp-binding.md`](../../specs/connect-csharp-binding.md)) instead of downgrading pins — the gap then **landed via a vendored fork** retargeted to uniffi 0.32 (fork dropped when upstream tags 0.32+). Same pattern for any other blocked surface: record the blocker, keep the target priority, define the revisit trigger, re-check via the documented regenerate → build → run sequence.

## Why This Matters

Registry choice is a cost and trust decision: npm/crates.io OIDC Trusted Publishing covers the primary TS/Rust surfaces; each binding language uses its ecosystem-native channel (GitHub Packages for C#/Kotlin, SPM/Go git tags, PyPI OIDC) without forcing public mirrors before demand exists. Lockstep SemVer keeps integrators on one pin across wire + connect + bindings.

## When to Apply

- Publishing any future connect surface — primary TS/Rust → npm/crates.io; each binding language → its locked channel in `connect-binding-channels.md` + lockstep tag.
- Adding a new language binding package — follow the channel contract for that language (managed sources + natives + release job or tag-only resolution as specified).
- A surface is blocked by tooling (e.g. bindgen version skew) — record the deferral + revisit trigger instead of downgrading pins or publishing unverified output.

## See also

- `.mstar/specs/connect-publish-strategy.md` — the decision record SSOT (surface inventory, stages, triggers, non-goals)
- `.mstar/specs/connect-binding-channels.md` — per-language packaging contract (coordinates, natives, CI jobs)
- `.mstar/specs/spoke-version-release.md` — lockstep SemVer + Trusted Publishing (normative)
- [`lockstep-semver-release.md`](../architecture-patterns/lockstep-semver-release.md) — the release pipeline this strategy extends
- [`connect-ts-client-sdk.md`](../architecture-patterns/connect-ts-client-sdk.md) — the TS package on npm
- [`connect-session-core-ffi-boundary.md`](../architecture-patterns/connect-session-core-ffi-boundary.md) — the binding surface
