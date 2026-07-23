# Pure operations actions over wire types

> **Category:** architecture-patterns  
> **Source:** compound 2026-07-23 (operations-deepen)  
> **Package:** `@42ch/spoke-operations`

## Problem

JSON Schema defines shapes; integrators still copy-paste lifecycle gates (OCC, status transitions, Scope filters, upsert/relate rules). Without shared pure helpers, products drift before adapters exist.

## Pattern

1. Hand-write pure TypeScript helpers over `@42ch/spoke-schemas` types only.
2. Unify rejects via `SpokeResult` / `SpokeRejectCode` (never throw for expected rejects).
3. OCC is **compare-only** — caller supplies `expected` vs `actual` revisions; library never fetches storage.
4. Uniqueness / Scope gates take **caller-owned collections** + opaque `scope_key` / `scope_id` (no World/Book required fields).
5. Map library rejects ↔ ops `error-envelope` by **code string only** (no HTTP/MCP tables).

## Gotchas

- Uniqueness helpers that take `entry_type`/`canonical_name` **params** must cross-check against `candidate` wire fields (`INVALID_INPUT`) — otherwise callers can bypass the gate.
- Relate self-edge checks must use **trimmed** ids after emptiness validation.
- Create-path `canonical_name` must reject whitespace-only (`EMPTY_CANONICAL_NAME`), matching promote.

## See also

- `.mstar/specs/spoke-operations.md` §5–11
- Residual R1 (operations deepen) — uniqueness param alignment
