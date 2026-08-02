---
module: spoke-connect
date: 2026-08-02
problem_type: tooling_decision
category: tooling-decisions
severity: medium
applies_when: ["publishing a new connect surface from the protocol repo", "staging the first registry publish of a protocol-repo npm package", "deciding what a protocol repository publishes vs what consumer repositories own"]
tags: [spoke-connect, publish, npm, staging, lockstep-semver, trusted-publishing, bindings]
---

# Connect publish staging (lowest-ops surface first)

## Context

The connect embedding model ships several surfaces with very different publishing costs: a pure-TS client package, a Rust reference spike (libp2p + `cdylib`), per-language uniffi bindings, and an integrator docs site. Publishing them all like the core wire packages would force native builds, per-language registries, and version channels onto every release cut. The connect publish strategy (`.mstar/specs/connect-publish-strategy.md`) stages publishes so the cheapest, most valuable surface ships first and everything native or host-specific stays out of the registry.

## Guidance

### Stage by ops cost: pure JS/TS first, native/spike deferred, bindings never

| Surface | Stage | Rule |
|---|---|---|
| TS connect client (`@42ch/spoke-connect-ts`) | **Stage 1** — first registry publish | Lowest-ops: pure JS/TS, no native `cdylib` in the tarball; matches the existing npm Trusted Publishing path; the integrator docs site ships as the Stage 1 companion |
| Rust connect core / spike (`crates/spoke-connect`) | **Stage 2+** — deferred | `publish = false` stays; publish only after a slim core-only split or real crates.io consumer demand — default is keeping the full spike private (consumers embed via path-dep / git-dep) |
| uniffi bindings (Swift / C# / …) | **Never** from this repo | Host-specific artifacts: consumers vendor generated bindings + link the `cdylib`, or ship bindings from consumer repositories. The protocol repo owns generate scripts + smokes only; version tracking is the git tag / lockstep version of the `cdylib` they were generated from |

### Lockstep SemVer continuation

Published connect surfaces stay on the monorepo lockstep `X.Y.Z` (`.mstar/specs/spoke-version-release.md`): one version across schemas + operations + connect-ts, already covered by `release:bump` / `verify:version`; integrators pin one version across all packages. Revisit triggers for an independent `connect-ts@Y` channel: the package gains a **native optional dependency**, or its release cadence substantially diverges from the wire packages. Breaking pre-1.0 connect-ts API changes remain allowed; call them out in the CHANGELOG.

### Published-shape prep (before any registry publish)

For the TS package, prep lands metadata-only while `private: true` stays; the build is a Stage 1 execution step, not a prep flip:

- **Exports contract** — root `"."` + `"./node"` subpath only; no `./src/*` wildcard; Node-only `ws` / `connectClient` stay off the root barrel (subpath-only).
- **Files allowlist** — prep: `["src", "README.md"]`; Stage 1: `["dist", "README.md"]`.
- **SPDX license field** — `license` field only, mirroring published siblings; authoritative text at repo-root `LICENSE`; a tarball LICENSE copy is an explicit Stage 1 option (npm `files` does not auto-include a root LICENSE).
- **`dist/` build mandated at publish time** — the first npm publish MUST ship a built tarball: emit `dist/` (tsc or tsup), retarget `exports` (`import` → `./dist/*.js`, `types` → `./dist/*.d.ts`, same shape for `./node`), include `dist/` in `files`.
- **Packed-tarball import smokes** — `npm pack`, then import both the root and the `./node` subpath from the packed tarball on the supported Node versions (≥ 20.19.0) before the first publish.
- **Dependencies at publish** — `@42ch/spoke-schemas` resolves on npm at the same lockstep version (`workspace:*` rewritten on pack); `ws` remains a `./node`-subpath concern for browser consumers.
- **Registry + auth** — npm Trusted Publishing OIDC via the top-level `release.yml` (same org `42ch-dev` / repo `spoke` binding as the sibling packages); no long-lived `NPM_TOKEN`. Stage 1 extends the publish-npm list and **co-updates `.mstar/specs/spoke-version-release.md` in the same change** (row 6, publish-only lists, package-name/install tables) plus the root `AGENTS.md` connect boundary line. A future crates.io publish (Stage 2+) uses the same `crates-io-auth-action` pattern.

### When a gate blocks the pipeline: defer with a record

C# bindings hit a toolchain gap (`uniffi-bindgen-cs` targets uniffi 0.31 vs the repo pin 0.32) and are **deferred** with a decision record + revisit trigger ([`connect-csharp-bindgen-deferred.md`](../../specs/connect-csharp-bindgen-deferred.md)) instead of downgrading pins or forcing the pipeline. Same pattern for any blocked surface: record the blocker, keep the target priority, define the revisit trigger, re-check via the documented regenerate → build → run sequence.

## Why This Matters

Publishing order is a cost and trust decision, not ceremony: the first registry package sets the Trusted Publishing binding, the release-spec surface, and the version-channel precedent for every connect artifact after it. Shipping the lowest-ops surface first proves the whole pipeline end-to-end (build → pack → smoke → publish → docs) before any native `cdylib` or host-specific binding is involved; keeping bindings out of the registry avoids per-language packages whose provenance cannot be tied to a `cdylib` git tag.

## When to Apply

- Publishing any future connect surface (or any similar protocol-repo package) — stage by ops cost: pure first, native/spike deferred, bindings consumer-owned.
- Preparing a package for its first registry publish — apply the published-shape checklist: exports contract, files allowlist, SPDX license, `dist/` build, packed-tarball import smokes.
- A surface is blocked by tooling (e.g. bindgen version skew) — record the deferral + revisit trigger instead of downgrading pins or publishing unverified output.

## Examples

### Published-shape exports map (prep state — src-publish intent, `private: true`)

```json
"exports": {
  ".": {
    "types": "./src/index.ts",
    "import": "./src/index.ts",
    "default": "./src/index.ts"
  },
  "./node": {
    "types": "./src/node/connect-client.ts",
    "import": "./src/node/connect-client.ts",
    "default": "./src/node/connect-client.ts"
  }
}
```

## See also

- `.mstar/specs/connect-publish-strategy.md` — the decision record SSOT (surface inventory, stages, triggers, non-goals, WT / js-libp2p evaluations)
- `.mstar/specs/spoke-version-release.md` — lockstep SemVer + Trusted Publishing (normative)
- [`lockstep-semver-release.md`](../architecture-patterns/lockstep-semver-release.md) — the release pipeline this strategy extends
- [`connect-ts-client-sdk.md`](../architecture-patterns/connect-ts-client-sdk.md) — the TS package whose publish this strategy governs
- [`connect-session-core-ffi-boundary.md`](../architecture-patterns/connect-session-core-ffi-boundary.md) — the binding surface (bindings never registry-published from this repo)
