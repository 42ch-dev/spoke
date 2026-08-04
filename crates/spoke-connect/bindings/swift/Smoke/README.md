# spoke-connect Swift smoke

macOS-only smoke for the uniffi Swift bindings of the `spoke-connect` sync-core
facade. `main.swift` derives the golden peer id from the golden Ed25519 seed,
signs and verifies the golden hello (asserting base64url signature parity), and
exercises the rest of the exported surface (allowlist, sequences, nonce store,
dispatch gate, correlation, protocol version) with the mapped error cases.

> Local env quirk: this machine's `~/.cargo/config.toml` carries
> `-Zno-embed-metadata` under `[unstable] rustflags` (a nightly-only flag).
> Run the cargo steps with the **nightly toolchain** (`cargo +nightly …`) so
> the flag is honored; CI builds on stable.

> Footgun: a plain `cargo build` (default features) between steps replaces the
> ffi cdylib in the shared `target/debug`. If the next smoke run fails with
> `dyld: Symbol not found: _ffi_spoke_connect_rustbuffer_free`, re-run step 1
> (`cargo build -p spoke-connect --features ffi`) before recompiling the smoke.

SPM layout validation: from the repository root, `swift build` compiles the
root package (`SpokeConnect` + `spoke_connectFFI` xcframework). See
`bindings/swift/README.md` for the consumer `.package(url:from:)` path.

Run from the repository root:

```bash
# Maintainer refresh (generated + xcframework):
./tooling/connect/build-swift-xcframework.sh

# SPM layout check:
swift build

# swiftc golden-parity smoke (links the debug cdylib directly):

# 1. Build the cdylib that carries the exported-surface metadata.
cargo build -p spoke-connect --features ffi

# 2. Regenerate only when FFI surface changed (otherwise use committed generated/).
cargo run -p spoke-connect --features bindgen-cli --bin uniffi-bindgen -- \
  generate --library target/debug/libspoke_connect.dylib \
  --language swift --out-dir crates/spoke-connect/bindings/swift/generated

# 3. Point the dylib install name at @rpath (cargo bakes in the absolute
#    deps-dir path, which would pin the smoke to one machine).
install_name_tool -id @rpath/libspoke_connect.dylib target/debug/libspoke_connect.dylib

# 4. Compile the smoke (Swift 5 language mode keeps top-level code simple;
#    `-fmodule-map-file` is required — the Clang importer does not discover
#    the uniffi module map from `-I` alone).
swiftc -Xcc -fmodule-map-file="$PWD/crates/spoke-connect/bindings/swift/generated/spoke_connectFFI.modulemap" \
  -L target/debug -lspoke_connect \
  -Xlinker -rpath -Xlinker "$PWD/target/debug" \
  -swift-version 5 \
  -o crates/spoke-connect/bindings/swift/Smoke/smoke \
  crates/spoke-connect/bindings/swift/Smoke/main.swift \
  crates/spoke-connect/bindings/swift/generated/spoke_connect.swift

# 5. Run it — every line must print PASS.
./crates/spoke-connect/bindings/swift/Smoke/smoke
```

The generated bindings live in `bindings/swift/generated/` (committed).
The smoke binary is a local build artifact; it is gitignored.

## iOS simulator golden-parity smoke

The macOS `swiftc` smoke above links the debug cdylib and cannot load an
iOS-simulator staticlib. iOS golden parity runs through the committed
xcframework simulator slice via the maintainer-local SwiftPM package
`bindings/swift/IosSmoke/` (not part of the SPM product layout):

```bash
cd crates/spoke-connect/bindings/swift/IosSmoke
xcodebuild test -scheme IosSmoke-Package \
  -destination 'platform=iOS Simulator,name=iPhone 17' \
  -derivedDataPath /tmp/ios-smoke-dd
```

Named simulator devices vary per Xcode install; `-destination
'generic/platform=iOS Simulator'` is the portable fallback. The tests assert
the same golden triad as the macOS smoke (peer id, hello signature, protocol
version) against the registered golden fixture copy. See `IosSmoke/README.md`.
