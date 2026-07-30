---
module: schemas/codegen
date: 2026-07-30
problem_type: knowledge
category: architecture-patterns
severity: low
plan_id: 2026-07-30-modules-wire-ship
tags: [modules, capability-flag, narrative-modules, ModuleMap, ExtensionMap, codegen, wire, ops]
applies_when: adding an optional cross-product functional-dialect bag (or any optional open namespace bag) to SPOKE durable objects behind a capability flag
---

# SPOKE capability-flagged optional namespace bag (`modules`)

## Context

SPOKE ships optional capabilities via **capability flags** (`l2-computable`, `l5-fork`) — opt-in fields declared in `spoke-protocol-layers.md` §Capability levels. Phase A W3 added a second open namespace bag, **`modules`** (cross-product functional dialects: `modules.activation`, `modules.pack`, `modules.placement`, `modules.activation_trace`), alongside the product-owned `extensions` bag. The demand-gate question ("ship now or wait for ≥2 consumers") was resolved by shipping `modules` **capability-flagged** so the first consumer (Nexus) gets a standard, round-trippable home without forcing all hosts or breaking baseline.

## Guidance — how to ship an optional open namespace bag

1. **Definition in `schemas/common/common.schema.json`**, sibling to `ExtensionMap`. For `ModuleMap`, values must accept **both** object (e.g. `modules.activation`) and array (e.g. `modules.placement`) namespaces without hard-coding inner field tables:
   ```jsonc
   "ModuleMap": { "type": "object",
     "additionalProperties": { "anyOf": [ { "type": "object", "additionalProperties": true }, { "type": "array" } ] },
     "propertyNames": { "pattern": "^[a-z][a-z0-9_-]*$" } }
   ```
   Inner shapes stay **handbook-defined**; the schema is an open envelope only.
2. **Optional field** on the durable objects: `$ref ModuleMap`, **NOT** in `required`; `additionalProperties: false` preserved. (Mirrors `Scope.extensions` — wire-optional via `#[serde(default, skip_serializing_if = …::is_empty)]`.)
3. **Capability flag** in `spoke-protocol-layers.md` §Capability levels (e.g. `narrative-modules`), worded like `l2-computable`/`l5-fork`: "optional `modules` on KnowledgeEntry + AssemblePacket; not `spoke-baseline`; adapters round-trip unknown namespaces verbatim."
4. **Pure ops helpers** `mergeModuleMaps` / `preserveModuleMaps` — generalize the namespace-merge **core** (`mergeNamespaceMaps` / `merge_json_values`): object namespaces deep-merge, array namespaces replace (last-write), unknown namespaces preserved verbatim. Extension helpers become thin wrappers over the same core (extension behavior byte-identical). **No engine/matcher/scoring.**
5. **Demand-gate resolution**: shipping capability-flagged satisfies the gate — baseline hosts need not emit/parse; only capability-declaring hosts do. Record the resolution in the triad ADR (`spoke-extension-modules.md`).
6. **Codegen**: a definition (not a new `.schema.json` file) → `assert-schema-count` stays unchanged (24). Regen TS + Rust; TS renders optional `?`; Rust renders `HashMap` with `#[serde(default, skip_serializing_if)]`.

## Why This Matters

This pattern lets SPOKE add a shared functional vocabulary (activation/pack/placement) to the wire **without** (a) breaking baseline, (b) forcing all consumers, or (c) hard-coding dialect field tables that should evolve via handbooks. It keeps the wire closed (`additionalProperties: false`) while giving cross-product dialects a standard, round-trippable home — so consumers don't invent interim product bags (`extensions.nexus.activation`) that later need migration.

## When to Apply

- A new cross-product functional dialect needs a wire carrier and the demand-gate has at least one credible consumer (Nexus + intent to standardize).
- An optional open namespace bag (object and/or array values) on durable objects, opt-in via capability.
- **Not** for product-specific data (use `extensions.<product>`) or baseline-required fields (core).

## See also

- [`spoke-extension-modules.md`](../../specs/spoke-extension-modules.md) — core / modules / extensions triad; demand-gate resolution.
- [`spoke-codegen-pipeline.md`](spoke-codegen-pipeline.md) — codegen inventory, `verify-codegen`, Rust typify optionality.
- [`proposed-wire-shape-companion-fixture.md`](proposed-wire-shape-companion-fixture.md) — staging proposed shapes before they ship.
- [`spoke-protocol-layers.md`](../../specs/spoke-protocol-layers.md) — capability levels (`l2-computable`, `l5-fork`, `narrative-modules`).
