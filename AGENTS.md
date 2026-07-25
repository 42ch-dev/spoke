# SPOKE — AGENTS.md

**SPOKE** = Standardized Programmable Ontology Knowledge Engine.

Greenfield protocol repository: JSON Schema SSOT for narrative KnowledgeEntry **data** and **ops** wire shapes, with generated TypeScript (`@42ch/spoke-schemas`) and Rust (`spoke-schemas`, `spoke-operations`) packages.

Normative entry: [`.mstar/specs/spoke-protocol.md`](.mstar/specs/spoke-protocol.md).

## Harness

Morning Star consumer. Harness SSOT: [`.mstar/AGENTS.md`](.mstar/AGENTS.md).

**Process vs results:** process paths under `.mstar/` (plans, iterations, status, notes, sdd, archived) are gitignored; shared results are `.mstar/specs/` and `.mstar/knowledge/`. See harness AGENTS for the full table.

**Git-shared records (HARD):** tracked specs, knowledge, roadmap, and root docs MUST NOT contain Morning Star iteration ids (e.g. the `v0-iterNNN` family), `iteration:v0-iterNNN` source tags, or links into `.mstar/iterations/`. Use capability, feature, or date naming instead. Iteration ids belong only in local process artifacts (`plans/`, `iterations/`, `sdd/`, `status.json`). Commit messages, PR titles/bodies, and other git-visible prose follow the same ban (branch names used for local orchestration may keep harness prefixes).

### Long-term precipitation — facts only (HARD)

Tracked **results** that agents and humans reuse as protocol SSOT must state **current facts**, not delivery archaeology:

| In (facts) | Out (do not deposit) |
|------------|----------------------|
| Current wire names, fields, vocabularies, dual-concern rules, capability levels | Deprecated / retired names (e.g. draft Keyblock), rename history, “formerly / superseded / early drafts” |
| Normative invariants and positive capability | Iteration ship banners, plan ids, “in this slice we renamed…” |
| Protocol-neutral examples | Named external product runtimes as concept owners inside `CONCEPTS.md` (product binding tables belong in adapters / future Showcases — not here) |

**Applies to:** `.mstar/specs/`, `.mstar/knowledge/`, `.mstar/roadmap.md` (durable rows), root `CONCEPTS.md`, and other tracked result docs. Process paths (`plans/`, `iterations/`, `sdd/`) may record history locally; do not promote that narrative into results without rewriting as present-tense facts.

Do not put plan progress or residual detail in this file.

## Human-readable docs — affirmative facts only (HARD)

**Audience:** humans (protocol consumers / integrators). Applies to root `README.md` / `README_CN.md`, package/crate READMEs, and other consumer-facing prose. Keep the EN/CN twin outline where twins exist.

| In (state the result) | Out (agent / harness only — this file; normative exclusion may live in specs) |
|-----------------------|----------------------------------------------------------------------------------|
| What the protocol and packages **do** now | Negation / exclusion rhetoric (“not a runtime”, “does not include…”, “out of scope…”, “never…”, “no longer requires…”) |
| How to install, consume, release, contribute | Anti-pattern lists, In/Out constraint tables meant to steer agents |
| Positive capability and auth as configured | Delivery archaeology, “formerly / skip / without X”, migration leftover (“revoke the old secret”) |
| Current wire names and happy-path procedures | Iteration IDs, ship banners, “do not confuse X with Y” |

**Rule:** human-readable docs state **final facts** and procedures affirmatively. Negation, anti-patterns, boundary “must not”, and agent constraints belong **only** in this file (and in `.mstar/specs/` when they are normative protocol invariants). Do not deposit them in READMEs.

### Boundaries agents must enforce (not human-README copy)

- SPOKE is a **protocol repo**, not a product runtime, daemon, or shared database.
- `adapters/` holds **README purpose text only** for now — no product subdirs, packages, or mapping code until an iteration schedules them.
- Core interchange owns wire shapes only — world history, fork semantics, checker engines, ranking, and retrieval stay in products.
- `fixtures/toy-world/` owns protocol sample JSON and its AJV/Vitest harness (`tests/`; workspace package `@42ch/spoke-fixture-toy-world`). `@42ch/spoke-operations` is a pure helper library. Fixtures MAY import operations; operations MUST NOT import fixtures or host fixture validation I/O.
- `@42ch/spoke-operations` is pure: no I/O, storage, LLM, HTTP, MCP, ranking, retrieval, or silent auto-promote.
- Consumer packages `@42ch/spoke-schemas`, `@42ch/spoke-operations` (npm), and `spoke-schemas`, `spoke-operations` (crates.io) publish on stable tagged releases via CI Trusted Publishing only (npm OIDC + crates.io OIDC); fixture and codegen packages remain workspace-private.
- Finding is checker output, not KnowledgeEntry `body`.
- Registry auth for CI publish is Trusted Publishing on both ecosystems: npm packages `@42ch/spoke-schemas` / `@42ch/spoke-operations`, and crates.io crates `spoke-schemas` / `spoke-operations` (org `42ch-dev`, repo `spoke`, workflow `release.yml`; crates job uses `rust-lang/crates-io-auth-action` → short-lived `CARGO_REGISTRY_TOKEN` env). Do not use long-lived `NPM_TOKEN` / `CARGO_REGISTRY_TOKEN` repository secrets for release publish. Do not document “token no longer required / revoke old secret” in human READMEs — that negation belongs here only.
- Stable tags (`vX.Y.Z`) and prerelease SemVer tags without `-rc.` (e.g. `v0.1.0-alpha.3`) publish to npm and crates.io; tags containing `-rc.` create GitHub pre-releases only. If `publish-crates` fails after npm succeeded, re-run that job or `cargo publish` the missing crate at the tagged version.
- Maintainer cut path: **New release** (`workflow_dispatch` → GitHub-signed bump on `release/<version>` + open PR with label `release`) → merge (or close to abort) → **Tag release on merge** creates annotated `vX.Y.Z` and `workflow_call` **Release**. Org must allow “Allow GitHub Actions to create and approve pull requests” so `GITHUB_TOKEN` can `createPullRequest`.
- **New release** MUST refuse when the requested SemVer is not strictly greater than `package.json` on `main`, or when `vX.Y.Z` already exists. `release:bump` also refuses non-increasing bumps.
- **New release** commits MUST be GitHub-verified (`tooling/release/push-github-signed-commit.mjs` → GraphQL `createCommitOnBranch`). Do not land unsigned `git commit` + `git push` from Actions — `main` ruleset `required_signatures` blocks merge.
- **CHANGELOG:** `release:bump` MUST NOT prepend a second `## [X.Y.Z]` when that section already exists; promote the existing section to top instead (duplicate headings make `extract-changelog-notes.mjs` publish the wrong notes).
- **Release** (`release.yml`) is tag-driven (`push.tags: v*`) or called from **Tag release on merge** (`workflow_call`). Do not add `workflow_dispatch` on Release.

## Tech direction (v0.1)

- **SSOT:** `schemas/`
- **Codegen:** `json-schema-to-typescript` + `typify`
- **TS package:** `@42ch/spoke-schemas` → `packages/spoke-schemas/`
- **Rust crates:** `spoke-schemas` → `crates/spoke-schemas/`; `spoke-operations` → `crates/spoke-operations/`
- **Extensions:** `extensions.<namespace>` only; core fields closed
- **Adapters:** deferred — `adapters/README.md` only until scheduled

## Conflict priority

1. Current user instruction  
2. This file  
3. `.mstar/AGENTS.md`  
4. `mstar-*` skills  
