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

Session core: `peer_id`, hello sign/verify, allowlist, nonce, sequence, correlation, dispatch. The package also carries the additive remote-adapter FFI surface — `RemoteAdapterFFI` (single peer), `MultiPeerRouterFFI` (multi-peer routing), the callback `Transport` interface, the in-memory loopback helpers, and the tool faces: `InvokeTool` on the adapter, router, and responder, `RegisterToolHandler` on the adapter and responder, the `ToolHandler` callback, and `ConnectResponderFfi` for the accept side. The optional-port dialer ops (`Project` / `Compute` / `ListForkTimelineEvents` on `RemoteAdapterFfi`) and the responder ports face (`PortsHandler` + the optional `ports:` constructor parameter) ride the same session invoke path. The native library is built from the production feature pair `ffi,remote-adapter`: regenerated bindings reference `remote-adapter` symbols at load time, so the release build always carries both features. Version locksteps with the spoke monorepo SemVer / git tag…

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

## Tool faces

- `InvokeTool(capabilityId, argumentsJson)` — invoke a tool on the peer (dialer → responder, responder → dialer, or router → capable peer); returns the tool's `result` as a JSON string. A non-`tools.` id or malformed arguments rejects `INVALID_INPUT` with zero wire traffic; a dispatch deny rejects `CAPABILITY_PORT_MISSING` with the peer's preserved `wireCode`.
- `RegisterToolHandler(capabilityId, handler)` — serve reverse invokes through a foreign `ToolHandler`; last-wins per id, never mutates the manifest. The callback's `Handle(string argumentsJson)` returns the result JSON; a thrown `FfiException.Rejected` passes through verbatim as an application reject, any other outcome is contained to `INTERNAL_ERROR` and the session survives.
- `SpokeConnectMethods.ConnectResponderFfi(...)` / `ConnectResponderFfi` — the accept side: wrap a connected (host-accepted) callback `Transport`. The constructor returns immediately in `Handshaking` — poll `State()` (bounded) to `Established` before invoking; a handshake failure surfaces as `State() == "Closed"` (never a thrown constructor error), config-validation failures throw `FfiException.Dial` with `kind == "config"`.
- Handlers run on the FFI blocking pool — do not synchronously call back into the FFI faces from `Handle`; hand off asynchronously in the host instead.

## Responder ports serving face (`PortsHandler`)

The optional `ports:` constructor parameter (between `peerKeys` and `invokeTimeoutMs`) serves every declared `port.*` family through a foreign `PortsHandler` callback: the nine baseline serve ops (`GetKnowledgeEntry` / `PutKnowledgeEntry` / `GetRelation` / `PutRelation` / `ListKnowledgeEntries` / `ListTimelineEvents` / `PutFindings` / `ListRules` / `ListPeerHostCapabilityManifests`) plus the three optional ops (`Project` / `Compute` / `ListForkTimelineEvents`) — every method returns the op's result as a JSON string. Passing nothing keeps the documented absent-ports behavior: the constructor is still valid and every `port.*` op answers the default deny branch (`CAPABILITY_PORT_MISSING` with the peer's preserved `op_unsupported` wire code). Optional ops are capability-gated like the baseline rows: a session whose negotiated capabilities lack `l2-computable` / `l5-fork` denies at the responder's dispatch gate with the same deny row.

Callback outcomes map strictly: `FfiException.Rejected` passes through verbatim as an application reject (kind re-hung onto details); a foreign exception, `Dial`, or panic is contained to `INTERNAL_ERROR` with `details: None` and the session survives. The ports callbacks behave like tool handlers — demand-driven, one blocking-pool thread per in-flight callback — so the same rules apply: never call back into the FFI surface from inside a ports callback (hand off asynchronously in the host instead), and size the host accordingly (one blocking-pool thread per transport end; ~256 full-duplex sessions at the tokio default cap). The dialer optional ops (`Project` / `Compute` / `ListForkTimelineEvents` on `RemoteAdapterFfi`) reject malformed JSON locally with `INVALID_INPUT` and zero wire traffic.
