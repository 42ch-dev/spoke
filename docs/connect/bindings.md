---
title: Native bindings
---

# Native bindings

Native bindings embed a **shared session core** into host languages through FFI: the pure, synchronous session rules (`peer_id` derivation, hello sign/verify, allowlist, nonce single-use, sequence allocation, correlation, dispatch gate) live in one core, while transport stays in each host language. The reference implementation is the `spoke-connect` crate's **Binding facade**.

## Binding facade

The crate's `src/core/` is the pure, synchronous, language-portable layer — libp2p, tokio, and I/O live in the transport layer, which converts `libp2p::PeerId` ↔ `String` at the boundary. The facade decision is recorded in the crate README's [Binding facade](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md#binding-facade) section, including the sync-vs-async boundary and the exported surface.

## Publish channels

Native bindings ship on **four channel types** (five languages), all lockstep with spoke git tags `vX.Y.Z`:

| Channel | Languages | Integrator entry |
|---------|-----------|------------------|
| **GitHub Packages NuGet** | C# | `42ch.Spoke.Connect` |
| **GitHub Packages Maven** | Kotlin | `dev.42ch:spoke-connect` |
| **Swift Package Manager** (git + tags) | Swift | Root `Package.swift` — `.package(url:from:)` |
| **Go modules** (git + tags) | Go | `go get github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go@vX.Y.Z` |
| **PyPI** (Trusted Publishing) | Python | `pip install spoke-connect` |

NuGet and Maven both use the **GitHub Packages** channel type (one registry family, two package ecosystems).

Packaging coordinates and native layouts: [connect-binding-channels.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-binding-channels.md). Staging and registry auth: [connect-publish-strategy.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-publish-strategy.md).

## C# NuGet (`42ch.Spoke.Connect`)

### Install

```xml
<!-- nuget.config (once per solution) -->
<packageSources>
  <add key="github-42ch" value="https://nuget.pkg.github.com/42ch-dev/index.json" />
</packageSources>
```

```xml
<PackageReference Include="42ch.Spoke.Connect" Version="X.Y.Z" />
```

Authenticate to GitHub Packages with a PAT that has `read:packages` (or `GITHUB_TOKEN` in Actions). Native `spoke_connect` / `libspoke_connect` ships under NuGet `runtimes/<rid>/native/` (`win-x64`, `linux-x64`, `osx-arm64`). Package SemVer locksteps with spoke git tags `vX.Y.Z`.

### Import & usage

```csharp
using uniffi.spoke_connect;

var peerId = SpokeConnectMethods.DerivePeerIdFromEd25519Pubkey(pubkey);
var version = SpokeConnectMethods.ProtocolVersion(); // 1
```

## Kotlin Maven (`dev.42ch:spoke-connect`)

### Install

```kotlin
// settings.gradle.kts or build.gradle.kts repository block
maven {
    url = uri("https://maven.pkg.github.com/42ch-dev/spoke")
    credentials {
        username = providers.gradleProperty("gpr.user").get()
        password = providers.gradleProperty("gpr.key").get()
    }
}
```

```kotlin
dependencies {
    implementation("dev.42ch:spoke-connect:X.Y.Z")
    // JNA is a transitive dependency of the published artifact
}
```

Set `gpr.user` and `gpr.key` in `gradle.properties` or `~/.gradle/gradle.properties` (GitHub username and a PAT with `read:packages`). Stable tags publish via `publish-maven` on [`release.yml`](https://github.com/42ch-dev/spoke/blob/main/.github/workflows/release.yml).

JNA loads platform natives from the jar per the Maven layout contract (`darwin-aarch64`, `linux-x86-64`, `win32-x86-64`). Version locksteps with spoke git tags `vX.Y.Z`. Binding README: [`bindings/kotlin/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/kotlin/README.md).

### Import & usage

```kotlin
import uniffi.spoke_connect.derivePeerIdFromEd25519Pubkey
import uniffi.spoke_connect.protocolVersion

val peerId = derivePeerIdFromEd25519Pubkey(pubkey)
val version = protocolVersion() // 1
```

## Swift (`SpokeConnect` via SPM)

### Install

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

At tag `vX.Y.Z`, SPM resolves the repo-root `Package.swift` for library product `SpokeConnect` with generated Swift and a `spoke_connectFFI` xcframework per the packaging contract. Binding README: [`bindings/swift/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/swift/README.md).

### Import & usage

```swift
import SpokeConnect

let peerId = try derivePeerIdFromEd25519Pubkey(pubkey: goldenPubkey)
let version = protocolVersion() // 1
```

## Go (`github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go`)

### Install

```bash
go get github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go@vX.Y.Z
```

At tag `vX.Y.Z`, the repo-root `go.mod` (`module github.com/42ch-dev/spoke`) versions the module; the import path is the subdirectory package `github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go` (Go treats subdirectory packages as first-class — no root re-export file is required for `go get`). cgo links shared libraries under `native/<goos>_<goarch>/` in the module tree ([packaging contract](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-binding-channels.md) §3.2). Current tags commit `darwin_arm64` and `darwin_amd64` (`libspoke_connect.dylib`) — macOS `go get` + link is complete. For Linux and Windows, before linking, place `libspoke_connect.so` or `spoke_connect.dll` at `native/linux_amd64/` or `native/windows_amd64/` in the module you build (local checkout + `replace` when the pinned tag omits that directory). Those shared libraries match the lockstep FFI natives already published for the same SemVer in NuGet `42ch.Spoke.Connect` (`runtimes/linux-x64/native/`, `runtimes/win-x64/native/`). On Windows, also place `spoke_connect.dll` beside the executable so the loader finds it at runtime. Consumers need a C toolchain and `CGO_ENABLED=1`. Binding README: [`bindings/go/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/go/README.md).

### Import & usage

```go
import spokeconnect "github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go"

peerID, err := spokeconnect.DerivePeerIdFromEd25519Pubkey(pubkey)
version := spokeconnect.ProtocolVersion() // 1
```

## Python (PyPI)

### Install

```bash
pip install spoke-connect==X.Y.Z
```

Platform wheels (`manylinux_2_35_x86_64`, `macosx_11_0_arm64`, `win_amd64`) publish to PyPI project **`spoke-connect`** via Trusted Publishing OIDC on the top-level `release.yml` workflow (`publish-pypi` job, repository `42ch-dev/spoke`). Version locksteps with spoke git tags `vX.Y.Z`. See [connect-binding-channels.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-binding-channels.md) §3.3.

### Import & usage

```python
import spoke_connect

peer_id = spoke_connect.derive_peer_id_from_ed25519_pubkey(pubkey)
version = spoke_connect.protocol_version()  # 1
```

## Target matrix

C#, Go, Python, Swift, Kotlin are the target languages (priority per product direction). **C#** (NuGet), **Swift** (SPM `SpokeConnect`), **Kotlin** (GitHub Packages Maven), **Go** (Go modules + golden-parity smoke), and **Python** (PyPI platform wheels + golden-parity smoke) are landed — C# and Go via vendored uniffi bindgen forks retargeted to uniffi 0.32 ([C# decision record](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-csharp-binding.md); Go [`bindings/go/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/go/README.md); Swift [`bindings/swift/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/swift/README.md); Kotlin [`bindings/kotlin/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/kotlin/README.md); Python [`bindings/python/README.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/python/README.md)). TypeScript is a parallel **language-native client** track.

## Integrator notes

- **Core-only** — the exported surface is the session core; host languages implement their own transport adapter against the wire contract.
- **Session-core surface** — keys cross the FFI boundary as raw bytes; peer ids as strings; manifests and hellos as JSON strings.
- **C# consumers** use `PackageReference`; maintainers regenerate bindings with the vendored bindgen fork when the FFI surface changes (see the Smoke README).

## Normative references

- [spoke-connect.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect.md) §Embedding model — native bindings definition and purity rules
- [connect-publish-strategy.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-publish-strategy.md) — publish staging and four-channel registry split
- [connect-binding-channels.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-binding-channels.md) — per-language packaging contract (coordinates, natives, CI jobs)
- [crates/spoke-connect/README.md#binding-facade](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md#binding-facade) — sync/async boundary, exported surface, target-language matrix
- [connect-csharp-binding.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-csharp-binding.md) — C# binding decision record
