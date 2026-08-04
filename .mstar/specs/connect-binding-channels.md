# Connect binding channels — packaging contract

**Status:** Informative decision record — freezes the per-language package shapes for Path B (**native bindings**) packages. Does not change connect envelopes, the lockstep release policy, or the channel split.

**Updated:** 2026-08-04

**Vocabulary:** **Path B** (internal) = consumer **native bindings** — host languages that embed the shared Rust session core via FFI. Distinct from **Path A** / **language-native client** (TypeScript) and from the **Rust reference** crate itself. Normative map: [`spoke-connect.md`](spoke-connect.md) §Embedding model.

---

## 1. Purpose

[`connect-publish-strategy.md`](connect-publish-strategy.md) decides **which** channel each connect surface uses and **when** publish runs. This record freezes **how** each Path B (native binding) language is packaged: module paths, package coordinates, native artifact layout, generator route, and CI job shape — so implementers build one shape and integrators read one shape.

| Spec | Role |
|------|------|
| [`connect-publish-strategy.md`](connect-publish-strategy.md) | Channel split (NuGet/Maven GH Packages, SPM git, Go modules git, PyPI), staging, registry & auth |
| [`spoke-version-release.md`](spoke-version-release.md) | Normative lockstep SemVer, annotated tags, Trusted Publishing |
| [`connect-csharp-binding.md`](connect-csharp-binding.md) | C# binding decision record (vendored bindgen fork, landed) |
| [`spoke-connect.md`](spoke-connect.md) | Normative connect wire + embedding model (Path B definition) |
| This document | Packaging contract per binding language (facts + coordinates) |

---

## 2. Common contract

| Rule | Fact |
|------|------|
| Single cdylib | One `spoke-connect` build — `ffi` feature, uniffi **0.32**, `crate-type = ["rlib", "cdylib"]` — carries the exported-surface metadata for every language. No per-language uniffi pins |
| Exported surface | 8 functions + 3 objects + 2 error enums (crate README "Binding facade"); **core-only sync** surface; transport stays product-owned |
| Generator routes | **First-party** (Swift, Kotlin, Python): the crate-local `uniffi-bindgen` bin (`bindgen-cli` feature, `uniffi::uniffi_bindgen_main`) generates from the pinned cdylib — no community version skew. **Community** (C#, Go): external bindgen behind the feasibility gate (§5) |
| Generated sources | **Committed** per language (C# `generated/spoke_connect.cs` is the reference); regenerated only when the FFI surface changes. Consumers never run bindgen |
| Golden parity | Every language smoke asserts the committed Rust fixtures: golden peer id `12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf`, the golden base64url hello signature, and protocol version `1` |
| Lockstep SemVer | Binding manifests carry the repo `X.Y.Z`; new version surfaces register in `tooling/release/lockstep-surfaces.mjs` (assert + bump). Tag-resolved channels (SPM, Go modules) take the version from the git tag `vX.Y.Z` itself |
| Native provenance | The `build-connect-ffi` matrix on `release.yml` (`linux-x64`, `win-x64`, `osx-arm64`) is the single native build; registry packagers assemble per-language layouts from those artifacts. Repo-committed natives (Swift xcframework, Go `native/`) are maintainer-built and refreshed when the FFI surface changes, not on every release |
| crates.io package | `crates/spoke-connect/Cargo.toml` `exclude`s the entire `bindings/**` tree (Path B sources + natives). crates.io ships the Rust crate only; language bindings stay on the four Path B channels and the git repo |
| Tag gate | Stable `vX.Y.Z` and non-`-rc.` prerelease tags publish; `-rc.` tags create GitHub pre-releases only |

---

## 3. Channel contracts

### 3.1 C# — GitHub Packages NuGet (landed reference)

| Field | Value |
|-------|-------|
| Package id | `42ch.Spoke.Connect` |
| Feed | `https://nuget.pkg.github.com/42ch-dev/index.json` |
| Layout | `crates/spoke-connect/bindings/csharp/` — packable csproj + committed `generated/spoke_connect.cs` + `runtimes/<rid>/native/` (CI-assembled, gitignored) |
| CI job | `publish-nuget` on `release.yml` (`needs: build-connect-ffi`) |
| Decision record | [`connect-csharp-binding.md`](connect-csharp-binding.md) |

### 3.2 Go — Go modules over git

| Field | Value |
|-------|-------|
| Module path | `github.com/42ch-dev/spoke` — declared by a **root `go.mod`** |
| Import path | `github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go` — integrator package `spokeconnect` re-exports the generated `generated/spoke_connect` cgo surface; consumers import `bindings/go`, not the generated subdirectory |
| Versioning | Repo tags `vX.Y.Z`; consumers `go get github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go@vX.Y.Z` |
| Layout | `crates/spoke-connect/bindings/go/` — committed generated Go + `native/<goos>_<goarch>/` + `Smoke/` + README |
| Native set | `linux_amd64/libspoke_connect.so`, `windows_amd64/spoke_connect.dll`, `darwin_arm64/libspoke_connect.dylib` — committed, rebuilt when the FFI surface changes |
| Native wiring | cgo (`CGO_ENABLED=1`): `#cgo LDFLAGS` selects `native/${GOOS}_${GOARCH}` under `${SRCDIR}` (per-platform shim files or `${GOOS}`/`${GOARCH}` substitution); rpath covers Linux/macOS lookup; Windows consumers place `spoke_connect.dll` beside the executable. Consumers need a C toolchain, never a Rust toolchain |
| Generator | Community `uniffi-bindgen-go` behind the feasibility gate (§5) |

**Why a root `go.mod`:** a Go module declared in a subdirectory is versioned by subdirectory-prefixed tags (`crates/spoke-connect/bindings/go/vX.Y.Z`), which forks the single-tag release model and doubles tag count. A root `go.mod` keeps one annotated-tag family (`vX.Y.Z`) as the only version surface; the whole-repo module zip is the accepted cost. If the long import path or repo-size zip becomes a measured consumer problem, the escape hatch is a dedicated `spoke-connect-go` repository — a recorded deferral, not part of this contract.

### 3.3 Python — PyPI Trusted Publishing

| Field | Value |
|-------|-------|
| Project dir | `crates/spoke-connect/bindings/python/` — `pyproject.toml` + import package `spoke_connect/` (generated module committed) + `Smoke/` |
| PyPI name | **`spoke-connect`** — registered Pending publisher for repository `42ch-dev/spoke` + workflow `release.yml` |
| Wheel shape | One platform wheel per ffi-matrix RID, PEP 425 tags `manylinux_2_35_x86_64` (linux-x64), `macosx_11_0_arm64` (osx-arm64), `win_amd64` (win-x64), each tagged `py3-none-<platform>`; each wheel bundles exactly its RID's shared library beside the generated module (the uniffi Python loader resolves the cdylib relative to the module file). The manylinux tag matches the CI build image glibc floor (`ubuntu-22.04` → `manylinux_2_35_x86_64`) |
| sdist | Not published in v1 (a source install cannot produce the native library without a Rust toolchain) — recorded deferral |
| CI job | `publish-pypi` on `release.yml` — sibling to `publish-nuget`, `needs: build-connect-ffi`, same non-`-rc.` tag gate; `pypa/gh-action-pypi-publish` with OIDC Trusted Publishing (no long-lived `PYPI_TOKEN`); if the Pending publisher registered an environment, the job declares the same `environment:` |
| Generator | First-party `--language python` from the crate-local CLI (uniffi 0.32) — no community skew |

### 3.4 Swift — Swift Package Manager over git

| Field | Value |
|-------|-------|
| Manifest | **Root `Package.swift`** — SPM resolves git-url dependencies from the repo-root manifest only; subdirectory manifests are not supported |
| Product | Library `SpokeConnect`; consumers `.package(url: "https://github.com/42ch-dev/spoke.git", from: "X.Y.Z")` + `.product(name: "SpokeConnect", package: "spoke")` |
| Targets | `SpokeConnect` — Swift sources at the committed generated path `crates/spoke-connect/bindings/swift/generated/`; `spoke_connectFFI` — local `.binaryTarget` xcframework committed under `crates/spoke-connect/bindings/swift/xcframework/` (module name matches the generated `import spoke_connectFFI`) |
| Generated policy | `bindings/swift/generated/` flips from gitignored to **committed** (mirroring C#); regenerated when the FFI surface changes |
| xcframework | Maintainer-built from `staticlib` slices via `xcodebuild -create-xcframework` (tooling script under `tooling/connect/`); macOS arm64 first slice, x86_64 / iOS slices on demand; rebuilt + committed when the FFI surface changes |
| Smoke | The macOS-local swiftc smoke (`bindings/swift/Smoke/`) stays the golden-parity gate; `swift build` on the root package validates the SPM layout |
| Scale-out | A release-asset `binaryTarget` (URL + checksum) replaces the committed xcframework only when pre-tag artifact + manifest-checksum automation exists — recorded deferral |

### 3.5 Kotlin — GitHub Packages Maven

| Field | Value |
|-------|-------|
| Coordinates | `dev.42ch:spoke-connect` — reverse-DNS of the owned domain `42ch.dev`, the standard Maven Central namespace requirement (keeps a future Maven Central mirror possible) |
| Repository | `https://maven.pkg.github.com/42ch-dev/spoke` |
| Project dir | `crates/spoke-connect/bindings/kotlin/` — Gradle `build.gradle.kts` (`maven-publish`) + committed generated Kotlin + `Smoke/` |
| Generated namespace | uniffi default (`uniffi.spoke_connect`) — mirrors the C# `RootNamespace` decision; a branded namespace is a later cosmetic change |
| Native layout | JNA classpath resources inside the jar: `darwin-aarch64/libspoke_connect.dylib`, `linux-x86-64/libspoke_connect.so`, `win32-x86-64/spoke_connect.dll` — assembled from the `build-connect-ffi` matrix |
| CI job | `publish-maven` on `release.yml` — sibling to `publish-nuget`, same non-`-rc.` tag gate, `GITHUB_TOKEN` with `packages: write` |
| Scope | JVM-first; Android AAR (per-ABI `.so` packaging) is a recorded deferral |
| Generator | First-party `--language kotlin` from the crate-local CLI (uniffi 0.32) — no community skew |

---

## 4. Version surfaces added by bindings

| Manifest | Lockstep mechanism |
|----------|--------------------|
| `crates/spoke-connect/bindings/python/pyproject.toml` | Register in `tooling/release/lockstep-surfaces.mjs` (assert + bump) |
| `crates/spoke-connect/bindings/kotlin/build.gradle.kts` | Register in `tooling/release/lockstep-surfaces.mjs` (assert + bump) |
| Root `go.mod` | No version field — version is the git tag |
| Root `Package.swift` | No version field — version is the git tag |

---

## 5. Feasibility gate and vendored-fork pattern

Community bindgen tools can lag the repo's uniffi pin; metadata encoding and runtime contract checksums change between uniffi versions. The gate runs **before** any generated binding is committed:

1. **Live upstream recheck** — latest tag + `main` workspace pins, dated.
2. **Stock `--library`** against the repo's current cdylib.
3. **Positive control** — the crate-local `uniffi-bindgen` generates a first-party language (e.g. Swift) from the same cdylib, proving the metadata is well-formed.
4. **Vendored fork** when the gap is real and small — pin upstream SHA, bump `uniffi*` deps to the repo pin, compile-fix new `Type` arms, commit patch + lockfile + recipe under `bindings/<lang>/bindgen/`. Generation-only tooling; the product cdylib keeps a single uniffi pin. Dropped when upstream tags the repo's uniffi line.
5. **Escalate** (dual-pin or hand-written binding) only when the fork delta is large — never a silent skip, never a pin downgrade.

Full technique: [`.mstar/knowledge/architecture-patterns/connect-uniffi-bindgen-fork.md`](../knowledge/architecture-patterns/connect-uniffi-bindgen-fork.md); landed C# instantiation: [`connect-csharp-binding.md`](connect-csharp-binding.md).

| Language | Generator | Gate posture |
|----------|-----------|--------------|
| C# | Community `uniffi-bindgen-cs` | **Landed via vendored fork** retargeted to uniffi 0.32; fork dropped when upstream tags 0.32+ |
| Go | Community `uniffi-bindgen-go` | Gate required — upstream's latest tag targets uniffi 0.31 (checked 2026-08-03); the vendored-fork route is the expected path |
| Kotlin | First-party (crate-local CLI) | No skew possible; gate = generate + Gradle compile + JNA load + golden parity |
| Python | First-party (crate-local CLI) | No skew possible; gate = generate + import + golden parity |
| Swift | First-party (crate-local CLI) | Landed (macOS smoke, golden parity) |

---

## 6. Non-goals

| Non-goal | Detail |
|----------|--------|
| GitHub Packages for Swift / Go / Python | SPM git, Go modules git, and PyPI are the locked channels for those languages |
| Registry mirrors | No nuget.org / Maven Central / Swift Package Registry primary feeds |
| Async node over FFI | Core-only sync surface; node lifecycle stays Rust-side |
| Per-language SemVer | Lockstep with the monorepo tag until the strategy's revisit trigger fires |
| Consumer-side bindgen | Generated sources and natives ship in the package; bindgen is maintainer tooling |
| Split binding repositories | Monorepo paths are the contract; dedicated repos are a recorded escape hatch (Go §3.2) |

---

## 7. Links

| Path | Use |
|------|-----|
| [`connect-publish-strategy.md`](connect-publish-strategy.md) | Channel split + staging SSOT |
| [`connect-csharp-binding.md`](connect-csharp-binding.md) | C# decision record (landed) |
| [`spoke-version-release.md`](spoke-version-release.md) | Lockstep SemVer + Trusted Publishing |
| [`.mstar/knowledge/architecture-patterns/connect-uniffi-bindgen-fork.md`](../knowledge/architecture-patterns/connect-uniffi-bindgen-fork.md) | Vendored-fork technique |
| [`.mstar/knowledge/architecture-patterns/connect-session-core-ffi-boundary.md`](../knowledge/architecture-patterns/connect-session-core-ffi-boundary.md) | FFI boundary + golden vectors |
| [`crates/spoke-connect/README.md`](../../crates/spoke-connect/README.md) | Binding facade (exported surface) |
| [`.github/workflows/release.yml`](../../.github/workflows/release.yml) | `build-connect-ffi` matrix + publish jobs |
