---
module: fixtures/codegen
date: 2026-07-30
problem_type: knowledge
category: architecture-patterns
severity: low
plan_id: 2026-07-30-knowledge-pack-handbook
tags: [fixtures, ajv, conformance, modules, toy-world, codegen]
applies_when: keeping illustrative pack/modules layouts out of the harness enumerate set
---

# Companion fixture isolation (pack / modules)

## Context

SPOKE ships **baseline** wire schemas with `additionalProperties: false`, and a conformance harness (`fixtures/toy-world/tests/`) that **auto-iterates** fixture JSON under `fixtures/toy-world/` (non-recursive) against `FIXTURE_SCHEMA_MAP`. Product-envelope pack catalogs and other non-baseline roots do not belong in that enumerate set.

## Pattern

- Prefer handbook prose + baseline-valid atoms under `fixtures/toy-world/` for Knowledge Pack atom patterns.
- Do **not** add product-envelope pack samples to the harness enumerate path.
- Pack catalog (`title` / `version` / `creator`) is product-transport — Pack ≠ AssemblePacket.
- Per-entry `modules.activation` on KnowledgeEntry is valid when declaring `narrative-modules`; illustrate in handbooks when a dedicated companion file is unnecessary.

## Why This Matters

A companion pack file that only restates handbook catalog fields + duplicated Harbor atoms is redundant once pack metadata is demoted from ModuleMap. Keep the harness corpus lean; document interchange in Domain Profile handbooks.

## See also

- [`fixture-ajv-harness-outside-dist.md`](fixture-ajv-harness-outside-dist.md)
- `.mstar/specs/domain-profile-narrative-knowledge-pack.md`
- `.mstar/specs/spoke-extension-modules.md`
