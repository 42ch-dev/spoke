# L2 closed KnowledgeEntry body and trait attributes

**Category:** architecture-patterns  
**Source:** compound 2026-07-26 (L2 typed body)  
**Status:** durable

## Problem

Baseline integrators need a protocol-typed general payload on KnowledgeEntry `body` — short blurb, labels, and scalar metadata — without an open bag and without requiring `l2-computable` maps. Product runtimes (Creader, Nexus) already carry summary/tags/attributes-shaped data; adapters need a stable wire mapping and a closed envelope so private keys cannot leak into core body.

## Decision

1. **Closed L2 body** — `knowledge-entry.schema.json` `body` has `additionalProperties: false`. Declared optional keys only: `summary` (string), `tags` (string[]), `attributes` (`BodyAttribute[]`), plus optional `state` / `computable` (`ComputableFieldMap` under `l2-computable`). Product-opaque bags go to `extensions.<namespace>` only.
2. **BodyAttribute** — shared def in `common.schema.json#/definitions/BodyAttribute`. Required `trait_type` (non-empty string) + `value` (`string` | `number` | `boolean` via schema `anyOf`). Optional `display_type`, `max_value`. Item closed (`additionalProperties: false`). **Duplicate `trait_type` allowed** (multi-valued traits).
3. **No nested trait values** — arrays/objects under `value` are out of core; use multiple traits or `extensions.<ns>`.
4. **Computable semantics unchanged** — SPOKE `body.computable` remains a Session projection map, not a product boolean. Product flags (e.g. Nexus `computable: bool`) stay adapter-local (`extensions.<ns>`).
5. **Pure read helpers** — `@42ch/spoke-operations` / `spoke-operations`: `listBodyAttributes` / `list_body_attributes`, `filterBodyAttributesByTraitType` / `filter_body_attributes_by_trait_type`, `findBodyAttribute` / `find_body_attribute`. Skip malformed array elements; never throw on missing/empty `attributes`.
6. **Schema inventory** — `BodyAttribute` is an in-place common definition; schema file count stays **23**.

## Adapter mapping (informative)

| Product shape | SPOKE |
|---------------|-------|
| Creader `description` / Nexus `summary` | `body.summary` |
| Creader / Nexus `tags` | `body.tags` |
| Creader typed metadata scalars / string[] | `body.attributes[]` (one trait per scalar or per array element) |
| Nexus object `attributes` | Flatten top-level scalars → trait array |
| Long-form `content`, chapter timelines, AI bags | `extensions.<ns>` or SourceAnchor — not invented body keys |
| Nexus `computable: bool` | **Not** SPOKE `body.computable` map |

## What not to do

- Do not reopen `body` with `additionalProperties: true` for product bags.
- Do not put long-form prose in a core `body.content` slot.
- Do not map product compute booleans onto SPOKE `body.computable` (`ComputableFieldMap`).
- Do not require `summary` / `tags` / `attributes` for every entry.

## Related

- Normative: `.mstar/specs/spoke-data-model.md`, `spoke-protocol-layers.md`, `spoke-operations.md` §13
- Computable maps: `architecture-patterns/l2-computable-session-model.md`
- Rust assemble wire preserve: `architecture-patterns/rust-spoke-operations-parity.md`
- Schemas: `schemas/data/knowledge-entry.schema.json`, `schemas/common/common.schema.json`
