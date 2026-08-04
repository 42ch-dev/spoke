# spoke-connect C# smoke

In-repo golden-parity smoke for the **`42ch.Spoke.Connect`** packable project (generated uniffi C# + native `spoke_connect` FFI). Integrators prefer the published NuGet from GitHub Packages; this smoke is for maintainers and CI.

## Consumer DX (preferred)

```xml
<!-- nuget.config once -->
<packageSources>
  <add key="github-42ch" value="https://nuget.pkg.github.com/42ch-dev/index.json" />
</packageSources>
```

```xml
<PackageReference Include="42ch.Spoke.Connect" Version="0.7.1" />
```

See [`PACKAGE.md`](../PACKAGE.md) and [`docs/how-to/connect-native-bindings.md`](../../../../../docs/how-to/connect-native-bindings.md).

## What's here

| Path | Contents |
|------|----------|
| `../42ch.Spoke.Connect.csproj` | Packable net8.0 library — PackageId `42ch.Spoke.Connect`, compiles `../generated/**/*.cs`, packs `runtimes/<rid>/native/*` |
| `../generated/spoke_connect.cs` | Generated binding (regenerate only when the FFI surface changes) |
| `Smoke.csproj` | net8.0 console — `ProjectReference` to the packable project |
| `Program.cs` / `tests/GoldenParity.cs` | Golden-parity checks across the exported surface |

## Maintainer: regenerate → build → run

Prefer **PackageReference** for product hosts. Regenerate only when changing the Rust FFI surface.

The generate step needs the **vendored fork bindgen** binary (build recipe:
[`bindgen/README.md`](../bindgen/README.md)). Commands from the **repository
root** (local nightly convention for cargo):

```bash
# 0. Build the cdylib (ffi feature)
cargo +nightly build -p spoke-connect --features ffi

# 1. Generate the binding (only when the FFI surface changes)
#    `--config` sets access_modifier=public for PackageReference consumers.
<fork>/target/debug/uniffi-bindgen-cs target/debug/libspoke_connect.dylib \
  --library --config crates/spoke-connect/bindings/csharp/uniffi.toml \
  --out-dir crates/spoke-connect/bindings/csharp/generated --no-format

# 2. Optional: assemble host RID for pack (or rely on cargo debug copy)
./tooling/connect/build-csharp-ffi-host.sh /tmp/spoke-ffi
./tooling/connect/assemble-csharp-runtimes.sh /tmp/spoke-ffi

# 3. Build + run smoke
dotnet build crates/spoke-connect/bindings/csharp/Smoke/Smoke.csproj
dotnet run --project crates/spoke-connect/bindings/csharp/Smoke/Smoke.csproj
```

Expected output ends with `GOLDEN PARITY: ALL PASS`.

## Generation mechanism

`uniffi-bindgen-cs` upstream still targets uniffi 0.31, so generation uses the
vendored fork. **Drop the fork when upstream tags a uniffi 0.32+ release.**
Consumers of the NuGet package never run bindgen.

## Reference

- Package readme: [`../PACKAGE.md`](../PACKAGE.md)
- Fork recipe: [`../bindgen/README.md`](../bindgen/README.md)
- FFI surface: [`../../../src/ffi.rs`](../../../src/ffi.rs)
- Publish strategy: [`.mstar/specs/connect-publish-strategy.md`](../../../../../.mstar/specs/connect-publish-strategy.md)
- Decision record: [`.mstar/specs/connect-csharp-binding.md`](../../../../../.mstar/specs/connect-csharp-binding.md)
