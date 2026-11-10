---
module: debugging / test failures
date: 2026-08-18
problem_type: testing_pattern
category: testing-patterns
severity: low
plan_id: local-ci-hygiene
tags:
  - typecheck
  - build-order
  - falsification
  - root-cause
  - error-inventory
  - ab-test
---

# Falsify the error classification before planning surgical fixes

## Context

A failing local gate produced a 27-row typecheck error inventory. A careful read classified the rows into "genuine source errors" (plan surgical fixes) and "build-order symptoms" (fix ordering). The classification was wrong: every row — including the "genuine" ones — was a single build-order collapse (a package resolved its dependency's types from `dist/`, which did not exist on a fresh checkout).

## Guidance

Before planning fixes from an error inventory, **falsify the classification with an artifact A/B**: run the identical failing command twice on the same tree, toggling only the suspected precondition (here: dependency `dist/` present vs removed). If the "genuine" errors appear and vanish with the precondition, they are symptoms — and the fix is the precondition, not the sources.

Signals that an error row is a collapse symptom rather than a genuine defect:

- The error type is `unknown`/`any`-adjacent (`TS18046`, `TS2322`, implicit-`any` `TS7006`) in code that consumes a package whose types failed to resolve (`TS2307` earlier in the log).
- The consuming package lacks a source path alias for the dependency (so it resolves types from `dist/`, which a fresh checkout does not have).
- The "genuine" errors sit downstream of a resolution failure in the same compilation.

## Why This Matters

Surgical fixes to symptomatic "errors" would have edited correct source code to placate a broken build order — real churn, wrong root cause, and the actual defect (ordering) left in place. One A/B run settled the classification with certainty that reading could not.

## When to Apply

Any multi-error build/typecheck failure where the fix plan assigns different root causes to different rows — especially monorepo chains with `dist/`-resolved types.

## Examples

- The `ci:typescript` case: with `packages/spoke-operations/dist` absent, demo typecheck emitted 27 rows including "genuine-looking" type errors; with `dist/` present, zero. The fix was stage reorder (`build:operations` before any typecheck that resolves it) — zero source edits.
