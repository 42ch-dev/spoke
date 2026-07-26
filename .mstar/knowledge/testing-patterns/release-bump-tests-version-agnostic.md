# Release bump unit tests must be SemVer-agnostic

**Category:** testing-patterns  
**Source:** compound 2026-07-26 (release bump CI hotfix)  
**Status:** durable

## Problem

`tooling/release/bump-version.test.mjs` used hardcoded from→to SemVer pairs (e.g. `0.1.0` → `0.1.1`). Release fixtures copy live root `package.json`. After a lockstep bump lands on `main`, the fixture's current version equals the hardcoding target and happy-path / refuse-path assertions fail in CI on every subsequent PR until tests are hand-edited again.

## Decision

1. **Derive versions from the fixture** — read the fixture tree's lockstep version as `current`; compute relative targets in the test (next patch, strictly lower core SemVer, drift suffix).
2. **Never hardcode live repo SemVer** in release unit tests that seed from copied `package.json`.
3. **Refuse / drift paths** use helpers such as `strictlyLowerRelease(current)` and `${current}-drift.test` so assertions stay valid across bumps.

## What not to do

- Do not assert exact string pairs that match today's published package version.
- Do not skip `pnpm run test:release` when changing bump/assert tooling.

## Related

- Lockstep release SSOT: `architecture-patterns/lockstep-semver-release.md`
- Tests: `tooling/release/bump-version.test.mjs`
- Spec: `.mstar/specs/spoke-version-release.md`
