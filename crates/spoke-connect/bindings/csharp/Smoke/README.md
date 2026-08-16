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

## Build features

| Artifact | Cargo features | Notes |
|----------|----------------|-------|
| Committed `generated/spoke_connect.cs` + packed natives | `ffi,remote-adapter` | Production surface: `RemoteAdapterFFI`, `Transport`, `loopbackTransportPair` — **no** `startLoopbackSmokeHost` |
| Local smoke cdylib + smoke C# bindings | `ffi,remote-adapter,ffi-smoke-host` | Adds loopback smoke host FFI for the RemoteAdapter loopback section (`-p:SmokeHost=true`) |

`ffi-smoke-host` is non-default and is **not** implied by `remote-adapter` or `ffi`.

> Fixture note: the loopback seeds are exactly 32 bytes and derive the
> fixture's golden pubkey / peer_id via the in-crate oracle
> (`crates/spoke-connect/src/test_support/mod.rs` → `loopback_oracle`,
> `tests/common/loopback_oracle_impl.rs`). `seed_host_hex` was corrected
> 33 → 32 bytes and `pubkey_client_hex` added for the D16 tool-pair smokes.

## What's here

|| Path | Contents |
||------|----------|
|| `../42ch.Spoke.Connect.csproj` | Packable net8.0 library — PackageId `42ch.Spoke.Connect`, compiles `../generated/**/*.cs`, packs `runtimes/<rid>/native/*` |
|| `../generated/spoke_connect.cs` | Generated binding (regenerate only when the FFI surface changes) |
|| `Smoke.csproj` | net8.0 console — `ProjectReference` to the packable project |
|| `Program.cs` / `tests/GoldenParity.cs` | Golden-parity checks across the exported surface |
|| `tests/ToolLoopbackSmoke.cs` | Tool faces over the loopback pair (D15/D16) — runs in the default `dotnet run` on the committed production binding, no smoke host needed |
|| `tests/LoopbackShared.cs` | Shared loopback harness (fixture load, callback transport, asserts) |

## Tool faces over the loopback pair (default)

The tool section drives both FFI faces over a loopback pair (D15/D16): dialer
`invoke_tool` served by a responder foreign `ToolHandler`, responder reverse
invoke served by a dialer-side handler, unregistered-tool `op_unsupported`
deny, and handler-thrown reject passthrough. Every face is on the committed
production binding, so the default Smoke run executes it:

```bash
dotnet run --project crates/spoke-connect/bindings/csharp/Smoke/Smoke.csproj
```

Expected tail: `loopback tool faces: PASS`, then `GOLDEN PARITY: ALL PASS`.

## RemoteAdapter loopback smoke (optional)

The loopback section dials `RemoteAdapterFFI` through a C# `Transport`
implementation and runs `PutKnowledgeEntry` → `GetKnowledgeEntry`, asserting
the golden host peer id and payload (parity with Swift `loopback_smoke.swift`).

Requires a smoke-host cdylib (`ffi-smoke-host`) and C# bindings regenerated from
that cdylib. Build with `-p:SmokeHost=true`:

```bash
# From repository root
cargo +nightly build -p spoke-connect --features ffi,remote-adapter,ffi-smoke-host
SMOKE_GEN="$(mktemp -d)"
<fork>/target/debug/uniffi-bindgen-cs target/debug/libspoke_connect.dylib   --library --config crates/spoke-connect/bindings/csharp/uniffi.toml   --out-dir "${SMOKE_GEN}" --no-format
cp "${SMOKE_GEN}/spoke_connect.cs" crates/spoke-connect/bindings/csharp/generated/
cp target/debug/libspoke_connect.dylib crates/spoke-connect/bindings/csharp/bin/Debug/net8.0/
dotnet build -p:SmokeHost=true crates/spoke-connect/bindings/csharp/Smoke/Smoke.csproj
dotnet run -p:SmokeHost=true --project crates/spoke-connect/bindings/csharp/Smoke/Smoke.csproj
```

Expected tail: `loopback RemoteAdapterFFI: PASS`, `loopback tool faces: PASS`,
then `GOLDEN PARITY: ALL PASS`.
Restore production `generated/spoke_connect.cs` (from `ffi,remote-adapter` only)
before landing binding changes.

## Maintainer: regenerate → build → run

Prefer **PackageReference** for product hosts. Regenerate only when changing the Rust FFI surface.

The generate step needs the **vendored fork bindgen** binary (build recipe:
[`bindgen/README.md`](../bindgen/README.md)). Commands from the **repository
root** (local nightly convention for cargo):

```bash
# 0. Build the cdylib (ffi + remote-adapter)
cargo +nightly build -p spoke-connect --features ffi,remote-adapter

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

Expected output ends with `loopback tool faces: PASS`, then
`GOLDEN PARITY: ALL PASS`.

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
