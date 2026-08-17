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

## Build features

| Artifact | Cargo features | Notes |
|----------|----------------|-------|
| Committed xcframework + `generated/` | `ffi,remote-adapter` | Production surface: `RemoteAdapterFFI`, `MultiPeerRouterFFI`, `Transport`, `loopbackTransportPair`, the tool faces (`invokeTool`, `registerToolHandler`, `ToolHandler`, `connectResponderFfi` / `ConnectResponderFfi`) — **no** `startLoopbackSmokeHost` |
| Local smoke cdylib + smoke Swift bindings | `ffi,remote-adapter,ffi-smoke-host` | Adds loopback smoke host FFI for the RemoteAdapter loopback section |

`ffi-smoke-host` is non-default and is **not** implied by `remote-adapter` or `ffi`.
Full loopback smoke procedure: [`Smoke/README.md`](Smoke/README.md).

## RemoteAdapter FFI surface

With `remote-adapter` enabled, the binding ships the additive remote-adapter surface: `RemoteAdapterFFI` (single peer), `MultiPeerRouterFFI` (multi-peer routing), the callback `Transport` protocol, the in-memory loopback helpers, and the tool faces — `invokeTool` on the adapter, router, and responder, `registerToolHandler` on the adapter and responder, the `ToolHandler` callback protocol, and `connectResponderFfi` / `ConnectResponderFfi` for the accept side.

### Transport contract

The callback `Transport` is a message-oriented interface: one envelope per call, blocking receive, idempotent close.

| Method | Behavior |
|--------|----------|
| `send(envelope:)` | Accepts exactly one connect envelope's bytes per call |
| `recv()` | Blocks until the next inbound envelope arrives or the connection closes; returns exactly one envelope per call |
| `close()` | Releases transport resources; idempotent — closing either end of a connection fails the peer's pending `recv` like a real connection drop |

The surface bounds messages at one envelope per call; byte-stream carriers apply length-prefix (or equivalent) delimiting before handing envelopes to the adapter.

### MultiPeerRouterFFI

`newMultiPeerRouterFfi()` returns the router as a synchronous object over the same runtime: a peer registry (`registerPeer(adapter:)` accepts an established `RemoteAdapterFfi` and returns its `peer_id`; `unregisterPeer(peerId:)`; `listPeers()`), the `BaselinePorts` six families routed per call to exactly one capable peer, and the two `HostManifestPort` aggregation views — the composed `getHostCapabilityManifest()` and the per-peer `listPeerHostCapabilityManifests()`. Selection matches each registered peer's cached `HostCapabilityManifest`: required capability, exact namespace, soft role preference, and a deterministic lowest-`peer_id` tie-break.

### Tool faces

- `invokeTool(capabilityId:argumentsJson:)` — invoke a tool on the peer (dialer → responder, responder → dialer, or router → capable peer); returns the tool's `result` as a JSON string. A non-`tools.` id or malformed arguments throws `FfiError.Rejected` with `code == "INVALID_INPUT"` and zero wire traffic; a dispatch deny throws `code == "CAPABILITY_PORT_MISSING"` with the peer's preserved `wireCode`.
- `registerToolHandler(capabilityId:handler:)` — serve reverse invokes through a foreign `ToolHandler`; last-wins per id, never mutates the manifest. The callback's `handle(argumentsJson:)` returns the result JSON; a thrown `FfiError.Rejected` passes through verbatim as an application reject, any other outcome is contained to `INTERNAL_ERROR` and the session survives.
- `connectResponderFfi(transport:seed:manifestJson:allowlist:peerKeys:invokeTimeoutMs:)` / `ConnectResponderFfi` — the accept side: wrap a connected (host-accepted) callback `Transport`. The constructor returns immediately in `Handshaking` — poll `state()` (bounded) to `Established` before invoking; a handshake failure surfaces as `state() == "Closed"` (never a thrown constructor error), config-validation failures throw `FfiError.Dial` with `kind == "config"`.
- Handlers run on the FFI blocking pool — do not synchronously call back into the FFI faces from `handle`; hand off asynchronously in the host instead.

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
