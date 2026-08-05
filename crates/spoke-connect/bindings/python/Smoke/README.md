# Python golden-parity + RemoteAdapter loopback smoke

Smoke for the uniffi Python bindings of the `spoke-connect` sync-core facade and
(optionally) the `RemoteAdapterFFI` loopback round-trip over a callback
`Transport`.

Golden parity asserts peer id, hello signature, verify round-trip, tamper
rejection, and protocol version `1`.

The loopback section dials `RemoteAdapterFFI` through a Python `Transport`
implementation, runs `put_knowledge_entry` → `get_knowledge_entry`, and asserts
the golden host peer id and payload (parity with Swift `loopback_smoke.swift`).

> Local env quirk: this machine's `~/.cargo/config.toml` carries
> `-Zno-embed-metadata` under `[unstable] rustflags` (a nightly-only flag).
> Run the cargo steps with the **nightly toolchain** (`cargo +nightly …`) so
> the flag is honored; CI builds on stable.

## Feature pairs

| Artifact | Cargo features | Notes |
|----------|----------------|-------|
| Committed `spoke_connect/` | `ffi,remote-adapter` | Production surface: `RemoteAdapterFFI`, `Transport`, `loopback_transport_pair` — **no** `start_loopback_smoke_host` |
| Local loopback smoke cdylib + bindings | `ffi,remote-adapter,ffi-smoke-host` | Adds loopback smoke host FFI for the RemoteAdapter section |

`ffi-smoke-host` is non-default and is **not** implied by `remote-adapter` or `ffi`.

## Golden parity only (committed production cdylib)

From the **repository root**:

```bash
PYTHONPATH=crates/spoke-connect/bindings/python \
  python3 -m unittest discover -s crates/spoke-connect/bindings/python/Smoke -v
```

Expected: 5 tests PASS (`GoldenParityTests`). `RemoteAdapterLoopbackTests` is skipped.

## Full smoke (golden parity + RemoteAdapter loopback)

From the **repository root**:

```bash
# 1. Build the smoke cdylib (includes ffi-smoke-host; not shipped in wheels).
cargo +nightly build -p spoke-connect --features ffi,remote-adapter,ffi-smoke-host

# 2. Regenerate Python bindings from the smoke cdylib into a local dir.
SMOKE_GEN="$(mktemp -d)"
cargo +nightly run -p spoke-connect --features ffi,bindgen-cli,remote-adapter,ffi-smoke-host --bin uniffi-bindgen -- \
  generate --library target/debug/libspoke_connect.dylib \
  --language python --out-dir "${SMOKE_GEN}"

# 3. Stage smoke bindings + cdylib beside the import package (macOS arm64 example).
cp "${SMOKE_GEN}/spoke_connect.py" crates/spoke-connect/bindings/python/spoke_connect/__init__.py
cp target/debug/libspoke_connect.dylib crates/spoke-connect/bindings/python/spoke_connect/
install_name_tool -id @rpath/libspoke_connect.dylib \
  crates/spoke-connect/bindings/python/spoke_connect/libspoke_connect.dylib

# 4. Run golden + loopback smokes.
PYTHONPATH=crates/spoke-connect/bindings/python \
  python3 -m unittest discover -s crates/spoke-connect/bindings/python/Smoke -v
```

Expected: 6 tests PASS (`GoldenParityTests` + `RemoteAdapterLoopbackTests`).

Restore committed production `spoke_connect/` (regenerate from `ffi,remote-adapter`
only) before landing binding changes — see `bindings/python/README.md`.
