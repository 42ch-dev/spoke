# spoke-connect Swift smoke

macOS-only smoke for the uniffi Swift bindings of the `spoke-connect` sync-core
facade. `main.swift` derives the golden peer id from the golden Ed25519 seed,
signs and verifies the golden hello (asserting base64url signature parity), and
exercises the rest of the exported surface (allowlist, sequences, nonce store,
dispatch gate, correlation, protocol version) with the mapped error cases, then
drives both FFI tool faces over the loopback pair (D15/D16): dialer
`invoke_tool` served by a responder foreign `ToolHandler`, responder reverse
invoke served by a dialer-side handler, unregistered-tool `op_unsupported`
deny, handler-thrown reject passthrough, and post-close `Closed` state — every
tool face is on the committed production binding, so the default gate runs
them without a smoke host. The smoke-host sections (RemoteAdapter put/get
round-trip against the reference smoke host, and the multi-peer router smoke)
compile only with `-D SMOKE_HOST`.

> Local env quirk: this machine's `~/.cargo/config.toml` carries
> `-Zno-embed-metadata` under `[unstable] rustflags` (a nightly-only flag).
> Run the cargo steps with the **nightly toolchain** (`cargo +nightly …`) so
> the flag is honored; CI builds on stable.

> Footgun: a plain `cargo build` (default features) between steps replaces the
> ffi cdylib in the shared `target/debug`. If the next smoke run fails with
> `dyld: Symbol not found: _ffi_spoke_connect_rustbuffer_free`, re-run the
> cdylib build before recompiling the smoke.

## Feature pairs

| Artifact | Cargo features | Notes |
|----------|----------------|-------|
| Committed xcframework + `bindings/swift/generated/` | `ffi,remote-adapter` | Production surface: `RemoteAdapterFFI`, `Transport`, `loopbackTransportPair`, the tool faces (`invokeTool`, `registerToolHandler`, `ToolHandler`, `connectResponderFfi` / `ConnectResponderFfi`) — **no** `startLoopbackSmokeHost` |
| Local smoke cdylib + smoke Swift bindings | `ffi,remote-adapter,ffi-smoke-host` | Adds loopback smoke host FFI for the RemoteAdapter loopback section |

`ffi-smoke-host` is non-default and is **not** implied by `remote-adapter` or `ffi`.

> Fixture note: the loopback seeds are exactly 32 bytes and derive the
> fixture's golden pubkey / peer_id via the in-crate oracle
> (`crates/spoke-connect/src/test_support/mod.rs` → `loopback_oracle`,
> `tests/common/loopback_oracle_impl.rs`). `seed_host_hex` was corrected
> 33 → 32 bytes and `pubkey_client_hex` added for the D16 tool-pair smokes.

SPM layout validation: from the repository root, `swift build` compiles the
root package (`SpokeConnect` + `spoke_connectFFI` xcframework). See
`bindings/swift/README.md` for the consumer `.package(url:from:)` path.

Run from the repository root:

```bash
# Maintainer refresh (production generated + xcframework; no smoke host):
./tooling/connect/build-swift-xcframework.sh

# SPM layout check (committed production xcframework):
swift build

# Default macOS smoke (committed production bindings + committed native;
# golden parity + tool faces; no smoke host):

# 1. Compile the smoke. The native is the committed production cdylib
#    (byte-identical across the bindings trees; e.g. bindings/go/native/darwin_arm64/).
swiftc -Xcc -fmodule-map-file="$PWD/crates/spoke-connect/bindings/swift/generated/spoke_connectFFI.modulemap" \
  -L "$PWD/crates/spoke-connect/bindings/go/native/darwin_arm64" -lspoke_connect \
  -Xlinker -rpath -Xlinker "$PWD/crates/spoke-connect/bindings/go/native/darwin_arm64" \
  -swift-version 5 \
  -o crates/spoke-connect/bindings/swift/Smoke/smoke \
  crates/spoke-connect/bindings/swift/Smoke/main.swift \
  crates/spoke-connect/bindings/swift/Smoke/tool_loopback_smoke.swift \
  crates/spoke-connect/bindings/swift/Smoke/loopback_transport.swift \
  crates/spoke-connect/bindings/swift/generated/spoke_connect.swift

# 2. Run it — every line must print PASS.
./crates/spoke-connect/bindings/swift/Smoke/smoke
```

Expected: `37 checks passed` (golden parity + tool faces, incl. post-close
`Closed` state), exit 0.

## Full smoke (golden parity + RemoteAdapter loopback + multi-peer router)

# 1. Build the smoke cdylib (includes ffi-smoke-host; not shipped in xcframework).
cargo +nightly build -p spoke-connect --features ffi,remote-adapter,ffi-smoke-host

# 2. Regenerate Swift bindings from the smoke cdylib into a local gitignored dir.
SMOKE_GEN="$(mktemp -d)"
cargo +nightly run -p spoke-connect --features ffi,bindgen-cli,remote-adapter,ffi-smoke-host --bin uniffi-bindgen -- \
  generate --library target/debug/libspoke_connect.dylib \
  --language swift --out-dir "${SMOKE_GEN}"

# 3. Point the dylib install name at @rpath (cargo bakes in the absolute
#    deps-dir path, which would pin the smoke to one machine).
install_name_tool -id @rpath/libspoke_connect.dylib target/debug/libspoke_connect.dylib

# 4. Compile the smoke (Swift 5 language mode keeps top-level code simple;
#    `-fmodule-map-file` is required — the Clang importer does not discover
#    the uniffi module map from `-I` alone). Link loopback_smoke.swift +
#    multi_peer_router_smoke.swift and use smoke-generated Swift (has
#    startLoopbackSmokeHost) with -D SMOKE_HOST.
swiftc -Xcc -fmodule-map-file="${SMOKE_GEN}/spoke_connectFFI.modulemap" \
  -L target/debug -lspoke_connect \
  -Xlinker -rpath -Xlinker "$PWD/target/debug" \
  -swift-version 5 -D SMOKE_HOST \
  -o crates/spoke-connect/bindings/swift/Smoke/smoke \
  crates/spoke-connect/bindings/swift/Smoke/main.swift \
  crates/spoke-connect/bindings/swift/Smoke/loopback_smoke.swift \
  crates/spoke-connect/bindings/swift/Smoke/multi_peer_router_smoke.swift \
  crates/spoke-connect/bindings/swift/Smoke/tool_loopback_smoke.swift \
  crates/spoke-connect/bindings/swift/Smoke/loopback_transport.swift \
  "${SMOKE_GEN}/spoke_connect.swift"

# 5. Run it — every line must print PASS.
./crates/spoke-connect/bindings/swift/Smoke/smoke
```

The committed production bindings live in `bindings/swift/generated/` (no smoke host).
The smoke binary is a local build artifact; it is gitignored.

## iOS simulator golden-parity smoke

The macOS `swiftc` smoke cannot load an iOS-simulator staticlib (the default
gate links the committed macOS dylib; the full smoke links the local debug
cdylib). iOS golden parity + tool faces run through the committed
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
version) plus the tool-faces loopback pair (D15/D16, incl. post-close
`Closed` state) against the registered golden fixture copies. See
`IosSmoke/README.md`.
