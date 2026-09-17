# spoke-connect C++ binding

Hand-written C ABI over the Rust `spoke-connect` reference: one header,
`include/spoke_connect.h`, plus a committed dynamic library per platform. A
C++17 host compiles against the header and links the native, with exceptions
and RTTI disabled. The boundary reports ABI revision `1` through
`spoke_connect_abi_version`.

The surface mirrors the native bindings' facade: the session core (peer id
derivation, hello signing and verification, the allowlist, the nonce store, the
sequence counters, the correlation and dispatch gates), `RemoteAdapter`,
`MultiPeerRouter`, `ConnectResponder` with the `PortsHandler` and `ToolHandler`
callbacks, and the in-memory loopback helpers. `parity.md` maps each production
facade member and error variant to its C declaration.

## Layout

| Path | Contents |
|------|----------|
| `include/spoke_connect.h` | The C contract: status values, value types, callback tables and every exported function |
| `native/osx-arm64/libspoke_connect_capi.dylib` | macOS arm64 carrier, install name `@rpath/libspoke_connect_capi.dylib` |
| `native/win-x64/` | Windows x64 carrier: `spoke_connect_capi.dll` plus the Rust-produced `spoke_connect_capi.dll.lib` import library |
| `native/provenance.json` | Per RID: source revision, target, `rustc -Vv`, compiler version, build flags, header SHA-256 and native SHA-256 |
| `Smoke/main.cpp` | C++17 smoke: golden-vector assertions, a ports round trip over a host-owned loopback, and the rejection/ownership rules |
| `parity.md` | C ABI ⇄ production facade parity table |

## Linking

macOS arm64 (`osx-arm64`) links the dylib and points the loader at its
directory:

```sh
clang++ -std=c++17 -fno-exceptions -fno-rtti \
  -I crates/spoke-connect/bindings/cpp/include \
  host.cpp crates/spoke-connect/bindings/cpp/native/osx-arm64/libspoke_connect_capi.dylib \
  -Wl,-rpath,<absolute path to native/osx-arm64> \
  -o host
```

Windows x64 (`win-x64`) uses the release C++ CRT (`/MD`) that matches the
carrier's Rust dynamic CRT runtime, links the import library, and loads the DLL
from the executable's directory:

```bat
cl.exe /nologo /std:c++17 /EHs-c- /GR- /MD /W4 /WX /I<absolute path to include> ^
  host.cpp <absolute path to native\win-x64\spoke_connect_capi.dll.lib> /Fe:host.exe
copy /Y <absolute path to native\win-x64\spoke_connect_capi.dll> .
host.exe
```

## Calling the ABI

| Concern | Contract |
|---------|----------|
| Status | Every call returns `int32_t`: `SPOKE_CONNECT_OK` (0) on success, or a `SPOKE_CONNECT_*` value with the detail in the caller's error record. A failed call leaves no result with the caller. |
| Out parameters | Zero-initialize each out value and the `SpokeConnectError` record before the call; the library fills the fields the result carries. |
| Owned results | Release `SpokeConnectBuffer` with `spoke_connect_buffer_free`, `SpokeConnectOptionalBuffer` with `spoke_connect_optional_buffer_free`, and `SpokeConnectError` with `spoke_connect_error_free`. |
| Borrowed inputs | `SpokeConnectSlice` borrows bytes for the duration of the call; the length is authoritative, text is UTF-8, and keys are 32 raw bytes. |
| Handles | Each handle owns one carrier object and releases with `<object>_free` (a NULL handle is a no-op); `close` ends a session and is distinct from `free`. Constructors and the router borrow the handles they are given. |
| Callback tables | `spoke_connect_transport_new`, `spoke_connect_ports_handler_new` and `spoke_connect_tool_handler_new` copy the table and take ownership of `user_data` on success, running its `destroy` once after the last reference and in-flight callback. |
| Returned buffers | A callback hands ownership of a populated `SpokeConnectForeignBuffer` to the carrier, which copies the bytes and calls the buffer's `release` exactly once. |
| Threading | Calls block the calling host thread; callbacks run on the carrier's blocking pool and may run concurrently, so host contexts are thread-safe. |
| Lifetime | Load the carrier for the process lifetime, and close sessions and release handles before host shutdown. |

## Smoke

`tooling/connect/cpp-smoke.mjs` builds `Smoke/main.cpp` in `target/cpp-smoke`,
links the staged native for the requested RID, and runs it against the shared
golden vector `crates/spoke-connect/tests/fixtures/golden-hello.json`:

```sh
node tooling/connect/cpp-smoke.mjs --rid osx-arm64
node tooling/connect/cpp-smoke.mjs --rid win-x64
```

A green run prints one banner per assertion group, in order:

```text
golden peer-id: PASS
golden hello signature: PASS
protocol version 1: PASS
loopback ports: PASS
rejection/ownership: PASS
C++ smoke: PASS
```

Each banner follows the assertions of its group, and the run exits non-zero at
the first failed check.

## Committed natives

| RID | Status |
|-----|--------|
| `osx-arm64` | **Committed** — built and staged by `tooling/connect/cpp-build.mjs` |
| `win-x64` | **Committed** — Windows x64 carrier and Rust-produced import library |

`tooling/connect/cpp-build.mjs` rebuilds a native from the integrated Rust
source and refreshes that RID's `provenance.json` entry with the revision,
toolchain and hashes of the shipped files.

## Header durability

`tooling/connect/cpp-symbol-check.mjs` compares the header's declaration block
against the native's exported `spoke_connect_*` symbols in both directions, then
compiles a C99 probe holding a typed function pointer to every declaration and a
C++17 inclusion check with exceptions and RTTI disabled, so a header edit and its
export land together.

It also pins the record and callback-table representation: the header's
`typedef struct` block is checked against the carrier's `#[repr(C)]` mirrors, and
one `_Static_assert` per `sizeof` / `_Alignof` / `offsetof` is compiled, so a
field reorder or resize fails the gate. The check needs `cargo` on `PATH`, since
the report it compiles against is produced by the carrier's own test target.

## Reference

- Decision record: [`.mstar/specs/connect-cpp-binding.md`](../../../../.mstar/specs/connect-cpp-binding.md)
- Header: [`include/spoke_connect.h`](include/spoke_connect.h)
- Parity table: [`parity.md`](parity.md)
- Rust facade: [`../../src/ffi.rs`](../../src/ffi.rs)
