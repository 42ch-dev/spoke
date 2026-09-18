# SpokeConnect — Unreal Engine module reference

A reference module template for consuming the spoke-connect C ABI carrier from an
Unreal Engine project or plugin. `SpokeConnect.Build.cs` declares an external
module that adds the carrier header to the include path and links/stages the
committed native carrier for the target platform. The project supplies the
`.uproject`/plugin descriptor, the source that calls the C ABI, and the transport.

Validation status: standalone macOS/Windows evidence is recorded separately; UE
editor and packaged-game integration are unverified. See the checklist at the end
of this file.

## Layout

| Path | Contents |
|------|----------|
| `ue/SpokeConnect.Build.cs` | The module template: include path, link wiring, native staging, platform/architecture acceptance |
| `../include/spoke_connect.h` | The C contract the module puts on the include path (C99 language floor, ABI revision 1) |
| `../include/spoke_connect.hpp` | The C++17 header-only convenience layer over that contract, on the same include path: move-only ownership, borrowed views, structured `Result` values and the callback bridges |
| `../native/osx-arm64/libspoke_connect_capi.dylib` | macOS arm64 carrier, install name `@rpath/libspoke_connect_capi.dylib` |
| `../native/win-x64/` | Windows x64 carrier: `spoke_connect_capi.dll` and the Rust-produced `spoke_connect_capi.dll.lib` import library |
| `../native/provenance.json` | Per RID: source revision, target, toolchains, build flags and artifact hashes |

The module directory (`ue/`) is a sibling of `include/` and `native/`, and the
module file resolves both trees from its parent directory.

## Placing the module

UnrealBuildTool discovers modules in a project's or plugin's `Source/` tree, so
vendor the `cpp/` tree as a unit and keep its shape — for example
`MyProject/Source/SpokeConnect/`, containing `ue/SpokeConnect.Build.cs` beside
`include/` and `native/`. The module name is `SpokeConnect`; the consuming game
or plugin module adds it to its dependency list:

```csharp
PublicDependencyModuleNames.AddRange(new string[] { "SpokeConnect" });
```

The module contributes include paths and link/staging wiring — both headers land on the same include path, so engine code includes `spoke_connect.h` for the raw C ABI or `spoke_connect.hpp` for the C++17 layer. The consuming module calls the ABI functions it needs, owns the transport implementation and decides which engine thread consumes callback results.

## Calling the ABI from engine code

| Concern | Behaviour in an engine target |
|---------|-------------------------------|
| Language surface | `spoke_connect.hpp` is C++17 with no RTTI and needs no exception handling; the module's include path covers it. Every fallible call returns `[[nodiscard]] Result<T>` / `Result<void>` carrying the original status and a structured `Error`, and there is no throwing API. |
| Ownership | Buffers and handles are move-only: a destructor releases, `Buffer::view()` borrows the owned payload while the buffer lives, and `Buffer::str()` returns an owned copy when a value must outlive it. |
| Build configuration | The layer compiles with exceptions disabled and with exceptions enabled. Pick one configuration for the whole target: translation units that include the header in one linked image must not diverge, and the header defines no configuration macro. |
| Session end | Destructors never close. Close the adapter and responder explicitly, then the module's own queues, and release the handles before shutdown. |

## Platform wiring

| Target | Link | Runtime staging |
|--------|------|-----------------|
| `Win64` (x86_64) | `native/win-x64/spoke_connect_capi.dll.lib` through `PublicAdditionalLibraries` | `RuntimeDependencies.Add("$(TargetOutputDir)/spoke_connect_capi.dll", …)` copies the DLL next to the executable |
| `Mac` (arm64) | `native/osx-arm64/libspoke_connect_capi.dylib` through `PublicAdditionalLibraries` | `RuntimeDependencies.Add(…, StagedFileType.NonUFS)` keeps the dylib loose beside the executable |

The module accepts the two committed targets: `Win64` with `x86_64` and `Mac`
with `arm64`. `SpokeConnect.Build.cs` reads the target architecture through the
`UnrealArch` API (`Target.Architecture`), which UE exposes from 5.2 onward; on an
earlier engine, express the same acceptance check through that version's
architecture type.

The Epic Games guide [Integrating Third-Party Libraries into Unreal
Engine](https://dev.epicgames.com/documentation/en-us/unreal-engine/integrating-third-party-libraries-into-unreal-engine)
documents the mechanisms used here: module setup for an external module,
`$(TargetOutputDir)` staging through `RuntimeDependencies`, and `@rpath`
handling for macOS dylibs.

## macOS load path

The carrier is committed with the install name `@rpath/libspoke_connect_capi.dylib`
(`otool -D` reports it), which is the form UnrealBuildTool expects for a
third-party dylib. UnrealBuildTool adds rpath search paths for third-party
dylibs located outside a `Source/` tree; when the vendored tree sits inside
`Source/`, confirm the resolved rpath with `otool -L` on the built binary as part
of the checklist below.

## Windows CRT pairing

The Windows carrier is built against the Rust dynamic CRT
(`-C target-feature=-crt-static`). The validated pairing is the release dynamic
C++ CRT (`/MD`) on the consuming side; `/MDd` and `/MT` targets pair a different
runtime with the same carrier.

## Threading, transport and lifetime

| Concern | Behaviour in an engine target |
|---------|-------------------------------|
| Call threads | A carrier call blocks the calling OS thread until it returns, so call the ABI from worker threads owned by the consuming module. |
| Callbacks | Transport (`send` / `recv` / `close`), ports and tool callbacks arrive on the carrier's blocking pool and may run concurrently. Host contexts are thread-safe, and the host owns the queue that carries callback results to the engine thread that consumes them. |
| Transport | The host owns the transport implementation, the callback tables and their contexts. `recv` blocks for one envelope, and an idempotent `close` unblocks a pending `recv`. |
| Context lifetime | A callback context's `destroy` runs once, after the last carrier reference and in-flight callback. The carrier copies any bytes it retains when a callback returns. |
| Library lifetime | The carrier stays resident for the process lifetime, and the module keeps it loaded through link/staging wiring. Close sessions, then release handles, before host shutdown. |

The boundary rules are stated in full in the [C++ binding README](../README.md).

## Maintainer verification checklist

UE editor and packaged-game integration are unverified: no engine was available
while this reference was written, so every item below is open until a maintainer
with an engine environment runs it. The standalone evidence recorded separately
is the C++17 smoke of `../Smoke/main.cpp` and `../Smoke/convenience.cpp`, run per
RID through `node tooling/connect/cpp-smoke.mjs --rid osx-arm64` and
`--rid win-x64`; that smoke exercises both headers, the native carrier and the
session surface outside an engine.

1. **Editor build and load.** Build a project with the vendored module in a
   Development Editor configuration: the module resolves `include/` and
   `native/`, the target links, and the carrier loads. On macOS, confirm the
   built binary's rpath entry and `otool -L` output; on Windows, confirm
   `spoke_connect_capi.dll` lands beside the editor executable.
2. **Packaged loose-library load.** Package a build and start it: the staged
   `libspoke_connect_capi.dylib` (macOS, `StagedFileType.NonUFS`) or
   `spoke_connect_capi.dll` (Windows) loads from the packaged output directory.
3. **One connected session.** Run one real connect session from engine worker
   code over the host transport — handshake to established, a
   `BaselinePorts`/`RemoteAdapter` round trip, and a callback result reaching the
   engine-owned consumer.
4. **Orderly shutdown.** Close sessions, release handles, and confirm the
   callback contexts' `destroy` runs and the process exits with the carrier
   resident.
5. **Target settings.** Confirm the architecture acceptance check matches the
   engine version in use, and that the Windows target uses `/MD`.

## Reference

- C++ binding README and link recipes: [`../README.md`](../README.md)
- C ABI ⇄ facade parity table: [`../parity.md`](../parity.md)
- C contract header: [`../include/spoke_connect.h`](../include/spoke_connect.h)
- Native provenance: [`../native/provenance.json`](../native/provenance.json)
