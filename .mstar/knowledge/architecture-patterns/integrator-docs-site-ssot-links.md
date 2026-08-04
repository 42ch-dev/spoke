---
module: spoke-docs
date: 2026-08-02
last_updated: 2026-08-04
problem_type: architecture_pattern
category: architecture-patterns
severity: medium
applies_when: ["building consumer-facing documentation for a protocol repository", "deploying a VitePress docs site to GitHub Pages via Actions", "structuring integrator docs to serve how-to-use as the primary job", "consolidating fragmented conceptual content without losing wire facts", "removing internal agent-spec links from integrator pages"]
tags: [docs, vitepress, github-pages, ssot-links, integrator-docs, docs-workflow, concurrency, diataxis, audience-boundary, en-cn-twin]
---

# Integrator docs site with consolidated reference and zero spec links (VitePress Diátaxis)

## Context

Protocol facts live in `.mstar/specs/` (normative, read-only) — integrator docs must not fork them, and integrators should not have to follow `.mstar/specs/` links to verify a wire fact. The SPOKE docs site (`docs/`, VitePress) serves integrators with a Diátaxis four-quadrant layout (Tutorials / How-to guides / Reference / Explanation): the **Reference** pages carry the field tables and wire shapes integrators need to answer "what fields / what values allowed" without leaving the site, while the **Tutorials** and **How-to guides** route the two primary integrator jobs (Build an Adapter / Open a Connect session) as first-class sidebar entries. The site deploys to GitHub Pages from Actions, and the deploy workflow carries a concurrency lesson: a naive workflow-wide cancel-in-progress group would let a docs PR run cancel an in-flight main deploy.

## Guidance

### Audience boundary: no `.mstar/specs/` links on integrator pages

`docs/**/*.md` (EN + CN) **must not** link to `.mstar/specs/*.md`. The agent-facing specs are an internal SSOT for contributors working in this repository; integrators consume the published packages and need wire facts on the docs site itself. The CI twin-parity + dead-link gates plus a grep for `.mstar/specs` enforce the boundary. Replace removed spec links with:

- **On-site Reference field tables** for wire shapes (the consolidated `reference/{protocol,data-model,ops,connect}.md` pages lift the field tables onto the page itself).
- **External canonical references** (RFC 8785, RFC 4648, rust-docs, npm, crates.io, GitHub source paths under `schemas/` / `packages/` / `crates/`).
- **Root `CONCEPTS.md`** as an optional vocabulary cross-link (vocabulary SSOT, not under `.mstar/specs/`).

### Diátaxis four-quadrant content model

The site is organized around the four Diátaxis quadrants, each with a single integrator purpose:

| Quadrant | Purpose | Pages (EN + CN 1:1 twins) |
|----------|---------|---------------------------|
| **Tutorials** (learning-oriented) | First-time path — install, first KnowledgeEntry round-trip, then first connect session | `tutorials/install-and-first-entry.md`, `tutorials/first-connect-session.md` |
| **How-to guides** (problem-oriented) | The two integrator jobs: Adapter implementation + Connect usage | `how-to/implement-adapter.md`, `how-to/orchestrate-ops.md`, `how-to/connect-ts-client.md`, `how-to/connect-native-bindings.md`, `how-to/walk-toy-world.md` |
| **Reference** (information-oriented) | Field tables and wire shapes — the on-site replacement for spec links | `reference/protocol.md`, `reference/data-model.md`, `reference/ops.md`, `reference/connect.md` |
| **Explanation** (understanding-oriented) | Key statements and vocabulary — no handbook sprawl | `explanation/concepts.md` (layers L0–L8 + capability flags + dual-concern pairs), `explanation/domain-profiles.md` (per-profile key statements + published open-string vocabulary tables) |

Plus `packages/quick-start.md` (install pins) and `release/versioning.md` (lockstep SemVer) as standalone pages referenced from the tutorials and sidebar. The home page (`docs/index.md`) carries two primary CTAs that route both jobs in one click: **"Build an Adapter"** → `how-to/implement-adapter`, **"Open a Connect session"** → `tutorials/first-connect-session`.

**Affirmative facts only** on every integrator page: state what the protocol and packages do, not what they do not do. No "out of scope", no anti-pattern lists, no "formerly / superseded", no agent-facing constraint tables. Negation and agent constraints belong only in root `AGENTS.md` and (when normative) in `.mstar/specs/`.

### Maintainer procedure lives in `CONTRIBUTING.md`, not `docs/how-to/`

Release-cut (`New release` workflow_dispatch → merge → `release.yml` publish) is a maintainer procedure that targets a different audience than the integrator docs. The sidebar links directly to root `CONTRIBUTING.md` on GitHub under a "Maintainers" group; `docs/how-to/cut-a-release` does **not** exist. The integrator-facing `docs/release/versioning.md` lockstep SemVer page stays — it serves integrators who consume published versions, not maintainers who cut them.

### Wire-fact fidelity (no silent loss during consolidation)

When consolidating fragmented conceptual content (e.g., merging the old 7-page `guide/` + 4-page `profiles/` + 3-page `connect/` trees into the four quadrants above), lift every normative fact onto its planned home. Facts at risk of silent loss in protocol-repo docs restructures:

1. **Binding package coordinates** — inline on the native-bindings how-to (NuGet `42ch.Spoke.Connect`, Maven `dev.42ch:spoke-connect`, SPM `SpokeConnect`, Go module path, PyPI `spoke-connect`). Do not leave coordinates behind a spec link.
2. **Domain Profile open-string vocabulary tables** — per-profile vocabulary (beat types, activation fields, pack dialect keys, placement hints) on the consolidated `explanation/domain-profiles.md`. Full mapping matrices remain agent SSOT in specs.
3. **Schema inventory count + codegen verify posture** — explicit subsection on `reference/protocol.md` (e.g., 30 committed `*.schema.json` files; `verify-codegen` drift gate).
4. **Discovery / peering boundary** (explicit peering as production path; mDNS as same-LAN dev convenience) — affirmative paragraph on `reference/connect.md`.
5. **Operations purity boundary** (no I/O, storage, LLM, HTTP, ranking, retrieval, silent auto-promote) — affirmative form on `reference/ops.md` and `how-to/orchestrate-ops.md`.

### VitePress layout

- `docs/` at repo root; `pnpm docs:build` is the CI gate; site config in `docs/.vitepress/config.mts`.
- `base: '/'` for the custom domain `https://spoke.42ch.dev/` (GitHub Pages project site; `*.github.io/spoke/` redirects to the custom domain). Use `base: '/spoke/'` only if the custom domain is removed and the site is served under the github.io project path again.
- One sidebar per Diátaxis quadrant, in both locales (EN root + `zh/` CN tree). Sidebar labels: Tutorials / How-to guides / Reference / Explanation (CN: 教程 / 操作指南 / 参考 / 讲解).
- Maintainer release procedure linked from a "Maintainers" sidebar group to root `CONTRIBUTING.md`; no `docs/how-to/cut-a-release` page.
- Complements the EN/CN README twin and `CONTRIBUTING.md`: the site is the consumer-facing entry; maintainer / local-dev / release how-to stays in `CONTRIBUTING.md`.

### EN ↔ CN twin parity (HARD CI gate)

Every page under `docs/<path>.md` has a twin at `docs/zh/<path>.md`. Page set is 1:1; `tooling/docs/twin-parity.mjs` fails the docs build on drift. Wire identifiers (`KnowledgeEntry`, `peer_id`, `orchestrateUpsert`, `ConnectHello`, …) stay EN on CN pages per the docs i18n glossary. Author EN first, CN immediately after each page to avoid drift.

### GitHub Pages deploy concurrency

The deploy workflow (`.github/workflows/docs.yml`) uses a **per-branch** concurrency group (e.g., `docs-${{ github.ref }}`), not workflow-wide. A workflow-wide `cancel-in-progress: true` group would let a docs PR run cancel an in-flight main deploy — a subtle race that loses the published site for the window between the cancel and the next merge. Per-branch groups let long-running main deploys finish while PR previews cancel each other freely.

### GitHub Pages deploy via Actions (SHA-pinned)

- Build gate on PRs touching `docs/**` + tooling (the workflow itself, `package.json`, `pnpm-lock.yaml`); deploy on push to `main` only.
- Official Pages actions, pinned to immutable commit SHAs (stricter than the repo's other workflows): `actions/upload-pages-artifact` + `actions/deploy-pages`; repo Settings → Pages → Source must be **GitHub Actions**.
- Deploy job permissions: `contents: read`, `pages: write`, `id-token: write`; environment `github-pages` with the deployment URL.

### Concurrency lesson: ref-scoped group for builds, main-only group for deploys

- Workflow-level `concurrency.group: docs-${{ github.ref }}` + `cancel-in-progress: true` — latest-wins **per ref**: a newer main push cancels an older main build, and PR runs never share the group with main runs, so a docs PR push can never cancel an in-flight main build or deploy.
- The deploy job keeps its own `concurrency.group: pages` (main-only, since the job is gated `if: github.ref == 'refs/heads/main'`) — deploy contention stays main-only; a newer main deploy cancels an in-flight one (latest main wins).
- The official Pages starter's `cancel-in-progress: false` (or a single workflow-wide group) does not provide this: prefer the newest content on every push to `main` while isolating PR churn from the production deploy.

## Why This Matters

An integrator site for a protocol repo must be a **window onto the specs, not a fork of them**: duplicated normative text rots and silently diverges, while SSOT links keep one authoritative body to update. The concurrency split protects the production Pages deploy from PR churn — without it, a burst of docs PR pushes can cancel the deploy mid-flight and leave the site stale.

## When to Apply

- Building consumer-facing docs for any protocol repository — integrator summaries + GitHub blob SSOT links; specs read-only; no body duplication.
- Deploying a VitePress site to GitHub Pages from Actions — SHA-pinned Pages actions, Pages source "GitHub Actions", ref-scoped build concurrency + main-only deploy group, `base: '/'` when a custom domain is attached; otherwise `base: '/<repo>/'` for bare project sites.
- Locale twin sites (EN root + `zh/`) — a zero-dep twin-parity script (`tooling/docs/twin-parity.mjs`) gates page-count drift: every `*.md` under the root locale has a path-identical twin under `zh/` (and vice versa), with an explicit allow-list for justified locale-specific pages. A dead-link script (`tooling/docs/deadlink-check.mjs`) crawls the built `dist/**/*.html` for internal `href`s under the site base and fails on any 404. Both run in the `build` job before `upload-pages-artifact`, on the same `docs/**` + `tooling/docs/**` path filter.

## Examples

### Concurrency split (`.github/workflows/docs.yml`)

```yaml
concurrency:
  group: docs-${{ github.ref }}   # per-ref latest-wins; PR runs never share the group with main runs
  cancel-in-progress: true

jobs:
  build:
    # checkout, pnpm install, pnpm docs:build, upload-pages-artifact

  deploy:
    needs: build
    if: github.ref == 'refs/heads/main'   # main-only
    concurrency:
      group: pages                        # main-only deploy serialization
      cancel-in-progress: true
    steps:
      - name: Deploy to GitHub Pages
        uses: actions/deploy-pages@<sha>  # SHA-pinned
```

## See also

- `.github/workflows/docs.yml` — the landed workflow (build gate + Pages deploy)
- `docs/.vitepress/config.mts` — site config (`base: '/'` for `spoke.42ch.dev`)
- [`consumer-readme-twin.md`](consumer-readme-twin.md) — the EN/CN README twin pattern the site complements
- `.mstar/specs/` — the normative SSOT the site summarizes and links to
