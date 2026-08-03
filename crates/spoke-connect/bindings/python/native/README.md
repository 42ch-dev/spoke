# Native staging — spoke-connect Python wheels

Each platform wheel bundles **one** shared library beside `spoke_connect/__init__.py`. Staging paths mirror the ffi-matrix RIDs from `release.yml` `build-connect-ffi`.

| Wheel platform tag | Artifact RID | Native basename | Staging path (optional) |
|--------------------|--------------|-----------------|-------------------------|
| `linux_x86_64` | `linux-x64` | `libspoke_connect.so` | `native/linux_x86_64/` |
| `macosx_arm64` | `osx-arm64` | `libspoke_connect.dylib` | committed under `../spoke_connect/` for in-tree smoke |
| `win_amd64` | `win-x64` | `spoke_connect.dll` | `native/win_amd64/` |

Copy artifacts from a downloaded `ffi-<rid>` CI artifact (or local `ffi-stage/<rid>/`) into the import package before `tooling/connect/build-python-wheel.sh` runs. The wheel build script accepts `--artifact-root` + `--rid` for that copy.

Do not commit placeholder binaries — stage real artifacts at pack time only.
