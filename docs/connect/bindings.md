---
title: Native bindings (Path B)
---

# Native bindings (Path B)

Path B embeds a **shared session core** into host languages through FFI: the pure, synchronous session rules (`peer_id` derivation, hello sign/verify, allowlist, nonce single-use, sequence allocation, correlation, dispatch gate) live in one core, while transport stays in each host language. The reference implementation is the `spoke-connect` crate's **Binding facade**.

## Binding facade

The crate's `src/core/` is the pure, synchronous, language-portable layer — libp2p, tokio, and I/O live in the transport layer, which converts `libp2p::PeerId` ↔ `String` at the boundary. The facade decision is recorded in the crate README's [Binding facade](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md#binding-facade) section, including the sync-vs-async boundary and the exported surface.

## C# NuGet (`42ch.Spoke.Connect`)

Integrators consume the session core via **GitHub Packages**:

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

## Swift sync-core binding

A **Swift sync-core skeleton** is landed through uniffi behind the optional `ffi` feature (`cdylib`): exported functions like `derivePeerIdFromEd25519Pubkey`, `signHelloEd25519` / `verifyHelloEd25519`, `isAllowlisted`, `checkResponseCorrelation`, `dispatchAllowed`, plus `NonceStore` / `OutboundSequence` / `InboundSequence` objects — with a macOS-local smoke asserting golden-vector parity. Async node lifecycle (start, listen, `connect`) stays Rust-side. Swift packaging on GitHub Packages follows the same registry rule when a packable project is added.

## Target matrix

C#, Go, Python, Swift, Kotlin are the target languages (priority per product direction); Swift and C# are landed — C# via a vendored `uniffi-bindgen-cs` fork retargeted to uniffi 0.32 ([decision record](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-csharp-binding.md)). TypeScript is a parallel **Path A** track (language-direct). Go / Python / Kotlin remain.

## Integrator notes

- **Core-only** — the exported surface is the session core; host languages implement their own transport adapter against the wire contract.
- Keys cross the FFI boundary as raw bytes; peer ids as strings; manifests / hellos as JSON strings.
- **C# consumers** use `PackageReference`; maintainers regenerate bindings with the vendored bindgen fork when the FFI surface changes (see the Smoke README).

## Normative references

- [spoke-connect.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect.md) §Embedding model — Path B definition and purity rules
- [connect-publish-strategy.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-publish-strategy.md) — npm/crates.io vs GitHub Packages registry split
- [crates/spoke-connect/README.md#binding-facade](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/README.md#binding-facade) — sync/async boundary, exported surface, target-language matrix
- [connect-csharp-binding.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-csharp-binding.md) — C# binding decision record
