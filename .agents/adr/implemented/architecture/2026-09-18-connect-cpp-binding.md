# ADR: Hand-written C ABI with a C++17 convenience layer for the connect binding

Status: implemented — C and C++ consumers reach the shared session core through a frozen C ABI instead of a generator toolchain

Date: 2026-09-18

Scope: the C/C++ native binding boundary for `spoke-connect` — carrier crate, C ABI header, C++17 convenience layer, native artifact shape and the executable drift gate

## Problem

C and C++ hosts — the Unreal Engine reference among them — need the shared Rust session core without a Rust toolchain, and they need a boundary that survives across compiler families, exception settings and game-thread constraints. The candidates differ on more than ergonomics: a generated C++ surface couples the consumer to a generator version, a C++ standard and a name-mangling ABI, while a hand-written C surface keeps the contract small but moves all drift detection onto maintainers.

The existing facts that framed the choice: wire, session core and transport are separate, so a native binding must preserve signed hello bytes and outcomes rather than re-implement rules; native calls are synchronous over a library-owned multi-thread Tokio runtime, so async code generation buys nothing; the Rust facade already exposes the required core, adapter, responder and router behaviour publicly.

## Decision

The C/C++ channel is a hand-written boundary over the public Rust facade:

- A workspace-private carrier crate (`spoke-connect-capi`, `publish = false`, `rlib` + `cdylib`) links `spoke-connect` with `ffi,remote-adapter` and reuses the public `spoke_connect::ffi` functions and objects. The carrier performs pointer validation, C/Rust value conversion, handle ownership and error projection only — no dispatch, cryptography, router selection, timeout or serving logic is copied.
- `include/spoke_connect.h` is a hand-written C99 header: opaque per-type handles, `int32_t` status plus a caller-supplied error record, borrowed slices, owned buffers freed by the producing library, callback tables with an explicit `user_data` and `destroy`, and ABI revision `1` through `spoke_connect_abi_version`. No packed structs, variadic calls, C++ types, ABI-sized enums or exported data variables.
- `include/spoke_connect.hpp` is a header-only C++17 convenience layer in namespace `spoke::connect`: move-only handle and buffer types, borrowed views with explicit copies, one `[[nodiscard]] Result<T>` error channel, and `std::function` callback bridges. It adds no exported symbol and no ABI revision, and requires neither exceptions nor RTTI.
- The carrier maintainer owns the Rust exports and both headers, and changes land together. Drift is caught by an executable gate (`tooling/connect/cpp-symbol-check.mjs`) that parses the marked declaration block, diffs it against the native's exported `spoke_connect_*` symbols in both directions, compiles a C99 probe holding a typed function pointer to every declaration, and compiles a C++17 inclusion probe over `.h` + `.hpp` with exceptions and RTTI disabled.
- Only dynamic consumer libraries are distributed, committed per RID with a git-lfs carrier object and a provenance record holding source revision, toolchain, build flags and header/artifact SHA-256 values.

## Alternatives considered

**The UniFFI C++ generator (`uniffi-bindgen-cpp`).** At the recorded version (`v0.8.1+v0.29.4`) it requires C++20 and does not support async functions. The C++20 requirement and the toolchain burden are a separate compatibility and supply-chain problem for a facade that is entirely synchronous — and it would still expose STL layout, C++ name mangling, exceptions and RTTI at the boundary. Rejected.

**Forking the C++ generator.** Rejected in favour of an explicit C boundary: a fork inherits the generator's ABI assumptions and adds a second binary to keep in step, while the C ABI can be written once against the facade that already exists. C++ ABI compatibility across compiler families then stops being a dependency of the design.

**Reusing UniFFI-generated C symbols or its serialization/checksum ABI.** Rejected: that is generator-internal surface, not a consumer contract. The carrier links the public `spoke_connect::ffi` objects instead, so envelope-auth internals stay encapsulated and no router `extract` is invented.

**A static library instead of dynamic carriers.** Rejected: a staticlib introduces a second native shape, platform transitive-link recipes and duplicate-runtime risk without a consumer requirement. The Swift xcframework stays a separate channel precedent, not a mandate here.

**A throwing C++ API, or a macro-switched API shape.** Rejected: the consumer default disables exceptions, and a single value-returned `Result` channel keeps one error API across exception-enabled and exception-disabled builds. A checked access on the wrong branch fails fast in every configuration, so the result stays the only error channel.

**C++20 integration, vcpkg/Conan, Release archives, extra platform artifacts, exposed async node lifecycle.** Excluded. Distribution stays git-based from the repository tag, the header stays the durable boundary, and independently loaded copies of the native library stay separate ownership domains rather than sharing handles.

## Consequences

- Two hand-maintained headers sit beside the Rust exports, so every facade change is a three-file change; the symbol gate, the parity table and the compiled probes are the only thing preventing quiet drift.
- The ABI revision number is a promise: additive members keep revision `1`, anything that changes calling convention, record layout, status values or ownership transfer bumps it and forces a carrier refresh.
- Error and status values are frozen integers, and C callbacks must contain their own exceptions — a C++ exception crossing the ABI, a process abort or an access violation is unrecoverable by design.
- Each RID needs its own provenance record with header and artifact hashes; a missing or mismatching hash fails closed instead of shipping unverified bytes.
- UE editor and packaged-game integration stay a maintainer checklist; the standalone smokes per RID are the recorded evidence, not an engine gate.
- Consumer-side tooling requirements are a C toolchain and, for the carrier libraries, git-lfs — never a Rust toolchain.

Related: [the connect wire contract](../../../specs/connect/spoke-connect.md), [the C ABI packaging contract](./2026-08-03-connect-binding-channels.md), [hand-written C ABI beside UniFFI](../../../knowledge/architecture-patterns/hand-written-c-abi-beside-uniffi.md).
