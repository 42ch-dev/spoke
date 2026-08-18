---
module: connect / CI native artifacts
date: 2026-08-18
last_updated: 2026-08-18
problem_type: tooling_decision
category: tooling-decisions
severity: medium
plan_id: xcframework-ci-automation
tags:
  - xcframework
  - ci
  - drift-gate
  - lfs
  - determinism
  - macos
  - apply-script
  - provenance
---

# CI-assembled committed native artifacts — drift gate + apply path

## Context

This repo commits compiled native artifacts (the three-slice `spoke_connectFFI.xcframework`, binding natives) so consumers resolve them without a Rust toolchain. The assembly was historically a maintainer-local build: slow, machine-bound (macOS + four Apple targets), and drift-prone — the committed artifact could silently diverge from the Rust sources.

## Guidance

The pattern has four parts:

1. **Path-filtered CI assembly.** A dedicated workflow (`xcframework.yml`) builds the artifact from the checkout on every FFI-surface-affecting change (filter: connect sources, `Cargo.toml`/`Cargo.lock`, the build script, the generated binding tree). Pinned exact toolchain + `--locked` builds. `macos-14` for xcframework (needs `xcodebuild`); `actions/checkout` with `lfs: true`.
2. **Committed artifact is the drift baseline.** A sorted per-file SHA-256 manifest diff (`verify-xcframework-drift.sh`) compares the committed artifact against the CI build; mismatch = red job. The built artifact uploads `if: always()` — a red run still ships the correct replacement.
3. **One-command apply, fail-closed.** `apply-xcframework-artifact.sh <run-id>` downloads the artifact, **mandatorily** checksum-verifies against its manifest (absent manifest = hard fail), verifies provenance (`gh run view`: expected workflow + acceptable conclusion + `headSha` match or an explicit loud override), asserts git-lfs handling, then rsyncs and stages. After committing a refresh, re-verify (a refresh-only push does not re-trigger the path-filtered gate — documented in CONTRIBUTING).
4. **No auto-commit.** `createCommitOnBranch` cannot carry LFS objects and `GITHUB_TOKEN` pushes do not re-trigger workflows; a maintainer applies the artifact locally with one command. That is the deliberate delivery mode — the maintainer never runs the build.

Determinism lessons (both were latent bugs the CI gate exposed):

- **`Info.plist` slice ordering**: `xcodebuild -create-xcframework` emits `AvailableLibraries` in nondeterministic order — normalize before hashing, or the same inputs produce different bytes run-to-run.
- **bash 3.2 + `set -u` empty arrays**: macOS system bash expands an empty array as unbound under `set -u`; use the `${arr[@]+"${arr[@]}"}` idiom in scripts that must run on stock macOS bash.

Two extensions proven by the first FFI-ABI refresh round (2026-08-18):

5. **DEFAULT-suite smokes consume the committed natives — an ABI change forces a natives rebuild before suites can pass.** Per-channel default smoke suites link the committed dylibs (`bindings/go/native/darwin_arm64` — which the Swift macOS `swiftc` gate also links — `bindings/kotlin/native/darwin-aarch64`, the Python smoke dylib); a changed FFI surface leaves them symbol-stale (`nm` shows the new checksum/vtable symbols missing) and the suites fail at load/link, by design. "Natives build at publish time" assumptions do not cover committed-artifact smoke consumption: plan for rebuild + commit of every consumed native in the same slice as the ABI change, then re-run every channel's DEFAULT suite as the closing gate. C# is the exception — its smoke consumes the locally built cargo cdylib, so it proves scenario semantics ahead of the artifact wave.
6. **Branch-independent refresh via dispatch.** When the push trigger is branch-scoped (e.g. `main`/`iteration/**`), a feature branch gets zero runs; add a top-level `workflow_dispatch:` to the assembly workflow — dispatch-only, filters and drift gate untouched. The maintainer sequence on any branch: dispatch → expect drift-red (stale committed artifact vs new sources) while the upload succeeds → `apply-xcframework-artifact.sh <run-id>` → commit → dispatch again at the new head → drift-green (same-SHA determinism proof).

## Why This Matters

The drift gate makes silent staleness impossible (red job with the fix attached), and the apply path removes the fragile local build without weakening the committed-artifact distribution model. The apply-then-match pair (apply a CI artifact, next run reports no drift) is the determinism proof.

## When to Apply

Any committed compiled artifact refreshed from CI — today the Swift xcframework; the same shape fits the Go linux/windows natives when a cross-link path lands. Swift matrix specifics: `connect-swift-xcframework-ios-matrix.md`.

## Examples

- FFI-surface PR: `xcframework.yml` runs → drift detected → red job + artifact uploaded → maintainer runs `apply-xcframework-artifact.sh <run-id>` → commit → next run green (no drift).
- Non-FFI PR: path filter skips the job — zero macOS minutes.
