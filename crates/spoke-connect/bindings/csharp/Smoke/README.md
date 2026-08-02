# spoke-connect C# smoke

The C# binding of the `spoke-connect` sync-core FFI facade, generated with a
vendored fork of `uniffi-bindgen-cs` retargeted to the repo's uniffi **0.32**
pin (see [`bindgen/README.md`](../bindgen/README.md)). The smoke is a net8.0
console project that loads the generated binding + the `ffi`-built cdylib and
asserts **golden parity** with the Rust vectors: `peer_id`
(`12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf`), hello signature,
verify, and protocol version.

## What's here

| Path | Contents |
|------|----------|
| `../generated/spoke_connect.cs` | Generated binding (8 functions + 3 objects + 2 error enums; see inventory in `bindgen/README.md`) |
| `Smoke.csproj` | net8.0 console project — `AllowUnsafeBlocks`, compiles `..\generated\**\*.cs`, copies the cdylib to the output dir |
| `Program.cs` | Runs the golden-parity checks, prints PASS lines, exits 0 only on full pass |
| `tests/GoldenParity.cs` | Golden constants + assertions (peer_id, signature, verify, tamper rejection, protocol version) |

## Regenerate → build → run

The generate step needs the **vendored fork bindgen** binary (build recipe:
[`bindgen/README.md`](../bindgen/README.md) — clone the pinned upstream tag,
apply `uniffi-bindgen-cs-0.32.patch`, `cargo +nightly build -p
uniffi-bindgen-cs`). All commands run from the **repository root** with the
local nightly convention.

```bash
# 0. Build the cdylib (ffi feature — non-default; a plain `cargo build`
#    replaces it with a default-features stub, so rebuild with `ffi` if the
#    smoke fails with missing symbols).
cargo +nightly build -p spoke-connect --features ffi

# 1. Generate the binding (only needed when the FFI surface changes)
<fork>/target/debug/uniffi-bindgen-cs target/debug/libspoke_connect.dylib \
  --library --out-dir crates/spoke-connect/bindings/csharp/generated --no-format

# 2. Build the smoke — must be 0 warnings / 0 errors
dotnet build crates/spoke-connect/bindings/csharp/Smoke/Smoke.csproj

# 3. Run the smoke — golden parity must PASS
dotnet run --project crates/spoke-connect/bindings/csharp/Smoke/Smoke.csproj
```

Expected output:

```text
derive_peer_id: PASS        # 12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf
sign_hello signature: PASS  # golden signature bytes in signed envelope
verify_hello: PASS
tampered_hello: PASS        # rejected with CoreException.InvalidHelloSignature
protocol: 1

GOLDEN PARITY: ALL PASS
```

## Generation mechanism

`uniffi-bindgen-cs` upstream still targets uniffi 0.31, so generation uses the
vendored fork (127-line source patch: uniffi workspace deps 0.31.0 → 0.32.0
plus the `Type::Box` / `Type::Set` interface additions). The fork is
generation-only tooling — the `spoke-connect` uniffi 0.32 pin, Swift bindings,
and the Rust suite are untouched. **Drop the fork patch when upstream tags a
uniffi 0.32+ release** — stock `uniffi-bindgen-cs` then replaces the vendored
build in step 1.

## Reference

- Fork recipe + patch inventory: [`../bindgen/README.md`](../bindgen/README.md)
- FFI surface + Rust golden tests: [`../../../src/ffi.rs`](../../../src/ffi.rs)
- Golden vectors (identity-byte proof): [`tooling/connect-identity-proof/`](../../../../../tooling/connect-identity-proof/README.md)
- Decision record: [`../../../../../.mstar/specs/connect-csharp-bindgen-deferred.md`](../../../../../.mstar/specs/connect-csharp-bindgen-deferred.md)
- Swift smoke (skeleton precedent): [`../../swift/Smoke/README.md`](../../swift/Smoke/README.md)
