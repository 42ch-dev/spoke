---
module: spoke-connect / C ABI carrier
date: 2026-09-18
problem_type: architecture_pattern
category: architecture-patterns
severity: medium
applies_when:
  - exposing a Rust library to C or C++ hosts (engine plugins, native game/desktop hosts)
  - deciding between a generated C++ binding and a hand-written C ABI
  - adding a third ABI verification surface beside an existing symbol gate
tags: [c-abi, ffi, carrier, opaque-handle, callback-table, symbol-gate, layout-gate, unreal-engine]
---

# Hand-written C ABI beside a generated binding surface

## Context

The connect crate already exported a uniffi surface (proc-macro, one cdylib, five generated language bindings). A C/C++ consumer — an Unreal Engine plugin — cannot use that surface: the available third-party C++ generator trails the pinned uniffi line (documented up to an older release, requires C++20, and reports async functions unsupported), and C++ ABIs are not stable across compiler families, which an engine's MSVC/Clang matrix makes a real constraint. The answer was a second, hand-written export face rather than another generator.

## Guidance

**Choose a C ABI when the consumer is an engine/toolchain, not a package manager.** A C ABI with opaque handles survives compiler, standard-library and RTTI/exception-setting differences that a generated C++ wrapper cannot. Reserve generated wrappers for hosts whose toolchain you control.

**Isolate the carrier so the existing surface is untouched.** A workspace-private crate (`publish = false`) that depends on the library with the existing FFI feature enabled keeps its own `crate-type = ["rlib", "cdylib"]`; adding `staticlib`/`cdylib` to the primary crate's `crate-type` would change every build's artifacts and timings.

**Wrap the public facade, never re-derive it.** The carrier implements the library's existing public callback traits and calls its public functions; it owns only pointer validation, value conversion, handle ownership and error projection. Duplicating dispatch, crypto, selection or timeout logic is the failure mode this rule prevents.

**Fix a small boundary vocabulary and keep it mechanical:**

- C99 (`<stdint.h>`/`<stddef.h>`), `extern "C"`, one symbol prefix, no packed structs, no variadic calls, no exported data.
- Borrowed values cross as `{pointer, length}` valid only for the synchronous call; empty is `NULL` + length 0; text is length-authoritative (never `strlen`).
- Owned output is a Rust-allocated buffer plus an explicit free function; the host reads but never reallocates or frees with its own allocator.
- Every call returns `int32_t` status and writes results through caller-supplied out-params, with an error record carrying text fields plus sequence `expected`/`actual`; release functions alone return `void`.
- Callbacks cross as a table of function pointers plus `void *user_data`, with a mandatory `destroy` whose ownership transfers on success only.
- Verify C++ ABI-independence claims with the real hostile flags: compile an inclusion check with exceptions and RTTI disabled.

**Gate the header on two axes, and prove the gate can fail.** Symbol-name/signature parity in both directions is not enough: representation drifts silently. Add `sizeof` / `_Alignof` / `offsetof` static assertions compiled from a report the carrier's own test target emits, and drive at least one deliberate mutation per axis (a field reorder in the header, a field reorder in the Rust mirror, an unparsable declaration) to non-zero exit. A gate with no failing control is not evidence.

**Give two export faces a neutral seam, not mutual imports.** When an adapter face and a responder face start importing each other's `pub(crate)` items for shared callback handles or output writers, extract those into one internal module and re-point both. Verify the move is behaviour-neutral: empty symbol diff, byte-identical header, unchanged check conclusions, and an import search that shows no production cross-face reference remains.

**State engine-side verification as unverified until an engine runs it.** A module skeleton (`ModuleType.External`, include paths, `PublicAdditionalLibraries`, `RuntimeDependencies` staging per platform directory) plus a standalone C++ smoke is the honest maximum without the engine; the smoke proves the ABI, not the engine.

## Why This Matters

The carrier is a compatibility contract with a lifetime: every record field, callback table slot and status value is public forever. Keeping it hand-written and minimal keeps that surface auditable, keeps the generated-binding surface free of engine toolchain constraints, and turns "the header and the library drifted" from a runtime crash into a red check.

## When to Apply

Any Rust library that must be consumable from an engine or an unmanaged native host (game engines, CAD/DCC plugins, embedded hosts), and any repo that already ships a generated binding surface and now needs one more, non-generated, consumer class.

## Examples

- Carrier layout: a private crate wrapping the public FFI facade, exposing `spoke_connect_*` C symbols, with committed per-RID dynamic libraries and a recorded provenance entry each.
- Gate command shape: `node tooling/connect/cpp-symbol-check.mjs --header <header> --library <carrier>` → symbol parity, record-layout assertions, C99 probe, C++17 inclusion check with `-fno-exceptions -fno-rtti`.
- Engine reference: an external-module `.Build.cs` selecting per-architecture link/staging wiring, documented as unverified engine-side until a maintainer runs it.

## Related

- `sync-block-on-async-ffi-bridge.md` — the runtime bridge the carrier reuses instead of re-creating (cdylib-owned tokio runtime, foreign callbacks on the blocking pool).
- `connect-session-core-ffi-boundary.md` — the pure-core/transport split that makes a thin export face possible.
- `connect-error-taxonomy-layering.md` — error projection rules the status/error-record mapping must respect.
- `ci-assembled-committed-native-artifacts.md` — how the per-RID artifacts the carrier ships are built and refreshed.
