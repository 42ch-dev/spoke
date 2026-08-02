# Connect C# uniffi bindings — deferred (bindgen version gap)

Status: **deferred**. The C# binding pipeline for the `spoke-connect` sync-core
facade could not be proven end-to-end with the pinned toolchain; work stops at
the feasibility gate until the blocker below clears. Swift remains the landed
binding skeleton; the target-language matrix keeps C# as the next binding
target, pending the tooling trigger below.

## The gap

| Component | Version |
|-----------|---------|
| `crates/spoke-connect` (ffi surface) | uniffi **0.32.0** (proc-macro cdylib; locked — not downgraded) |
| `uniffi-bindgen-cs` (NordSecurity) | **v0.11.0+v0.31.0** — the latest published tag; upstream `main` HEAD is the same commit (`e10ce410eb3a10cc19c7928b93ea8d84e038c034`) and its workspace pins `uniffi_bindgen`/`uniffi_meta`/`uniffi_udl` at **0.31.0** |

`uniffi-bindgen-cs` v0.11.0 targets uniffi-rs 0.31; no published tag or main
commit targets uniffi-rs 0.32 (checked 2026-08-02). The version gap breaks the
pipeline in two independent ways:

1. **Generation (`--library` mode)** — the bindgen reads the exported
   `_UNIFFI_META_*` section of the cdylib to discover the surface. The uniffi
   0.32 metadata encoding is not readable by the 0.31 reader:
   `Error: extracting metadata for '_UNIFFI_META_SPOKE_CONNECT_CONSTRUCTOR_INBOUNDSEQUENCE_NEW' — Invalid string data (invalid utf-8 sequence of 1 bytes from index 1009)`.
   A positive control (the crate-local uniffi 0.32 `uniffi-bindgen` generates
   Swift from the same cdylib) confirms the cdylib metadata is well-formed; the
   failure is reader/version-specific.

2. **Runtime integrity gate (UDL-mode fallback)** — the sanctioned alternate
   CLI form (hand-written UDL mirror of the proc-macro surface) generates and
   compiles, but every generated binding carries a checksum expectation the
   0.32 cdylib cannot satisfy: all 14 exported symbols (8 functions, 3
   constructors, 3 methods) mismatch (`UniffiContractChecksumException`,
   e.g. `check_response_correlation` expected 37894, library 57062). The
   checksums are derived from the uniffi metadata model, which includes the
   Rust module path (`spoke_connect::ffi`) and version-specific inputs that a
   UDL cannot reproduce. The process loads the dylib and the contract-version
   check passes (30 == 30), but the integrity gate rejects every call before
   it executes.

## What was tried (both sanctioned attempts; results)

| Attempt | Command form | Result |
|---------|--------------|--------|
| Primary pin (AD-P0-1) | `uniffi-bindgen-cs <cdylib> --library` | FAIL — metadata read (see above) |
| Secondary A: upstream `main` HEAD | same CLI; main == tag commit `e10ce410`, still uniffi 0.31 | FAIL — identical metadata read |
| Secondary B: alternate CLI flag (UDL mode) | `uniffi-bindgen-cs <surface>.udl` | generate PASS, csproj compile PASS (net8.0, 0 warnings/errors), runtime FAIL — checksum gate on all 14 symbols |

No attempt downgraded `spoke-connect`'s uniffi pin; no generated bindings were
patched. Surface inventory of the generated (UDL-mode) output matches the
Binding facade: 8 functions + 3 objects + 2 error enums (7 + 3 exception
classes) — the gap is not surface drift, it is toolchain version skew.

## Revisit trigger

Re-attempt the C# pipeline when a `uniffi-bindgen-cs` tag (or main commit)
targets uniffi-rs **0.32+**. The locked CLI form (`--library`, out-dir
`bindings/csharp/generated/`) and the net8.0 smoke shape are documented in
`crates/spoke-connect/bindings/csharp/Smoke/README.md`; the regenerate → build
→ run sequence there is the re-check gate.

## Status of the binding matrix

- Swift: landed skeleton (uniffi 0.32, macOS smoke, golden parity).
- C#: **deferred** — see this record. C# remains the next target in priority;
  the binding work does not proceed until the tooling trigger above fires.
- Go / Python / Kotlin: not started; community bindgen tools must be verified
  against uniffi 0.32 with the same feasibility gate before binding work
  starts (the C# outcome is the template for that check).
