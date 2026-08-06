# Committed natives for Kotlin smoke / JNA load

JNA resolves `spoke_connect` via `uniffi.component.spoke_connect.libraryOverride`
in smoke (see `build.gradle.kts`) or from classpath resources in the published jar
(Task 4+).

| RID | Artifact | Status |
|-----|----------|--------|
| `darwin-aarch64` | `libspoke_connect.dylib` | **Committed** — maintainer-built from `cargo +nightly build -p spoke-connect --features ffi,remote-adapter --release` |
| `darwin-x86-64` | `libspoke_connect.dylib` | Deferred |
| `linux-x86-64` | `libspoke_connect.so` | Deferred — stage from `release.yml` `build-connect-ffi` |
| `win32-x86-64` | `spoke_connect.dll` | Deferred |

After copying a macOS dylib, run `install_name_tool -id @rpath/libspoke_connect.dylib`
on the committed file so JNA `@rpath` lookup works across machines.
