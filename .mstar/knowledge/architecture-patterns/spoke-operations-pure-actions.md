# Pure operations actions over wire types

> **Category:** architecture-patterns  
> **Source:** compound 2026-07-23 (operations-deepen); updated 2026-07-25 (rust-ops-parity); 2026-07-28 (timeline sequence)  
> **Packages:** `@42ch/spoke-operations` (npm), `spoke-operations` (crates.io)

## Problem

JSON Schema defines shapes; integrators still copy-paste lifecycle gates (OCC, status transitions, Scope filters, upsert/relate rules). Without shared pure helpers, products drift before adapters exist.

## Pattern

1. Hand-write pure helpers over generated wire types only — TypeScript (`@42ch/spoke-schemas`) or Rust (`spoke-schemas`).
2. Unify rejects via `SpokeResult` / `SpokeRejectCode` with **identical code strings** across languages (never throw/panic for expected rejects).
3. OCC is **compare-only** — caller supplies `expected` vs `actual` revisions; library never fetches storage.
4. Uniqueness / Scope gates take **caller-owned collections** + opaque `scope_key` / `scope_id` (no World/Book required fields).
5. Map library rejects ↔ ops `error-envelope` by **code string only** (no HTTP/MCP tables).

## Gotchas

- Uniqueness helpers that take `entry_type`/`canonical_name` **params** must cross-check against `candidate` wire fields (`INVALID_INPUT`) — otherwise callers can bypass the gate.
- Relate self-edge checks must use **trimmed** ids after emptiness validation.
- Create-path `canonical_name` must reject whitespace-only (`EMPTY_CANONICAL_NAME`), matching promote.

## Timeline sequence (moment filter / order)

Caller-supplied `TimelineEvent[]` + `Relation[]` helpers live under the `timeline` module (`sequence.ts` / `timeline.rs`): moment-scale filter, explicit id order, and `precedes` Kahn sort via dual-KE `extensions.spoke.timeline_entry_id`. Contracts: `.mstar/specs/spoke-operations.md` §14. Pattern: `architecture-patterns/beat-assist-moment-sequence.md`.

## See also

- `.mstar/specs/spoke-operations.md` §5–11, §14
- `architecture-patterns/adapter-injection-orchestration.md` — ports + injection orchestration over these helpers
- `architecture-patterns/rust-spoke-operations-parity.md` — Rust crate layout, typify body wire preservation, crates.io dep pin
- `architecture-patterns/beat-assist-moment-sequence.md` — dual KE + precedes ordering for beat sheets
- Residual R1 (operations deepen) — uniqueness param alignment
