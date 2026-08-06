# Go bindgen — vendored fork retargeted to uniffi 0.32

The Go generation path uses `NordSecurity/uniffi-bindgen-go` **v0.7.1+v0.31.0**
(commit `0b7fb4ceef12021bd7f790cc516fa9133e001813`) retargeted to uniffi-rs
**0.32.0** (the `spoke-connect` cdylib pin). Upstream still targets uniffi 0.31
(verified 2026-08-03: latest tag == `main` HEAD, workspace pins `uniffi* =
0.31.0`), so this directory carries the minimal fork delta as a patch plus the
rebuild recipe.

## Why a fork (not stock)

Stock `uniffi-bindgen-go` 0.31 cannot read the uniffi 0.32 cdylib metadata
(`--library` mode fails with `Invalid string data: invalid utf-8 sequence of
1 bytes from index 1009` on
`_UNIFFI_META_SPOKE_CONNECT_CONSTRUCTOR_INBOUNDSEQUENCE_NEW`). A positive
control (crate-local uniffi 0.32 `uniffi-bindgen` generates Swift from the
same cdylib) confirms the metadata is well-formed; the failure is
reader/version-specific.

The fork restores the locked `--library` CLI form against the 0.32 cdylib.

## Patch contents

| File | Change |
|------|--------|
| `Cargo.toml` | workspace uniffi deps 0.31.0 → 0.32.0 (7 crates) |
| `bindgen/src/lib.rs` | migrate CLI to uniffi 0.32 `library_mode::generate_bindings` API |
| `bindgen/src/gen_go/compounds.rs` | `+SetCodeType` (`map[T]struct{}`) |
| `bindgen/src/gen_go/mod.rs` | `+Type::Box` (transparent) / `+Type::Set` match arms |
| `bindgen/templates/SetTemplate.go` | new — set RustBuffer converter |
| `bindgen/templates/Types.go` | `+Set` include arm; `+Box` exhaustiveness arm |
| `bindgen/src/gen_go/filters.rs` | `free_fn_name` — prefixes `New` when the return type is an object whose class name equals that name (e.g. `loopback_transport_pair` → `NewLoopbackTransportPair`) |
| `bindgen/templates/TopLevelFunctionTemplate.go` | use `free_fn_name` for top-level exports |

The delta is the uniffi 0.32 interface additions (`Type::Box`, `Type::Set`) plus
the bindgen-loader API migration required by uniffi 0.32. Upstream retarget:
when a `uniffi-bindgen-go` tag targets 0.32+, drop this patch and use stock.

## Build recipe (macOS arm64, repo nightly convention)

Each step's shell context is explicit — the clone and the repo root are
separate directories: the patch applies **inside the clone**, and the
generate step runs from the **repo root**:

```bash
# 1. Clone upstream at the exact pinned commit — from the repo root.
rm -rf uniffi-bindgen-go
git clone https://github.com/NordSecurity/uniffi-bindgen-go
cd uniffi-bindgen-go
git checkout 0b7fb4ceef12021bd7f790cc516fa9133e001813   # v0.7.1+v0.31.0

# 2. Apply the fork delta — INSIDE the clone.
git apply ../crates/spoke-connect/bindings/go/bindgen/uniffi-bindgen-go-0.32.patch

# 2b. Copy the fork's resolved lockfile — INSIDE the clone.
cp ../crates/spoke-connect/bindings/go/bindgen/uniffi-bindgen-go-0.32.Cargo.lock Cargo.lock

# 3. Build the bindgen binary (nightly per root AGENTS.md) — still inside the clone
cargo +nightly build --locked -p uniffi-bindgen-go
# binary: target/debug/uniffi-bindgen-go (inside the clone)

# 4. Generate against the 0.32 cdylib — from the REPO ROOT
cd ..   # back to the repo root
cargo +nightly build -p spoke-connect --features ffi,remote-adapter
./uniffi-bindgen-go/target/debug/uniffi-bindgen-go \
  target/debug/libspoke_connect.dylib --library \
  --out-dir crates/spoke-connect/bindings/go/generated --no-format

# 5. Golden-parity smoke — from the repo root (see ../README.md)
CGO_ENABLED=1 go test -v ./crates/spoke-connect/bindings/go/Smoke/
```

`cargo install --git` from a GitHub fork of this delta is an equivalent
distribution once a 42ch-dev fork exists; the patch form above is
self-contained and works without publishing anything.

## Isolation guarantees

- Only the Go generation toolchain is forked. The `spoke-connect` uniffi
  0.32 pin, Swift/C# bindings, and the Rust suite are untouched.
- No dual-pin: a single uniffi 0.32 cdylib serves every Path B language on
  the pin.
