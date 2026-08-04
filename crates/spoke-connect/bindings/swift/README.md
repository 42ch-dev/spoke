# spoke-connect Swift binding

Swift Package Manager library **`SpokeConnect`**: SPOKE Connect session-core bindings (committed uniffi Swift + `spoke_connectFFI` xcframework).

Packaging contract: [connect-binding-channels.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-binding-channels.md) §3.4.

## SPM consume

At spoke lockstep tag `vX.Y.Z`:

```swift
// Package.swift
dependencies: [
    .package(url: "https://github.com/42ch-dev/spoke.git", from: "X.Y.Z"),
],
targets: [
    .target(
        name: "MyApp",
        dependencies: [
            .product(name: "SpokeConnect", package: "spoke"),
        ]
    ),
]
```

```swift
import SpokeConnect

let peerId = try derivePeerIdFromEd25519Pubkey(pubkey: goldenPubkey)
let version = protocolVersion() // 1
```

Root `Package.swift` exposes product `SpokeConnect` from package `spoke`. The `spoke_connectFFI` binary target ships the committed xcframework; generated Swift sources live under `generated/`.

**git-lfs:** the xcframework static libraries are tracked via [git-lfs](https://git-lfs.com) (`.gitattributes`). Xcode resolves them transparently; command-line `swift build` users run `git lfs install` once so the real binaries are smudged on clone.

## Layout

| Path | Contents |
|------|----------|
| `generated/` | Committed uniffi Swift (`spoke_connect.swift`) + FFI header/modulemap for swiftc smoke |
| `xcframework/` | Committed `spoke_connectFFI.xcframework` (three slices, four target triples) |
| `Smoke/` | macOS swiftc golden-parity gate |
| `IosSmoke/` | Maintainer-local SwiftPM package — iOS Simulator golden-parity test (not part of the SPM product layout) |

## xcframework coverage

The committed `spoke_connectFFI.xcframework` carries three library slices covering
macOS arm64 hosts, iOS devices, and iOS simulators on both Apple Silicon and
Intel hosts:

| LibraryIdentifier | Architectures | Platform | Built from |
|-------------------|---------------|----------|------------|
| `macos-arm64` | arm64 | macOS | `aarch64-apple-darwin` staticlib |
| `ios-arm64` | arm64 | iOS device | `aarch64-apple-ios` staticlib |
| `ios-arm64_x86_64-simulator` | arm64 + x86_64 | iOS simulator | `aarch64-apple-ios-sim` + `x86_64-apple-ios` staticlibs, `lipo`-combined |

An iOS integrator adds one SPM dependency and links on device and in both
simulator hosts without running Rust or bindgen. `xcodebuild -create-xcframework`
requires one `-library` entry per platform, so the two simulator staticlibs are
combined into the single simulator slice before assembly.

## Maintainer: regenerate → xcframework → validate

From the **repository root** (local nightly convention: `cargo +nightly …`):

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios \
  --toolchain nightly   # one-time prerequisite; the script asserts these
./tooling/connect/build-swift-xcframework.sh
swift build
# Smoke/README.md — swiftc golden-parity smoke (macOS)
# IosSmoke/README.md — xcodebuild test (iOS Simulator)
```

Rebuild and commit `generated/` and `xcframework/` when the FFI surface
changes. The script builds one release staticlib per target triple, lipo-combines
the simulator pair, assembles the three-slice xcframework with
`xcodebuild -create-xcframework`, and lints the result. The iOS smoke lives in
`IosSmoke/` and is validated with `xcodebuild test` on an iOS Simulator
destination — see `IosSmoke/README.md` for the exact invocation.
