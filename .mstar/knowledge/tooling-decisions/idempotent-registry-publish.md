---
module: release / registry publish
date: 2026-08-18
problem_type: tooling_decision
category: tooling-decisions
severity: medium
plan_id: release-idempotent-skip
tags:
  - release
  - publish
  - idempotent
  - pypi
  - maven
  - pre-check
  - re-probe
  - fail-loud
---

# Idempotent registry publish — pre-check + per-file guard + unconditional re-probe

## Context

A Release workflow publishes one lockstep version to several registries (npm, crates.io, NuGet, PyPI, Maven/GitHub Packages). When one job fails after sibling registries succeeded, the maintainer re-runs the failed job. Registries reject duplicate uploads (PyPI: duplicate-file error; GitHub Packages Maven: 409), so a naive re-run hard-fails and forces manual registry surgery.

## Guidance

Three cooperating mechanisms make a publish job safe to re-run:

1. **Pre-check is the primary mechanism.** A testable script (not inline YAML) queries the registry for the exact expected artifact set at the tag version and emits `publish_needed` / `missing_files` outputs. The build+publish chain is gated on it — a fully published version skips the expensive build entirely.
2. **Per-file duplicate guard.** Where the publish tool offers one (PyPI: `skip-existing: true` on `pypa/gh-action-pypi-publish`), enable it as a guard for the partial case. Maven/Gradle has no equivalent — see the arbiter rule below.
3. **Unconditional confirm re-probe closes the job.** An `if: always()` step re-queries the registry after any path (skip or publish) and greens only on a verified full set. For publish tools that hard-fail on duplicates (Gradle 409), put `continue-on-error: true` on the publish step so the re-probe is the sole arbiter: still-partial → red; full set after a concurrent winner → green.

Semantics that must hold:

- **Fail loud on doubt.** Registry unreachable, auth failure, malformed payload, unexpected response shape → non-zero exit. Never skip on doubt.
- **Full-set skip; partial never silent-green.** Skip only when every expected artifact is present. A partial state attempts a publish and is judged by the re-probe.
- **Expected-set-subset semantics.** Extra registry entries outside the locked expected set do not fail the check (registries are append-mostly; a historical extra must not permanently block re-runs) — but log them to stderr for observability.
- **Content sanity on 200s.** A 200 with an HTML error page is not "present" — sniff body prefixes (`<?xml` for POM, `{` for `.module`) and verify archive contents (e.g. JNA resource entries inside a jar) where the artifact shape matters.
- **Tag↔manifest cross-check before any probe.** Exit non-zero when the checked-out tree's manifest version disagrees with the tag SemVer — otherwise a drifted tag can upload wrong-version artifacts before the confirm step reds.

Registry facts that shaped the implementation:

- **PyPI**: JSON API `pypi.org/pypi/<pkg>/<version>/json`; `urls[].yanked === true` must count as **absent** (PEP 592 — yank is the only un-publish action; a yanked file is not installable). Wheel filenames carry **PEP 440-normalized** versions (`0.2.0-alpha.1` → `0.2.0a1`) — normalize before constructing expected filenames.
- **GitHub Packages Maven**: no standard "exists" API; probe the artifact paths with an authenticated GET (200 present / 404 absent / 401-403 auth-fail → fail loud). This repo's Kotlin publication has **no classifiers** — natives ride inside the jar as JNA resources; the expected set is `pom` + `module` + jar (verify `.module` serving once against the live registry with a `read:packages` token).

## Why This Matters

Re-run safety removes the last manual release-lane recovery step. The confirm-re-probe-as-arbiter design keeps one green criterion — a verified full set — regardless of which path (skip, publish, concurrent-winner) produced it.

## When to Apply

Any new publish job or new registry channel for this repo. Reference implementation: `tooling/release/check-pypi-published.mjs` + `check-maven-published.test.mjs` (fixtures pin the contract: present/absent/partial/auth-error/malformed/yanked/prerelease-normalization) wired in `release.yml` (`publish-pypi`, `publish-maven` jobs). Re-run semantics are recorded in `.mstar/specs/connect-publish-strategy.md`.

## Examples

- Skip path: all 3 wheels present → pre-check `publish_needed=false` → build/publish skipped → confirm re-probe verifies → green in seconds.
- Partial path: 2 of 3 wheels present → publish attempted → `skip-existing` tolerates the 2 duplicates → confirm verifies the full set → green.
- Doubt path: PyPI 5xx → pre-check exits non-zero → job red with a clear message → safe re-run later.
