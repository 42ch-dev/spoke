# C# bindgen — vendored fork retargeted to uniffi 0.32

The C# generation path uses `NordSecurity/uniffi-bindgen-cs` **v0.11.0+v0.31.0**
retargeted to uniffi-rs **0.32.0** (the `spoke-connect` cdylib pin). Upstream
still targets uniffi 0.31 (verified 2026-08-03: latest tag == `main` HEAD
`e10ce410eb3a10cc19c7928b93ea8d84e038c034`, workspace pins `uniffi* = 0.31.0`),
so this directory carries the minimal fork delta as a patch plus the rebuild
recipe.

## Why a fork (not stock)

Stock `uniffi-bindgen-cs` 0.31 cannot read the uniffi 0.32 cdylib metadata
(`--library` mode fails with `Invalid string data: invalid utf-8 sequence of
1 bytes from index 1009`), and the UDL-mode fallback passes generation but
fails the runtime checksum gate on all 14 symbols. See
`.mstar/specs/connect-csharp-bindgen-deferred.md` for the full T1 record.

The fork restores the locked `--library` CLI form against the 0.32 cdylib;
the generated bindings load and pass the checksum gate at runtime (golden
parity verified).

## Patch contents (127 lines, 6 files)

| File | Change |
|------|--------|
| `Cargo.toml` | workspace uniffi deps 0.31.0 → 0.32.0 (7 crates) |
| `bindgen/src/gen_cs/compounds.rs` | `+SetCodeType` (`HashSet<{}>`) |
| `bindgen/src/gen_cs/mod.rs` | `+Type::Box` (transparent) / `+Type::Set` match arms |
| `bindgen/templates/SetTemplate.cs` | new — HashSet RustBuffer converter |
| `bindgen/templates/Types.cs` | `+Set` include arm; `+Box` exhaustiveness arm |

The delta is exactly the uniffi 0.32 interface additions (`Type::Box`,
`Type::Set`); no bindgen behavior beyond those variants changed. Upstream
retarget: when a `uniffi-bindgen-cs` tag targets 0.32+, drop this patch and
use stock.

## Build recipe (macOS arm64, repo nightly convention)

```bash
# 1. Clone upstream at the exact pinned commit
git clone https://github.com/NordSecurity/uniffi-bindgen-cs
cd uniffi-bindgen-cs
git checkout e10ce410eb3a10cc19c7928b93ea8d84e038c034   # v0.11.0+v0.31.0

# 2. Apply the fork delta (from repo root)
git apply crates/spoke-connect/bindings/csharp/bindgen/uniffi-bindgen-cs-0.32.patch

# 3. Build the bindgen binary (nightly per root AGENTS.md)
cargo +nightly build -p uniffi-bindgen-cs
# binary: target/debug/uniffi-bindgen-cs

# 4. Generate against the 0.32 cdylib
cargo +nightly build -p spoke-connect --features ffi
./uniffi-bindgen-cs/target/debug/uniffi-bindgen-cs \
  target/debug/libspoke_connect.dylib --library \
  --out-dir crates/spoke-connect/bindings/csharp/generated --no-format

# 5. Build + run the net8.0 smoke
dotnet build crates/spoke-connect/bindings/csharp/Smoke/Smoke.csproj
dotnet run --project crates/spoke-connect/bindings/csharp/Smoke/Smoke.csproj
```

`cargo install --git` from a GitHub fork of this delta is an equivalent
distribution once a 42ch-dev fork exists; the patch form above is
self-contained and works without publishing anything.

## Isolation guarantees

- Only the C# generation toolchain is forked. The `spoke-connect` uniffi
  0.32 pin, Swift bindings, and the Rust suite are untouched (verified:
  `cargo +nightly test -p spoke-connect --features ffi` green after this
  path was proven).
- No dual-pin: a single uniffi 0.32 cdylib serves Swift and C#.
