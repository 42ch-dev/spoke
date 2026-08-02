# Connect publish strategy

**Status:** Informative decision — does not change connect envelopes or the lockstep release policy for core wire packages.

**Updated:** 2026-08-02

---

## 1. Purpose

This document is the **publish-strategy SSOT** for SPOKE connect surfaces. Integrators and maintainers use it to answer:

- which connect artifacts become registry packages (and which stay workspace-private or consumer-repo artifacts);
- the ordered publish stages and the trigger for each stage;
- how versions move relative to core wire packages;
- which registries and auth model apply.

**Relationship to other specs:**

| Spec | Role |
|------|------|
| [`spoke-version-release.md`](spoke-version-release.md) | Normative lockstep SemVer, annotated tags, CI-gated GitHub Release, npm/crates.io Trusted Publishing for core packages |
| [`spoke-connect.md`](spoke-connect.md) | Normative connect wire, identity, framing, embedding model |
| [`spoke-connect-ts-route.md`](spoke-connect-ts-route.md) | Informative TS client stack route (pure-TS-minimal primary; js-libp2p mesh fallback) |
| This document | Informative **when / what / how to publish** connect client surfaces; does not fork wire or release lockstep rules |

---

## 2. Surface inventory

| Surface | Path today | Registry candidate | Current publish state |
|---------|------------|--------------------|------------------------|
| **TS connect client** | `packages/spoke-connect-ts` | npm `@42ch/spoke-connect-ts` | Workspace-private (`private: true`); lockstep version |
| **Docs site** | `docs/` | GitHub Pages (site, not a package registry) | Planned companion to Stage 1 |
| **Rust connect core / spike** | `crates/spoke-connect` | crates.io `spoke-connect` (deferred) | `publish = false` reference spike |
| **uniffi bindings (Swift / C# / …)** | `crates/spoke-connect/bindings/*` | Registry packages from this repo: none | Generate scripts + smokes in-repo; product bindings ship from consumer repositories |
| **Core wire packages** (context) | `@42ch/spoke-schemas`, `@42ch/spoke-operations`, crates `spoke-schemas` / `spoke-operations` | npm + crates.io | Already published under lockstep SemVer |

---

## 3. Publish staging

| Surface | Path today | Registry candidate | Stage | Trigger to publish / go live |
|---------|------------|--------------------|-------|------------------------------|
| **TS connect client** | `packages/spoke-connect-ts` (`private: true`, lockstep version) | **npm** `@42ch/spoke-connect-ts` | **Stage 1 — first publish** | (a) published-shape checklist complete; (b) golden + suite green on main; (c) maintainer elects connect publish in a release cut that extends `release.yml` publish-npm list; (d) docs site home links the package |
| **Docs site** | `docs/` | **GitHub Pages** (not a package registry) | **Stage 1 companion** | Docs workflow deploys on main; not blocked on npm publish |
| **Rust connect core / spike** | `crates/spoke-connect` (`publish = false`) | crates.io `spoke-connect` **deferred** | **Stage 2+** | Separate decision: either (i) publish a **slim** `spoke-connect-core` (sync core only, no libp2p) or (ii) keep spike private and document path-dep / git-dep embed — **default recommendation: keep full spike `publish = false`**; revisit when a crates.io consumer demand is real and the core is split from libp2p |
| **uniffi bindings (Swift/C#/…)** | `crates/spoke-connect/bindings/*` | **Not published** from this repo | **Never as registry packages here** | Consumers vendor generated bindings + link `cdylib` **or** ship bindings from **consumer repositories**; protocol repo keeps generate scripts + smokes only |
| **Core wire packages** (context) | `@42ch/spoke-schemas`, `@42ch/spoke-operations`, crates | Already on npm/crates.io | **Unchanged** | Existing `spoke-version-release.md` / `release.yml` |

**Staging rationale:** The TS Path A client is the lowest-ops publish (pure JS/TS, no native cdylib in the tarball) and matches the existing npm Trusted Publishing path. The Rust spike carries libp2p + cdylib policy and is not first. Bindings are host-specific artifacts; product connect bindings ship in consumer repositories.

**Owner class per stage:**

| Stage | Owner class |
|-------|-------------|
| Stage 1 npm `@42ch/spoke-connect-ts` | Maintainer CI — extend top-level `release.yml` publish-npm list + Trusted Publishing OIDC binding |
| Stage 1 companion docs site | Docs workflow (GitHub Pages deploy on main) |
| Stage 2+ Rust core | Maintainer decision + release cut only after slim-core split or explicit keep-private confirmation |
| Bindings | Consumer repositories (or path/git embed); protocol repo maintainers own generate scripts + smokes only |

---

## 4. Versioning recommendation

| Decision | **Continue monorepo lockstep SemVer** for `@42ch/spoke-connect-ts` (and any future published connect crate that remains a workspace member) |
|----------|--------------------------------------------------------------------------------------------------------------------------------------------|
| Rationale | Already asserted in `spoke-version-release.md` row 6 / row 10; integrators pin one `X.Y.Z` across schemas + operations + connect-ts; `release:bump` / `verify:version` already cover the package; per-surface independent SemVer would fork bump scripts, CHANGELOG story, and Trusted Publishing mental model for little gain pre-1.0 |
| Pre-1.0 note | Breaking connect API changes still allowed without long deprecation; call out in CHANGELOG when connect-ts public surface breaks |
| Revisit trigger | Connect-ts gains a **native optional dependency** or a **substantially different release cadence** from wire packages → then evaluate `connect-ts@Y` independent channel; record in this decision doc before splitting |
| Bindings | No registry version — track against the **git tag / spoke lockstep version** of the cdylib they were generated from |

---

## 5. Registry & auth

| Target | Artifact | Auth model |
|--------|----------|------------|
| **npm** | `@42ch/spoke-connect-ts` (Stage 1) | **Trusted Publishing OIDC** via top-level `.github/workflows/release.yml` (same org `42ch-dev` / repo `spoke` binding as `@42ch/spoke-schemas` and `@42ch/spoke-operations`). No long-lived `NPM_TOKEN` repository secret for release publish |
| **crates.io** | `spoke-connect` (Stage 2+, only if publish decision revises) | Same Trusted Publishing pattern as `spoke-schemas` / `spoke-operations` (`rust-lang/crates-io-auth-action` → short-lived `CARGO_REGISTRY_TOKEN`); OIDC binds to top-level `release.yml` filename |
| **GitHub Pages** | Integrator docs site from `docs/` | Pages deploy workflow on main; separate from registry Trusted Publishing |

**Alignment constraints (from `spoke-version-release.md` / repo release policy):**

- `release.yml` remains the **sole top-level publish workflow** (tag push `v*` or `release`-labeled PR merge). Trusted Publishing OIDC binds to that filename — keep Stage 1 npm publish inside `publish-npm`, do not wrap via `workflow_call`.
- Stable tags `vX.Y.Z` and prerelease SemVer tags without `-rc.` publish registries; tags containing `-rc.` create GitHub pre-releases only.
- Stage 1 execution extends the publish-npm package list to include `@42ch/spoke-connect-ts` after published-shape and suite gates are green; until then the package stays `private: true`.

---

## 6. TS package published-shape checklist

Package: `packages/spoke-connect-ts` (`@42ch/spoke-connect-ts`).

| Field | Locked shape (prep → Stage 1) |
|-------|-------------------------------|
| `private` | **`true` remains** until Stage 1 execution |
| `name` / `version` / `license` | Keep; version stays lockstep with monorepo |
| `type` | `"module"` |
| `exports` | **Root + `./node` subpath only** — no `./src/*` wildcard; Node-only `ws` / `connectClient` stay off the root barrel |
| | Prep (workspace / src-publish intent): |
| | ```json |
| | "exports": { |
| |   ".": { |
| |     "types": "./src/index.ts", |
| |     "import": "./src/index.ts", |
| |     "default": "./src/index.ts" |
| |   }, |
| |   "./node": { |
| |     "types": "./src/node/connect-client.ts", |
| |     "import": "./src/node/connect-client.ts", |
| |     "default": "./src/node/connect-client.ts" |
| |   } |
| | } |
| | ``` |
| | Stage 1 first npm publish **should** emit `dist/` (tsc or tsup) and point `exports` at JS + `.d.ts`. Src-only npm is acceptable only as an explicit interim; **default recommendation = add build before first publish**. |
| `files` | Prep: `["src", "README.md", "LICENSE"]` (exclude `tests/`, `vitest.config.*`, tsbuildinfo). Stage 1 with build: `["dist", "README.md"]` (plus license as configured) |
| Metadata | `repository` `{ type, url: git+https://github.com/42ch-dev/spoke.git, directory: packages/spoke-connect-ts }`; `homepage` (docs site URL once live, else repo URL); keep `keywords`, `description`, `engines.node` (≥20.19.0) |
| `publishConfig` | `{ "access": "public" }` present or documented while `private: true` (inert until Stage 1). Provenance: npm Trusted Publishing OIDC via `release.yml` |
| Dependencies at publish | `@42ch/spoke-schemas` resolves on npm at the **same lockstep version** (`workspace:*` rewritten on pack). `ws` remains a dependency of the `./node` subpath only — browser consumers import `"."` only |
| README | Short **Publish guidance**: private until Stage 1; subpath map; peer/lockstep expectation on `@42ch/spoke-schemas` |
| Behavior | Published-shape is metadata/exports only until Stage 1; workspace consumers keep current import paths |

**Prep owner:** package maintainer applying the checklist in-repo. **Stage 1 flip** (`private: false` + `release.yml` list) is a separate maintainer release cut after the checklist and suites are green on main.

---

## 7. Bindings disposition

uniffi-generated bindings under `crates/spoke-connect/bindings/*` are **not registry packages from this repository**.

| Fact | Detail |
|------|--------|
| What this repo ships | Generate scripts, smokes, and the sync-core `ffi` surface on the unpublished `spoke-connect` crate |
| What consumers do | Vendor generated bindings + link the `cdylib`, **or** ship language bindings from **consumer repositories** |
| Version tracking | Bind against the **git tag / spoke lockstep version** of the cdylib they were generated from (no independent registry SemVer) |
| Language matrix | **C#, Go, Python, Swift, Kotlin — C# first** (Swift skeleton landed; C# deferred on bindgen toolchain gap — see [`connect-csharp-bindgen-deferred.md`](connect-csharp-bindgen-deferred.md)) |

---

## 8. WebTransport evaluation

| Item | Value |
|------|--------|
| **Conclusion** | **Defer implementation** — keep **WebSocket** as the default Path A transport for `@42ch/spoke-connect-ts` |
| **Evidence** | *pending T3 evidence* (browser availability matrix; ordered bidirectional streams vs WebSocket message framing; fit to connect “one JSON envelope per message”; Node support maturity) |
| **Recommend** | *pending T3 evidence* — structure locked: recommend WebSocket default; WT as future framing option under transport-adapter ownership |
| **Defer** | Implementation this/next slice unless a product host **requires** WebTransport |
| **Trigger to implement** | Product request + CI-able browser test plan |
| **Docs** | Docs may mention WebTransport as a future framing option; transport-adapter ownership stays per [`spoke-connect.md`](spoke-connect.md) / [`spoke-connect-ts-route.md`](spoke-connect-ts-route.md) |

---

## 9. js-libp2p evaluation

| Item | Value |
|------|--------|
| **Conclusion** | **Remain fallback route only** — does not overturn pure-TS-minimal primary ([`spoke-connect-ts-route.md`](spoke-connect-ts-route.md)) |
| **Evidence** | *pending T3 evidence* (when Noise/yamux mesh with `crates/spoke-connect` is required; dependency weight; session-core still owns JCS hello) |
| **Recommend** | *pending T3 evidence* — structure locked: do **not** add `js-libp2p` to `@42ch/spoke-connect-ts` default dependencies |
| **Defer** | Shipping a mesh helper inside the default package |
| **Trigger to ship a mesh helper** | Product requires shared libp2p network with the Rust spike; then prefer an **optional companion package or consumer-repo adapter**, not a forced default export |
| **Primary route** | pure-TS-minimal (WebSocket + WebCrypto/`@noble` Ed25519 + JCS + ported session core) remains locked |

---

## 10. Non-goals

| Non-goal | Detail |
|----------|--------|
| Daemon / runtime packages | No connect daemon, MCP server, or multi-product runtime package from this repo |
| Bindings registry publish | No npm/crates.io (or language-specific registry) packages for uniffi bindings from this monorepo |
| Per-surface independent SemVer pre-1.0 | No split version channels for connect-ts vs schemas/ops until the revisit trigger in §4 fires |
| Premature registry publish | Stage 1 executes only after published-shape + suite gates + maintainer release cut; prep leaves `private: true` and crate `publish = false` |
| Overturn pure-TS-minimal | This strategy does not re-litigate the TS connectivity primary route |
| Wire / schema changes | Publish strategy is packaging and staging only; connect envelopes stay under `spoke-connect.md` |

---

## 11. Execution roadmap pointer

| Slice | What ships | Trigger | Owner |
|-------|------------|---------|-------|
| **connect publish execution (Stage 1)** | First registry publish of `@42ch/spoke-connect-ts` + docs site live; extend `release.yml` publish-npm list + Trusted Publishing; keep `spoke-connect` crate and bindings unpublished unless Stage 2 revises | Decision record accepted on main + published-shape checklist complete + golden/suite green on main + maintainer release cut | Release maintainer / next delivery slice |

Mirror row: [`.mstar/roadmap.md`](../roadmap.md) **Up next**.

---

## 12. Links

| Path | Use |
|------|-----|
| [`.mstar/specs/spoke-connect.md`](spoke-connect.md) | Normative connect wire + embedding |
| [`.mstar/specs/spoke-connect-ts-route.md`](spoke-connect-ts-route.md) | TS client route (pure-TS-minimal) |
| [`.mstar/specs/spoke-version-release.md`](spoke-version-release.md) | Lockstep SemVer + Trusted Publishing |
| [`.mstar/specs/connect-csharp-bindgen-deferred.md`](connect-csharp-bindgen-deferred.md) | C# uniffi bindgen version-gap record |
| [`.mstar/roadmap.md`](../roadmap.md) | Durable project roadmap (Stage 1 execution row) |
| [`packages/spoke-connect-ts/`](../../packages/spoke-connect-ts/) | TS connect client package |
| [`crates/spoke-connect/`](../../crates/spoke-connect/) | Rust reference spike (`publish = false`) |
| [`crates/spoke-connect/bindings/`](../../crates/spoke-connect/bindings/) | uniffi binding trees (not registry products here) |
| [`.github/workflows/release.yml`](../../.github/workflows/release.yml) | Top-level publish workflow (OIDC Trusted Publishing) |
