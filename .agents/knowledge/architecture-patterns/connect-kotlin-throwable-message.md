---
module: spoke-connect
date: 2026-08-03
problem_type: architecture_pattern
category: architecture-patterns
severity: medium
applies_when:
  - "generating Kotlin uniffi bindings from spoke-connect CoreError"
  - "Kotlin compile fails with Conflicting declarations: val message on CoreException subclasses"
  - "choosing between Rust FFI field rename and language-local post-generate patch"
tags: [spoke-connect, uniffi, kotlin, throwable, ffi, path-b, post-generate]
---

# Kotlin uniffi `message` payload vs `Throwable.message`

## Context

uniffi 0.32’s Kotlin backend maps Rust error payload fields onto exception properties and also generates `override val message` for `Throwable`. When a Rust error variant exposes a field named `message`, stock Kotlin sources fail to compile (`Conflicting declarations: val message`). Swift, C#, Python, and Go tolerate the same Rust field name.

In SPOKE, `CoreError` variants `InvalidNonce`, `Crypto`, `Jcs`, and `TokenInvalid` use a `message: String` payload in `crates/spoke-connect/src/ffi.rs`. Renaming those fields in Rust would change the published C# NuGet public surface (`42ch.Spoke.Connect`) and every other binding at once.

## Guidance

### Prefer Kotlin-local post-generate rename

1. Generate stock Kotlin with crate-local `uniffi-bindgen --language kotlin` at the repo uniffi pin.
2. Run a documented post-generate script that renames the conflicting payload property `message` → `detail` on the four `CoreException` subclasses (and matching accessors).
3. Commit the patched sources under `crates/spoke-connect/bindings/kotlin/generated/`.
4. Keep the regenerate recipe in `bindings/kotlin/README.md` / `bindgen/README.md` so maintainers never hand-edit without re-running the patch.

Reference: `crates/spoke-connect/bindings/kotlin/bindgen/patch-kotlin-core-error-fields.sh`.

### When to rename Rust instead

Only when product accepts a **breaking** FFI/API change across all Path B languages (and published C#), with lockstep SemVer bump and regenerate-all. Prefer that path when multiple language backends share the same clash, or when Kotlin-only patches become a maintenance burden.

### Feasibility gate note

First-party Kotlin bindgen can still report **NO-GO (stock as-is)** while generate succeeds: the gate must include Gradle compile, not only metadata emission. A patched probe that loads JNA and passes golden parity proves the route before packaging commits.

## Why This Matters

Keeps Path B Kotlin packaging unblocked without breaking already-published C# consumers, and records a reusable pattern for other Throwable-sensitive language backends.
