# 42ch.Spoke.Connect

SPOKE Connect **session-core** bindings for .NET (generated uniffi C# + native `spoke_connect` / `libspoke_connect`).

## Install

Add the GitHub Packages NuGet source once per solution (`nuget.config`):

```xml
<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <add key="github-42ch" value="https://nuget.pkg.github.com/42ch-dev/index.json" />
  </packageSources>
</configuration>
```

Authenticate to GitHub Packages (PAT with `read:packages`, or `GITHUB_TOKEN` in Actions), then:

```xml
<PackageReference Include="42ch.Spoke.Connect" Version="0.7.1" />
```

## Usage

```csharp
using uniffi.spoke_connect;

var peerId = SpokeConnectMethods.DerivePeerIdFromEd25519Pubkey(pubkey);
```

Native libraries resolve via NuGet RID assets (`runtimes/<rid>/native/`). Transport / WebSocket stays in the host product.

## Scope

Session core only: `peer_id`, hello sign/verify, allowlist, nonce, sequence, correlation, dispatch. Version locksteps with the spoke monorepo SemVer / git tag `vX.Y.Z`.
