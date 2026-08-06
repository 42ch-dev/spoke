# Kotlin bindgen — post-generate patch

Stock first-party `uniffi-bindgen --language kotlin` (uniffi **0.32.0**) is the generator. The Rust FFI surface keeps error payload fields named `message` (published C# / other bindings depend on the wire shape).

uniffi 0.32 Kotlin codegen emits both a payload property `` `message` `` and `override val message` on `CoreException` subclasses — that does not compile in Kotlin. **Do not rename Rust FFI fields.** Apply this patch after every generate.

## Recipe

From the **repository root** (local nightly convention: `cargo +nightly …`):

```bash
# 1. Build the ffi cdylib
cargo +nightly build -p spoke-connect --features ffi,remote-adapter --release

# 2. Generate (first-party, stock)
cargo +nightly run -p spoke-connect --features ffi,bindgen-cli --bin uniffi-bindgen -- \
  generate --library target/release/libspoke_connect.dylib \
  --language kotlin \
  --out-dir crates/spoke-connect/bindings/kotlin/generated \
  --no-format

# 3. Patch Kotlin payload field names
./crates/spoke-connect/bindings/kotlin/bindgen/patch-kotlin-core-error-fields.sh

# 4. Stage host native for smoke (macOS arm64 example)
mkdir -p crates/spoke-connect/bindings/kotlin/native/darwin-aarch64
cp target/release/libspoke_connect.dylib crates/spoke-connect/bindings/kotlin/native/darwin-aarch64/
install_name_tool -id @rpath/libspoke_connect.dylib \
  crates/spoke-connect/bindings/kotlin/native/darwin-aarch64/libspoke_connect.dylib

# 5. Golden-parity smoke
cd crates/spoke-connect/bindings/kotlin && gradle test
```

## What the patch changes

On `CoreException.InvalidNonce`, `Crypto`, `Jcs`, and `TokenInvalid` and on
`FfiException.Dial` / `Rejected` payload fields:

| Before (stock generate) | After (committed) |
|-------------------------|-------------------|
| `` val `message`: String `` | `` val `detail`: String `` |
| `` override val message get() = "message=${ `message` }" `` | `` … "detail=${ `detail` }" `` |
| `` value.`message` `` in FfiConverter write/allocationSize | `` value.`detail` `` |

On `LoopbackTransport`, `LoopbackSmokeHost`, and `RemoteAdapterFfi`: removes the
Disposable `close()` that collides with the domain `close()` export and keeps a
single synchronized domain `close()`.

FFI wire encoding is unchanged — only the Kotlin property name differs from the Rust field label.
