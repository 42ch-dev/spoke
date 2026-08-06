# 42ch.Spoke.Connect

SPOKE Connect **session-core** bindings for .NET (generated uniffi C# + native `spoke_connect` / `libspoke_connect`), with the additive remote-adapter FFI surface.

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

Session core: `peer_id`, hello sign/verify, allowlist, nonce, sequence, correlation, dispatch. The package also carries the additive remote-adapter FFI surface — `RemoteAdapterFFI` (single peer), `MultiPeerRouterFFI` (multi-peer routing), the callback `Transport` interface, and the in-memory loopback helpers. The native library is built from the production feature pair `ffi,remote-adapter`: regenerated bindings reference `remote-adapter` symbols at load time, so the release build always carries both features. Version locksteps with the spoke monorepo SemVer / git tag `vX.Y.Z`.

## Transport contract

The callback `Transport` is a message-oriented interface: one envelope per call, blocking receive, idempotent close.

| Method | Behavior |
|--------|----------|
| `Send(envelope)` | Accepts exactly one connect envelope's bytes per call |
| `Recv()` | Blocks until the next inbound envelope arrives or the connection closes; returns exactly one envelope per call |
| `Close()` | Releases transport resources; idempotent — closing either end of a connection fails the peer's pending `Recv` like a real connection drop |

The surface bounds messages at one envelope per call; byte-stream carriers apply length-prefix (or equivalent) delimiting before handing envelopes to the adapter.

## MultiPeerRouterFFI

`NewMultiPeerRouterFfi()` returns the router as a synchronous object over the cdylib-owned tokio runtime: a peer registry (`RegisterPeer` accepts an established `RemoteAdapterFfi` and returns its `peer_id`; `UnregisterPeer`; `ListPeers`), the `BaselinePorts` six families routed per call to exactly one capable peer, and the two `HostManifestPort` aggregation views — the composed `GetHostCapabilityManifest` and the per-peer `ListPeerHostCapabilityManifests`. Selection matches each registered peer's cached `HostCapabilityManifest`: required capability, exact namespace, soft role preference, and a deterministic lowest-`peer_id` tie-break.
