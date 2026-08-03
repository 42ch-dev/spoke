---
title: Native bindings (Path B)
---

# Native bindings (Path B)

Path B embeds a **shared session core** into host languages through FFI: the pure, synchronous session rules (`peer_id` derivation, hello sign/verify, allowlist, nonce single-use, sequence allocation, correlation, dispatch gate) live in one core, while transport stays in each host language. The reference implementation is the `spoke-connect` crate's **Binding facade**.

## Binding facade

The crate's `src/core/` is the pure, synchronous, language-portable layer — libp2p, tokio, and I/O live in the transport layer, which converts `libp2p::PeerId` ↔ `String` at the boundary. The facade decision is recorded in the crate README's [Binding facade](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md#binding-facade) section, including the sync-vs-async boundary and the exported surface.

## Publish channels

Path B bindings ship on **four channel types** (five languages), all lockstep with spoke git tags `vX.Y.Z`:

| Channel | Languages | Integrator entry |
|---------|-----------|------------------|
| **GitHub Packages NuGet** | C# | `42ch.Spoke.Connect` |
| **GitHub Packages Maven** | Kotlin | `io.github.42ch-dev:spoke-connect` |
| **Swift Package Manager** (git + tags) | Swift | Root `Package.swift` — `.package(url:from:)` |
| **Go modules** (git + tags) | Go | `go get github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go@vX.Y.Z` |
| **PyPI** (Trusted Publishing) | Python | `pip install <registered-name>` |

Packaging coordinates and native layouts: [connect-binding-channels.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-binding-channels.md). Staging and registry auth: [connect-publish-strategy.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-publish-strategy.md).

## C# NuGet (`42ch.Spoke.Connect`)

Integrators consume the session core via **GitHub Packages NuGet**:

```xml
<!-- nuget.config (once per solution) -->
<packageSources>
  <add key="github-42ch" value="https://nuget.pkg.github.com/42ch-dev/index.json" />
</packageSources>
```

```xml
<PackageReference Include="42ch.Spoke.Connect" Version="0.7.1" />
```

```csharp
using uniffi.spoke_connect;
var peerId = SpokeConnectMethods.DerivePeerIdFromEd25519Pubkey(pubkey);
```

Native `spoke_connect` / `libspoke_connect` ships under NuGet `runtimes/<rid>/native/` (`win-x64`, `linux-x64`, `osx-arm64`). Authenticate to GitHub Packages with a PAT that has `read:packages` (or `GITHUB_TOKEN` in Actions). Package SemVer locksteps with spoke git tags `vX.Y.Z`.

## Kotlin Maven (`io.github.42ch-dev:spoke-connect`)

Integrators add the GitHub Packages Maven repository and depend on the binding artifact:

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
    implementation("io.github.42ch-dev:spoke-connect:0.7.1")
}
```

JNA loads platform natives from the published jar (`darwin-aarch64`, `linux-x86-64`, `win32-x86-64`). Version locksteps with spoke git tags `vX.Y.Z`.

## Swift (`SpokeConnect` via SPM)

Integrators add the spoke repository as an SPM dependency:

```swift
// Package.swift
dependencies: [
    .package(url: "https://github.com/42ch-dev/spoke.git", from: "0.7.1"),
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

The root `Package.swift` exposes library product `SpokeConnect` with committed generated Swift and a local `spoke_connectFFI` xcframework. Version resolves from git tags `vX.Y.Z`.

## Go (`github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go`)

Integrators pin the binding module at the spoke tag:

```bash
go get github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go@v0.7.1
```

```go
import spokeconnect "github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go"
```

The root `go.mod` (`module github.com/42ch-dev/spoke`) versions the whole repo at one tag family. cgo loads committed natives under `native/<goos>_<goarch>/`; consumers need a C toolchain.

## Python (PyPI)

Integrators install the published wheel:

```bash
pip install <registered-name>==0.7.1
```

```python
import spoke_connect
```

PyPI publishes platform wheels (`linux_x86_64`, `macosx_arm64`, `win_amd64`) via Trusted Publishing on `release.yml`. The PyPI project name matches the registered Pending publisher; see [connect-binding-channels.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-binding-channels.md) §3.3.

## Target matrix

C#, Go, Python, Swift, Kotlin are the target languages (priority per product direction). **C#** (NuGet) and **Swift** (sync-core skeleton) are landed — C# via a vendored `uniffi-bindgen-cs` fork retargeted to uniffi 0.32 ([decision record](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-csharp-binding.md)). **Go, Python, and Kotlin** follow the same feasibility gate and channel contract above. TypeScript is a parallel **Path A** track (language-direct).

## Integrator notes

- **Core-only** — the exported surface is the session core; host languages implement their own transport adapter against the wire contract.
- Keys cross the FFI boundary as raw bytes; peer ids as strings; manifests / hellos as JSON strings.
- **C# consumers** use `PackageReference`; maintainers regenerate bindings with the vendored bindgen fork when the FFI surface changes (see the Smoke README).

## Normative references

- [spoke-connect.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect.md) §Embedding model — Path B definition and purity rules
- [connect-publish-strategy.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-publish-strategy.md) — publish staging and four-channel registry split
- [connect-binding-channels.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-binding-channels.md) — per-language packaging contract (coordinates, natives, CI jobs)
- [crates/spoke-connect/README.md#binding-facade](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md#binding-facade) — sync/async boundary, exported surface, target-language matrix
- [connect-csharp-binding.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-csharp-binding.md) — C# binding decision record
