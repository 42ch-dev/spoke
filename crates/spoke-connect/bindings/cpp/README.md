# spoke-connect C++ binding

Hand-written C ABI over the Rust `spoke-connect` reference, plus a C++17
convenience layer over that ABI: `include/spoke_connect.h` is the C contract and
`include/spoke_connect.hpp` is the header-only C++ surface, with a committed
dynamic library per platform. The boundary reports ABI revision `1` through
`spoke_connect_abi_version`.

A C++17 host includes `spoke_connect.hpp` (which includes the C header) and
links the native. The convenience layer wraps every production capability of the
C ABI in namespace `spoke::connect`: the session core (peer id derivation, hello
signing and verification, the allowlist, the nonce store, the sequence counters,
the correlation and dispatch gates), `RemoteAdapter`, `MultiPeerRouter`,
`ConnectResponder` with the `PortsHandler` and `ToolHandler` callbacks, and the
in-memory loopback helpers. `parity.md` maps each production facade member and
error variant to its C declaration and its C++ counterpart.

Full consumer guide, with a complete runnable session:
<https://github.com/42ch-dev/spoke/blob/main/docs/how-to/connect-cpp-binding.md>.

## Layout

| Path | Contents |
|------|----------|
| `include/spoke_connect.h` | The C contract: status values, value types, callback tables and every exported function (C99 language floor, `extern "C"` inclusion for C++) |
| `include/spoke_connect.hpp` | C++17 header-only convenience layer over the C contract: move-only ownership, borrowed views, structured `Result` values and the callback bridges |
| `native/osx-arm64/libspoke_connect_capi.dylib` | macOS arm64 carrier, install name `@rpath/libspoke_connect_capi.dylib` |
| `native/win-x64/` | Windows x64 carrier: `spoke_connect_capi.dll` plus the Rust-produced `spoke_connect_capi.dll.lib` import library |
| `native/provenance.json` | Per RID: source revision, target, `rustc -Vv`, compiler version, build flags, header SHA-256 and native SHA-256 |
| `Smoke/main.cpp` | C++17 smoke (raw C ABI): golden-vector assertions, a ports round trip over a host-owned loopback, and the rejection/ownership rules |
| `Smoke/convenience.cpp` | C++17 smoke (convenience layer): the value/ownership layer, the core functions and objects, the session and tool callbacks, and the router |
| `Smoke/support.hpp` | Shared smoke support: the golden-vector read and parse, the assertion primitives, and the convenience-group entry point |
| `parity.md` | C ABI ⇄ production facade parity table with the C++ counterpart column |

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
cl.exe /nologo /std:c++17 /EHs-c- /GR- /MD /D_HAS_EXCEPTIONS=0 /W4 /WX ^
  /I<absolute path to include> ^
  host.cpp <absolute path to native\win-x64\spoke_connect_capi.dll.lib> /Fe:host.exe
copy /Y <absolute path to native\win-x64\spoke_connect_capi.dll> .
host.exe
```

`-fno-exceptions -fno-rtti` (`/EHs-c- /GR- /D_HAS_EXCEPTIONS=0`) is the
consumer default; an exception-enabled build uses `-fexceptions -fno-rtti`
(`/EHsc`) with the same error API. Translation units that include
`spoke_connect.hpp` in one linked image must use a consistent exception and
standard-library configuration.

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

### The C++ layer

| Concern | Contract |
|---------|----------|
| Borrowed views | `Buffer::view()` borrows the owned payload — valid while that `Buffer` lives, binary length authoritative, deleted on a temporary. `Buffer::str()` returns an owned `std::string`. |
| Results | Every fallible call returns `[[nodiscard]] Result<T>` / `Result<void>`; `Error` carries the original status plus `message`, optional `code` / `kind` / `wire_code`, `expected` and `actual`. No throwing API. |
| Handles | Move-only RAII classes: the destructor performs the release, a moved-from object is empty, and `get()` / `release()` / `adopt()` cover raw-C interop. |
| Explicit close | Destructors free only. `RemoteAdapter::close()` and `ConnectResponder::close()` end a session; the C ABI exports no standalone transport close, so the host closes its own queue or connection. Close sessions before releasing the last host resources. |
| Callback context | The factories take `std::unique_ptr<TransportCallbacks>&` (and the ports/tool equivalents), refuse an incomplete record with `SPOKE_CONNECT_INVALID_ARGUMENT`, and release the pointer only on C success. The carrier then owns the record and runs its `destroy` once; capture what a callback needs by value. |
| Exceptions | With exceptions disabled the header contains no `try` / `catch` / `throw` and a callback reports failure through its `Result`; with exceptions enabled an escaping callback exception is contained into `SPOKE_CONNECT_TRANSPORT_IO` (transport) or `SPOKE_CONNECT_FFI_REJECTED` with `code="INTERNAL_ERROR"` (ports/tool). |

## Smoke

`tooling/connect/cpp-smoke.mjs` builds `Smoke/main.cpp` and
`Smoke/convenience.cpp` in `target/cpp-smoke`, links the staged native for the
requested RID, and runs them against the shared golden vector
`crates/spoke-connect/tests/fixtures/golden-hello.json`:

```sh
node tooling/connect/cpp-smoke.mjs --rid osx-arm64
node tooling/connect/cpp-smoke.mjs --rid win-x64
```

Each RID is built in two configurations — exceptions disabled (the consumer
default) and exceptions enabled — and each configuration must print its own
ordered banner list exactly:

```text
golden peer-id: PASS
golden hello signature: PASS
protocol version 1: PASS
loopback ports: PASS
rejection/ownership: PASS
C++ convenience values/core: PASS
C++ convenience callbacks/session: PASS
C++ convenience router: PASS
C++ smoke: PASS
```

The exceptions-enabled configuration additionally prints
`C++ callback exception containment: PASS` before the final line. A missing,
extra, repeated or reordered banner fails the run.

## Committed natives

| RID | Status |
|-----|--------|
| `osx-arm64` | **Committed** — built and staged by `tooling/connect/cpp-build.mjs` |
| `win-x64` | **Committed** — Windows x64 carrier and Rust-produced import library |

`tooling/connect/cpp-build.mjs` rebuilds a native from the integrated Rust
source and refreshes that RID's `provenance.json` entry with the revision,
toolchain and hashes of the shipped files. It accepts the two committed targets,
`aarch64-apple-darwin` and `x86_64-pc-windows-msvc`.

`node tooling/connect/cpp-build.mjs --verify --target <triple>` is the read-only
counterpart consumers run: it re-reads the committed entry and hashes the
committed header and staged native, so a copy that no longer matches the record
fails instead of being quietly re-recorded. The C contract header is pinned to
LF (`crates/spoke-connect/bindings/cpp/include/spoke_connect.h text eol=lf` in
`.gitattributes`) for that check's sake — the recorded `headerSha256` has to
describe the header's bytes on every platform, not the bytes one checkout's
end-of-line conversion happened to produce.

## Header durability

`tooling/connect/cpp-symbol-check.mjs` compares the header's declaration block
against the native's exported `spoke_connect_*` symbols in both directions, then
compiles a C99 probe holding a typed function pointer to every declaration and a
C++17 inclusion probe that includes `spoke_connect.h` and `spoke_connect.hpp`
(twice, covering repeated inclusion) and instantiates the convenience layer's
values, handle set and callback factories with exceptions and RTTI disabled — so
a header edit and its export land together, and a `.hpp` that stopped compiling
fails instead of degrading to a C-only pass. `--self-test` re-runs those probes
on temporary copies carrying a missing declaration, an invented symbol, a
mutated callback signature and a `.hpp` exception-syntax mutation.

It also pins the record and callback-table representation: the header's
`typedef struct` block is checked against the carrier's `#[repr(C)]` mirrors, and
one `_Static_assert` per `sizeof` / `_Alignof` / `offsetof` is compiled, so a
field reorder or resize fails the gate. The check needs `cargo` on `PATH`, since
the report it compiles against is produced by the carrier's own test target.

## Reference

- Consumer how-to: [docs/how-to/connect-cpp-binding.md](../../../../docs/how-to/connect-cpp-binding.md)
- C contract header: [`include/spoke_connect.h`](include/spoke_connect.h)
- C++17 convenience header: [`include/spoke_connect.hpp`](include/spoke_connect.hpp)
- Parity table: [`parity.md`](parity.md)
- Rust facade: [`../../src/ffi.rs`](../../src/ffi.rs)
