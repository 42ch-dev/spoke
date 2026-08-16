# spoke-connect Swift iOS smoke (maintainer-local)

iOS-simulator smoke for the `SpokeConnect` SPM product. Runs the
same golden checks as the macOS `Smoke/` (peer id, hello signature, protocol
version) plus the tool faces over the in-repo loopback pair (D15/D16: dialer
`invoke_tool` served by a responder foreign `ToolHandler`, responder reverse
invoke served by a dialer-side handler, unregistered-tool `op_unsupported`
deny, handler-thrown reject passthrough) through the committed
`spoke_connectFFI.xcframework` simulator slice, proving an iOS integrator links
and executes the session core without running Rust or bindgen. No smoke host is
required — every tool face is on the production xcframework.

> Fixture note: the loopback seeds (`Tests/IosSmokeTests/fixtures/loopback-smoke.json`)
> are exactly 32 bytes and derive the fixture's golden pubkey / peer_id via the
> in-crate oracle (`crates/spoke-connect/src/test_support/mod.rs` →
> `loopback_oracle`, `tests/common/loopback_oracle_impl.rs`). `seed_host_hex`
> was corrected 33 → 32 bytes and `pubkey_client_hex` added for the D16
> tool-pair smokes.

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
