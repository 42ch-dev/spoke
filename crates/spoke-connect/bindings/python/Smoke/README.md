# Python golden-parity + RemoteAdapter loopback smoke

Smoke for the uniffi Python bindings of the `spoke-connect` sync-core facade and
(optionally) the `RemoteAdapterFFI` loopback round-trip over a callback
`Transport`.

Golden parity asserts peer id, hello signature, verify round-trip, tamper
rejection, and protocol version `1`.

The loopback section dials `RemoteAdapterFFI` through a Python `Transport`
implementation. The tool section drives both FFI faces over a loopback pair
(D15/D16): dialer `invoke_tool` served by a responder foreign `ToolHandler`,
responder reverse invoke served by a dialer-side handler, unregistered-tool
`op_unsupported` deny, handler-thrown reject passthrough, and the error rows — an unknown reject code downgraded to `INTERNAL_ERROR`, a foreign (non-Rejected) fault contained to `INTERNAL_ERROR` with the serve loop surviving — every face is on
the committed production binding, so it runs in the default `unittest discover`
suite. The ports section drives the optional-port dialer ops (`project` /
`compute` / `list_fork_timeline_events` on `RemoteAdapterFFI`) and the
responder ports face (D16): baseline + optional serving round-trips through a
foreign `PortsHandler` (user lock), application-reject passthrough, the error
rows — capability-gate deny, absent-ports fail-closed deny, foreign-fault
containment with serve-loop survival — and malformed-JSON pre-validation;
also in the default `unittest discover` suite (no smoke host needed). The
smoke-host put/get round-trip additionally dials the reference smoke host
(`start_loopback_smoke_host`, `ffi-smoke-host` only) and asserts the golden
host peer id and payload (parity with Swift `loopback_smoke.swift`).

> Local env quirk: this machine's `~/.cargo/config.toml` carries
> `-Zno-embed-metadata` under `[unstable] rustflags` (a nightly-only flag).
> Run the cargo steps with the **nightly toolchain** (`cargo +nightly …`) so
> the flag is honored; CI builds on stable.

## Feature pairs

| Artifact | Cargo features | Notes |
|----------|----------------|-------|
| Committed `spoke_connect/` | `ffi,remote-adapter` | Production surface: `RemoteAdapterFFI`, `Transport`, `loopback_transport_pair`, the tool faces, the optional-port dialer ops (`project` / `compute` / `list_fork_timeline_events` on `RemoteAdapterFFI`), and the responder ports face (optional `ports:` + the `PortsHandler` callback) — **no** `start_loopback_smoke_host` |
| Local loopback smoke cdylib + bindings | `ffi,remote-adapter,ffi-smoke-host` | Adds loopback smoke host FFI for the RemoteAdapter section |

`ffi-smoke-host` is non-default and is **not** implied by `remote-adapter` or `ffi`.

> Fixture note: the loopback seeds are exactly 32 bytes and derive the
> fixture's golden pubkey / peer_id via the in-crate oracle
> (`crates/spoke-connect/src/test_support/mod.rs` → `loopback_oracle`,
> `tests/common/loopback_oracle_impl.rs`). `seed_host_hex` was corrected
> 33 → 32 bytes and `pubkey_client_hex` added for the D16 tool-pair smokes.

## Golden parity + tool faces (committed production cdylib)

From the **repository root**:

```bash
PYTHONPATH=crates/spoke-connect/bindings/python \
  python3 -m unittest discover -s crates/spoke-connect/bindings/python/Smoke -v
```

Expected: 9 tests PASS (`GoldenParityTests` + `ToolLoopbackFfiPairTests` +
`PortsLoopbackFfiPairTests`, 0 skips among them). `RemoteAdapterLoopbackTests`
(smoke-host put/get) is skipped.

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

Expected: 10 tests PASS (`GoldenParityTests` + `ToolLoopbackFfiPairTests` +
`PortsLoopbackFfiPairTests` + `RemoteAdapterLoopbackTests`), 0 skips.

Restore committed production `spoke_connect/` (regenerate from `ffi,remote-adapter`
only) before landing binding changes — see `bindings/python/README.md`.
