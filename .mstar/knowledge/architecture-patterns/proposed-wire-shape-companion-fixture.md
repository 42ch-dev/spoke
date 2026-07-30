---
module: fixtures/codegen
date: 2026-07-30
problem_type: knowledge
category: architecture-patterns
severity: low
plan_id: 2026-07-30-knowledge-pack-handbook
tags: [fixtures, ajv, conformance, proposed-wire, modules, toy-world, codegen]
applies_when: illustrating a proposed wire shape (not yet on the baseline schema) inside the toy-world fixture corpus
---

# Proposed-wire-shape companion fixture isolation

## Context

SPOKE ships **baseline** wire schemas with `additionalProperties: false`, and a conformance harness (`fixtures/toy-world/tests/`, package `@42ch/spoke-fixture-toy-world`) that **auto-iterates** fixture JSON and asserts the enumerated set exactly equals `FIXTURE_SCHEMA_MAP`. When a Domain Profile handbook proposes a **future** wire dialect (e.g. `modules.activation`, `modules.pack`) that is intentionally **not** on the baseline schema yet (demand-gated), an illustrative sample must not break that closed enumeration or fail baseline AJV validation.

## Guidance

Place illustrative **proposed-shape** samples as sidecar companion files **outside** the harness enumerate path:

- Put them under `fixtures/toy-world/proposed/` (a sibling subdir). The conformance `readdirSync(FIXTURES_ROOT)` is **non-recursive**, so `proposed/` is never enumerated.
- Carry the proposed fields (e.g. `modules.pack`, `modules.activation`) on a **pack/companion envelope** or a documented companion annotation — **never** as a property on a baseline atom JSON object (baseline atoms have `additionalProperties: false`; a `modules` key on a KnowledgeEntry root would fail validation).
- Baseline atoms referenced by the companion (KnowledgeEntry / Relation / SourceAnchor) remain individually conformant.
- Mark the file **PROPOSED — not harness-validated; do not add to the AJV enumerate list** (top-level `_documentation` note + a README sentence).
- Reference the companion path from the handbook so integrators find it, but do not wire it into `FIXTURE_SCHEMA_MAP`.

`pnpm run test:fixtures` stays green and never attempts to validate the proposed-shape file.

## Why This Matters

Without isolation, a proposed-shape sample either (a) fails baseline AJV (`additionalProperties:false`) if placed in the enumerated set, or (b) forces a premature schema change to accommodate dialects that are still demand-gated. The companion pattern lets handbooks show concrete proposed shapes **now** while keeping the baseline wire closed and the harness authoritative.

## When to Apply

- A Domain Profile / handbook proposes a wire dialect (`modules.*`) that is **not** baseline schema yet.
- A fixture should illustrate that proposed shape over valid atoms.
- The dialect is gated (≥2 consumers) before promotion to wire (see `.mstar/specs/spoke-extension-modules.md`).

Do **not** use this pattern for baseline-valid samples — those belong in the enumerated set so the harness validates them.

## Examples

- `fixtures/toy-world/proposed/pack_tw_harbor_companion.json` — a Narrative Knowledge Pack sample: baseline-valid KE/Relation/SourceAnchor atoms + proposed `modules.pack` metadata + one entry's proposed `modules.activation` as a companion annotation. Referenced by `.mstar/specs/domain-profile-narrative-knowledge-pack.md`; excluded from the conformance enumerate set.

## See also

- [`fixture-ajv-harness-outside-dist.md`](fixture-ajv-harness-outside-dist.md) — where the AJV conformance harness lives and why it is outside `@42ch/spoke-operations`.
- [`spoke-codegen-pipeline.md`](spoke-codegen-pipeline.md) — codegen inventory + `verify-codegen` schema-count assert.
- `.mstar/specs/spoke-extension-modules.md` — core / proposed modules / extensions triad + demand gate.
