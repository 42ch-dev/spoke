---
module: iteration orchestration / shared spec docs
date: 2026-09-17
problem_type: convention
category: conventions
severity: medium
tags: [parallel-plans, merge-conflicts, shared-docs, tail-append, schema-count, inventory-ownership, integration-merge]
---

# Parallel plans over shared spec docs — conflict-face pre-declaration and count ownership

## Context

Two plans implemented in parallel feature worktrees both appended rows to the same normative documents (`spoke-protocol.md`, `spoke-operations.md`, `spoke-data-model.md`) and both touched the schema-count story. Unmanaged, this produces merge conflicts that read as design disputes instead of mechanical insertions.

## Guidance

- **Pre-declare the conflict face** in the iteration compass (Global Constraints per plan): which shared files each plan may append to, and which lines/sections are exclusively theirs.
- **Tail-append**: each plan appends its rows/sections at the end of tables and section lists — never rewrites shared prose. Adjacent insertion points still conflict; resolving "keep both rows" is then mechanical.
- **Count constants have exactly one owner**: when a capability adds schema files, exactly one plan owns the `EXPECTED_SCHEMA_COUNT` / inventory bump and updates constants + inventory prose atomically with schemas + generated output. Plans adding **zero** files state "gains no delta — 32 at the recorded base, 34 once <owner-plan> integrates" and must **never reset an integrated count back to their own base value**.
- **Prose enumeration merges are semantic**: when two plans each extend the same capability-enumeration paragraph, the merge keeps every flag and every description sentence in one canonical order (baseline → layer flags → new families) — not a one-side win.
- Cross-checks after integration: grep for stale counts (both the old total and per-family breakdowns), grep that each plan's flag names appear in the shared tables.

## Why This Matters

The conflict surface of v0-iter041 was fully enumerated up front; both serial integration merges landed with only two line-level conflicts (one keep-both table, one enumeration merge) and no semantic surprise. Unowned count constants are the classic silent regression — a later plan "fixing" the count back to its base breaks CI verify on integrated branches.

## When to Apply

Any iteration with ≥2 plans writing shared tracked docs (protocol/data-model/operations specs, codegen constants, fixture READMEs).

## Examples

- v0-iter041 compass Risk Register + both plans' Global Constraints ("Inventory ownership" clauses); integration merges 9bcec1a (clean) and 13d7fae (two resolved conflicts, both keep-both).
