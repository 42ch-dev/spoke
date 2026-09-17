# Connect binding channels — packaging contract

**Status:** Informative decision record — freezes the per-language package shapes for Path B (**native bindings**) packages. Does not change connect envelopes, the lockstep release policy, or the channel split.

**Updated:** 2026-09-18

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
| Single cdylib | One `spoke-connect` build — `ffi` feature, uniffi **0.32**, `crate-type = ["rlib", "cdylib"]` — carries the exported-surface metadata for every uniffi language. No per-language uniffi pins. The C/C++ channel is the one separate build: the workspace-private `spoke-connect-capi` carrier (§3.6) links `spoke-connect` with `ffi,remote-adapter` and exports a hand-written C ABI from its own cdylib |
| Exported surface | Core sync facade (8 functions + 3 objects + 2 error enums: `CoreError` with 8 variants, `CoreInvokeError` with 3) plus the additive remote-adapter FFI: the sync `RemoteAdapterFFI` (constructor + 21 methods), `MultiPeerRouterFFI` (constructor + 14 methods) and `ConnectResponderFFI` (constructor + 7 methods) objects; the foreign-callback `Transport` (`send`/`recv`/`close`), `PortsHandler` (13 callbacks — nine baseline serve methods, `project`, `compute`, `list_fork_timeline_events`, `extract`) and `ToolHandler` (`handle`) interfaces; the in-memory loopback helpers; and the `FfiError` / `TransportError` enums (crate README "Binding facade"; member-by-member inventory in `bindings/cpp/parity.md`). The cdylib owns a process-wide tokio runtime and the RemoteAdapter / router FFI is a synchronous block-on-async surface; concrete transports stay product-owned |
| Generator routes | **First-party** (Swift, Kotlin, Python): the crate-local `uniffi-bindgen` bin (`bindgen-cli` feature, `uniffi::uniffi_bindgen_main`) generates from the pinned cdylib — no community version skew. **Community** (C#, Go): external bindgen behind the feasibility gate (§5). **Hand-written** (C/C++, §3.6): the carrier maintainer writes `bindings/cpp/include/spoke_connect.h` together with the Rust exports, and the executable `tooling/connect/cpp-symbol-check.mjs` gate holds the two sets in step |
| Generated sources | **Committed** per language (C# `generated/spoke_connect.cs` is the reference); regenerated only when the FFI surface changes. Consumers never run bindgen. The C/C++ channel commits a hand-written header instead of generated sources |
| Golden parity | Every language smoke asserts the committed Rust fixtures: golden peer id `12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf`, the golden base64url hello signature, and protocol version `1` |
| Lockstep SemVer | Binding manifests carry the repo `X.Y.Z`; new version surfaces register in `tooling/release/lockstep-surfaces.mjs` (assert + bump). Tag-resolved channels (SPM, Go modules, C/C++) take the version from the git tag `vX.Y.Z` itself; the C/C++ channel adds no version-bearing manifest — the header's ABI revision is independent of the repo version |
| Native provenance | The `build-connect-ffi` matrix on `release.yml` (`linux-x64`, `win-x64`, `osx-arm64`) is the single native build; registry packagers assemble per-language layouts from those artifacts. The Swift xcframework is CI-assembled on `macos-14` by the `xcframework` job ([`.github/workflows/xcframework.yml`](../../.github/workflows/xcframework.yml)) when the FFI surface changes; the Go `native/` dylibs are maintainer-built and refreshed when the FFI surface changes, not on every release. The Swift xcframework static libraries are tracked via **git-lfs** (`.gitattributes`) so refreshes do not accumulate staticlib churn in git history; SPM consumers need git-lfs (bundled with Xcode; CLI `swift build` users run `git lfs install` once). The Go `native/` dylibs stay as plain git blobs — Go modules resolve through the module proxy, which does not smudge LFS. The C/C++ carrier natives are separate from that matrix: `crates/spoke-connect/bindings/cpp/native/provenance.json` records, per RID, the source revision, target, `rustc -Vv`, compiler version, build flags, header SHA-256 and artifact SHA-256 of the committed files; `tooling/connect/cpp-build.mjs` stages them from the release source tree, and the [`.github/workflows/cpp-connect.yml`](../../.github/workflows/cpp-connect.yml) lane produces the Windows pair (§3.6) |
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
| Native set | `darwin_amd64/libspoke_connect.dylib`, `darwin_arm64/libspoke_connect.dylib` — committed, rebuilt when the FFI surface changes; `linux_amd64` / `windows_amd64` natives are a recorded follow-up |
| Native wiring | cgo (`CGO_ENABLED=1`): `#cgo LDFLAGS` selects `native/${GOOS}_${GOARCH}` under `${SRCDIR}` (per-platform shim files or `${GOOS}`/`${GOARCH}` substitution); rpath covers Linux/macOS lookup; Windows consumers place `spoke_connect.dll` beside the executable. Consumers need a C toolchain, never a Rust toolchain |
| Generator | Community `uniffi-bindgen-go` — vendored fork retargeted to uniffi 0.32 (landed; §5) |

**Why a root `go.mod`:** a Go module declared in a subdirectory is versioned by subdirectory-prefixed tags (`crates/spoke-connect/bindings/go/vX.Y.Z`), which forks the single-tag release model and doubles tag count. A root `go.mod` keeps one annotated-tag family (`vX.Y.Z`) as the only version surface; the whole-repo module zip is the accepted cost. If the long import path or repo-size zip becomes a measured consumer problem, the escape hatch is a dedicated `spoke-connect-go` repository — a recorded deferral, not part of this contract.

### 3.3 Python — PyPI Trusted Publishing

| Field | Value |
|-------|-------|
| Project dir | `crates/spoke-connect/bindings/python/` — `pyproject.toml` + import package `spoke_connect/` (generated module committed) + `Smoke/` |
| PyPI name | **`spoke-connect`** — Trusted Publishing publisher registered for repository `42ch-dev/spoke` + workflow `release.yml` |
| Wheel shape | One platform wheel per ffi-matrix RID, PEP 425 tags `manylinux_2_35_x86_64` (linux-x64), `macosx_11_0_arm64` (osx-arm64), `win_amd64` (win-x64), each tagged `py3-none-<platform>`; each wheel bundles exactly its RID's shared library beside the generated module (the uniffi Python loader resolves the cdylib relative to the module file). The manylinux tag matches the CI build image glibc floor (`ubuntu-22.04` → `manylinux_2_35_x86_64`) |
| sdist | Not published in v1 (a source install cannot produce the native library without a Rust toolchain) — recorded deferral |
| CI job | `publish-pypi` on `release.yml` — sibling to `publish-nuget`, `needs: build-connect-ffi`, same non-`-rc.` tag gate; `pypa/gh-action-pypi-publish` with OIDC Trusted Publishing (no long-lived `PYPI_TOKEN`); if the publisher registered an environment, the job declares the same `environment:` |
| Generator | First-party `--language python` from the crate-local CLI (uniffi 0.32) — no community skew |

### 3.4 Swift — Swift Package Manager over git

| Field | Value |
|-------|-------|
| Manifest | **Root `Package.swift`** — SPM resolves git-url dependencies from the repo-root manifest only; subdirectory manifests are not supported |
| Product | Library `SpokeConnect`; consumers `.package(url: "https://github.com/42ch-dev/spoke.git", from: "X.Y.Z")` + `.product(name: "SpokeConnect", package: "spoke")` |
| Targets | `SpokeConnect` — Swift sources at the committed generated path `crates/spoke-connect/bindings/swift/generated/`; `spoke_connectFFI` — local `.binaryTarget` xcframework committed under `crates/spoke-connect/bindings/swift/xcframework/` (module name matches the generated `import spoke_connectFFI`) |
| Generated policy | `bindings/swift/generated/` flips from gitignored to **committed** (mirroring C#); regenerated when the FFI surface changes |
| xcframework | CI-assembled on `macos-14` by the `xcframework` job ([`.github/workflows/xcframework.yml`](../../.github/workflows/xcframework.yml)) from the checkout's Rust sources (pinned toolchain 1.96.0, four Apple targets, `--locked` build via `tooling/connect/build-swift-xcframework.sh`, `xcodebuild -create-xcframework` slice assembly); three library slices — `macos-arm64`, `ios-arm64`, `ios-arm64_x86_64-simulator` (arm64 + x86_64 lipo-combined) — covering macOS arm64 hosts, iOS devices, and iOS simulators on both Apple Silicon and Intel hosts. The path-filtered job runs when the FFI surface changes; `tooling/connect/verify-xcframework-drift.sh` fails the job on any per-file SHA-256 mismatch against the committed (LFS) artifact, and the built xcframework + hash manifest upload every run. Refresh = `tooling/connect/apply-xcframework-artifact.sh <run-id>` then commit |
| Smoke | The macOS-local swiftc smoke (`bindings/swift/Smoke/`) stays the golden-parity gate; `swift build` on the root package validates the SPM layout; `bindings/swift/IosSmoke/` (maintainer-local SwiftPM package) runs the same golden triad through the simulator slice via `xcodebuild test` |
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

### 3.6 C / C++ — hand-written C ABI over git

| Field | Value |
|-------|-------|
| Status | **Landed** — hand-written header, committed `osx-arm64` and `win-x64` carriers, symbol gate green (`83 declarations, 83 exports, 0 missing, 0 extra`), macOS and Windows smokes green |
| Carrier | `crates/spoke-connect-capi` — package `spoke-connect-capi`, library `spoke_connect_capi`, `publish = false`, `crate-type = ["rlib", "cdylib"]`; depends on `spoke-connect` with `ffi,remote-adapter` and wraps the public `spoke_connect::ffi` functions and objects. `crates/spoke-connect` keeps its own targets, features and uniffi surface |
| Header | `crates/spoke-connect/bindings/cpp/include/spoke_connect.h` — hand-written, C99 language floor (`<stdint.h>` / `<stddef.h>`) with `extern "C"` inclusion for C++, opaque per-type handles, `int32_t` status plus a caller-supplied error record, and ABI revision `1` through `spoke_connect_abi_version` |
| Layout | `crates/spoke-connect/bindings/cpp/` — `include/spoke_connect.h`, `native/osx-arm64/libspoke_connect_capi.dylib`, `native/win-x64/spoke_connect_capi.dll` + `spoke_connect_capi.dll.lib`, `native/provenance.json`, `Smoke/main.cpp`, `parity.md`, `ue/SpokeConnect.Build.cs` + `ue/README.md`, `README.md` |
| Generator route | **Hand-written header, no generator** — the maintainer writes the header together with the Rust exports. The drift gate is executable: `node tooling/connect/cpp-symbol-check.mjs --header <header> --library <native>` parses the marked declaration block, diffs it against the native's exported `spoke_connect_*` symbols in both directions, compiles a C99 probe holding a typed function pointer to every declaration, and compiles a C++17 inclusion check with exceptions and RTTI disabled |
| Parity | `crates/spoke-connect/bindings/cpp/parity.md` maps every production facade member, callback and error variant to its C declaration — 82 facade members + 15 error variants across 83 declarations, unmatched count zero |
| Native provenance | `crates/spoke-connect/bindings/cpp/native/provenance.json` — per RID: source revision, target, `rustc -Vv`, compiler version, build flags, header SHA-256 and artifact SHA-256. Separate from the UniFFI `build-connect-ffi` matrix; `tooling/connect/cpp-build.mjs` stages a native from the release source tree and records the fields |
| CI lane | [`.github/workflows/cpp-connect.yml`](../../.github/workflows/cpp-connect.yml) — job `windows-smoke` on `windows-latest`, path-filtered to the carrier, the linked Rust sources, the header/smoke/scripts and the workflow itself, running on `pull_request`, `push` to `main` and `workflow_dispatch`; it builds the carrier for `x86_64-pc-windows-msvc` (stable Rust 1.96.0, MSVC, `-C target-feature=-crt-static`), runs the symbol gate and the C/C++ probes, then compiles, links and runs the shared smoke. The lane is green on Windows, and its uploaded artifacts are the committed `win-x64` pair |
| Smoke | `node tooling/connect/cpp-smoke.mjs --rid osx-arm64` (Apple clang) and `--rid win-x64` (MSVC) compile, link and run `Smoke/main.cpp` against `crates/spoke-connect/tests/fixtures/golden-hello.json`; the six banners are golden peer-id, golden hello signature, protocol version 1, loopback ports, rejection/ownership and `C++ smoke` |
| Distribution | **Git-based** — the committed header and platform natives resolve together from the repository tag `vX.Y.Z`, the resolution model the Go and SPM channels use; the channel has no registry feed and no Release archive assets |
| Decision record | [`connect-cpp-binding.md`](connect-cpp-binding.md) |
| Non-goals | §6 records the channel exclusions; the decision record lists the same boundary (convenience/RAII wrappers, C++20 generator integration, vcpkg/Conan, Release archives, additional platform artifacts, engine-internal verification, new wire/session semantics, exposed envelope-auth helpers, async node lifecycle, cross-image handle interoperability) |

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
| Go | Community `uniffi-bindgen-go` | **Landed via vendored fork** retargeted to uniffi 0.32; fork dropped when upstream tags 0.32+ |
| Kotlin | First-party (crate-local CLI) | No skew possible; gate = generate + Gradle compile + JNA load + golden parity |
| Python | First-party (crate-local CLI) | No skew possible; gate = generate + import + golden parity |
| Swift | First-party (crate-local CLI) | Landed (macOS smoke + iOS Simulator golden parity through the committed xcframework) |
| C / C++ | Hand-written header (no generator, §3.6) | No bindgen skew by construction; gate = `tooling/connect/cpp-symbol-check.mjs` declaration ⇄ export diff in both directions, C99 link-all-declarations probe, C++17 no-exceptions/no-RTTI inclusion check, then the shared C++17 smoke per RID |

---

## 6. Non-goals

| Non-goal | Detail |
|----------|--------|
| GitHub Packages for Swift / Go / Python | SPM git, Go modules git, and PyPI are the locked channels for those languages |
| Registry mirrors | No nuget.org / Maven Central / Swift Package Registry primary feeds |
| Async node over FFI | The cdylib owns a process-wide tokio runtime; `RemoteAdapterFFI` / `MultiPeerRouterFFI` are synchronous block-on-async surfaces over that runtime; node lifecycle stays Rust-side |
| Per-language SemVer | Lockstep with the monorepo tag until the strategy's revisit trigger fires |
| Consumer-side bindgen | Generated sources and natives ship in the package; bindgen is maintainer tooling |
| Split binding repositories | Monorepo paths are the contract; dedicated repos are a recorded escape hatch (Go §3.2) |
| C/C++ convenience layer | RAII / `std::string`-friendly wrappers are a demand-gated enhancement; the channel ships the C ABI boundary (§3.6) |
| C++20 generator integration | The C/C++ header is hand-written; a UniFFI C++ generator path is outside this contract |
| C/C++ package managers and Release archives | vcpkg / Conan integration and public Release archive assets are outside the channel; distribution stays git-based (§3.6) |
| C/C++ natives beyond `osx-arm64` and `win-x64` | Linux / Android / iOS C++ natives are outside the committed artifact set; `tooling/connect/cpp-build.mjs` builds other targets when a consumer needs one |
| C/C++ engine-internal verification | UE editor and packaged-game integration is a maintainer checklist ([`bindings/cpp/ue/README.md`](../../crates/spoke-connect/bindings/cpp/ue/README.md)), not a package gate; the standalone smokes are the recorded evidence |
| Cross-image C ABI handles | Handles and owned buffers stay within the loaded carrier image; independently loaded copies of the native library are separate ownership domains |

---

## 7. Links

| Path | Use |
|------|-----|
| [`connect-publish-strategy.md`](connect-publish-strategy.md) | Channel split + staging SSOT |
| [`connect-csharp-binding.md`](connect-csharp-binding.md) | C# decision record (landed) |
| [`connect-cpp-binding.md`](connect-cpp-binding.md) | C/C++ C ABI decision record (landed) |
| [`crates/spoke-connect/bindings/cpp/parity.md`](../../crates/spoke-connect/bindings/cpp/parity.md) | C ABI ⇄ production facade parity table |
| [`crates/spoke-connect/bindings/cpp/include/spoke_connect.h`](../../crates/spoke-connect/bindings/cpp/include/spoke_connect.h) | The C/C++ contract header |
| [`.github/workflows/cpp-connect.yml`](../../.github/workflows/cpp-connect.yml) | C carrier Windows smoke lane |
| [`spoke-version-release.md`](spoke-version-release.md) | Lockstep SemVer + Trusted Publishing |
| [`.mstar/knowledge/architecture-patterns/connect-uniffi-bindgen-fork.md`](../knowledge/architecture-patterns/connect-uniffi-bindgen-fork.md) | Vendored-fork technique |
| [`.mstar/knowledge/architecture-patterns/connect-session-core-ffi-boundary.md`](../knowledge/architecture-patterns/connect-session-core-ffi-boundary.md) | FFI boundary + golden vectors |
| [`crates/spoke-connect/README.md`](../../crates/spoke-connect/README.md) | Binding facade (exported surface) |
| [`.github/workflows/release.yml`](../../.github/workflows/release.yml) | `build-connect-ffi` matrix + publish jobs |
