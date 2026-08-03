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

## Layout

| Path | Contents |
|------|----------|
| `generated/` | Committed uniffi Swift (`spoke_connect.swift`) + FFI header/modulemap for swiftc smoke |
| `xcframework/` | Committed `spoke_connectFFI.xcframework` (macOS arm64 static slice) |
| `Smoke/` | macOS swiftc golden-parity gate |

## Maintainer: regenerate → xcframework → validate

From the **repository root** (local nightly convention: `cargo +nightly …`):

```bash
./tooling/connect/build-swift-xcframework.sh
swift build
# Smoke/README.md — swiftc golden-parity smoke
```

Rebuild and commit `generated/` and `xcframework/` when the FFI surface changes. Additional slices (macOS x86_64, iOS) follow the same `xcodebuild -create-xcframework` pattern when scheduled.
