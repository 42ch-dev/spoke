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
| Committed `native/` + `generated/` | `ffi,remote-adapter` | Production surface: `RemoteAdapterFFI`, `MultiPeerRouterFFI`, `Transport`, `loopbackTransportPair` — **no** `startLoopbackSmokeHost` |
| Local smoke cdylib + smoke Go bindings | `ffi,remote-adapter,ffi-smoke-host` | Adds loopback smoke host FFI for the RemoteAdapter loopback section |

`ffi-smoke-host` is non-default and is **not** implied by `remote-adapter` or `ffi`.
Full loopback smoke procedure: [`Smoke/loopback_remote_adapter_test.go`](Smoke/loopback_remote_adapter_test.go) (`-tags smokehost`).

## RemoteAdapter FFI surface

With `remote-adapter` enabled, the binding ships the additive remote-adapter surface: `RemoteAdapterFFI` (single peer), `MultiPeerRouterFFI` (multi-peer routing), the callback `Transport` interface, and the in-memory loopback helpers.

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

## Layout

| Path | Contents |
|------|----------|
| `spokeconnect.go` | Integrator-facing re-export of the generated FFI surface |
| `generated/spoke_connect/` | Generated Go + C header + cgo link shims (maintainer-internal; regenerate when FFI surface changes) |
| `native/<goos>_<goarch>/` | Committed cdylib per platform (`libspoke_connect.dylib` / `.so` / `.dll`) |
| `bindgen/` | Vendored `uniffi-bindgen-go` fork retargeted to uniffi 0.32 |
| `Smoke/` | Golden-parity smoke (`go test`) |

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

# 4. Golden-parity smoke
CGO_ENABLED=1 go test -v ./crates/spoke-connect/bindings/go/Smoke/
```

Expected: all five golden tests PASS (`derive_peer_id`, hello signature, verify, tamper rejection, `protocol_version == 1`).

## Generation mechanism

`uniffi-bindgen-go` upstream still targets uniffi 0.31; generation uses the vendored fork under `bindgen/`. Drop the fork when upstream tags uniffi 0.32+ and stock `--library` passes against the current cdylib.


## RemoteAdapter loopback smoke (optional)

`Smoke/loopback_remote_adapter_test.go` is gated with `-tags smokehost` and
requires bindings regenerated from a smoke cdylib (`ffi-smoke-host`):

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
