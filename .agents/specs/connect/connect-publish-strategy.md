# Connect publish strategy

**Status:** Informative decision — does not change connect envelopes or the lockstep release policy for core wire packages.
**Updated:** 2026-09-18
---

## Scope

States the current publish facts for SPOKE connect surfaces: which artifacts are registry packages and which resolve from the repository tag, the publish stage and trigger per surface, how versions move against the core wire packages, the registry and auth model per channel, and the re-run semantics of the publish jobs. It is packaging and staging only — connect envelopes, the lockstep release policy for core wire packages, and the binding packaging shapes are owned elsewhere. The rationale for the channel split and for the transports that were not adopted lives in the decision records this page references.

## Current state

### 1. Purpose

This document is the **publish-strategy SSOT** for SPOKE connect surfaces. Integrators and maintainers use it to answer:

- which connect artifacts become registry packages (and which stay workspace-private or consumer-repo artifacts);
- the ordered publish stages and the trigger for each stage;
- how versions move relative to core wire packages;
- which registries and auth model apply.

**Relationship to other specs:**

| Spec | Role |
|------|------|
| [`spoke-version-release.md`](../protocol/spoke-version-release.md) | Normative lockstep SemVer, annotated tags, CI-gated GitHub Release, npm/crates.io Trusted Publishing for core packages |
| [`spoke-connect.md`](./spoke-connect.md) | Normative connect wire, identity, framing, embedding model |
| [`2026-08-01-spoke-connect-ts-route.md`](../../adr/implemented/architecture/2026-08-01-spoke-connect-ts-route.md) | Informative TS client stack route (pure-TS-minimal primary; js-libp2p mesh fallback) |
| [`2026-08-03-connect-binding-channels.md`](../../adr/implemented/architecture/2026-08-03-connect-binding-channels.md) | Informative **packaging contract** per binding language — module paths, package coordinates, native layouts, generator routes, CI job shapes |
| This document | Informative **when / what / how to publish** connect client surfaces; does not fork wire or release lockstep rules |

---

### 2. Surface inventory

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
| **C/C++ binding** | `crates/spoke-connect/bindings/cpp/` | **git-based — committed headers + platform carriers** (`include/spoke_connect.h`, `include/spoke_connect.hpp`, `native/<rid>/`); workspace-private carrier (`spoke-connect-capi`, `publish = false`), no registry package, no version-bearing manifest | Resolved from the repo at lockstep tag `vX.Y.Z`; no publish job |
| **Core wire packages** (context) | `@42ch/spoke-schemas`, `@42ch/spoke-operations`, crates `spoke-schemas` / `spoke-operations` | npm + crates.io | Published under lockstep SemVer |

---

### 3. Publish staging

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
| **C/C++ binding** | **git — committed headers + platform carriers** at `crates/spoke-connect/bindings/cpp/` | **Bindings Stage B — done** | Shipped in-repo; headers and the `native/<rid>/` carrier resolve from the same `vX.Y.Z` tag — no publish job |
| **Core wire packages** | npm + crates.io | **Unchanged** | Existing `spoke-version-release.md` / `release.yml` |

**Channel classification (authoritative statement):**

> **Native bindings use registry-backed distribution for C# (GitHub Packages NuGet), Kotlin (GitHub Packages Maven), and Python (PyPI), and git-based distribution for Swift (Swift Package Manager), Go (Go modules), and C/C++ (committed headers and platform carriers). Registry packages carry the lockstep release version; git-based consumers resolve the matching repository tag `vX.Y.Z`. NuGet and Maven share the GitHub Packages registry family.**

**Registry-backed channels:**

| Channel | Languages | Registry / mechanism |
|---------|-----------|----------------------|
| **GitHub Packages NuGet** | C# (`42ch.Spoke.Connect`) | `GITHUB_TOKEN` with `packages: write` on `release.yml`; lockstep SemVer tag gate |
| **GitHub Packages Maven** | Kotlin (`dev.42ch:spoke-connect`) | `GITHUB_TOKEN` with `packages: write` on `release.yml`; push to `maven.pkg.github.com/42ch-dev/spoke` |
| **PyPI Trusted Publishing** | Python (`spoke-connect`) | `publish-pypi` on `release.yml` via OIDC; Trusted Publishing publisher registered to repository `42ch-dev/spoke` and workflow `release.yml` |

**Git-based channels:**

| Channel | Languages | Mechanism |
|---------|-----------|-----------|
| **Swift Package Manager** | Swift (product `SpokeConnect`) | `Package.swift` at the repo path + `vX.Y.Z` tags; consumers `.package(url:from:)` |
| **Go modules** | Go (module `github.com/42ch-dev/spoke`) | `go.mod` at the module path + `vX.Y.Z` tags; consumers `go get …@vX.Y.Z` |
| **Committed headers and platform carriers** | C and C++ | `crates/spoke-connect/bindings/cpp/include/{spoke_connect.h,spoke_connect.hpp}` + `native/<rid>/` resolved from the same `vX.Y.Z` tag — workspace-private carrier, no registry publish job, no version-bearing manifest |

**Owner class:**

| Stage | Owner class |
|-------|-------------|
| npm / crates.io connect + wire | Maintainer CI — top-level `release.yml` Trusted Publishing OIDC |
| Docs site | Docs workflow (GitHub Pages on main) |
| GitHub Packages bindings (C# NuGet, Kotlin Maven) | Maintainer CI — `publish-nuget` / `publish-maven` with `packages: write` + `GITHUB_TOKEN` |
| PyPI binding (Python) | Maintainer CI — `publish-pypi` via Trusted Publishing OIDC |
| SPM / Go module / C/C++ git bindings (Swift, Go, C++) | Maintainer tags `vX.Y.Z` on the repo; consumers resolve via SPM / Go modules / the tag's committed headers and carriers — no publish job |

---

### 4. Versioning recommendation

| Decision | **Continue monorepo lockstep SemVer** for `@42ch/spoke-connect`, `spoke-connect`, and binding packages (`42ch.Spoke.Connect`) |
|----------|------------------------------------------------------------------------------------------------------------------------------|
| Rationale | Integrators pin one `X.Y.Z` across schemas + operations + connect surfaces; `release:bump` / `verify:version` cover the manifests; independent SemVer would fork bump scripts and CHANGELOG story for little gain pre-1.0 |
| Binding packages | Lockstep with spoke SemVer / git tag `vX.Y.Z` (same tag gate as npm/crates publish). Registry-backed packages (C#, Kotlin, Python) carry that version in their package metadata; git-based consumers (Swift, Go, C/C++) resolve the tag `vX.Y.Z` itself — the C/C++ channel ships no version-bearing manifest |
| Pre-1.0 note | Breaking connect API changes still allowed without long deprecation; call out in CHANGELOG when a public connect surface breaks |
| Revisit trigger | A binding gains a substantially different release cadence or a nuget.org mirror demand → record the split in this decision doc before changing channels |

---

### 5. Registry & auth

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
| **Committed git carriers** | C/C++ binding headers (`include/spoke_connect.h`, `include/spoke_connect.hpp`) + platform carriers under `crates/spoke-connect/bindings/cpp/native/<rid>/` | None — resolved from the repo at tag `vX.Y.Z`; no registry auth, no publish job |

**Alignment constraints:**

- `release.yml` remains the **sole top-level publish workflow** (tag push `v*` or `release`-labeled PR merge). Trusted Publishing OIDC binds to that filename — keep npm/crates/PyPI publish inside `publish-npm` / `publish-crates` / `publish-pypi`; C# NuGet and Kotlin Maven publish are sibling jobs in the same workflow.
- SPM, Go module and C/C++ bindings resolve from the repo via `vX.Y.Z` tags; they require the tag gate but no `release.yml` publish job.
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

### 7. Bindings disposition

Bindings under `crates/spoke-connect/bindings/*` are packaged per language: uniffi-generated sources for C#, Kotlin, Swift, Go and Python, plus the hand-written C ABI carrier for C and C++. The publish channel is chosen per language ecosystem — C# and Kotlin use **GitHub Packages**; Swift, Go and C/C++ resolve from the **repo** via SPM / Go modules / committed headers and platform carriers; Python publishes to **PyPI** via Trusted Publishing.

| Language | Channel | Package / mechanism | State |
|----------|---------|--------------------|-------|
| **C#** | GitHub Packages NuGet | **`42ch.Spoke.Connect`** — generated C# + multi-RID native `spoke_connect` / `libspoke_connect` under `runtimes/<rid>/native/`; session core only (transport stays product-owned) | **Landed** |
| **Kotlin** | GitHub Packages Maven | **`dev.42ch:spoke-connect`** — committed uniffi Kotlin + JNA natives in jar; `publish-maven` on `release.yml` | **Landed** |
| **Swift** | GitHub repo + SPM | Root `Package.swift` product **`SpokeConnect`** + committed xcframework; consumers `.package(url:from:)` at `vX.Y.Z` | **Landed** |
| **Go** | GitHub repo + Go modules | Root `go.mod` (module `github.com/42ch-dev/spoke`) + `vX.Y.Z` tags; consumers `go get github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go@vX.Y.Z` | **Landed** |
| **Python** | PyPI | `pip install spoke-connect==X.Y.Z` via Trusted Publishing OIDC on `release.yml` (`publish-pypi`) — no GitHub Packages | **Landed** — platform wheels + golden-parity smoke |
| **C / C++** | git — committed headers + platform carriers | `include/spoke_connect.h` (C99 ABI) + `include/spoke_connect.hpp` (C++17 convenience layer) + `native/<rid>/` taken from the `vX.Y.Z` tag; workspace-private carrier, no registry publish job | **Landed** — macOS arm64 (`osx-arm64`) + Windows x64 (`win-x64`) carriers |

| Fact | Detail |
|------|--------|
| What this repo ships | Generate scripts, smokes, the sync-core `ffi` surface, and per-language binding packages on the channel above |
| C# package | **`42ch.Spoke.Connect`** — generated C# + multi-RID native `spoke_connect` / `libspoke_connect` under `runtimes/<rid>/native/`; session core only (transport stays product-owned) |
| C# consumer DX | One-time `nuget.config` source for `https://nuget.pkg.github.com/42ch-dev/index.json` + `PackageReference` — no Rust toolchain required |
| C/C++ mechanism | Committed headers `include/spoke_connect.h` (C99 ABI) and `include/spoke_connect.hpp` (C++17 convenience layer) plus the platform carriers under `native/<rid>/`; the `spoke-connect-capi` carrier crate is workspace-private (`publish = false`) and consumers take the files from the `vX.Y.Z` tag |
| Version tracking | Lockstep SemVer with spoke / git tag `vX.Y.Z` — registry-backed packages carry the version in their metadata; git-based channels (SPM, Go modules, C/C++) resolve the tag itself |
| Maintainer regenerate | Vendored bindgen fork until upstream 0.32+; consumers never run bindgen |

---

### 10. Non-goals

| Non-goal | Detail |
|----------|--------|
| Daemon / runtime packages | No connect daemon, MCP server, or multi-product runtime package from this repo |
| nuget.org mirror | GitHub Packages NuGet only for the C# binding until integrator demand justifies a second feed |
| maven Central mirror | GitHub Packages Maven only for the Kotlin binding until integrator demand justifies a second feed |
| GitHub Packages for Swift / Go / Python | Swift uses SPM git; Go uses Go module git; Python uses PyPI Trusted Publishing — these ecosystems do not map onto GitHub Packages |
| Package-manager or release-asset distribution for C/C++ | The C/C++ channel ships committed headers and platform carriers resolved from the repository tag — no vcpkg / Conan integration and no Release archive assets |
| WebSocket client inside `42ch.Spoke.Connect` | Session core only in v1; a later `42ch.Spoke.Connect.WebSocket` (or product package) may add transport |
| Per-surface independent SemVer pre-1.0 | No split version channels for connect-ts / bindings vs schemas/ops until the revisit trigger in §4 fires |
| Overturn pure-TS-minimal | This strategy does not re-litigate the TS connectivity primary route |
| Wire / schema changes | Publish strategy is packaging and staging only; connect envelopes stay under `spoke-connect.md` |

---

### 12. Links

| Path | Use |
|------|-----|
| [`.agents/specs/connect/spoke-connect.md`](./spoke-connect.md) | Normative connect wire + embedding |
| [`.agents/adr/implemented/architecture/2026-08-01-spoke-connect-ts-route.md`](../../adr/implemented/architecture/2026-08-01-spoke-connect-ts-route.md) | TS client route (pure-TS-minimal) |
| [`.agents/specs/protocol/spoke-version-release.md`](../protocol/spoke-version-release.md) | Lockstep SemVer + Trusted Publishing |
| [`.agents/specs/connect/connect-csharp-binding.md`](./connect-csharp-binding.md) | C# uniffi binding current state |
| [`.agents/adr/implemented/architecture/2026-08-03-connect-binding-channels.md`](../../adr/implemented/architecture/2026-08-03-connect-binding-channels.md) | Binding channel packaging contract (ADR) |
| [`packages/spoke-connect-ts/`](../../../packages/spoke-connect-ts) | TS connect client package |
| [`crates/spoke-connect/`](../../../crates/spoke-connect) | Rust connect crate |
| [`crates/spoke-connect/bindings/csharp/`](../../../crates/spoke-connect/bindings/csharp) | C# binding + NuGet project |
| [`crates/spoke-connect/bindings/cpp/`](../../../crates/spoke-connect/bindings/cpp) | C/C++ binding: C99 ABI header, C++17 convenience header, committed platform carriers |
| [`.github/workflows/release.yml`](../../../.github/workflows/release.yml) | Top-level publish workflow (OIDC + GitHub Packages) |

### 13. Binding channel facts (per language)

The per-language packaging shapes behind the channel split. The decision and its rejected alternatives: [`.agents/adr/implemented/architecture/2026-08-03-connect-binding-channels.md`](../../adr/implemented/architecture/2026-08-03-connect-binding-channels.md).

**Common contract**

| Rule | Fact |
|------|------|
| Single cdylib | One `spoke-connect` build — `ffi` feature, uniffi **0.32**, `crate-type = ["rlib", "cdylib"]` — carries the exported-surface metadata for every uniffi language; no per-language uniffi pins. The C/C++ channel is the one separate build: the workspace-private `spoke-connect-capi` carrier links `spoke-connect` with `ffi,remote-adapter` and exports a hand-written C ABI from its own cdylib |
| Exported surface | Core sync facade (8 functions + 3 objects + 2 error enums) plus the additive remote-adapter FFI: `RemoteAdapterFFI` (constructor + 21 methods), `MultiPeerRouterFFI` (constructor + 14 methods), `ConnectResponderFFI` (constructor + 7 methods); the `Transport`, `PortsHandler` (13 callbacks) and `ToolHandler` interfaces; loopback helpers; `FfiError` / `TransportError` |
| Generator routes | **First-party** (Swift, Kotlin, Python): the crate-local `uniffi-bindgen` bin generates from the pinned cdylib. **Community** (C#, Go): external bindgen behind the feasibility gate, vendored fork where the gap is real. **Hand-written** (C/C++): the carrier maintainer writes both headers together with the Rust exports |
| Generated sources | Committed per language; regenerated only when the FFI surface changes; consumers never run bindgen |
| Golden parity | Every language smoke asserts the committed Rust fixtures: golden peer id `12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf`, the golden base64url hello signature, and protocol version `1` |
| Native provenance | The `build-connect-ffi` matrix on `release.yml` (`linux-x64`, `win-x64`, `osx-arm64`) supplies registry-native inputs; packagers assemble per-language layouts from those artifacts. Swift static libraries and C/C++ carrier dynamic libraries are git-lfs objects |
| crates.io package | `crates/spoke-connect/Cargo.toml` `exclude`s the entire `bindings/**` tree — crates.io ships the Rust crate only |
| Cognition payload boundary | `spoke-cognition` fields ride the existing JSON-string request/response channels and opaque op payloads; no binding gains a typed cognition callback, C ABI slot, native artifact or generated-surface change |

**Version surfaces**

| Manifest | Lockstep mechanism |
|----------|--------------------|
| `crates/spoke-connect/bindings/python/pyproject.toml` | Registered in `tooling/release/lockstep-surfaces.mjs` (assert + bump) |
| `crates/spoke-connect/bindings/kotlin/build.gradle.kts` | Registered in `tooling/release/lockstep-surfaces.mjs` (assert + bump) |
| Root `go.mod` | No version field — the version is the git tag |
| Root `Package.swift` | No version field — the version is the git tag |

#### C# — GitHub Packages NuGet

| Field | Value |
|-------|-------|
| Package id | `42ch.Spoke.Connect` |
| Feed | `https://nuget.pkg.github.com/42ch-dev/index.json` |
| Layout | `crates/spoke-connect/bindings/csharp/` — packable csproj + committed `generated/spoke_connect.cs` + `runtimes/<rid>/native/` (CI-assembled, gitignored) |
| CI job | `publish-nuget` on `release.yml` (`needs: build-connect-ffi`) |
| Generator | Community `uniffi-bindgen-cs` via a vendored fork retargeted to uniffi 0.32 |

#### Go — Go modules over git

| Field | Value |
|-------|-------|
| Module path | `github.com/42ch-dev/spoke` — declared by a **root `go.mod`** |
| Import path | `github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go` — integrator package `spokeconnect` re-exports the generated `generated/spoke_connect` cgo surface; consumers import `bindings/go`, not the generated subdirectory |
| Versioning | Repo tags `vX.Y.Z`; consumers `go get github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go@vX.Y.Z` |
| Layout | `crates/spoke-connect/bindings/go/` — committed generated Go + `native/<goos>_<goarch>/` + `Smoke/` + README |
| Native set | `darwin_amd64/libspoke_connect.dylib`, `darwin_arm64/libspoke_connect.dylib` — committed, rebuilt when the FFI surface changes; `linux_amd64` and `windows_amd64` natives are a recorded follow-up |
| Native wiring | cgo (`CGO_ENABLED=1`): `#cgo LDFLAGS` selects `native/${GOOS}_${GOARCH}` under `${SRCDIR}`; rpath covers Linux/macOS lookup; Windows consumers place `spoke_connect.dll` beside the executable. Consumers need a C toolchain, never a Rust toolchain |
| Generator | Community `uniffi-bindgen-go` — vendored fork retargeted to uniffi 0.32 |

A module declared in a subdirectory would be versioned by subdirectory-prefixed tags and fork the single-tag release model, which is why the module lives at the repository root and accepts the whole-repo module zip as its cost.

#### Python — PyPI Trusted Publishing

| Field | Value |
|-------|-------|
| Project dir | `crates/spoke-connect/bindings/python/` — `pyproject.toml` + import package `spoke_connect/` (generated module committed) + `Smoke/` |
| PyPI name | **`spoke-connect`** — Trusted Publishing publisher registered for repository `42ch-dev/spoke` + workflow `release.yml` |
| Wheel shape | One platform wheel per ffi-matrix RID, PEP 425 tags `manylinux_2_35_x86_64` (linux-x64), `macosx_11_0_arm64` (osx-arm64), `win_amd64` (win-x64), each tagged `py3-none-<platform>`; each wheel bundles exactly its RID's shared library beside the generated module. The manylinux tag matches the CI build image glibc floor (`ubuntu-22.04`) |
| sdist | Not published in v1 — a source install cannot produce the native library without a Rust toolchain |
| CI job | `publish-pypi` on `release.yml` — sibling to `publish-nuget`, `needs: build-connect-ffi`, same non-`-rc.` tag gate; `pypa/gh-action-pypi-publish` with OIDC |
| Generator | First-party `--language python` from the crate-local CLI |

#### Swift — Swift Package Manager over git

| Field | Value |
|-------|-------|
| Manifest | **Root `Package.swift`** — SPM resolves git-url dependencies from the repo-root manifest only |
| Product | Library `SpokeConnect`; consumers `.package(url: "https://github.com/42ch-dev/spoke.git", from: "X.Y.Z")` + `.product(name: "SpokeConnect", package: "spoke")` |
| Targets | `SpokeConnect` at the committed generated path `crates/spoke-connect/bindings/swift/generated/`; `spoke_connectFFI` — local `.binaryTarget` xcframework committed under `crates/spoke-connect/bindings/swift/xcframework/` |
| xcframework | CI assembles the three-slice xcframework on `macos-15` from the checkout's Rust sources with pinned toolchain 1.98.1 and `--locked` on every pull request that changes code (a docs-only diff short-circuits the build steps), uploads it with a hash manifest, and runs the committed-slice smoke. Slices: macOS arm64, iOS arm64, iOS simulators on arm64 and x86_64. Refresh with `tooling/connect/apply-xcframework-artifact.sh <run-id>` |
| Smoke | `swift build` plus the golden-parity Swift smoke against the committed xcframework's macOS slice; `bindings/swift/IosSmoke/` runs the simulator smoke via `xcodebuild test` |
| Scale-out | A release-asset `binaryTarget` replaces the committed xcframework only when pre-tag artifact + manifest-checksum automation exists |
| Generator | First-party `--language swift` from the crate-local CLI |

#### Kotlin — GitHub Packages Maven

| Field | Value |
|-------|-------|
| Coordinates | `dev.42ch:spoke-connect` — reverse-DNS of the owned domain `42ch.dev`, the Maven Central namespace requirement (keeps a future mirror possible) |
| Repository | `https://maven.pkg.github.com/42ch-dev/spoke` |
| Project dir | `crates/spoke-connect/bindings/kotlin/` — Gradle `build.gradle.kts` (`maven-publish`) + committed generated Kotlin + `Smoke/` |
| Generated namespace | uniffi default (`uniffi.spoke_connect`) |
| Native layout | JNA classpath resources inside the jar: `darwin-aarch64/libspoke_connect.dylib`, `linux-x86-64/libspoke_connect.so`, `win32-x86-64/spoke_connect.dll` — assembled from the `build-connect-ffi` matrix |
| CI job | `publish-maven` on `release.yml` — sibling to `publish-nuget`, same non-`-rc.` tag gate, `GITHUB_TOKEN` with `packages: write` |
| Scope | JVM-first; an Android AAR (per-ABI `.so` packaging) is a recorded deferral |
| Generator | First-party `--language kotlin` from the crate-local CLI |

#### C / C++ — hand-written C ABI over git

| Field | Value |
|-------|-------|
| Carrier | `crates/spoke-connect-capi` — package `spoke-connect-capi`, library `spoke_connect_capi`, `publish = false`, `crate-type = ["rlib", "cdylib"]`; wraps the public `spoke_connect::ffi` functions and objects |
| Header | `crates/spoke-connect/bindings/cpp/include/spoke_connect.h` — C99 floor with `extern "C"` inclusion for C++, opaque per-type handles, `int32_t` status plus a caller-supplied error record, ABI revision `1` through `spoke_connect_abi_version`; `include/spoke_connect.hpp` — header-only C++17 convenience layer (namespace `spoke::connect`) |
| Layout | `crates/spoke-connect/bindings/cpp/` — both headers, `native/osx-arm64/libspoke_connect_capi.dylib`, `native/win-x64/spoke_connect_capi.dll` + `spoke_connect_capi.dll.lib`, `native/provenance.json`, `Smoke/`, `parity.md`, `ue/`, `README.md` |
| Generator route | Hand-written headers, no generator; the drift gate is executable: `node tooling/connect/cpp-symbol-check.mjs --header <header> --library <native>` |
| Parity | `crates/spoke-connect/bindings/cpp/parity.md` maps every production facade member, callback and error variant to its C declaration and C++ counterpart (83 declarations) |
| CI lane | `.github/workflows/cpp-connect.yml` runs `windows-smoke` on `windows-latest` and `macos-smoke` on `macos-15` for every pull request, with path-filtered pushes and manual dispatch |
| Distribution | Git-based — headers and per-RID natives resolve together from the tag `vX.Y.Z`; the two carrier libraries are git-lfs objects; no registry feed and no Release archive assets |

**Feasibility gate for community generators** (runs before any generated binding is committed):

1. Live upstream recheck — latest tag and `main` workspace pins, dated.
2. Stock `--library` against the repo's current cdylib.
3. Positive control — the crate-local `uniffi-bindgen` generates a first-party language from the same cdylib, proving the metadata is well-formed.
4. Vendored fork when the gap is real and small — pin the upstream SHA, bump `uniffi*` deps to the repo pin, compile-fix new `Type` arms, commit patch + lockfile + recipe under `bindings/<lang>/bindgen/`. Generation-only tooling; the product cdylib keeps a single uniffi pin. Dropped when upstream tags the repo's uniffi line.
5. Escalate (dual-pin or hand-written binding) only when the fork delta is large — never a silent skip, never a pin downgrade.

| Language | Generator | Gate posture |
|----------|-----------|--------------|
| C# | Community `uniffi-bindgen-cs` | Landed via a vendored fork retargeted to uniffi 0.32; fork dropped when upstream tags 0.32+ |
| Go | Community `uniffi-bindgen-go` | Landed via a vendored fork retargeted to uniffi 0.32; fork dropped when upstream tags 0.32+ |
| Kotlin | First-party (crate-local CLI) | No skew possible; gate = generate + Gradle compile + JNA load + golden parity |
| Python | First-party (crate-local CLI) | No skew possible; gate = generate + import + golden parity |
| Swift | First-party (crate-local CLI) | Landed (macOS smoke + iOS Simulator golden parity through the committed xcframework) |
| C / C++ | Hand-written headers | No bindgen skew by construction; gate = declaration ⇄ export diff in both directions, C99 link-all-declarations probe, C++17 no-exceptions/no-RTTI inclusion check, then the shared C++17 smoke per RID |

Implementation technique: [`.agents/knowledge/architecture-patterns/connect-uniffi-bindgen-fork.md`](../../knowledge/architecture-patterns/connect-uniffi-bindgen-fork.md).

**Packaging non-goals beyond §10**

| Non-goal | Detail |
|----------|--------|
| Async node over FFI | The cdylib owns a process-wide Tokio runtime; `RemoteAdapterFFI` / `MultiPeerRouterFFI` are synchronous block-on-async surfaces over it; node lifecycle stays Rust-side |
| Consumer-side bindgen | Generated sources and natives ship in the package; bindgen is maintainer tooling |
| Split binding repositories | Monorepo paths are the contract; a dedicated repository is the recorded escape hatch for the Go channel |
| C++20 generator integration | The C/C++ headers are hand-written; a UniFFI C++ generator path is outside the contract |
| C/C++ package managers and Release archives | vcpkg / Conan integration and public Release archive assets are outside the channel |
| C/C++ natives beyond `osx-arm64` and `win-x64` | Linux / Android / iOS C++ natives are outside the committed artifact set; `tooling/connect/cpp-build.mjs` accepts exactly `aarch64-apple-darwin` and `x86_64-pc-windows-msvc` |
| C/C++ engine-internal verification | UE editor and packaged-game integration is a maintainer checklist, not a package gate; the standalone smokes are the recorded evidence |
| Cross-image C ABI handles | Handles and owned buffers stay within the loaded carrier image; independently loaded copies of the native library are separate ownership domains |

## Invariants

- `.github/workflows/release.yml` stays the sole top-level publish workflow (tag push `v*` or `release`-labeled PR merge); Trusted Publishing OIDC binds to that filename, so npm, crates.io and PyPI publishing live inside `publish-npm` / `publish-crates` / `publish-pypi` and the GitHub Packages jobs are siblings in the same workflow.
- The channel split is fixed: registry-backed C# (GitHub Packages NuGet), Kotlin (GitHub Packages Maven) and Python (PyPI); git-based Swift (Swift Package Manager), Go (Go modules) and C/C++ (committed headers plus platform carriers).
- Lockstep SemVer: every binding surface ships the repository `X.Y.Z`. Registry packages carry that version in their metadata, tag-resolved channels take it from the git tag `vX.Y.Z`, and the C/C++ channel adds no version-bearing manifest.
- Stable `vX.Y.Z` and non-`-rc.` prerelease tags publish; tags containing `-rc.` create GitHub pre-releases only, with no npm, crates.io, GitHub Packages or PyPI push.
- No long-lived `NPM_TOKEN`, `CARGO_REGISTRY_TOKEN` or `PYPI_TOKEN` repository secret is on the release path — each registry channel uses OIDC Trusted Publishing or `GITHUB_TOKEN` with `packages: write`.
- The C/C++ channel has no registry feed, no publish job and no Release archive assets; consumers resolve headers and carriers from the tag, and the two carrier dynamic libraries are git-lfs objects.
- The C# binding is session core only — transport stays product-owned — and crates.io ships the Rust crate with `bindings/**` excluded from the package.
- Re-running Release at an already-published tag is safe: the pre-check over the expected artifact set is the skip gate, and registry doubt (network error, HTTP 401/403/5xx, malformed JSON, unexpected payload) fails the job instead of reporting green.

## Where it lives

- `.github/workflows/release.yml` — `publish-npm`, `publish-crates`, `publish-pypi`, `publish-nuget`, `publish-maven` and the `build-connect-ffi` matrix (`linux-x64`, `win-x64`, `osx-arm64`).
- `.github/workflows/xcframework.yml` — CI-assembled Swift xcframework (`macos-15`).
- `.github/workflows/cpp-connect.yml` — C/C++ carrier smoke lanes (`windows-smoke`, `macos-smoke`).
- `packages/spoke-connect-ts/` — npm `@42ch/spoke-connect`, current metadata and exports in `package.json`.
- `crates/spoke-connect/Cargo.toml` — crates.io `spoke-connect`, `bindings/**` excluded.
- `crates/spoke-connect/bindings/{csharp,kotlin,swift,go,python,cpp}/` — the binding packages; C# `generated/spoke_connect.cs`, Python `pyproject.toml`, Kotlin `build.gradle.kts`, C++ `include/spoke_connect.h`, `include/spoke_connect.hpp`, `native/<rid>/`, `native/provenance.json`.
- Root `Package.swift` (product `SpokeConnect`) and root `go.mod` (module `github.com/42ch-dev/spoke`) — the two tag-resolved version surfaces.
- `tooling/release/lockstep-surfaces.mjs` — assert + bump for every version-bearing binding manifest; `tooling/connect/verify-python-wheels.sh` — the expected PyPI wheel set; `tooling/connect/cpp-symbol-check.mjs` — the C ABI gate.
- `crates/spoke-connect/bindings/csharp/` — the packable csproj carrying the NuGet id `42ch.Spoke.Connect`.

## Change obligations

- A channel, feed, package id or coordinates change → update the surface inventory, the channel classification and the registry & auth table in §Current state in the same PR, and record the reason in `.agents/adr/implemented/process/2026-08-02-connect-publish-channels.md` (or a new record) rather than only in this page.
- A new binding gain a version-bearing manifest → register it in `tooling/release/lockstep-surfaces.mjs` (assert + bump) and add its row to §Current state.
- A publish job is added, renamed or moved → update the staging table, the registry & auth table and the re-run semantics in §Current state, and keep every OIDC publisher registration pointing at `release.yml`.
- The expected artifact set for a registry lane changes (wheel tags, Maven files, JNA resources, NuGet RIDs) → update the pre-check description and the verifying tool under `tooling/` in the same PR.
- The tag gate or lockstep policy changes → update §Current state and the normative release rules in `.agents/specs/protocol/spoke-version-release.md` together.
- A git-based channel gains a registry feed, or a registry channel gains a mirror → update the channel matrix and the non-goals here, plus the corresponding ADR.
