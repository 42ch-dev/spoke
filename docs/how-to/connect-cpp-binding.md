---
title: Connect from C and C++
---

# Connect from C and C++

The C and C++ channel consumes the shared connect **session core** through a hand-written C ABI: one header, `spoke_connect.h`, plus a committed dynamic library per platform. A C++17 host compiles against the header with `-fno-exceptions -fno-rtti` (MSVC `/EHs-c-` `/GR-`) and links the carrier. The boundary reports ABI revision `1` through `spoke_connect_abi_version`.

The carrier wraps the same public Rust facade the generated **native bindings** expose — the session core (`peer_id` derivation, hello sign/verify, allowlist, nonce store, sequence counters, response correlation, dispatch gate), `RemoteAdapter`, `MultiPeerRouter` and `ConnectResponder` with the `PortsHandler` and `ToolHandler` callbacks, and the in-memory loopback helpers. The parity table [`bindings/cpp/parity.md`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/cpp/parity.md) maps every production facade member, callback and error variant to its C declaration.

The header and the platform natives live in the repository and resolve together from the release tag `vX.Y.Z`: check out the tag you consume and take both from that revision.

## 1. Take the header and the native

| Path | Contents |
|------|----------|
| `crates/spoke-connect/bindings/cpp/include/spoke_connect.h` | The C contract: status values, value types, callback tables and every exported function |
| `crates/spoke-connect/bindings/cpp/native/osx-arm64/libspoke_connect_capi.dylib` | macOS arm64 carrier, install name `@rpath/libspoke_connect_capi.dylib` |
| `crates/spoke-connect/bindings/cpp/native/win-x64/spoke_connect_capi.dll` | Windows x64 carrier |
| `crates/spoke-connect/bindings/cpp/native/win-x64/spoke_connect_capi.dll.lib` | The Rust-produced import library the Windows link step consumes |
| `crates/spoke-connect/bindings/cpp/native/provenance.json` | Per RID: source revision, target, `rustc -Vv`, compiler version, build flags, header and artifact hashes |
| `crates/spoke-connect/bindings/cpp/README.md` | The binding README with the full calling contract |

The header compiles as C99 (fixed-width integers from `<stdint.h>`, lengths from `<stddef.h>`) and uses `extern "C"` inclusion for C++. Windows functions and callback pointers use `__cdecl`; macOS uses the platform default C calling convention. Every public symbol is prefixed `spoke_connect_`.

## 2. Compile and link

macOS arm64 links the dylib and points the loader at its directory:

```sh
clang++ -std=c++17 -fno-exceptions -fno-rtti \
  -I crates/spoke-connect/bindings/cpp/include \
  host.cpp crates/spoke-connect/bindings/cpp/native/osx-arm64/libspoke_connect_capi.dylib \
  -Wl,-rpath,<absolute path to native/osx-arm64> \
  -o host
```

Windows x64 links the import library, pairs the release C++ CRT (`/MD`) with the carrier's Rust dynamic CRT, and loads the DLL from the executable's directory:

```bat
cl.exe /nologo /std:c++17 /EHs-c- /GR- /MD /W4 /WX /I<absolute path to include> ^
  host.cpp <absolute path to native\win-x64\spoke_connect_capi.dll.lib> /Fe:host.exe
copy /Y <absolute path to native\win-x64\spoke_connect_capi.dll> .
host.exe
```

The validated Windows pairing is `/MD`: the release C++ CRT matches the carrier's Rust dynamic CRT.

## 3. Call the ABI

| Concern | Contract |
|---------|----------|
| Status | Every call returns `int32_t`: `SPOKE_CONNECT_OK` (0) on success, or a `SPOKE_CONNECT_*` value with the detail in the caller's error record. Failed calls preserve caller ownership of the out values. |
| Out parameters | Zero-initialize each out value and the `SpokeConnectError` record before the call; the library fills the fields the result carries. |
| Owned results | Release `SpokeConnectBuffer` with `spoke_connect_buffer_free`, `SpokeConnectOptionalBuffer` with `spoke_connect_optional_buffer_free`, and `SpokeConnectError` with `spoke_connect_error_free`. |
| Borrowed inputs | `SpokeConnectSlice` borrows bytes for the duration of the call; the length is authoritative, text is validated UTF-8, and keys are 32 raw bytes. |
| Handles | Each handle owns one carrier object and releases with `<object>_free` (releasing a null handle is idempotent); `close` ends a session and leaves the release to `<object>_free`. Constructors and the router borrow the handles they are given. |
| Callback tables | `spoke_connect_transport_new`, `spoke_connect_ports_handler_new` and `spoke_connect_tool_handler_new` copy the table and take ownership of `user_data` on success, running its `destroy` once after the last reference and in-flight callback. |
| Returned buffers | A callback hands ownership of a populated `SpokeConnectForeignBuffer` to the carrier, which copies the bytes and calls the buffer's `release` exactly once. |
| Threading | Calls block the calling host thread; callbacks run on the carrier's blocking pool and may run concurrently, so host contexts are thread-safe. |
| Lifetime | Load the carrier for the process lifetime, and close sessions and release handles before host shutdown. |

Status values group the failure vocabulary:

| Range | Meaning |
|-------|---------|
| 0 | `SPOKE_CONNECT_OK` |
| 1 / 2 | `SPOKE_CONNECT_INVALID_ARGUMENT` (C-boundary validation) / `SPOKE_CONNECT_PANIC` (a contained panic) |
| 100–107 | Core errors: invalid hello signature, nonce replay, handshake failed, invalid nonce, crypto, JCS, token invalid, protocol version mismatch |
| 200–202 | Core invoke errors: sequence exhausted, inbound sequence mismatch, correlation mismatch |
| 300 / 301 | Dial error / rejected (application and dispatch refusals, with `code`, `kind` and `wire_code` preserved where the mapping defines them) |
| 400 / 401 | Transport closed / transport I/O |

## 4. Run the golden-parity smoke

`tooling/connect/cpp-smoke.mjs` compiles the committed `Smoke/main.cpp`, links the staged native for the requested RID, and runs it against the shared golden vector `crates/spoke-connect/tests/fixtures/golden-hello.json`:

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

Each banner follows the assertions of its group; the run stops at the first failed check and exits with a non-zero status. The Windows lane [`.github/workflows/cpp-connect.yml`](https://github.com/42ch-dev/spoke/blob/main/.github/workflows/cpp-connect.yml) compiles, links and runs the same smoke with MSVC on every pull request and main push that touches the carrier, its header, the smoke or the build/check/smoke scripts.

## 5. Consume the carrier from Unreal Engine

The Unreal Engine module reference ([`bindings/cpp/ue/SpokeConnect.Build.cs`](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/cpp/ue/SpokeConnect.Build.cs) and its [README](https://github.com/42ch-dev/spoke/blob/main/crates/spoke-connect/bindings/cpp/ue/README.md)) declares an external module that puts the header on the include path and links and stages the committed carrier for the target platform. UnrealBuildTool discovers modules in a project's or plugin's `Source/` tree, so vendor the `cpp/` tree as a unit — for example `MyProject/Source/SpokeConnect/`, containing `ue/SpokeConnect.Build.cs` beside `include/` and `native/` — and add the module to the consuming module's dependencies:

```csharp
PublicDependencyModuleNames.AddRange(new string[] { "SpokeConnect" });
```

| Target | Link | Runtime staging |
|--------|------|-----------------|
| `Win64` (x86_64) | `native/win-x64/spoke_connect_capi.dll.lib` through `PublicAdditionalLibraries` | `RuntimeDependencies.Add("$(TargetOutputDir)/spoke_connect_capi.dll", …)` copies the DLL beside the executable |
| `Mac` (arm64) | `native/osx-arm64/libspoke_connect_capi.dylib` through `PublicAdditionalLibraries` | `RuntimeDependencies.Add(…, StagedFileType.NonUFS)` keeps the dylib loose beside the executable |

The module accepts `Win64` with `x86_64` and `Mac` with `arm64`, reads the target architecture through the `UnrealArch` API (UE 5.2 onward), and contributes include paths plus link/staging wiring — the consuming module calls the C ABI functions it needs, owns the transport implementation, and decides which engine thread consumes callback results. A carrier call blocks the calling OS thread, so call the ABI from worker threads; callbacks arrive on the carrier's blocking pool and may run concurrently. Keep the carrier loaded for the process lifetime, close sessions, then release handles before host shutdown.

Validation status: the executed evidence is the standalone C++17 smoke for `osx-arm64` and `win-x64` (step 4); the module README lists the engine-side checks — editor build and load, packaged loose-library load, one connected session, orderly shutdown, target settings — as the maintainer checklist a machine with an engine environment runs.

## 6. Keep the header and the exports in step

`tooling/connect/cpp-symbol-check.mjs` is the executable drift gate: it parses the header's declaration block, compares it against the carrier's exported `spoke_connect_*` symbols in both directions, checks the header's `typedef struct` block against the carrier's reported record layouts, compiles a C99 probe holding a typed function pointer to every declaration, and compiles a C++17 inclusion check with `-fno-exceptions -fno-rtti`.

```sh
node tooling/connect/cpp-symbol-check.mjs \
  --header crates/spoke-connect/bindings/cpp/include/spoke_connect.h \
  --library crates/spoke-connect/bindings/cpp/native/osx-arm64/libspoke_connect_capi.dylib
```

```text
C ABI symbols: PASS (83 declarations, 83 exports, 0 missing, 0 extra)
Record layout: 11 records, 66 assertions (sizeof/_Alignof/offsetof) match the carrier mirrors
C probe (clang -std=c99 -Wall -Wextra -Werror): PASS
C++17 inclusion (-fno-exceptions -fno-rtti): PASS
```

The `C++17 inclusion` line is an equivalent restatement of the gate's check label using the exact compilation flags.

## Next steps

- [Bridge a native binding to RemoteAdapter](/how-to/remote-adapter-native-binding) — the adapter, router, responder, ports and tool surface the C ABI carries.
- [Connect wire reference](/reference/connect) — envelope field tables and identity binding.
- [Open your first connect session](/tutorials/first-connect-session) — the handshake flow every binding implements.
