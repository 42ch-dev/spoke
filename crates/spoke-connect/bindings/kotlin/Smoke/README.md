# Kotlin golden-parity + RemoteAdapter loopback smoke

JVM smoke for the uniffi Kotlin bindings of the `spoke-connect` sync-core facade
and (optionally) the `RemoteAdapterFFI` loopback round-trip over a callback
`Transport`.

Golden parity asserts peer id, hello signature, verify round-trip, tamper
rejection, and protocol version `1`.

The loopback section dials `RemoteAdapterFFI` through a Kotlin `Transport`
implementation. The tool section drives both FFI faces over a loopback pair
(D15/D16): dialer `invoke_tool` served by a responder foreign `ToolHandler`,
responder reverse invoke served by a dialer-side handler, unregistered-tool
`op_unsupported` deny, handler-thrown reject passthrough, and the error rows — an unknown reject code downgraded to `INTERNAL_ERROR`, a foreign (non-Rejected) fault contained to `INTERNAL_ERROR` with the serve loop surviving — every face is on
the committed production binding, so it runs in the default `gradle test`.
The smoke-host put/get round-trip additionally dials the reference smoke host
(`startLoopbackSmokeHost`, `ffi-smoke-host` only) and asserts the golden host
peer id and payload (parity with Swift `loopback_smoke.swift`).

> Local env quirk: this machine's `~/.cargo/config.toml` carries
> `-Zno-embed-metadata` under `[unstable] rustflags` (a nightly-only flag).
> Run the cargo steps with the **nightly toolchain** (`cargo +nightly …`) so
> the flag is honored; CI builds on stable.

## Feature pairs

| Artifact | Cargo features | Notes |
|----------|----------------|-------|
| Committed `generated/` + `native/` | `ffi,remote-adapter` | Production surface: `RemoteAdapterFFI`, `Transport`, `loopbackTransportPair` — **no** `startLoopbackSmokeHost` |
| Local loopback smoke cdylib + bindings | `ffi,remote-adapter,ffi-smoke-host` | Adds loopback smoke host FFI for the RemoteAdapter section |

`ffi-smoke-host` is non-default and is **not** implied by `remote-adapter` or `ffi`.

> Fixture note: the loopback seeds are exactly 32 bytes and derive the
> fixture's golden pubkey / peer_id via the in-crate oracle
> (`crates/spoke-connect/src/test_support/mod.rs` → `loopback_oracle`,
> `tests/common/loopback_oracle_impl.rs`). `seed_host_hex` was corrected
> 33 → 32 bytes and `pubkey_client_hex` added for the D16 tool-pair smokes.

## Golden parity + tool faces (committed production cdylib)

From `crates/spoke-connect/bindings/kotlin/` (requires host native under
`native/<rid>/` — see `native/README.md`):

```bash
gradle test
```

Expected: 6 tests PASS (`GoldenParityTest` + `ToolLoopbackFfiPairTest`).

Override native path explicitly:

```bash
gradle test -PnativeLib="$PWD/native/darwin-aarch64/libspoke_connect.dylib"
```

## Full smoke (golden parity + RemoteAdapter loopback)

From the **repository root**:

```bash
# 1. Build the smoke cdylib (includes ffi-smoke-host; not shipped in Maven).
cargo +nightly build -p spoke-connect --features ffi,remote-adapter,ffi-smoke-host

# 2. Regenerate Kotlin bindings from the smoke cdylib into a local dir.
SMOKE_GEN="$(mktemp -d)"
cargo +nightly run -p spoke-connect --features ffi,bindgen-cli,remote-adapter,ffi-smoke-host --bin uniffi-bindgen -- \
  generate --library target/debug/libspoke_connect.dylib \
  --language kotlin --out-dir "${SMOKE_GEN}" --no-format
./crates/spoke-connect/bindings/kotlin/bindgen/patch-kotlin-core-error-fields.sh \
  "${SMOKE_GEN}/uniffi/spoke_connect/spoke_connect.kt"

# 3. Stage the smoke cdylib for JNA (macOS arm64 example).
mkdir -p crates/spoke-connect/bindings/kotlin/native/darwin-aarch64
cp target/debug/libspoke_connect.dylib crates/spoke-connect/bindings/kotlin/native/darwin-aarch64/
install_name_tool -id @rpath/libspoke_connect.dylib \
  crates/spoke-connect/bindings/kotlin/native/darwin-aarch64/libspoke_connect.dylib

# 4. Run golden + loopback smokes (smoke bindings dir + loopback test sources).
cd crates/spoke-connect/bindings/kotlin
gradle test \
  -PsmokeHost=true \
  -PsmokeBindingsDir="${SMOKE_GEN}" \
  -PnativeLib="$PWD/native/darwin-aarch64/libspoke_connect.dylib"
```

Expected: 7 tests PASS (`GoldenParityTest` + `ToolLoopbackFfiPairTest` +
`RemoteAdapterLoopbackTest`).

## Maintainer: regenerate committed production bindings

Production regenerate sequence (no smoke host): [`../bindgen/README.md`](../bindgen/README.md)
and [`../README.md`](../README.md). Build with `ffi,remote-adapter` only before
committing `generated/` and staging `native/`.
