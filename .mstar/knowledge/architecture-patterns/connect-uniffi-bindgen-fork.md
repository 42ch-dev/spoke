---
module: spoke-connect
date: 2026-08-03
problem_type: architecture_pattern
category: architecture-patterns
severity: high
applies_when:
  - "a community uniffi bindgen lags the repo's pinned uniffi version"
  - "planning a Path B binding slice (C#, Go, Python, Kotlin, …)"
  - "stock --library generation fails metadata read against a well-formed cdylib"
  - "UDL fallback generates but runtime contract checksums reject every symbol"
tags: [spoke-connect, uniffi, bindgen-fork, path-b, csharp, ffi, version-gap]
---

# Vendored uniffi bindgen fork (version-gap bridge)

## Context

Community uniffi bindgen tools (C# / Go / Python / Kotlin) often lag the
repo's pinned uniffi line. Uniffi metadata encoding and runtime contract
checksums change between versions, so a bindgen built for an older uniffi
fails against a well-formed current `cdylib` even when the FFI surface itself
is correct. The binding-pipeline verification gate (generate → compile →
link/load → runtime checksum) catches this before a binding slice starts;
when the gap is real and small, a **vendored fork retargeted to the repo pin**
restores the locked `--library` path without dual-pinning the product cdylib
or hand-maintaining P/Invoke.

Reference instantiation: C# under
`crates/spoke-connect/bindings/csharp/bindgen/` against uniffi **0.32**,
with the decision record at [`.mstar/specs/connect-csharp-binding.md`](../../specs/connect-csharp-binding.md).
The same technique applies to any community bindgen that trails the pin.

## Guidance

### When to apply

Use the vendored-fork path when **all** of the following hold:

1. **Upstream still targets the older uniffi** — latest tag and `main` pin
   the lagging line (re-check tags + workspace `Cargo.toml` before every
   binding slice).
2. **Stock `--library` fails metadata read** against the repo's current
   cdylib (e.g. `Invalid string data` / invalid UTF-8 in `_UNIFFI_META_*`).
3. **Positive control passes** — the crate-local uniffi bindgen (or another
   language already on the pin) generates from the **same** cdylib, proving
   the metadata is well-formed and the failure is reader/version-specific.
4. **UDL fallback is insufficient** — generate + host compile may succeed,
   but the runtime checksum gate rejects symbols because checksums derive
   from the uniffi metadata model (module path + version-specific inputs)
   that a hand-written UDL cannot reproduce against a proc-macro surface.
5. **The retarget delta is small** — typically: bump workspace `uniffi*`
   deps to the repo pin, then fix non-exhaustive `Type` (or equivalent)
   match arms for interface additions in the newer uniffi. If the fork
   becomes a long-lived multi-crate carry across many upstream versions,
   escalate rather than silently expand scope.

Prefer this over:

| Alternative | Why fork wins when the delta is small |
|-------------|----------------------------------------|
| Dual-pin cdylib (older uniffi for the lagging language) | Second binary matrix; ffi / proc-macro divergence risk; easy to couple features into the main pin |
| Hand-written P/Invoke over exported symbols | No regen when the FFI surface changes; diverges from the generate pipeline other languages use |
| Downgrading the main uniffi pin | Breaks landed bindings and the locked Rust suite on the current pin |

### Isolation rules (HARD)

| Rule | Detail |
|------|--------|
| Generation-only | The fork builds a **bindgen CLI binary**. It is never linked into the runtime `cdylib`, never a path-dep of `spoke-connect`, and never shipped as a product runtime dependency. |
| Main pin untouched | The product crate's uniffi version stays at the repo lockstep pin. Do not dual-build the default `ffi` artifact on the lagging line. |
| Single cdylib | One uniffi-versioned `cdylib` serves every Path B language already on the pin (e.g. Swift + C#). The fork only changes how bindings are **generated** from that library. |
| Other languages untouched | Landed skeletons, their smokes, and `cargo test -p <crate> --features ffi` stay green without edits for the fork path. |
| Generated output gitignored | Language bindings regenerate from the documented recipe; commit the **patch + lockfile + recipe**, not a frozen generator tree inside the product crate (unless the language's pipeline already commits generated sources by policy). |

### Minimal-patch approach

1. **Pin upstream** at the exact lagging tag/commit (record SHA).
2. **Bump** workspace `uniffi`, `uniffi_bindgen`, `uniffi_meta`, `uniffi_udl`,
   and sibling crates from the lagging version → the repo pin.
3. **Compile-fix the Type-variant breaks** — newer uniffi adds enum variants
   (`Type::Box`, `Type::Set`, …). Add transparent / compound code-type arms
   and matching templates (e.g. `HashSet` RustBuffer converter for `Set`),
   mirroring patterns already present for `Sequence` / `Optional`. Touch only
   what the compiler and templates require; do not refactor bindgen behavior
   beyond the new variants.
4. **Capture the delta as a patch** plus a committed post-retarget
   `Cargo.lock` so clean rebuilds are reproducible.
5. **Prove the locked pipeline** — generate (`--library`) → host project
   build (zero warnings/errors) → runtime load with checksum gate satisfied
   → golden parity (peer_id, hello sign/verify, protocol version, or the
   language's equivalent smoke).

### Reproducibility

| Artifact | Role |
|----------|------|
| `*.patch` | Source-only delta against the pinned upstream commit |
| `*.Cargo.lock` (committed) | Resolved graph after the uniffi bump — copy into the clone before `cargo build --locked` |
| Recipe README | Clone → checkout SHA → `git apply` → copy lockfile → `cargo +nightly build --locked` → generate from **repo root** against the ffi-built cdylib → host smoke |

Build the fork with the repo's local Rust convention (nightly +
`-Zno-embed-metadata` per root `AGENTS.md`); CI stays on stable for product
crates. Prefer `--locked` so a clean machine does not re-resolve against the
live registry and silently pick a different uniffi patch.

### Drop-when-upstream-catches-up off-ramp

Re-check on every binding-adjacent change (or a periodic gate):

1. Upstream tag or `main` targets the repo's uniffi pin (or newer).
2. Stock bindgen `--library` against the current cdylib succeeds.
3. Host build + runtime checksum + golden smoke pass **without** the patch.

Then delete the patch, lockfile copy, and fork-specific recipe steps; point
the generate command at stock `uniffi-bindgen-<lang>`. Until that gate is
green, keep the fork. Document the drop trigger next to the binding decision
record so the next agent does not treat the fork as permanent product surface.

### Attempt order for a new language bindgen

1. Live upstream recheck (tags + workspace pins).
2. Stock `--library` against the repo cdylib.
3. Positive control with the crate-local / already-landed bindgen.
4. Vendored-fork spike (time-boxed; this doc).
5. Dual-pin only if the fork delta is large.
6. Hand P/Invoke last — and only for an explicitly scoped minimal surface.

## Why This Matters

Path B bindings are a compatibility matrix: one wrong reaction to a version
gap (downgrade the pin, dual-build forever, or hand-write the surface) either
regresses every landed language or freezes maintenance cost into the product
crate. A generation-only fork keeps the **runtime contract** single-versioned
and byte-stable while the **tooling** absorbs the lag. The minimal-patch +
committed lockfile pattern makes the carry cheap enough for an M-slice and
disposable the day upstream catches up. Go / Python / Kotlin face the same
feasibility gate before their slices start; this technique is the reusable
playbook when stock fails and the delta is small.

## When to Apply

- A community bindgen's latest release targets an older uniffi than
  `spoke-connect` (or any other FFI crate) pins.
- Planning or unblocking a Path B binding after the pipeline verification
  gate fails on metadata read or runtime checksum.
- Choosing among fork / dual-pin / hand P/Invoke for a binding M-slice —
  prefer fork when the Type-variant (or equivalent) delta is localized.

## Examples

### Reference layout (C# instantiation)

```
crates/spoke-connect/bindings/csharp/
  bindgen/
    README.md                         # clone → apply → --locked build → generate
    uniffi-bindgen-cs-0.32.patch      # minimal retarget delta
    uniffi-bindgen-cs-0.32.Cargo.lock # post-retarget lock for --locked
  generated/                          # gitignored; regenerate
  Smoke/                              # net8.0 golden parity
```

Normative decision record: [`.mstar/specs/connect-csharp-binding.md`](../../specs/connect-csharp-binding.md).
FFI boundary and language matrix:
[`connect-session-core-ffi-boundary.md`](connect-session-core-ffi-boundary.md).

### What the minimal patch typically contains

| Change class | Example |
|--------------|---------|
| Dep bump | workspace `uniffi*` 0.31.0 → 0.32.0 |
| Code-type arms | `Type::Box` → transparent inner; `Type::Set` → `HashSet` compound |
| Templates | new `SetTemplate` converter; exhaustiveness arms in `Types` template |

No product `ffi.rs` edits; no second cdylib feature for the lagging uniffi.

## See also

- [`connect-session-core-ffi-boundary.md`](connect-session-core-ffi-boundary.md) — pure core, sync FFI surface, binding-pipeline verification gate, language matrix.
- [`connect-csharp-binding.md`](../../specs/connect-csharp-binding.md) — C# landed path, stock failure modes, drop-fork trigger.
- [`connect-publish-staging.md`](../tooling-decisions/connect-publish-staging.md) — bindings stay unpublished from the protocol repo; record blockers with revisit triggers.
- [`connect-identity-parity-proof.md`](../testing-patterns/connect-identity-parity-proof.md) — golden-vector discipline the host-language smoke asserts.
