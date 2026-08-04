# Connect C# uniffi bindings — landed via vendored bindgen fork

Status: **landed**. The C# binding pipeline for the `spoke-connect` sync-core
facade is proven end-to-end: generated bindings plus a net8.0 golden-parity
smoke live under `crates/spoke-connect/bindings/csharp/`. Generation uses a
**vendored fork of `uniffi-bindgen-cs` retargeted to uniffi 0.32** (the repo's
cdylib pin); the fork is generation-only tooling and is dropped when upstream
tags a uniffi 0.32+ release. All five binding targets now live under
`crates/spoke-connect/bindings/` (C# / Swift / Kotlin / Go / Python) — see
[`connect-publish-strategy.md`](connect-publish-strategy.md) §7 for the landed
channel matrix.

## Current state

| Component | Version |
|-----------|---------|
| `crates/spoke-connect` (ffi surface) | uniffi **0.32.0** (proc-macro cdylib; locked — not downgraded) |
| NuGet package | **`42ch.Spoke.Connect`** on GitHub Packages — lockstep SemVer; generated C# + multi-RID natives |
| `uniffi-bindgen-cs` (NordSecurity) | **v0.11.0+v0.31.0** — the latest published tag; upstream `main` HEAD is the same commit (`e10ce410eb3a10cc19c7928b93ea8d84e038c034`) and its workspace pins `uniffi_bindgen`/`uniffi_meta`/`uniffi_udl` at **0.31.0** |

`uniffi-bindgen-cs` v0.11.0 targets uniffi-rs 0.31; no published tag or main
commit targets uniffi-rs 0.32 (re-checked 2026-08-03: latest tag == `main`
HEAD, workspace pins still 0.31.0). The version gap breaks the **stock**
toolchain in two independent ways, which is why the vendored fork exists:

1. **Generation (`--library` mode)** — the stock bindgen cannot read the
   exported `_UNIFFI_META_*` section of the 0.32 cdylib:
   `Error: extracting metadata for '_UNIFFI_META_SPOKE_CONNECT_CONSTRUCTOR_INBOUNDSEQUENCE_NEW' — Invalid string data (invalid utf-8 sequence of 1 bytes from index 1009)`.
   A positive control (the crate-local uniffi 0.32 `uniffi-bindgen` generates
   Swift from the same cdylib) confirms the cdylib metadata is well-formed;
   the failure is reader/version-specific.

2. **Runtime integrity gate (UDL-mode fallback)** — the alternate CLI form
   (hand-written UDL mirror of the proc-macro surface) generates and
   compiles, but every generated binding carries a checksum expectation the
   0.32 cdylib cannot satisfy: all 14 exported symbols (8 functions, 3
   constructors, 3 methods) mismatch (`UniffiContractChecksumException`,
   e.g. `check_response_correlation` expected 37894, library 57062). The
   checksums derive from the uniffi metadata model, which includes the Rust
   module path (`spoke_connect::ffi`) and version-specific inputs that a UDL
   cannot reproduce.

## What was tried (evidence behind the landed path)

| Attempt | Command form | Result |
|---------|--------------|--------|
| Stock primary pin | `uniffi-bindgen-cs <cdylib> --library` | FAIL — metadata read (see above) |
| Stock secondary A: upstream `main` HEAD | same CLI; main == tag commit `e10ce410`, still uniffi 0.31 | FAIL — identical metadata read |
| Stock secondary B: UDL mode | `uniffi-bindgen-cs <surface>.udl` | generate PASS, csproj compile PASS (net8.0, 0 warnings/errors), runtime FAIL — checksum gate on all 14 symbols |
| **Vendored fork (chosen)** | fork `--library` against the 0.32 cdylib | **PASS — landed** (generate → build → run golden parity) |

The fork is a 129-line source patch (5 files): workspace uniffi deps
0.31.0 → 0.32.0 plus the uniffi 0.32 interface additions (`Type::Box`
transparent, `Type::Set` via a new `SetTemplate.cs` HashSet converter).
It restores the locked `--library` CLI form against the 0.32 cdylib; the
generated bindings load, pass the contract-version and checksum integrity
gates, and reproduce the Rust golden vectors: `peer_id`
`12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf`, hello signature,
verify, protocol version 1. No attempt downgraded `spoke-connect`'s uniffi
pin; the generated surface matches the FFI inventory exactly (8 functions + 3
objects + 2 error enums — 7 + 3 exception classes). The Rust suite and Swift
bindings are untouched (single 0.32 cdylib serves both languages).

## Regenerate → build → run

- Fork build recipe + patch: `crates/spoke-connect/bindings/csharp/bindgen/README.md`
- Smoke regenerate → build → run sequence: `crates/spoke-connect/bindings/csharp/Smoke/README.md`

## Drop the fork when upstream catches up

Re-verify on a `uniffi-bindgen-cs` tag (or main commit) targeting uniffi-rs
**0.32+** — stock `--library` then replaces the vendored build. Checked
2026-08-03: not yet (latest `v0.11.0+v0.31.0`, published 2026-06-23). The
regenerate → build → run sequence in the Smoke README is the re-check gate.

## Status of the binding matrix

- Swift: landed skeleton (uniffi 0.32, macOS smoke, golden parity).
- C#: **landed** — generated binding + net8.0 golden-parity smoke + **`42ch.Spoke.Connect`** NuGet (GitHub Packages) via the vendored fork; fork dropped when upstream targets uniffi 0.32+.
- Go / Python / Kotlin: not started; community bindgen tools must be verified against uniffi 0.32 with the same feasibility gate before binding work starts (the C# outcome is the template for that check).
