# spoke-connect (Python)

PyPI package **`spoke-connect`**: SPOKE Connect session-core bindings (generated uniffi Python + native `spoke_connect` / `libspoke_connect`).

Packaging contract: [connect-binding-channels.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-binding-channels.md) §3.3.

## Install

At spoke lockstep tag `vX.Y.Z`:

```bash
pip install spoke-connect==X.Y.Z
```

Platform wheels (`py3-none-<platform>`) ship for `manylinux_2_35_x86_64`, `macosx_11_0_arm64`, and `win_amd64`. Each wheel bundles that platform's shared library beside the `spoke_connect` module (uniffi ctypes loader contract). Version locksteps with the spoke monorepo SemVer / git tag `vX.Y.Z`.

Release publishes via Trusted Publishing on the top-level `release.yml` workflow (`publish-pypi` job, OIDC). The PyPI Pending publisher for **`spoke-connect`** binds repository `42ch-dev/spoke` to that workflow filename; the first successful stable-tag publish activates the project on PyPI.

## Usage

```python
import spoke_connect

peer_id = spoke_connect.derive_peer_id_from_ed25519_pubkey(pubkey)
protocol = spoke_connect.protocol_version()  # 1
```

Transport / WebSocket stays in the host product.

## Build features

| Artifact | Cargo features | Notes |
|----------|----------------|-------|
| Committed `spoke_connect/` | `ffi,remote-adapter` | Production surface: `RemoteAdapterFFI`, `MultiPeerRouterFFI`, `Transport`, `loopback_transport_pair`, the tool faces (`invoke_tool`, `register_tool_handler`, `ToolHandler`, `connect_responder_ffi` / `ConnectResponderFFI`) — **no** `start_loopback_smoke_host` |
| Local loopback smoke cdylib + bindings | `ffi,remote-adapter,ffi-smoke-host` | Adds loopback smoke host FFI for the RemoteAdapter section |

`ffi-smoke-host` is non-default and is **not** implied by `remote-adapter` or `ffi`.
Full loopback smoke procedure: [`Smoke/README.md`](Smoke/README.md).

## RemoteAdapter FFI surface

With `remote-adapter` enabled, the binding ships the additive remote-adapter surface: `RemoteAdapterFFI` (single peer), `MultiPeerRouterFFI` (multi-peer routing), the callback `Transport` interface, the in-memory loopback helpers, and the tool faces — `invoke_tool` on the adapter, router, and responder, `register_tool_handler` on the adapter and responder, the `ToolHandler` callback, and `connect_responder_ffi` / `ConnectResponderFFI` for the accept side.

### Transport contract

The callback `Transport` is a message-oriented interface: one envelope per call, blocking receive, idempotent close.

| Method | Behavior |
|--------|----------|
| `send(envelope)` | Accepts exactly one connect envelope's bytes per call |
| `recv()` | Blocks until the next inbound envelope arrives or the connection closes; returns exactly one envelope per call |
| `close()` | Releases transport resources; idempotent — closing either end of a connection fails the peer's pending `recv` like a real connection drop |

The surface bounds messages at one envelope per call; byte-stream carriers apply length-prefix (or equivalent) delimiting before handing envelopes to the adapter.

### MultiPeerRouterFFI

`new_multi_peer_router_ffi()` returns the router as a synchronous object over the same runtime: a peer registry (`register_peer(adapter)` accepts an established `RemoteAdapterFFI` and returns its `peer_id`; `unregister_peer(peer_id)`; `list_peers()`), the `BaselinePorts` six families routed per call to exactly one capable peer, and the two `HostManifestPort` aggregation views — the composed `get_host_capability_manifest()` and the per-peer `list_peer_host_capability_manifests()`. Selection matches each registered peer's cached `HostCapabilityManifest`: required capability, exact namespace, soft role preference, and a deterministic lowest-`peer_id` tie-break.

### Tool faces

- `invoke_tool(capability_id, arguments_json)` — invoke a tool on the peer (dialer → responder, responder → dialer, or router → capable peer); returns the tool's `result` as a JSON string. A non-`tools.` id or malformed arguments rejects `INVALID_INPUT` with zero wire traffic; a dispatch deny rejects `CAPABILITY_PORT_MISSING` with the peer's preserved `wire_code`.
- `register_tool_handler(capability_id, handler)` — serve reverse invokes through a foreign `ToolHandler`; last-wins per id, never mutates the manifest. The callback's `handle(arguments_json)` returns the result JSON; a raised `FfiError.Rejected` passes through verbatim as an application reject, any other outcome is contained to `INTERNAL_ERROR` and the session survives.
- `connect_responder_ffi(...)` / `ConnectResponderFFI` — the accept side: wrap a connected (host-accepted) callback `Transport`. The constructor returns immediately in `Handshaking` — poll `state()` (bounded) to `Established` before invoking; a handshake failure surfaces as `state() → "Closed"` (never a thrown constructor error), config-validation failures return `Dial { kind: "config" }`.
- Handlers run on the FFI blocking pool — do not synchronously call back into the FFI faces from `handle`; hand off asynchronously in the host instead.

## Layout

| Path | Contents |
|------|----------|
| `pyproject.toml` | Packable project (`name = spoke-connect`; lockstep SemVer) |
| `setup.py` | Platform wheel tag hook (`py3-none-<platform>`; no sdist) |
| `spoke_connect/` | Import package — committed generated `__init__.py` + host native for smoke |
| `native/` | Staging contract for non-host RIDs (see `native/README.md`) |
| `Smoke/` | Golden-parity + RemoteAdapter loopback smoke — see `Smoke/README.md` |

## Maintainer: regenerate → stage native → wheel → smoke

Commands from the **repository root** (local nightly convention for cargo: `cargo +nightly …`).

Production regenerate uses `ffi,remote-adapter` only (no `ffi-smoke-host`). Full loopback smoke
recipe: [`Smoke/README.md`](Smoke/README.md).

```bash
# 1. Build the production ffi cdylib (must run before generate — default build replaces the stub)
cargo +nightly build -p spoke-connect --features ffi,remote-adapter --release

# 2. Generate bindings (first-party uniffi 0.32)
cargo +nightly run -p spoke-connect --features ffi,bindgen-cli,remote-adapter --bin uniffi-bindgen -- \
  generate --library target/release/libspoke_connect.dylib \
  --language python --out-dir crates/spoke-connect/bindings/python/spoke_connect
mv crates/spoke-connect/bindings/python/spoke_connect/spoke_connect.py \
   crates/spoke-connect/bindings/python/spoke_connect/__init__.py

# 3. Stage host native beside the module (darwin arm64 example)
cp target/release/libspoke_connect.dylib \
  crates/spoke-connect/bindings/python/spoke_connect/
install_name_tool -id @rpath/libspoke_connect.dylib \
  crates/spoke-connect/bindings/python/spoke_connect/libspoke_connect.dylib

# 4. Golden-parity smoke (no wheel required)
PYTHONPATH=crates/spoke-connect/bindings/python \
  python3 -m unittest discover -s crates/spoke-connect/bindings/python/Smoke -v

# 5. Build platform wheel (host RID or CI artifact — recipe: tooling/connect/build-python-wheel.sh)
./tooling/connect/build-python-wheel.sh

# Release: all three wheels from build-connect-ffi artifacts
./tooling/connect/build-python-wheels-from-ffi.sh ffi-assembled
```

v1 publishes **platform wheels only** (no sdist): a source install cannot produce the native library without a Rust toolchain.

## Committed natives (in-tree smoke)

| Wheel platform (PEP 425) | Native | Status |
|--------------------------|--------|--------|
| `macosx_11_0_arm64` | `spoke_connect/libspoke_connect.dylib` | **Committed** — maintainer-built (release) |
| `manylinux_2_35_x86_64` | `libspoke_connect.so` | **CI** — stage from `release.yml` `build-connect-ffi` (`linux-x64`, `ubuntu-22.04`) before wheel build |
| `win_amd64` | `spoke_connect.dll` | **CI** — stage from `build-connect-ffi` (`win-x64`) before wheel build |

## Reference

- Wheel recipe: [`tooling/connect/build-python-wheel.sh`](../../../../tooling/connect/build-python-wheel.sh)
- Release wheels (all RIDs): [`tooling/connect/build-python-wheels-from-ffi.sh`](../../../../tooling/connect/build-python-wheels-from-ffi.sh)
- Pre-publish inventory gate: [`tooling/connect/verify-python-wheels.sh`](../../../../tooling/connect/verify-python-wheels.sh)
- FFI surface: [`../../src/ffi.rs`](../../src/ffi.rs)
- Publish strategy: [connect-publish-strategy.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-publish-strategy.md)
