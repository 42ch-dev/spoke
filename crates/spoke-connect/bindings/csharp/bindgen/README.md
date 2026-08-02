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
`.mstar/specs/connect-csharp-binding.md` for the full decision record.

The fork restores the locked `--library` CLI form against the 0.32 cdylib;
the generated bindings load and pass the checksum gate at runtime (golden
parity verified).

## Patch contents (129 lines, 5 files)

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

**Template threading caveat:** the fork's `bindgen/templates/*.cs` are
generator-internal implementation detail. Regenerate `spoke_connect.cs` from
the patched source (recipe step 4) rather than threading template files into
the generated output or consumers — a threaded snapshot drifts from the
generator's contract-version and checksum gates.

## Build recipe (macOS arm64, repo nightly convention)

Each step's shell context is explicit — the clone and the repo root are
separate directories: the patch applies **inside the clone**, and the
generate step runs from the **repo root**:

```bash
# 1. Clone upstream at the exact pinned commit — from the repo root.
#    `rm -rf` keeps re-runs (e.g. the periodic drop-fork re-check) idempotent.
rm -rf uniffi-bindgen-cs
git clone https://github.com/NordSecurity/uniffi-bindgen-cs
cd uniffi-bindgen-cs
git checkout e10ce410eb3a10cc19c7928b93ea8d84e038c034   # v0.11.0+v0.31.0

# 2. Apply the fork delta — INSIDE the clone. The patch *file* path is
#    repo-root-relative, but the patch's *content* paths (Cargo.toml,
#    bindgen/...) are clone-relative, so `git apply` must run from here.
git apply ../crates/spoke-connect/bindings/csharp/bindgen/uniffi-bindgen-cs-0.32.patch

# 2b. Copy the fork's resolved lockfile — INSIDE the clone. The committed
#     lock pins the post-retarget dependency graph (uniffi 0.32), so a clean
#     clone + apply + this copy builds reproducibly with `--locked` instead
#     of re-resolving against the live registry.
cp ../crates/spoke-connect/bindings/csharp/bindgen/uniffi-bindgen-cs-0.32.Cargo.lock Cargo.lock

# 3. Build the bindgen binary (nightly per root AGENTS.md) — still inside the clone
cargo +nightly build --locked -p uniffi-bindgen-cs
# binary: target/debug/uniffi-bindgen-cs (inside the clone)

# 4. Generate against the 0.32 cdylib — from the REPO ROOT
cd ..   # back to the repo root
cargo +nightly build -p spoke-connect --features ffi
./uniffi-bindgen-cs/target/debug/uniffi-bindgen-cs \
  target/debug/libspoke_connect.dylib --library \
  --out-dir crates/spoke-connect/bindings/csharp/generated --no-format

# 5. Build + run the net8.0 smoke — from the repo root
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
