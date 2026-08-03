# Connect publish strategy

**Status:** Informative decision — does not change connect envelopes or the lockstep release policy for core wire packages.

**Updated:** 2026-08-03

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

| Surface | Path today | Registry | Publish state |
|---------|------------|----------|---------------|
| **TS connect client** | `packages/spoke-connect-ts` | npm `@42ch/spoke-connect` | Published lockstep via `release.yml` |
| **Docs site** | `docs/` | GitHub Pages | Live companion site |
| **Rust connect crate** | `crates/spoke-connect` | crates.io `spoke-connect` | Published lockstep via `release.yml` |
| **C# NuGet binding** | `crates/spoke-connect/bindings/csharp/` | GitHub Packages `42ch.Spoke.Connect` | Published lockstep via `release.yml` `publish-nuget` |
| **Other uniffi bindings** (Swift / Go / …) | `crates/spoke-connect/bindings/*` | GitHub Packages (language-specific) when each binding is package-ready | Generate scripts + smokes in-repo; packages follow the C# pattern |
| **Core wire packages** (context) | `@42ch/spoke-schemas`, `@42ch/spoke-operations`, crates `spoke-schemas` / `spoke-operations` | npm + crates.io | Published under lockstep SemVer |

---

## 3. Publish staging

| Surface | Registry | Stage | Trigger |
|---------|----------|-------|---------|
| **TS connect client** | **npm** `@42ch/spoke-connect` | **Stage 1 — done** | Stable / non-`-rc.` tags via `publish-npm` |
| **Docs site** | **GitHub Pages** | **Stage 1 companion — done** | Docs workflow on main |
| **Rust connect crate** | crates.io `spoke-connect` | **Stage 1 — done** | Stable / non-`-rc.` tags via `publish-crates` |
| **C# NuGet binding** | **GitHub Packages** `42ch.Spoke.Connect` | **Bindings Stage A — done when first tag ships the job** | Multi-RID `ffi` matrix + `dotnet pack` + `dotnet nuget push` on the same tag gate as npm/crates |
| **Further language bindings** | GitHub Packages | **Bindings Stage B+** | Same GH Packages + lockstep rule when each language package is ready |
| **Core wire packages** | npm + crates.io | **Unchanged** | Existing `spoke-version-release.md` / `release.yml` |

**Registry split:**

| Ecosystem | Surfaces |
|-----------|----------|
| **npm** + **crates.io** | Primary TS / Rust connect packages (`@42ch/spoke-connect`, `spoke-connect`) and core wire packages |
| **GitHub Packages** | Multi-language binding packages published from this repo (NuGet first: `42ch.Spoke.Connect`; Swift and others later) |

**Owner class:**

| Stage | Owner class |
|-------|-------------|
| npm / crates.io connect + wire | Maintainer CI — top-level `release.yml` Trusted Publishing OIDC |
| Docs site | Docs workflow (GitHub Pages on main) |
| GitHub Packages bindings | Maintainer CI — `publish-nuget` (and future language jobs) with `packages: write` + `GITHUB_TOKEN` |

---

## 4. Versioning recommendation

| Decision | **Continue monorepo lockstep SemVer** for `@42ch/spoke-connect`, `spoke-connect`, and binding packages (`42ch.Spoke.Connect`) |
|----------|------------------------------------------------------------------------------------------------------------------------------|
| Rationale | Integrators pin one `X.Y.Z` across schemas + operations + connect surfaces; `release:bump` / `verify:version` cover the manifests; independent SemVer would fork bump scripts and CHANGELOG story for little gain pre-1.0 |
| Binding packages | Lockstep with spoke SemVer / git tag `vX.Y.Z` (same tag gate as npm/crates publish) |
| Pre-1.0 note | Breaking connect API changes still allowed without long deprecation; call out in CHANGELOG when a public connect surface breaks |
| Revisit trigger | A binding gains a substantially different release cadence or a nuget.org mirror demand → record the split in this decision doc before changing channels |

---

## 5. Registry & auth

| Target | Artifact | Auth model |
|--------|----------|------------|
| **npm** | `@42ch/spoke-connect` (+ wire packages) | **Trusted Publishing OIDC** via top-level `.github/workflows/release.yml`. No long-lived `NPM_TOKEN` repository secret for release publish |
| **crates.io** | `spoke-connect` (+ wire crates) | Same Trusted Publishing pattern (`rust-lang/crates-io-auth-action` → short-lived `CARGO_REGISTRY_TOKEN`); OIDC binds to top-level `release.yml` |
| **GitHub Packages** | `42ch.Spoke.Connect` (NuGet) and future binding packages | `GITHUB_TOKEN` with `packages: write` on `release.yml`; push to `https://nuget.pkg.github.com/42ch-dev/index.json` |
| **GitHub Pages** | Integrator docs site from `docs/` | Pages deploy workflow on main |

**Alignment constraints:**

- `release.yml` remains the **sole top-level publish workflow** (tag push `v*` or `release`-labeled PR merge). Trusted Publishing OIDC binds to that filename — keep npm/crates publish inside `publish-npm` / `publish-crates`; binding NuGet publish is a sibling job in the same workflow.
- Stable tags `vX.Y.Z` and prerelease SemVer tags without `-rc.` publish registries; tags containing `-rc.` create GitHub pre-releases only (no npm / crates.io / GitHub Packages binding push).

---

## 6. TS package published-shape (historical Stage 1)

Package: `packages/spoke-connect-ts` (`@42ch/spoke-connect`).

Stage 1 execution is complete: built `dist/` tarball, Trusted Publishing on `release.yml`, lockstep with wire packages. Current package metadata and exports live in `packages/spoke-connect-ts/package.json`; see the package README for integrator install.

---

## 7. Bindings disposition

uniffi-generated bindings under `crates/spoke-connect/bindings/*` are packaged from this repository onto **GitHub Packages** when a language has a packable project and release job.

| Fact | Detail |
|------|--------|
| What this repo ships | Generate scripts, smokes, the sync-core `ffi` surface, and published binding packages on GitHub Packages |
| C# package | **`42ch.Spoke.Connect`** — generated C# + multi-RID native `spoke_connect` / `libspoke_connect` under `runtimes/<rid>/native/`; session core only (transport stays product-owned) |
| Consumer DX | One-time `nuget.config` source for `https://nuget.pkg.github.com/42ch-dev/index.json` + `PackageReference` — no Rust toolchain required |
| Version tracking | Lockstep SemVer with spoke / git tag `vX.Y.Z` |
| Language matrix | **C#, Go, Python, Swift, Kotlin — C# first** (Swift skeleton landed; C# NuGet via vendored `uniffi-bindgen-cs` fork until upstream tags 0.32+; see [`connect-csharp-binding.md`](connect-csharp-binding.md)) |
| Maintainer regenerate | Vendored bindgen fork until upstream 0.32+; consumers never run bindgen |

---

## 8. WebTransport evaluation

| Item | Value |
|------|--------|
| **Conclusion** | **Defer implementation** — keep **WebSocket** as the default Path A transport for `@42ch/spoke-connect` |
| **Evidence** | **Browser availability (2026-08):** WebTransport is **Baseline "newly available" since March 2026** (MDN compat) — Chrome 97+ / Edge 97+ (2022-01), Firefox 114+ (2023-06), Safari 26.4+ (2026-03; Safari ≤ 26.3 unsupported). **Secure context (HTTPS) required**; available in Web Workers; sub-features (e.g. `congestionControl`, datagrams) still vary by browser. **Framing fit:** the connect contract is **one JSON document = one envelope** over an **ordered, reliable, bidirectional stream**, with WebSocket one-message-per-envelope listed as conforming ([`spoke-connect.md`](spoke-connect.md) §Transport framing). WebSocket message semantics map 1:1 onto that contract; WebTransport's reliable surface is byte-stream oriented (`WebTransportBidirectionalStream` = `ReadableStream` + `WritableStream`, no preserved message boundaries) and its datagram side is unreliable — envelope delimiting would have to be re-imposed in the adapter. **Node.js maturity:** the undici `WebTransport` client ships behind `--experimental-webtransport` (experimental, Stability 1); native `node:quic` (Stability 1, `--experimental-quic`, not yet in a stable release) does **not** implement WebTransport yet (jasnell.me, 2026) |
| **Recommend** | Keep **WebSocket** as the default Path A transport; WebTransport remains a **future framing option** under transport-adapter ownership (delimiting is transport-adapter-owned per [`spoke-connect.md`](spoke-connect.md) §Transport framing) |
| **Defer** | Implementation this/next slice unless a product host **requires** WebTransport |
| **Trigger to implement** | Product host requires WebTransport (HTTP/3) transport, plus a CI-able browser test plan. Browser baseline availability (reached 2026-03) does **not** alone flip the decision: Node-side WebTransport remains experimental (flag-gated only), the reference spike has no WebTransport transport, and the one-envelope-per-message contract maps 1:1 onto WebSocket with no adapter-level framing work |
| **Docs** | Docs may mention WebTransport as a future framing option; transport-adapter ownership stays per [`spoke-connect.md`](spoke-connect.md) / [`spoke-connect-ts-route.md`](spoke-connect-ts-route.md) |

---

## 9. js-libp2p evaluation

| Item | Value |
|------|--------|
| **Conclusion** | **Remain fallback route only** — does not overturn pure-TS-minimal primary ([`spoke-connect-ts-route.md`](spoke-connect-ts-route.md)) |
| **Evidence** | **When mesh interop matters:** only for **direct libp2p-network participation** — a product host that must dial/listen on the same Noise/yamux mesh as the Rust reference spike (`crates/spoke-connect`). Envelope-level interop — the v1 goal — requires only the same hello / `peer_id` / session rules over **any ordered stream** (WebSocket today); Noise multistream is not required for it ([`spoke-connect-ts-route.md`](spoke-connect-ts-route.md) evaluation criterion 10). **Dependency weight:** js-libp2p is a deep `@libp2p/*` monorepo tree — large supply-chain and bundle surface (criterion 7), inverse of the thin `@42ch/*` helper pattern; Noise/multistream interop with rust-libp2p 0.56 additionally needs version pinning and periodic re-verify (criterion 8). Current package reality: `@42ch/spoke-connect` dependencies are `@42ch/spoke-schemas`, `@noble/ed25519`, `@noble/hashes`, `canonicalize`, `ws` — no libp2p deps. **Session-core ownership:** js-libp2p does **not** replace the TS session-core port — sequence, nonce, allowlist, `request_id` correlation, and the dispatch gate remain SDK-owned (criterion 6); the SPOKE hello is JCS-signed, not a libp2p native hello (criterion 4) |
| **Recommend** | Do **not** add `js-libp2p` to `@42ch/spoke-connect` default dependencies; keep the pure-TS-minimal primary intact |
| **Defer** | Shipping a mesh helper inside the default package |
| **Trigger to ship a mesh helper** | Product requires shared libp2p network with the Rust spike; then prefer an **optional companion package or consumer-repo adapter**, not a forced default export |
| **Primary route** | pure-TS-minimal (WebSocket + WebCrypto/`@noble` Ed25519 + JCS + ported session core) remains locked |

---

## 10. Non-goals

| Non-goal | Detail |
|----------|--------|
| Daemon / runtime packages | No connect daemon, MCP server, or multi-product runtime package from this repo |
| nuget.org mirror | GitHub Packages only for binding NuGet until integrator demand justifies a second feed |
| WebSocket client inside `42ch.Spoke.Connect` | Session core only in v1; a later `42ch.Spoke.Connect.WebSocket` (or product package) may add transport |
| Per-surface independent SemVer pre-1.0 | No split version channels for connect-ts / NuGet vs schemas/ops until the revisit trigger in §4 fires |
| Overturn pure-TS-minimal | This strategy does not re-litigate the TS connectivity primary route |
| Wire / schema changes | Publish strategy is packaging and staging only; connect envelopes stay under `spoke-connect.md` |

---

## 11. Execution roadmap pointer

| Slice | What ships | Trigger | Owner |
|-------|------------|---------|-------|
| **C# GitHub Packages NuGet** | `42ch.Spoke.Connect` pack + multi-RID ffi + `publish-nuget` on stable tags; docs PackageReference DX | Decision record on main + release tag without `-rc.` | Release maintainer |
| **Further bindings on GH Packages** | Swift / Go / Python / Kotlin packages when each language is pack-ready | Feasibility gate + packable project | Binding maintainers |

Mirror row: [`.mstar/roadmap.md`](../roadmap.md) **Up next** / **Done**.

---

## 12. Links

| Path | Use |
|------|-----|
| [`.mstar/specs/spoke-connect.md`](spoke-connect.md) | Normative connect wire + embedding |
| [`.mstar/specs/spoke-connect-ts-route.md`](spoke-connect-ts-route.md) | TS client route (pure-TS-minimal) |
| [`.mstar/specs/spoke-version-release.md`](spoke-version-release.md) | Lockstep SemVer + Trusted Publishing |
| [`.mstar/specs/connect-csharp-binding.md`](connect-csharp-binding.md) | C# uniffi binding decision record |
| [`.mstar/roadmap.md`](../roadmap.md) | Durable project roadmap |
| [`packages/spoke-connect-ts/`](../../packages/spoke-connect-ts/) | TS connect client package |
| [`crates/spoke-connect/`](../../crates/spoke-connect/) | Rust connect crate |
| [`crates/spoke-connect/bindings/csharp/`](../../crates/spoke-connect/bindings/csharp/) | C# binding + NuGet project |
| [`.github/workflows/release.yml`](../../.github/workflows/release.yml) | Top-level publish workflow (OIDC + GitHub Packages) |
