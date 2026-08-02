---
module: fixtures/codegen
date: 2026-07-30
problem_type: knowledge
category: architecture-patterns
severity: low
plan_id: 2026-07-30-knowledge-pack-handbook
tags: [fixtures, ajv, conformance, proposed-wire, modules, toy-world, codegen]
applies_when: illustrating a companion pack / activation layout outside the harness enumerate set
---

# Companion fixture isolation (pack / modules)

## Context

SPOKE ships **baseline** wire schemas with `additionalProperties: false`, and a conformance harness (`fixtures/toy-world/tests/`, package `@42ch/spoke-fixture-toy-world`) that **auto-iterates** fixture JSON and asserts the enumerated set exactly equals `FIXTURE_SCHEMA_MAP`. Companion samples that carry product-envelope pack catalog fields (or other non-baseline roots) must not break that closed enumeration.

## Pattern

- Put companion files under `fixtures/toy-world/proposed/` — `readdirSync(FIXTURES_ROOT)` is **non-recursive**, so `proposed/` is never enumerated.
- Knowledge Pack catalog (`title` / `version` / `creator`) is **product-envelope** root fields on the companion file — not `modules.pack` on KnowledgeEntry.
- Per-entry `modules.activation` may appear on KE atoms in the companion when `narrative-modules` is on the wire; baseline atoms without `modules` remain individually conformant.
- Pack ≠ AssemblePacket — do not treat the companion pack file as an AssemblePacket sample.
- Mark the file with a top-level `_documentation` note; reference it from the handbook; do not add it to `FIXTURE_SCHEMA_MAP`.

## Example

- `fixtures/toy-world/proposed/pack_tw_harbor_companion.json` — product-envelope catalog + baseline-valid KE/Relation/SourceAnchor atoms + one entry's `modules.activation`. Referenced by `.mstar/specs/domain-profile-narrative-knowledge-pack.md`.

## Why This Matters

Without isolation, a companion sample either fails baseline AJV if placed in the enumerated set, or forces a premature container schema. The companion pattern lets handbooks show concrete interchange layouts while keeping the baseline wire closed and the harness authoritative.

## See also

- [`fixture-ajv-harness-outside-dist.md`](fixture-ajv-harness-outside-dist.md)
- [`spoke-codegen-pipeline.md`](spoke-codegen-pipeline.md)
- `.mstar/specs/spoke-extension-modules.md`
- `.mstar/specs/domain-profile-narrative-knowledge-pack.md`
