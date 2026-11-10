# Connect C# native bindings

Status: **landed**. The C# binding pipeline for the `spoke-connect` sync-core
facade is proven end-to-end: generated bindings plus a net8.0 golden-parity
smoke live under `crates/spoke-connect/bindings/csharp/`. Generation uses a
**vendored fork of `uniffi-bindgen-cs` retargeted to uniffi 0.32** (the repo's
cdylib pin); the fork is generation-only tooling and is dropped when upstream
tags a uniffi 0.32+ release. All five binding targets now live under
`crates/spoke-connect/bindings/` (C# / Swift / Kotlin / Go / Python) — see
[`connect-publish-strategy.md`](./connect-publish-strategy.md) §7 for the landed
channel matrix.

## Scope

Records the landed C# binding for the `spoke-connect` sync-core facade: the generated surface under `crates/spoke-connect/bindings/csharp/`, the pinned uniffi and bindgen versions, the NuGet package identity, and the regenerate → build → run sequence with its re-check gate. The evidence behind the vendored bindgen fork and the drop condition live in `.agents/adr/implemented/process/2026-08-02-connect-csharp-uniffi-bindgen-fork.md`. This page does not re-decide the fork, the channel split or the wire contract.

## Current state

### Current state

| Component | Version |
|-----------|---------|
| `crates/spoke-connect` (ffi surface) | uniffi **0.32.0** (proc-macro cdylib; locked — not downgraded) |
| NuGet package | **`42ch.Spoke.Connect`** on GitHub Packages — lockstep SemVer; generated C# + multi-RID natives |
| `uniffi-bindgen-cs` (NordSecurity) | **v0.11.0+v0.31.0** — the latest published tag; upstream `main` HEAD is the same commit (`e10ce410eb3a10cc19c7928b93ea8d84e038c034`) and its workspace pins `uniffi_bindgen`/`uniffi_meta`/`uniffi_udl` at **0.31.0** |

`uniffi-bindgen-cs` v0.11.0 targets uniffi-rs 0.31; no published tag or main
commit targets uniffi-rs 0.32. Generation therefore runs through the vendored
fork under `crates/spoke-connect/bindings/csharp/bindgen/`. The two stock
failure modes, the patch contents and the drop trigger are recorded in the
ADR.

### Regenerate → build → run

- Fork build recipe + patch: `crates/spoke-connect/bindings/csharp/bindgen/README.md`
- Smoke regenerate → build → run sequence: `crates/spoke-connect/bindings/csharp/Smoke/README.md`

### Drop the fork when upstream catches up

Re-verify on a `uniffi-bindgen-cs` tag (or main commit) targeting uniffi-rs
**0.32+** — stock `--library` then replaces the vendored build. Checked
2026-08-03: not yet (latest `v0.11.0+v0.31.0`, published 2026-06-23). The
regenerate → build → run sequence in the Smoke README is the re-check gate.

### Status of the binding matrix

- Swift: **landed** — uniffi 0.32, committed xcframework, macOS and iOS smoke, golden parity.
- C#: **landed** — generated binding + net8.0 golden-parity smoke + **`42ch.Spoke.Connect`** NuGet (GitHub Packages) via the vendored fork; fork dropped when upstream targets uniffi 0.32+.
- Kotlin: **landed** — GitHub Packages Maven (`dev.42ch:spoke-connect`), JNA natives, Linux smoke.
- Go: **landed** — Go modules under `crates/spoke-connect/bindings/go`, golden-parity smoke.
- Python: **landed** — PyPI `spoke-connect` platform wheels via Trusted Publishing, golden-parity smoke.


## Invariants

- The `spoke-connect` cdylib pin is uniffi **0.32.0**, locked and never downgraded; the single cdylib serves both C# and Swift.
- Generated C# is committed (`generated/spoke_connect.cs` is the reference) and regenerated only when the FFI surface changes; consumers never run bindgen.
- The vendored `uniffi-bindgen-cs` fork is generation-only tooling that restores the `--library` CLI form against the 0.32 cdylib; it never changes the product cdylib's uniffi pin.
- The generated surface must match the complete FFI inventory in [`connect-publish-strategy.md`](./connect-publish-strategy.md). The core subset has 8 functions, 3 objects and 2 error enums; the full binding also includes the remote-adapter FFI, loopback helpers, callback interfaces and their errors.
- Golden parity asserts the Rust fixtures: golden `peer_id` `12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf`, the golden hello signature, verify, and protocol version `1`.
- The fork is dropped when an upstream `uniffi-bindgen-cs` tag or main commit targets uniffi-rs 0.32+; until then the regenerate → build → run sequence in the Smoke README is the re-check gate.

## Where it lives

- `crates/spoke-connect/bindings/csharp/` — packable csproj, committed `generated/spoke_connect.cs`, `runtimes/<rid>/native/` (CI-assembled, gitignored).
- `crates/spoke-connect/bindings/csharp/bindgen/` — fork patch (`uniffi-bindgen-cs-0.32.patch`), lockfile and build recipe (`README.md`).
- `crates/spoke-connect/bindings/csharp/Smoke/README.md` — regenerate → build → run sequence and the golden-parity smoke.
- `crates/spoke-connect/Cargo.toml` — the uniffi 0.32 pin, `ffi` feature, `rlib`/`cdylib` targets.
- `.github/workflows/release.yml` — `build-connect-ffi` matrix and the `publish-nuget` job for `42ch.Spoke.Connect` on GitHub Packages.

## Change obligations

- The FFI surface changes → regenerate `generated/spoke_connect.cs` with the vendored fork, re-run the net8.0 golden-parity smoke, and update `parity`-style counts if the surface inventory moves.
- The uniffi pin changes → re-check upstream `uniffi-bindgen-cs`, update the version table in §Current state, and either drop the fork or refresh the patch in the same PR.
- An upstream release targets uniffi-rs 0.32+ → verify stock `--library`, then delete the vendored fork, its patch and the fork recipe, and update §Current state plus the drop-condition note.
- The NuGet identity, feed or RID set changes → update §Current state and `.agents/specs/connect/connect-publish-strategy.md` together.
- A new binding target reuses this path as template → record the feasibility-gate outcome before committing generated sources (the C# outcome is the template).
