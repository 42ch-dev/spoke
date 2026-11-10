# ADR: One frozen packaging shape per connect binding language

Status: implemented — every Path B language has one recorded package shape, generator route and CI job instead of per-implementation invention

Date: 2026-08-03

Scope: packaging contract for the native binding channels — module paths, package coordinates, native artifact layout, generator route, version surface and CI job shape

## Problem

The publish strategy decides *which* channel each connect surface uses and *when* publish runs, but it does not fix *how* a binding language is packaged. Without that, six implementations would each invent a module path, a native-artifact layout, a generator route and a CI job shape, and integrators would read six different sets of installation instructions for one session-core surface. The binding languages also disagree on fundamentals: some need a registry manifest with a version field, some resolve only from a repository-root manifest, and one has no version-bearing manifest at all.

## Decision

Every Path B language gets one frozen shape, and all shapes share a common contract.

Common contract:

- **One cdylib** carries the exported-surface metadata for every uniffi language: the `spoke-connect` `ffi` feature, uniffi 0.32, `crate-type = ["rlib", "cdylib"]`, with no per-language uniffi pins. The C/C++ channel is the one separate build — the workspace-private `spoke-connect-capi` carrier with its own hand-written ABI.
- **First-party generators** for Swift, Kotlin and Python use the crate-local `uniffi-bindgen` bin, so no community version skew is possible. **Community generators** (C#, Go) pass a feasibility gate and, where the gap is real and small, a vendored fork; **hand-written** headers cover C/C++.
- **Generated sources are committed** per language and regenerated only when the FFI surface changes; consumers never run bindgen.
- **Golden parity** is mandatory: every language smoke asserts the committed Rust fixtures — golden peer id, golden hello signature, protocol version `1`.
- **Lockstep version surfaces**: a manifest that carries a version registers in the lockstep assert-and-bump tooling; tag-resolved channels take the version from the tag, and the C/C++ channel adds none.
- **Native provenance** comes from one `build-connect-ffi` matrix (`linux-x64`, `win-x64`, `osx-arm64`); packagers assemble per-language layouts from those artifacts, and crates.io excludes the whole `bindings/**` tree.
- **Tag gate** is shared: stable and non-`-rc.` prerelease tags publish, `-rc.` tags create pre-releases only.

Per-language shapes:

| Language | Shape decision |
|---|---|
| C# | Packable project with committed generated C#, CI-assembled `runtimes/<rid>/native/`, published to GitHub Packages NuGet |
| Go | Integrator package `spokeconnect` over committed generated cgo sources, per-`GOOS_GOARCH` native directories wired by cgo `LDFLAGS` and rpath; version from the tag |
| Python | One platform wheel per RID tagged `py3-none-<platform>`, each bundling exactly its RID's shared library beside the generated module; no sdist |
| Swift | Root manifest only, library product `SpokeConnect`, committed generated sources, committed xcframework as a local binary target |
| Kotlin | Gradle `maven-publish`, committed generated Kotlin, native libraries as JNA classpath resources inside the jar |
| C / C++ | Two committed hand-written headers plus committed per-RID carriers, resolved from the tag, gated by the symbol check |

## Alternatives considered

**GitHub Packages for Swift, Go and Python.** Rejected: those ecosystems resolve from git — Swift Package Manager reads a repository-root manifest only, Go modules read tags, PyPI is the Python registry — so a GitHub Packages layer would be packaging for its own sake.

**A `go.mod` inside the binding subdirectory.** Rejected: a module declared in a subdirectory is versioned by subdirectory-prefixed tags, which forks the single-tag release model and doubles the tag count. A root `go.mod` keeps one annotated-tag family as the only version surface and accepts the whole-repository module zip as the cost. If the long import path or the zip size becomes a measured consumer problem, the escape hatch is a dedicated binding repository — recorded, not adopted.

**Splitting the bindings into their own repositories.** Rejected as the default: monorepo paths are the contract, and the single tag drives every channel. The dedicated-repository escape hatch stays recorded for the Go case above.

**Consumer-side bindgen, or generated sources generated at install time.** Rejected: generated sources and natives ship inside the package, and bindgen stays maintainer tooling, so a consumer needs a C toolchain at most — never a Rust toolchain and never a generator version.

**A release-asset binary target for the Swift xcframework instead of a committed one.** Deferred: it replaces the committed artifact only once pre-tag artifact upload and manifest-checksum automation exist. Until then the xcframework is CI-assembled on every pull request and committed.

**Async node lifecycle over FFI.** Rejected as out of scope: the cdylib owns a process-wide Tokio runtime, and the adapter/router/responder objects are synchronous block-on-async surfaces over it; node lifecycle stays Rust-side, which is why every language ships a blocking API rather than an async-shaped one.

**Per-language SemVer.** Rejected: every channel stays on the monorepo tag until the publish strategy's revisit trigger fires.

## Consequences

- Adding a binding language now means adding a row to the packaging contract and a version-surface registration, not inventing an install story.
- The commit policy for generated sources makes binding shape reviewable in diffs, at the cost of regeneration discipline whenever the FFI surface changes.
- The feasibility gate is mandatory and evidence-producing before any community-generated binding is committed; a silent skip or a pin downgrade is not an available outcome.
- The Go channel ships the whole repository as its module zip — an accepted cost with a recorded escape hatch.
- Native layouts differ per ecosystem by design (JNA resources, `runtimes/<rid>`, wheels, xcframework slices, git-lfs carriers), which is why the packaging contract is the place a consumer looks rather than a single universal layout.
- The C/C++ channel has no registry feed and no version-bearing manifest, so its ABI revision is deliberately independent of the repository version.

Related: [the publish strategy current state](../../../specs/connect/connect-publish-strategy.md), [the C/C++ boundary decision](./2026-09-18-connect-cpp-binding.md), [the vendored bindgen fork](../process/2026-08-02-connect-csharp-uniffi-bindgen-fork.md).
