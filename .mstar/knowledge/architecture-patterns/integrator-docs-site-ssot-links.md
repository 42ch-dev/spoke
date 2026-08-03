---
module: spoke-docs
date: 2026-08-02
problem_type: architecture_pattern
category: architecture-patterns
severity: medium
applies_when: ["building consumer-facing documentation for a protocol repository", "deploying a VitePress docs site to GitHub Pages via Actions", "linking integrator pages to read-only spec SSOTs without duplicating them"]
tags: [docs, vitepress, github-pages, ssot-links, integrator-docs, docs-workflow, concurrency]
---

# Integrator docs site with SSOT links (VitePress + GitHub Pages)

## Context

Protocol facts live in `.mstar/specs/` (normative, read-only) — integrator docs must not fork them. The SPOKE docs site (`docs/`, VitePress, 17 pages) serves integrators with summaries and procedures while pointing every normative statement back at the spec SSOT via GitHub blob links. The site deploys to GitHub Pages from Actions, and the deploy workflow carries a concurrency lesson: a naive workflow-wide cancel-in-progress group would let a docs PR run cancel an in-flight main deploy.

## Guidance

### Content model: integrator summary + SSOT links, no body duplication

- Every page is an integrator-facing summary or procedure; anything normative (wire tables, MUST blocks, invariants) links to the spec blob (`https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/<file>.md`) instead of being copied. Pages end with a "Normative references" section listing the spec links (e.g. `docs/connect/bindings.md` → `spoke-connect.md` §Embedding model, `connect-csharp-binding.md`).
- `.mstar/specs/` stays read-only — the single source of truth; docs pages summarize and point at it. The same policy is documented for maintainers in `CONTRIBUTING.md` (Integrator docs site section).
- Consistency checks grep the site for copied spec tables / MUST-block duplication against the specs.

### VitePress layout

- `docs/` at repo root; `pnpm docs:build` is the CI gate; site config in `docs/.vitepress/config.mts`.
- `base: '/'` for the custom domain `https://spoke.42ch.dev/` (GitHub Pages project site; `*.github.io/spoke/` redirects to the custom domain). Use `base: '/spoke/'` only if the custom domain is removed and the site is served under the github.io project path again.
- One sidebar per audience: Guides, Domain Profiles, Connect, Packages, Release.
- Complements the EN/CN README twin and `CONTRIBUTING.md`: the site is the consumer-facing entry; maintainer / local-dev / release how-to stays in CONTRIBUTING.

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
