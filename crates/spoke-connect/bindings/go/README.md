# spoke-connect Go binding

Go module path: **`github.com/42ch-dev/spoke`** (root `go.mod`). Import the binding package:

```go
import spokeconnect "github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go"
```

Integrators pin at spoke lockstep tag `vX.Y.Z`:

```bash
go get github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go@vX.Y.Z
```

Packaging contract: [connect-binding-channels.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-binding-channels.md) §3.2.

## Build features

| Artifact | Cargo features | Notes |
|----------|----------------|-------|
| Committed `native/` + `generated/` | `ffi,remote-adapter` | Production surface: `RemoteAdapterFFI`, `MultiPeerRouterFFI`, `Transport`, `loopbackTransportPair`, the tool faces (`InvokeTool`, `RegisterToolHandler`, `ToolHandler`, `NewConnectResponderFfi` / `ConnectResponderFfi`), the optional-port dialer ops (`Project` / `Compute` / `ListForkTimelineEvents` on `RemoteAdapterFfi`), and the responder ports face (optional `ports:` on `NewConnectResponderFfi` + the `PortsHandler` callback) — **no** `startLoopbackSmokeHost` |
| Local smoke cdylib + smoke Go bindings | `ffi,remote-adapter,ffi-smoke-host` | Adds loopback smoke host FFI for the RemoteAdapter loopback section |

`ffi-smoke-host` is non-default and is **not** implied by `remote-adapter` or `ffi`.
The tool-faces loopback test (`Smoke/tool_loopback_ffi_test.go`,
`TestToolLoopbackFfiPair`) runs in the **default** `go test` suite against the
committed production binding + native — no `-tags smokehost` needed. The
optional smoke-host put/get suite (`Smoke/loopback_remote_adapter_test.go`)
requires `-tags smokehost`; full procedure below.

## RemoteAdapter FFI surface

With `remote-adapter` enabled, the binding ships the additive remote-adapter surface: `RemoteAdapterFFI` (single peer), `MultiPeerRouterFFI` (multi-peer routing), the callback `Transport` interface, the in-memory loopback helpers, and the tool faces — `InvokeTool` on the adapter, router, and responder, `RegisterToolHandler` on the adapter and responder, the `ToolHandler` callback, and `NewConnectResponderFfi` / `ConnectResponderFfi` for the accept side. The optional-port dialer ops (`Project` / `Compute` / `ListForkTimelineEvents` on `RemoteAdapterFfi`) and the responder ports face (`PortsHandler` + the optional `ports:` constructor parameter) ride the same session invoke path.

### Transport contract

The callback `Transport` is a message-oriented interface: one envelope per call, blocking receive, idempotent close.

| Method | Behavior |
|--------|----------|
| `Send(envelope)` | Accepts exactly one connect envelope's bytes per call |
| `Recv()` | Blocks until the next inbound envelope arrives or the connection closes; returns exactly one envelope per call |
| `Close()` | Releases transport resources; idempotent — closing either end of a connection fails the peer's pending `Recv` like a real connection drop |

The surface bounds messages at one envelope per call; byte-stream carriers apply length-prefix (or equivalent) delimiting before handing envelopes to the adapter.

### MultiPeerRouterFFI

`NewMultiPeerRouterFfi()` returns the router as a synchronous object over the same runtime: a peer registry (`RegisterPeer(adapter)` accepts an established `RemoteAdapterFfi` and returns its `peer_id`; `UnregisterPeer(peerId)`; `ListPeers()`), the `BaselinePorts` six families routed per call to exactly one capable peer, and the two `HostManifestPort` aggregation views — the composed `GetHostCapabilityManifest()` and the per-peer `ListPeerHostCapabilityManifests()`. Selection matches each registered peer's cached `HostCapabilityManifest`: required capability, exact namespace, soft role preference, and a deterministic lowest-`peer_id` tie-break.

### Tool faces

- `InvokeTool(capabilityId, argumentsJson)` — invoke a tool on the peer (dialer → responder, responder → dialer, or router → capable peer); returns the tool's `result` as a JSON string. A non-`tools.` id or malformed arguments rejects `INVALID_INPUT` with zero wire traffic; a dispatch deny rejects `CAPABILITY_PORT_MISSING` with the peer's preserved `wire_code`.
- `RegisterToolHandler(capabilityId, handler)` — serve reverse invokes through a foreign `ToolHandler`; last-wins per id, never mutates the manifest. The callback's `Handle(argumentsJson)` returns the result JSON; a returned `FfiError.Rejected` passes through verbatim as an application reject, any other outcome is contained to `INTERNAL_ERROR` and the session survives.
- `NewConnectResponderFfi(...)` / `ConnectResponderFfi` — the accept side: wrap a connected (host-accepted) callback `Transport`. The constructor returns immediately in `Handshaking` — poll `State()` (bounded) to `Established` before invoking; a handshake failure surfaces as `State() == "Closed"` (never a thrown constructor error), config-validation failures return `FfiErrorDial` with `Kind == "config"`.
- Handlers run on the FFI blocking pool — do not synchronously call back into the FFI faces from `Handle`; hand off asynchronously in the host instead.

### Responder ports serving face (`PortsHandler`)

The optional `ports:` constructor parameter (between `peerKeys` and `invokeTimeoutMs`) serves every declared `port.*` family through a foreign `PortsHandler` callback: the nine baseline serve ops (`GetKnowledgeEntry` / `PutKnowledgeEntry` / `GetRelation` / `PutRelation` / `ListKnowledgeEntries` / `ListTimelineEvents` / `PutFindings` / `ListRules` / `ListPeerHostCapabilityManifests`) plus the three optional ops (`Project` / `Compute` / `ListForkTimelineEvents`) — every method returns the op's result as a JSON string. Passing nothing keeps the documented absent-ports behavior: the constructor is still valid and every `port.*` op answers the default deny branch (`CAPABILITY_PORT_MISSING` with the peer's preserved `op_unsupported` wire code). Optional ops are capability-gated like the baseline rows: a session whose negotiated capabilities lack `l2-computable` / `l5-fork` denies at the responder's dispatch gate with the same deny row.

Callback outcomes map strictly: `FfiErrorRejected` passes through verbatim as an application reject (kind re-hung onto details); a foreign error, `FfiErrorDial`, or panic is contained to `INTERNAL_ERROR` with `details: None` and the session survives. The ports callbacks behave like tool handlers — demand-driven, one blocking-pool thread per in-flight callback — so the same rules apply: never call back into the FFI surface from inside a ports callback (hand off asynchronously in the host instead), and size the host accordingly (one blocking-pool thread per transport end; ~256 full-duplex sessions at the tokio default cap). The dialer optional ops (`Project` / `Compute` / `ListForkTimelineEvents` on `RemoteAdapterFfi`) reject malformed JSON locally with `INVALID_INPUT` and zero wire traffic.

## Layout

| Path | Contents |
|------|----------|
| `spokeconnect.go` | Integrator-facing re-export of the generated FFI surface |
| `generated/spoke_connect/` | Generated Go + C header + cgo link shims (maintainer-internal; regenerate when FFI surface changes) |
| `native/<goos>_<goarch>/` | Committed cdylib per platform (`libspoke_connect.dylib` / `.so` / `.dll`) |
| `bindgen/` | Vendored `uniffi-bindgen-go` fork retargeted to uniffi 0.32 |
| `Smoke/` | Golden-parity + tool-faces loopback smoke (default `go test`; smoke-host put/get via `-tags smokehost`) |

cgo selects `native/<goos>_<goarch>/` via per-platform `#cgo LDFLAGS` in `generated/spoke_connect/cgo_link*.go`. Consumers need `CGO_ENABLED=1` and a C toolchain; never a Rust toolchain.

## Committed natives

| RID | Status |
|-----|--------|
| `darwin_arm64` | **Committed** — maintainer-built (release) |
| `darwin_amd64` | **Committed** — cross-built from macOS host (`--target x86_64-apple-darwin`) |
| `linux_amd64` | **Deferred** — host cross-link failed locally; stage from `release.yml` `build-connect-ffi` (`linux-x64`) artifact |
| `windows_amd64` | **Deferred** — requires Windows or mingw cross toolchain; stage from `build-connect-ffi` (`win-x64`) artifact |

Until `linux_amd64` and `windows_amd64` natives are committed, Go consumers on those platforms must copy the matching shared library into `native/<goos>_<goarch>/` (same basename as the contract) or build fails at link time.

## Maintainer: regenerate → stage natives → smoke

Commands from the **repository root** (local nightly convention for cargo: `cargo +nightly …`).

```bash
# 1. Build the ffi cdylib (must run before generate — default build replaces the stub)
cargo +nightly build -p spoke-connect --features ffi,remote-adapter --release

# 2. Generate bindings (vendored fork — full recipe: bindgen/README.md)
#    After building uniffi-bindgen-go from bindgen/ patch:
./uniffi-bindgen-go/target/debug/uniffi-bindgen-go \
  target/release/libspoke_connect.dylib --library \
  --out-dir crates/spoke-connect/bindings/go/generated --no-format

# 3. Stage committed natives for the host / cross targets you can build
#    darwin_arm64 (host Apple Silicon):
cp target/release/libspoke_connect.dylib \
  crates/spoke-connect/bindings/go/native/darwin_arm64/
install_name_tool -id @rpath/libspoke_connect.dylib \
  crates/spoke-connect/bindings/go/native/darwin_arm64/libspoke_connect.dylib

#    darwin_amd64 (cross from macOS, after: rustup target add x86_64-apple-darwin --toolchain nightly):
cargo +nightly build -p spoke-connect --features ffi,remote-adapter --release --target x86_64-apple-darwin
cp target/x86_64-apple-darwin/release/libspoke_connect.dylib \
  crates/spoke-connect/bindings/go/native/darwin_amd64/
install_name_tool -id @rpath/libspoke_connect.dylib \
  crates/spoke-connect/bindings/go/native/darwin_amd64/libspoke_connect.dylib

#    linux_amd64 / windows_amd64: copy from release.yml build-connect-ffi artifacts
#    into native/linux_amd64/libspoke_connect.so and native/windows_amd64/spoke_connect.dll

# 4. Golden-parity + tool-faces smoke (default suite)
CGO_ENABLED=1 go test -v ./crates/spoke-connect/bindings/go/Smoke/
```

Expected: all seven tests PASS — the five golden tests (`derive_peer_id`,
hello signature, verify, tamper rejection, `protocol_version == 1`),
`TestToolLoopbackFfiPair` (tool faces over the loopback pair, D15/D16: dialer
`invoke_tool` served by a responder foreign `ToolHandler`, responder reverse
invoke served by a dialer-side handler, unregistered-tool `op_unsupported`
deny, handler-thrown reject passthrough, the error rows — an unknown reject code downgraded to `INTERNAL_ERROR`, a foreign (non-Rejected) fault contained to `INTERNAL_ERROR` with the serve loop surviving — and post-close `Closed` state), and `TestPortsLoopbackFfiPair` (optional-port dialer ops + responder ports face over the loopback pair, D16: baseline + optional serving round-trips through a foreign `PortsHandler`, application-reject passthrough, the error rows — capability-gate deny, absent-ports fail-closed deny, foreign-fault containment with serve-loop survival — and malformed-JSON pre-validation). The
tool and ports tests run on the **committed production binding + native** — every face
is on `ffi,remote-adapter`; no `-tags smokehost`.

## Generation mechanism

`uniffi-bindgen-go` upstream still targets uniffi 0.31; generation uses the vendored fork under `bindgen/`. Drop the fork when upstream tags uniffi 0.32+ and stock `--library` passes against the current cdylib.


## RemoteAdapter loopback smoke (optional)

> Fixture note: the loopback seeds are exactly 32 bytes and derive the
> fixture's golden pubkey / peer_id via the in-crate oracle
> (`crates/spoke-connect/src/test_support/mod.rs` → `loopback_oracle`,
> `tests/common/loopback_oracle_impl.rs`). `seed_host_hex` was corrected
> 33 → 32 bytes and `pubkey_client_hex` added for the D16 tool-pair smokes
> (shared fixture `Smoke/fixtures/loopback-smoke.json`).

`Smoke/loopback_remote_adapter_test.go` (smoke-host put/get round-trip) is
gated with `-tags smokehost` and requires bindings regenerated from a smoke
cdylib (`ffi-smoke-host`) — distinct from `Smoke/tool_loopback_ffi_test.go`,
which runs in the default suite on the committed production binding:

```bash
cargo +nightly build -p spoke-connect --features ffi,remote-adapter,ffi-smoke-host
./uniffi-bindgen-go/target/debug/uniffi-bindgen-go   target/debug/libspoke_connect.dylib --library   --out-dir crates/spoke-connect/bindings/go/generated --no-format
cp target/debug/libspoke_connect.dylib crates/spoke-connect/bindings/go/native/darwin_arm64/
CGO_ENABLED=1 go test -tags smokehost -v ./crates/spoke-connect/bindings/go/Smoke/
```

Restore production `generated/` (from `ffi,remote-adapter` only) before landing.
## Reference

- Fork recipe: [`bindgen/README.md`](bindgen/README.md)
- FFI surface: [`../../src/ffi.rs`](../../src/ffi.rs)
- C# parallel (NuGet): [`../csharp/`](../csharp/)
