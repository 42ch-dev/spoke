---
module: spoke-connect
date: 2026-08-04
last_updated: 2026-08-18
problem_type: tooling_decision
category: tooling-decisions
severity: medium
applies_when:
  - extending the Swift xcframework to a new Apple platform slice
  - refreshing the committed xcframework after an FFI surface change
  - cross-compiling the Rust crate for iOS targets
tags:
  - spoke-connect
  - swift
  - xcframework
  - ios
  - cross-compile
  - lipo
  - swiftpm
  - uniffi
---

# Swift xcframework multi-target build matrix (iOS slices)

## Context

The Swift binding ships the `SpokeConnect` SwiftPM product from a **committed** `spoke_connectFFI.xcframework` in the repo. The first slice covered macOS arm64. iOS integrators need the same FFI surface on devices (arm64) and on both simulator hosts (Apple Silicon and Intel Mac). Extending the matrix means cross-compiling the full Rust crate (libp2p + tokio) for three iOS targets and assembling one multi-slice xcframework, then validating arch/linkability and golden behavior on macOS and iOS consumer paths.

## Guidance

### 1. Target matrix and the nightly-toolchain gotcha

- rustup targets: `aarch64-apple-ios` (device), `aarch64-apple-ios-sim`, `x86_64-apple-ios`, plus the explicit host slice `aarch64-apple-darwin` (not implicit host-only).
- Staticlib per triple: `cargo +nightly rustc -p spoke-connect --features ffi --release --crate-type staticlib --target <triple>` (nightly per the repo toolchain convention).
- **Gotcha:** `rustup target add <ios-targets>` installs the targets for the default (stable) toolchain; `cargo +nightly` then fails with `error[E0463]: can't find crate for 'core'`. The fix is `rustup target add --toolchain nightly aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios`. The build script asserts targets with `rustup target list --installed --toolchain nightly` when nightly is active (plain list in CI) and prints a toolchain-accurate install hint on miss.

### 2. Assembler shape: one `-library` pair per platform, lipo fallback

- Assemble with one `xcodebuild -create-xcframework` carrying one `-library <slice.a> -headers <hdrs>` pair per slice, sharing one header set (`spoke_connectFFI.h` + `module.modulemap`).
- **Fallback:** Xcode rejects two discrete `-library` entries for the same platform — *"Both 'ios-x86_64-simulator' and 'ios-arm64-simulator' represent two equivalent library definitions"*. The plan-locked fallback: `lipo -create` the two simulator staticlibs into one **fat multi-arch slice**, then three `-library` pairs.
- Delivered shape: three `LibraryIdentifier`s covering four target triples — `macos-arm64`, `ios-arm64`, `ios-arm64_x86_64-simulator` (arm64 + x86_64 fat). Coverage equals four discrete slices.
- `xcodebuild -validate-xcframework` does not exist in Xcode 26.6 (only `-create-xcframework`). Validate with `plutil -lint` on the Info.plist, per-slice `lipo -info` arch assertions, and the consumer-path link builds below.

### 3. Committed artifact, CI-assembled

- The xcframework stays **committed in the repo**; between FFI changes the committed framework is the consumer artifact. Assembly runs in CI (`.github/workflows/xcframework.yml`, path-filtered on the FFI surface): the job builds the slices on `macos-14` and the committed artifact is the drift baseline — a sorted per-file SHA-256 manifest diff (`tooling/connect/verify-xcframework-drift.sh`) fails the job on mismatch, and the built artifact uploads on every run.
- Refresh path: `tooling/connect/apply-xcframework-artifact.sh <run-id>` downloads the CI artifact, checksum-verifies against its manifest, verifies run provenance, and stages the LFS pointers — no local four-target Rust build. After committing a refresh, re-run the drift check locally or via the workflow (a refresh-only push does not re-trigger the path-filtered gate).
- Build determinism: the script normalizes the `Info.plist` `AvailableLibraries` ordering (xcodebuild emits it nondeterministically) — same inputs, same bytes. Pattern details: [`ci-assembled-committed-native-artifacts.md`](ci-assembled-committed-native-artifacts.md).
- Generated Swift sources (`generated/spoke_connect.swift`, `spoke_connectFFI.h`, modulemap) stay **byte-identical** across rebuilds while the FFI surface is unchanged — the script verifies with SHA-256 before/after and must not rewrite them.
- Repo hygiene: `.gitignore` covers SwiftPM artifacts (`.build/`, `.swiftpm/`) at the repo root and local packages, mirroring the C#/Kotlin artifact-ignore pattern.

### 4. Consumer-path validation

- **macOS:** root `swift build` resolves the binary library; `bindings/swift/Smoke/` (swiftc host binary) stays green — golden peer id, golden base64url hello signature, tamper/verify, allowlist, sequences, nonce store, dispatch, correlation, protocol version.
- **iOS link checks:** `xcodebuild build -destination 'generic/platform=iOS'` (device) and `'generic/platform=iOS Simulator'` (simulator) against the root scheme — both must succeed.
- **Functional iOS proof:** a **local-only** SwiftPM package at `bindings/swift/IosSmoke/` — its own `Package.swift` + test target, a path dependency on the root `spoke` package product, **not** referenced from root `Package.swift` (root stays product-only). Run via `xcodebuild test -destination 'platform=iOS Simulator,name=<device>'`, asserting the golden peer id, golden base64url hello signature, and protocol version through the committed simulator slice. The golden fixture is a registered byte-identical copy under `tooling/connect/golden-vector-sync.mjs`.

## Why This Matters

- Integrators get device + both simulator hosts with one SPM dependency and no Rust toolchain; bindgen stays a maintainer operation.
- The lipo fat-simulator fallback is a known Xcode constraint; its reason is recorded in the script header so a future Xcode that supports per-platform multi-`-library` can re-try the discrete four-library form.
- Validation covers arch/link (lipo, plutil, destination builds) **and** behavior (IosSmoke golden parity), so the cross-compiled session core is proven identical on iOS, not merely linkable.
- The `--toolchain nightly` target-install gotcha costs a confusing `can't find crate for 'core'` failure every time it is missed; the script's assertion turns it into a one-line install hint.

## When to Apply

- Adding a new platform slice to the Swift binding (e.g. visionOS/tvOS) or refreshing the xcframework after an FFI surface change (CI assembles; apply script refreshes).
- Any future binding that ships prebuilt per-platform artifacts — reuse the committed-artifact + CI drift-gate + local-package-smoke shape.

## Examples

```bash
# Target install (nightly toolchain — the gotcha)
rustup target add --toolchain nightly aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios

# Staticlib per triple
cargo +nightly rustc -p spoke-connect --features ffi --release \
  --crate-type staticlib --target aarch64-apple-ios

# Fat simulator slice (Xcode rejects dual -library per platform)
lipo -create ios-arm64-sim/libspoke_connect.a ios-x86_64-sim/libspoke_connect.a \
  -output sim/libspoke_connect.a

# Assembler — one -library pair per slice, shared headers
xcodebuild -create-xcframework \
  -library macos/libspoke_connect.a -headers Headers \
  -library ios/libspoke_connect.a -headers Headers \
  -library sim/libspoke_connect.a -headers Headers \
  -output spoke_connectFFI.xcframework

# Functional iOS proof (local-only SwiftPM package, not on root Package.swift)
xcodebuild test -scheme IosSmoke-Package \
  -destination 'platform=iOS Simulator,name=iPhone 17'
```

Delivered `LibraryIdentifier`s: `macos-arm64`, `ios-arm64`, `ios-arm64_x86_64-simulator` (arm64 + x86_64).

## See also

- [`connect-session-core-ffi-boundary.md`](../architecture-patterns/connect-session-core-ffi-boundary.md) — the pure session-core extraction this FFI surface exposes; binding-pipeline verification discipline.
- [`connect-publish-staging.md`](connect-publish-staging.md) — SPM git-channel packaging for the Swift binding (repo tags + `.package(url:from:)`).
- [`connect-uniffi-bindgen-fork.md`](../architecture-patterns/connect-uniffi-bindgen-fork.md) — the bindgen tooling behind the generated Swift sources.
- [`connect-binding-channels.md`](../../specs/connect-binding-channels.md) — canonical Swift xcframework coverage wording (three slices covering four target triples).
