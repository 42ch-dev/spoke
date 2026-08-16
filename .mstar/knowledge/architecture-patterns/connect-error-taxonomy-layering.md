---
module: connect / error taxonomy
date: 2026-08-13
problem_type: architecture-decision
category: architecture-patterns
severity: high
plan_id: 2026-08-13-connect-version-mismatch-align
tags: [connect, error-taxonomy, protocol_version_mismatch, CoreError, ConnectError, RemoteAdapterError, FfiError, dual-ffi-surface, uniffi, ordinal-stability]
---

# Connect error taxonomy layering — CoreError → ConnectError → RemoteAdapterError → FfiError

## Context

When adding a new error kind to the SPOKE connect protocol, the error must propagate through four layers: session-core (`CoreError`), transport boundary (`ConnectError`), RemoteAdapter contract (`RemoteAdapterError`), and FFI surface (`FfiError`). Each layer has its own enum, and the FFI surface has **two** independent error representations.

## Guidance

### The four-layer chain

```
CoreError (session-core)
  → map_core_error (error.rs) → ConnectError (transport boundary)
    → RemoteAdapter dial discrimination → RemoteAdapterError (frozen contract)
      → map_dial_error (ffi.rs) → FfiError (FFI surface)
```

### The dual FFI surface (architect F3)

`ffi.rs` has **two** independent error surfaces:

| Surface | Type | Regen cost | Used by |
|---------|------|-----------|---------|
| `FfiError::Dial { kind: String }` | String value | **Zero** (new String, no enum variant) | `connect_remote_adapter_ffi` (primary dial path) |
| FFI `CoreError` mirror | `uniffi::Error` enum | **Requires regen** for all 5 bindings | standalone `verify_hello_ed25519` export |

When adding a new error kind: check **both** surfaces. The String-kind path is free; the uniffi mirror path requires regenerating all five native bindings.

### Ordinal stability

When adding a variant to a `uniffi::Error` enum, **append at the end** (after the last existing variant). Do not insert mid-enum — that shifts ordinals and breaks ABI compatibility with committed native bindings.

### The frozen contract rule

`RemoteAdapterError` is the frozen RemoteAdapter contract surface (`spoke-remote-adapter.md` D7). Adding a variant is a public-API change — update D7 in the same program increment.

### Checklist for new error kinds

1. Add `CoreError` variant in `core/error.rs`.
2. Emit at the detection point (e.g. `hello_crypto.rs` version gate).
3. Add explicit `map_core_error` arm in `error.rs` — without it, the match is non-exhaustive (compile error) or silently falls through (semantic bug).
4. Add `RemoteAdapterError` variant + `map_err` discrimination in `remote/remote_adapter.rs`.
5. Add `FfiError::Dial` kind String (zero regen) + `CoreError` mirror variant (regen required).
6. Regenerate all 5 native bindings (C#, Kotlin, Swift, Go, Python).
7. Update `spoke-remote-adapter.md` D7 + D12.
8. Update `spoke-connect.md` error-mapping section.
9. Add cross-language golden vector or parity test.
10. Close any related residuals.

## Why This Matters

The `protocol_version_mismatch` alignment (2026-08-13) required touching all four layers. The architect pass caught the dual FFI surface (F3) and the omitted `map_core_error` arm (F1) — both would have caused silent failures or compile errors. The four-layer chain is not obvious from reading any single file.

## When to Apply

- Adding a new error kind to the connect protocol
- Modifying the error taxonomy at any layer
- Regenerating FFI bindings (check ordinal stability)

## Examples

**protocol_version_mismatch (2026-08-13):**
- `CoreError::ProtocolVersionMismatch { reason: String }` in `core/error.rs`
- Emitted at `hello_crypto.rs:151` (version gate, before signature)
- `map_core_error` arm in `error.rs:30-31`
- `RemoteAdapterError::ProtocolVersionMismatch` in `remote/remote_adapter.rs`
- `FfiError::Dial { kind: "protocol_version_mismatch" }` (String, zero regen) + `CoreError` mirror variant ordinal 8 (regen all 5 bindings)
- Golden vector: `golden-hello-version-mismatch.json` (byte-identical TS + Rust)

## See also

- [`.mstar/specs/spoke-connect.md`](../specs/spoke-connect.md) — connect protocol spec (error mapping section)
- [`.mstar/specs/spoke-remote-adapter.md`](../specs/spoke-remote-adapter.md) — RemoteAdapter frozen contract (D7 dial kinds, D12 FfiError)
- [`mind-axis-ownership-boundary.md`](mind-axis-ownership-boundary.md) — ownership boundary pattern (settled home vs derivative)
