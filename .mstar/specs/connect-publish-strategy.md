# Connect publish strategy

**Status:** Informative decision — does not change connect envelopes or the lockstep release policy for core wire packages.

**Updated:** 2026-08-17

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
| [`connect-binding-channels.md`](connect-binding-channels.md) | Informative **packaging contract** per binding language — module paths, package coordinates, native layouts, generator routes, CI job shapes |
| This document | Informative **when / what / how to publish** connect client surfaces; does not fork wire or release lockstep rules |

---

## 2. Surface inventory

| Surface | Path today | Registry | Publish state |
|---------|------------|----------|---------------|
| **TS connect client** | `packages/spoke-connect-ts` | npm `@42ch/spoke-connect` | Published lockstep via `release.yml` |
| **Docs site** | `docs/` | GitHub Pages | Live companion site |
| **Rust connect crate** | `crates/spoke-connect` | crates.io `spoke-connect` | Published lockstep via `release.yml` |
| **C# binding** | `crates/spoke-connect/bindings/csharp/` | **GitHub Packages NuGet** `42ch.Spoke.Connect` | Published lockstep via `release.yml` `publish-nuget` |
| **Kotlin binding** | `crates/spoke-connect/bindings/kotlin/` | **GitHub Packages Maven** `dev.42ch:spoke-connect` | Published lockstep via `release.yml` `publish-maven` |
| **Swift binding** | `crates/spoke-connect/bindings/swift/` | **GitHub repo + Swift Package Manager** (root `Package.swift` product `SpokeConnect` + `vX.Y.Z` tags) | SPM git dependency at lockstep tag |
| **Go binding** | `crates/spoke-connect/bindings/go/` | **GitHub repo + Go modules** (root `go.mod` → module `github.com/42ch-dev/spoke` + `vX.Y.Z` tags) | `go get github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go@vX.Y.Z` at lockstep tag |
| **Python binding** | `crates/spoke-connect/bindings/python/` | **PyPI** `spoke-connect` (Trusted Publishing OIDC on `release.yml` `publish-pypi`) | Package-ready; platform wheels + `publish-pypi` on stable tags |
| **Core wire packages** (context) | `@42ch/spoke-schemas`, `@42ch/spoke-operations`, crates `spoke-schemas` / `spoke-operations` | npm + crates.io | Published under lockstep SemVer |

---

## 3. Publish staging

| Surface | Registry | Stage | Trigger |
|---------|----------|-------|---------|
| **TS connect client** | **npm** `@42ch/spoke-connect` | **Stage 1 — done** | Stable / non-`-rc.` tags via `publish-npm` |
| **Docs site** | **GitHub Pages** | **Stage 1 companion — done** | Docs workflow on main |
| **Rust connect crate** | crates.io `spoke-connect` | **Stage 1 — done** | Stable / non-`-rc.` tags via `publish-crates` |
| **C# binding** | **GitHub Packages NuGet** `42ch.Spoke.Connect` | **Bindings Stage A — done** | Multi-RID `ffi` matrix + `dotnet pack` + `dotnet nuget push` on the same tag gate as npm/crates |
| **Kotlin binding** | **GitHub Packages Maven** `dev.42ch:spoke-connect` | **Bindings Stage B — done** | `publish-maven` on the same tag gate as npm/crates; Gradle publish to `maven.pkg.github.com/42ch-dev/spoke` |
| **Swift binding** | **GitHub repo + SPM** | **Bindings Stage B — done** | Root `Package.swift` product `SpokeConnect` + `vX.Y.Z` tags; SPM resolves the dependency from the repo |
| **Go binding** | **GitHub repo + Go modules** | **Bindings Stage B — done** | Root `go.mod` + `vX.Y.Z` tags; `go get …/bindings/go@vX.Y.Z` resolves from the repo |
| **Python binding** | **PyPI** | **Bindings Stage B — done** | `publish-pypi` on `release.yml` via Trusted Publishing OIDC; `pip install spoke-connect==X.Y.Z` |
| **Core wire packages** | npm + crates.io | **Unchanged** | Existing `spoke-version-release.md` / `release.yml` |

**Channel split (four channel types across five binding languages):**

| Channel type | Languages | Registry / mechanism |
|--------------|-----------|----------------------|
| **GitHub Packages** | C# (NuGet `42ch.Spoke.Connect`), Kotlin (Maven `maven.pkg.github.com/42ch-dev/spoke`) | `GITHUB_TOKEN` with `packages: write` on `release.yml`; lockstep SemVer tag gate |
| **GitHub repo + Swift Package Manager** | Swift | `Package.swift` at the repo path + `vX.Y.Z` tags; consumers `.package(url:from:)` |
| **GitHub repo + Go modules** | Go | `go.mod` at the module path + `vX.Y.Z` tags; consumers `go get …@vX.Y.Z` |
| **PyPI Trusted Publishing** | Python | `publish-pypi` on `release.yml` via OIDC; Trusted Publishing publisher registered to repository `42ch-dev/spoke` and workflow `release.yml` |

**Owner class:**

| Stage | Owner class |
|-------|-------------|
| npm / crates.io connect + wire | Maintainer CI — top-level `release.yml` Trusted Publishing OIDC |
| Docs site | Docs workflow (GitHub Pages on main) |
| GitHub Packages bindings (C# NuGet, Kotlin Maven) | Maintainer CI — `publish-nuget` / `publish-maven` with `packages: write` + `GITHUB_TOKEN` |
| SPM / Go module bindings (Swift, Go) | Maintainer tags `vX.Y.Z` on the repo; consumers resolve via SPM / Go modules — no publish job |
| PyPI binding (Python) | Maintainer CI — `publish-pypi` via Trusted Publishing OIDC |

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
| **GitHub Packages NuGet** | `42ch.Spoke.Connect` (C#) | `GITHUB_TOKEN` with `packages: write` on `release.yml`; push to `https://nuget.pkg.github.com/42ch-dev/index.json` |
| **GitHub Packages Maven** | Kotlin binding | `GITHUB_TOKEN` with `packages: write` on `release.yml`; publish to `maven.pkg.github.com/42ch-dev/spoke` |
| **Swift Package Manager** | Swift binding | Tag-driven: `Package.swift` at the repo path + `vX.Y.Z` tags; SPM resolves over git — no registry auth |
| **Go modules** | Go binding | Tag-driven: `go.mod` at the module path + `vX.Y.Z` tags; `go get` resolves over git — no registry auth |
| **PyPI** | Python binding | **Trusted Publishing OIDC** via `publish-pypi` on `release.yml`; Trusted Publishing publisher registered to repository `42ch-dev/spoke` and workflow `release.yml` — no long-lived `PYPI_TOKEN` |
| **GitHub Pages** | Integrator docs site from `docs/` | Pages deploy workflow on main |

**Alignment constraints:**

- `release.yml` remains the **sole top-level publish workflow** (tag push `v*` or `release`-labeled PR merge). Trusted Publishing OIDC binds to that filename — keep npm/crates/PyPI publish inside `publish-npm` / `publish-crates` / `publish-pypi`; C# NuGet and Kotlin Maven publish are sibling jobs in the same workflow.
- SPM and Go module bindings resolve from the repo via `vX.Y.Z` tags; they require the tag gate but no `release.yml` publish job.
- Stable tags `vX.Y.Z` and prerelease SemVer tags without `-rc.` publish registries; tags containing `-rc.` create GitHub pre-releases only (no npm / crates.io / GitHub Packages binding / PyPI push).

**Re-run semantics:**

Re-running Release at an already-published lockstep tag is safe for the PyPI and Maven lanes: the `publish-pypi` and `publish-maven` jobs pre-check the registry for the full expected artifact set at the tag SemVer and skip build + publish when it is complete. The pre-check is the skip gate; registry flags are per-file guards only.

| Lane | Pre-check | Skip when complete | Partial-set behavior |
|------|-----------|--------------------|----------------------|
| **PyPI** (`publish-pypi`) | `GET https://pypi.org/pypi/spoke-connect/<version>/json` (package name from `crates/spoke-connect/bindings/python/pyproject.toml`; version = tag minus the `v` prefix). Expected set = the three platform wheels locked by `tooling/connect/verify-python-wheels.sh` — `spoke_connect-<ver>-py3-none-manylinux_2_35_x86_64.whl`, `spoke_connect-<ver>-py3-none-macosx_11_0_arm64.whl`, `spoke_connect-<ver>-py3-none-win_amd64.whl`; no sdist | All three wheels present → build and publish steps skipped, skip reason logged. `skip-existing: true` on the pinned `pypa/gh-action-pypi-publish` step is a per-file duplicate guard during an actual publish attempt only — the unconditional re-probe decides green. Yanked wheels (PEP 592) count as absent, so a yanked expected file keeps the probe red | Missing wheels are attempted; already-present wheels are tolerated as duplicates during the resume. The unconditional `Confirm published set` re-probe must show all three wheels or the job fails |
| **Maven** (`publish-maven`) | Authenticated `GET` on `https://maven.pkg.github.com/42ch-dev/spoke/dev/42ch/spoke-connect/<version>/` for `spoke-connect-<ver>.pom`, `spoke-connect-<ver>.module`, `spoke-connect-<ver>.jar` with `Authorization: Bearer $GITHUB_TOKEN`, plus a jar-entries check for the three JNA resources (`linux-x86-64/libspoke_connect.so`, `darwin-aarch64/libspoke_connect.dylib`, `win32-x86-64/spoke_connect.dll`). Expected set enumerated from the Gradle publication (`from(components["java"])`); no sources/javadoc or native classifiers | Full set (pom + module + jar with all JNA entries) present → `gradle publish` skipped, skip reason logged | The job attempts `gradle publish` for the whole set with the publish step set to `continue-on-error`, so the unconditional re-probe is the sole arbiter: a verified full set greens the job (a concurrent winner's full set included), while a still-partial set fails the job naming the missing files. Recovery is manual — delete the package version in the GitHub Packages UI and re-run (no automated registry deletion) |

Both jobs close with an unconditional `Confirm published set` re-probe step (`if: ${{ always() }}`): only the re-probe decides green — the job is green on a verified full set, on the skip path and the publish path alike. Registry doubt fails the job: network errors, HTTP 401/403/5xx (HTTP 404 means the version is absent → publish needed), malformed JSON, or an unexpected payload shape exit non-zero with a message naming the registry and the status.

Each probe decides on the expected artifact set alone: the expected set is the sole input to the skip decision. Registry entries outside the expected set — extra `urls[]` files on PyPI, extra files listed in the Gradle module metadata on Maven — are logged to stderr as warnings for observability; the verdict always reflects the expected set alone.

---

## 6. TS package published-shape (historical Stage 1)

Package: `packages/spoke-connect-ts` (`@42ch/spoke-connect`).

Stage 1 execution is complete: built `dist/` tarball, Trusted Publishing on `release.yml`, lockstep with wire packages. Current package metadata and exports live in `packages/spoke-connect-ts/package.json`; see the package README for integrator install.

---

## 7. Bindings disposition

uniffi-generated bindings under `crates/spoke-connect/bindings/*` are packaged per language. The publish channel is chosen per language ecosystem — C# and Kotlin use **GitHub Packages**; Swift and Go resolve from the **repo** via SPM / Go modules; Python publishes to **PyPI** via Trusted Publishing.

| Language | Channel | Package / mechanism | State |
|----------|---------|--------------------|-------|
| **C#** | GitHub Packages NuGet | **`42ch.Spoke.Connect`** — generated C# + multi-RID native `spoke_connect` / `libspoke_connect` under `runtimes/<rid>/native/`; session core only (transport stays product-owned) | **Landed** |
| **Kotlin** | GitHub Packages Maven | **`dev.42ch:spoke-connect`** — committed uniffi Kotlin + JNA natives in jar; `publish-maven` on `release.yml` | **Landed** |
| **Swift** | GitHub repo + SPM | Root `Package.swift` product **`SpokeConnect`** + committed xcframework; consumers `.package(url:from:)` at `vX.Y.Z` | **Landed** |
| **Go** | GitHub repo + Go modules | Root `go.mod` (module `github.com/42ch-dev/spoke`) + `vX.Y.Z` tags; consumers `go get github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go@vX.Y.Z` | **Landed** |
| **Python** | PyPI | `pip install spoke-connect==X.Y.Z` via Trusted Publishing OIDC on `release.yml` (`publish-pypi`) — no GitHub Packages | **Landed** — platform wheels + golden-parity smoke |

| Fact | Detail |
|------|--------|
| What this repo ships | Generate scripts, smokes, the sync-core `ffi` surface, and per-language binding packages on the channel above |
| C# package | **`42ch.Spoke.Connect`** — generated C# + multi-RID native `spoke_connect` / `libspoke_connect` under `runtimes/<rid>/native/`; session core only (transport stays product-owned) |
| C# consumer DX | One-time `nuget.config` source for `https://nuget.pkg.github.com/42ch-dev/index.json` + `PackageReference` — no Rust toolchain required |
| Version tracking | Lockstep SemVer with spoke / git tag `vX.Y.Z` |
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
| nuget.org mirror | GitHub Packages NuGet only for the C# binding until integrator demand justifies a second feed |
| maven Central mirror | GitHub Packages Maven only for the Kotlin binding until integrator demand justifies a second feed |
| GitHub Packages for Swift / Go / Python | Swift uses SPM git; Go uses Go module git; Python uses PyPI Trusted Publishing — these ecosystems do not map onto GitHub Packages |
| WebSocket client inside `42ch.Spoke.Connect` | Session core only in v1; a later `42ch.Spoke.Connect.WebSocket` (or product package) may add transport |
| Per-surface independent SemVer pre-1.0 | No split version channels for connect-ts / bindings vs schemas/ops until the revisit trigger in §4 fires |
| Overturn pure-TS-minimal | This strategy does not re-litigate the TS connectivity primary route |
| Wire / schema changes | Publish strategy is packaging and staging only; connect envelopes stay under `spoke-connect.md` |

---

## 11. Execution roadmap pointer

| Slice | What ships | Trigger | Owner |
|-------|------------|---------|-------|
| **C# GitHub Packages NuGet** | `42ch.Spoke.Connect` pack + multi-RID ffi + `publish-nuget` on stable tags; docs PackageReference DX | Decision record on main + release tag without `-rc.` | Release maintainer |
| **Kotlin GitHub Packages Maven** | `dev.42ch:spoke-connect` Gradle publish + multi-RID JNA natives + `publish-maven` on stable tags; Maven coordinates DX in docs/Smoke | **Done** — `publish-maven` on `release.yml` |
| **Swift SPM** | Root `Package.swift` product `SpokeConnect` + xcframework + `vX.Y.Z` tags; `.package(url:from:)` docs | **Done** — SPM resolves at lockstep tag |
| **Go modules** | Root `go.mod` + module path + `vX.Y.Z` tags; `go get …@vX.Y.Z` docs + golden-parity smoke | **Done** — Go modules resolve at lockstep tag |
| **Python PyPI** | `publish-pypi` on `release.yml` via Trusted Publishing OIDC matching the registered publisher; `pip install` docs | Feasibility gate + packable project + registered publisher | Binding maintainers |

Mirror row: [`.mstar/roadmap.md`](../roadmap.md) **Up next** / **Done**.

---

## 12. Links

| Path | Use |
|------|-----|
| [`.mstar/specs/spoke-connect.md`](spoke-connect.md) | Normative connect wire + embedding |
| [`.mstar/specs/spoke-connect-ts-route.md`](spoke-connect-ts-route.md) | TS client route (pure-TS-minimal) |
| [`.mstar/specs/spoke-version-release.md`](spoke-version-release.md) | Lockstep SemVer + Trusted Publishing |
| [`.mstar/specs/connect-csharp-binding.md`](connect-csharp-binding.md) | C# uniffi binding decision record |
| [`.mstar/specs/connect-binding-channels.md`](connect-binding-channels.md) | Binding channel packaging contract (ADR) |
| [`.mstar/roadmap.md`](../roadmap.md) | Durable project roadmap |
| [`packages/spoke-connect-ts/`](../../packages/spoke-connect-ts/) | TS connect client package |
| [`crates/spoke-connect/`](../../crates/spoke-connect/) | Rust connect crate |
| [`crates/spoke-connect/bindings/csharp/`](../../crates/spoke-connect/bindings/csharp/) | C# binding + NuGet project |
| [`.github/workflows/release.yml`](../../.github/workflows/release.yml) | Top-level publish workflow (OIDC + GitHub Packages) |
