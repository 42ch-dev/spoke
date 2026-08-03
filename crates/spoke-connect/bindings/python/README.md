# spoke-connect (Python)

PyPI package **`spoke-connect`**: SPOKE Connect session-core bindings (generated uniffi Python + native `spoke_connect` / `libspoke_connect`).

Packaging contract: [connect-binding-channels.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-binding-channels.md) §3.3.

## Install

At spoke lockstep tag `vX.Y.Z`:

```bash
pip install spoke-connect==X.Y.Z
```

Platform wheels (`py3-none-<platform>`) ship for `manylinux_2_17_x86_64`, `macosx_11_0_arm64`, and `win_amd64`. Each wheel bundles that platform's shared library beside the `spoke_connect` module (uniffi ctypes loader contract). Version locksteps with the spoke monorepo SemVer / git tag `vX.Y.Z`.

Release publishes via Trusted Publishing on the top-level `release.yml` workflow (`publish-pypi` job, OIDC). The PyPI Pending publisher for **`spoke-connect`** binds repository `42ch-dev/spoke` to that workflow filename; the first successful stable-tag publish activates the project on PyPI.

## Usage

```python
import spoke_connect

peer_id = spoke_connect.derive_peer_id_from_ed25519_pubkey(pubkey)
protocol = spoke_connect.protocol_version()  # 1
```

Transport / WebSocket stays in the host product.

## Layout

| Path | Contents |
|------|----------|
| `pyproject.toml` | Packable project (`name = spoke-connect`; lockstep SemVer) |
| `setup.py` | Platform wheel tag hook (`py3-none-<platform>`; no sdist) |
| `spoke_connect/` | Import package — committed generated `__init__.py` + host native for smoke |
| `native/` | Staging contract for non-host RIDs (see `native/README.md`) |
| `Smoke/` | Golden-parity smoke (`python3 -m unittest discover -s Smoke -v`) |

## Maintainer: regenerate → stage native → wheel → smoke

Commands from the **repository root** (local nightly convention for cargo: `cargo +nightly …`).

```bash
# 1. Build the ffi cdylib (must run before generate — default build replaces the stub)
cargo +nightly build -p spoke-connect --features ffi --release

# 2. Generate bindings (first-party uniffi 0.32)
cargo +nightly run -p spoke-connect --features bindgen-cli --bin uniffi-bindgen -- \
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
| `manylinux_2_17_x86_64` | `libspoke_connect.so` | **CI** — stage from `release.yml` `build-connect-ffi` (`linux-x64`) before wheel build |
| `win_amd64` | `spoke_connect.dll` | **CI** — stage from `build-connect-ffi` (`win-x64`) before wheel build |

## Reference

- Wheel recipe: [`tooling/connect/build-python-wheel.sh`](../../../../tooling/connect/build-python-wheel.sh)
- Release wheels (all RIDs): [`tooling/connect/build-python-wheels-from-ffi.sh`](../../../../tooling/connect/build-python-wheels-from-ffi.sh)
- Pre-publish inventory gate: [`tooling/connect/verify-python-wheels.sh`](../../../../tooling/connect/verify-python-wheels.sh)
- FFI surface: [`../../src/ffi.rs`](../../src/ffi.rs)
- Publish strategy: [connect-publish-strategy.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/connect-publish-strategy.md)
