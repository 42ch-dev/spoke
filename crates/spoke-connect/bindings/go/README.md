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
cargo +nightly build -p spoke-connect --features ffi --release

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
cargo +nightly build -p spoke-connect --features ffi --release --target x86_64-apple-darwin
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

## Reference

- Fork recipe: [`bindgen/README.md`](bindgen/README.md)
- FFI surface: [`../../src/ffi.rs`](../../src/ffi.rs)
- C# parallel (NuGet): [`../csharp/`](../csharp/)
