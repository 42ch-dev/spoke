---
title: Connect from native bindings
---

# Connect from native bindings

**Native bindings** embed the shared connect **session core** into host languages through FFI: the pure session rules — `peer_id` derivation, hello sign/verify, allowlist, nonce single-use, sequence allocation, correlation, dispatch gate — live in one core, while transport stays in each host language. Native bindings use registry-backed distribution for C# (GitHub Packages NuGet), Kotlin (GitHub Packages Maven), and Python (PyPI), and git-based distribution for Swift (Swift Package Manager), Go (Go modules), and C/C++ (committed headers and platform carriers). Registry packages carry the lockstep release version; git-based consumers resolve the matching repository tag `vX.Y.Z`. NuGet and Maven share the GitHub Packages registry family.

| Language | Channel | Package |
|----------|---------|---------|
| C# | GitHub Packages NuGet | `42ch.Spoke.Connect` |
| Kotlin | GitHub Packages Maven | `dev.42ch:spoke-connect` |
| Swift | Swift Package Manager (git + tags) | Product `SpokeConnect` |
| Go | Go modules (git + tags) | `github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go` |
| Python | PyPI | `spoke-connect` |
| C / C++ | git (committed headers + platform carriers) | [`spoke_connect.h` + `spoke_connect.hpp` + `native/<rid>/`](/how-to/connect-cpp-binding) |

Every binding exposes the same synchronous core surface; golden-parity smokes assert byte-identical behavior from each host side. Every native library is built from the production feature pair `ffi,remote-adapter` — regenerated bindings reference `remote-adapter` symbols (`RemoteAdapterFFI`, `MultiPeerRouterFFI`, the callback `Transport`) at load time, so the release build always carries both features.

## C# — GitHub Packages NuGet

```xml
<!-- nuget.config (once per solution) -->
<packageSources>
  <add key="github-42ch" value="https://nuget.pkg.github.com/42ch-dev/index.json" />
</packageSources>

<PackageReference Include="42ch.Spoke.Connect" Version="X.Y.Z" />
```

Authenticate to GitHub Packages with a token that has `read:packages` scope. Native `libspoke_connect` ships under NuGet `runtimes/<rid>/native/` (`win-x64`, `linux-x64`, `osx-arm64`).

```csharp
using uniffi.spoke_connect;

var peerId = SpokeConnectMethods.DerivePeerIdFromEd25519Pubkey(pubkey);
var version = SpokeConnectMethods.ProtocolVersion(); // 1
```

Package detail: [`bindings/csharp/PACKAGE.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/csharp/PACKAGE.md).

## Kotlin — GitHub Packages Maven

```kotlin
// settings.gradle.kts or build.gradle.kts repository block
maven {
    url = uri("https://maven.pkg.github.com/42ch-dev/spoke")
    credentials {
        username = providers.gradleProperty("gpr.user").get()
        password = providers.gradleProperty("gpr.key").get()
    }
}

dependencies {
    implementation("dev.42ch:spoke-connect:X.Y.Z")
    // JNA is a transitive dependency of the published artifact
}
```

Set `gpr.user` and `gpr.key` in `gradle.properties` or `~/.gradle/gradle.properties` (GitHub username and a token with `read:packages`). JNA loads the platform natives from the jar (`darwin-aarch64`, `linux-x86-64`, `win32-x86-64`).

```kotlin
import uniffi.spoke_connect.derivePeerIdFromEd25519Pubkey
import uniffi.spoke_connect.protocolVersion

val peerId = derivePeerIdFromEd25519Pubkey(pubkey)
val version = protocolVersion() // 1
```

Binding README: [`bindings/kotlin/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/kotlin/README.md).

## Swift — Swift Package Manager

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

At tag `vX.Y.Z`, SPM resolves the repo-root `Package.swift` for library product `SpokeConnect` with generated Swift and a `spoke_connectFFI` xcframework.

```swift
import SpokeConnect

let peerId = try derivePeerIdFromEd25519Pubkey(pubkey: goldenPubkey)
let version = protocolVersion() // 1
```

Binding README: [`bindings/swift/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/swift/README.md).

## Go — Go modules

```bash
go get github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go@vX.Y.Z
```

At tag `vX.Y.Z`, the repo-root `go.mod` (`module github.com/42ch-dev/spoke`) versions the module; the import path is the subdirectory package. cgo links the shared libraries under `native/<goos>_<goarch>/` in the module tree; consumers need a C toolchain and `CGO_ENABLED=1` (never a Rust toolchain).

```go
import spokeconnect "github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go"

peerID, err := spokeconnect.DerivePeerIdFromEd25519Pubkey(pubkey)
version := spokeconnect.ProtocolVersion() // 1
```

Binding README: [`bindings/go/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/go/README.md).

## Python — PyPI

```bash
pip install spoke-connect==X.Y.Z
```

Platform wheels (`manylinux`, `macosx_11_0_arm64`, `win_amd64`) publish to the PyPI project **`spoke-connect`** via Trusted Publishing on the release workflow.

```python
import spoke_connect

peer_id = spoke_connect.derive_peer_id_from_ed25519_pubkey(pubkey)
version = spoke_connect.protocol_version()  # 1
```

Binding README: [`bindings/python/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/python/README.md).

## C and C++ — git

```bash
git clone --branch vX.Y.Z --depth 1 https://github.com/42ch-dev/spoke.git
git lfs install   # once per machine
git lfs pull      # fetch the carrier libraries; a fresh clone smudges them automatically
```

- `crates/spoke-connect/bindings/cpp/include/spoke_connect.h` — C99 ABI header
- `crates/spoke-connect/bindings/cpp/include/spoke_connect.hpp` — C++17 convenience header
- `crates/spoke-connect/bindings/cpp/native/osx-arm64/libspoke_connect_capi.dylib` — macOS arm64 carrier
- `crates/spoke-connect/bindings/cpp/native/win-x64/spoke_connect_capi.dll` — Windows x64 carrier

The committed C/C++ carriers target macOS arm64 (`osx-arm64`) and Windows x64 (`win-x64`). Take `spoke_connect.h`, `spoke_connect.hpp`, and the native files for your target from the same repository tag `vX.Y.Z`. The two carrier libraries are Git LFS objects (`.gitattributes`); the headers, the Windows import library and `provenance.json` are ordinary Git objects.

The C++17 header is header-only and includes the C header. `spoke::connect` wraps the same session core in move-only RAII handles, an explicit `Result` error channel, borrowed text views over returned buffers, and host callback bridges for the transport, ports and tool surfaces. The C99 header stays the ABI contract for C hosts — status values plus the raw record and callback-table layout.

Full walkthrough — acquire, compile, open a session, make one call: [Connect from C and C++](/how-to/connect-cpp-binding).

## The shared session core

Every binding exposes the same synchronous core surface: `peer_id` derivation, hello sign/verify, allowlist, nonce store, sequence allocation, response correlation, dispatch gate, and protocol version. Keys cross the FFI boundary as raw bytes (validated to exactly 32 bytes), peer ids as strings, and manifests / hello envelopes as JSON strings — transport adapters stay in the host language against the wire contract.

The TypeScript **language-native client** ([Connect from the TypeScript client](/how-to/connect-ts-client)) implements the same session-core rules directly in TypeScript — it is the sibling path, not a binding row. The **Rust reference** (`spoke-connect` on crates.io) is the session-core reference and the binding source; see the [connect wire reference](/reference/connect) for the shared contract. The RemoteAdapter contract ships over the same FFI surface as synchronous objects (`RemoteAdapterFFI`, `MultiPeerRouterFFI`, the callback `Transport`) — see [RemoteAdapter from native bindings](/how-to/remote-adapter-native-binding). The same surface carries the tool contract: `invoke_tool` on the adapter, router, and responder; `register_tool_handler` on the adapter and responder, with the foreign `ToolHandler` callback for tool serving; and the accept-side `ConnectResponderFFI` / `connect_responder_ffi`. The same surface carries the optional port families: `RemoteAdapterFFI` exposes `project` / `compute` / `list_fork_timeline_events` (JSON in / JSON out), and the responder's `ports` argument accepts an optional foreign `PortsHandler` serving the baseline and optional `port.*` families — see [RemoteAdapter from native bindings](/how-to/remote-adapter-native-binding) and [Optional port families](/reference/connect#optional-port-families). The same `PortsHandler` callback surface carries the optional extraction service face (`PortsHandler.extract`) and the ownership gate — see [Remote extraction and the ownership gate](/how-to/remote-adapter-native-binding#_4-remote-extraction-and-the-ownership-gate).

## Next steps

- [Open your first connect session](/tutorials/first-connect-session) — the handshake flow every binding implements.
- [Use RemoteAdapter from a native binding](/how-to/remote-adapter-native-binding) — dial a `Transport`, call port methods, and route across peers over FFI.
- [Expose and invoke remote tools](/how-to/connect-remote-tools) — advertise, discover, and reverse-invoke tools from a native host.
- [Connect from C and C++](/how-to/connect-cpp-binding) — compile against the hand-written C header and link the committed carrier.
- [Connect wire reference](/reference/connect) — envelope field tables and identity binding.
