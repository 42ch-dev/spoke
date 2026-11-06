---
module: connect / CI native artifacts
date: 2026-08-18
last_updated: 2026-09-22
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

# CI-assembled committed native artifacts — functional checks and apply path

## Context

This repo commits compiled native artifacts (the three-slice `spoke_connectFFI.xcframework`, binding natives) so consumers resolve them without a Rust toolchain. The assembly was historically a maintainer-local build: slow, machine-bound (macOS + four Apple targets), and drift-prone — the committed artifact could silently diverge from the Rust sources.

## Guidance

The pattern has four parts:

1. **Required-check CI assembly.** `xcframework.yml` builds from the checkout on every pull request on `macos-15`, using a pinned toolchain and `--locked`; the push trigger remains path-filtered.
2. **Consumer functionality.** CI builds the committed Swift package and runs the functional smoke against its committed macOS slice. A failed smoke still allows the built refresh artifact to upload and fails the job afterward.
3. **One-command apply, fail-closed.** `apply-xcframework-artifact.sh <run-id>` verifies run provenance, the mandatory transfer manifest and producer record, checks git-lfs handling, then stages the artifact. It accepts a failure solely on the committed consumer smoke when upload succeeded.
4. **No auto-commit.** `createCommitOnBranch` cannot carry LFS objects and `GITHUB_TOKEN` pushes do not re-trigger workflows; a maintainer applies the artifact locally with one command. That is the deliberate delivery mode — the maintainer never runs the build.

Determinism lessons (both were latent bugs the CI gate exposed):

- **`Info.plist` slice ordering**: `xcodebuild -create-xcframework` emits `AvailableLibraries` in nondeterministic order — normalize before hashing, or the same inputs produce different bytes run-to-run.
- **bash 3.2 + `set -u` empty arrays**: macOS system bash expands an empty array as unbound under `set -u`; use the `${arr[@]+"${arr[@]}"}` idiom in scripts that must run on stock macOS bash.

Two extensions proven by the first FFI-ABI refresh round (2026-08-18):

5. **Consumer smokes exercise committed natives.** A changed FFI surface may require refreshed natives before the consumer smokes pass. The Swift smoke links the committed xcframework's macOS slice.
6. **Branch-independent refresh via dispatch.** `workflow_dispatch` permits an assembly run for a pushed branch independently of the push filter.

## Why This Matters

Functional consumer checks establish behavior while the artifact manifest protects the refresh transfer.

## When to Apply

Any committed compiled artifact refreshed from CI — today the Swift xcframework; the same shape fits the Go linux/windows natives when a cross-link path lands. Swift matrix specifics: `connect-swift-xcframework-ios-matrix.md`.

## Examples

- FFI-surface PR: CI builds and uploads the artifact, and a maintainer applies it when the committed consumer smoke requires a refresh.
- Non-FFI PR: the required `xcframework` check still runs (pull-request triggers are unfiltered) — the macOS minutes are the price of a required context that must never be skipped.
