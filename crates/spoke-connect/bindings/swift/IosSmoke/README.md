# spoke-connect Swift iOS smoke (maintainer-local)

iOS-simulator golden-parity smoke for the `SpokeConnect` SPM product. Runs the
same golden checks as the macOS `Smoke/` (peer id, hello signature, protocol
version) through the committed `spoke_connectFFI.xcframework` simulator slice,
proving an iOS integrator links and executes the session core without running
Rust or bindgen.

> This package is **maintainer validation only**: it is not referenced from the
> repo-root `Package.swift` and is not part of the shipped SPM product layout.
> Do not add it to the root manifest.

The test target depends on the root `spoke` package by path and consumes the
exact `SpokeConnect` product an iOS consumer would import.

Run from this directory:

```bash
xcodebuild test -scheme IosSmoke-Package \
  -destination 'platform=iOS Simulator,name=iPhone 17' \
  -derivedDataPath /tmp/ios-smoke-dd
```

Named simulator devices vary per Xcode install; the portable fallback is
`-destination 'generic/platform=iOS Simulator'`. Requires an Xcode install with
the iOS runtime and the committed xcframework with three slices
(`macos-arm64`, `ios-arm64`, `ios-arm64_x86_64-simulator`) covering four target
triples (`../xcframework/spoke_connectFFI.xcframework`).
