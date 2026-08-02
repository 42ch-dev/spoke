---
title: Extensions & modules
---

# Extensions & modules

Every field an integrator adds to a SPOKE object belongs in one of three bags: **core fields** (protocol-owned, closed), **`modules.*`** (optional cross-product functional dialects), or **`extensions.<namespace>`** (one product's private bag). Placement authority is a normative ADR; this page is the quick reference.

## The triad

- **Core fields** — protocol identity and closed body envelope (`entry_id`, `body.summary`, `schema_version`); every baseline host reads and writes them.
- **`modules.*`** — optional, capability-flagged (`narrative-modules`) bag on KnowledgeEntry and AssemblePacket for cross-product functional dialects: `modules.activation` (lore activation), `modules.pack` (knowledge pack metadata), `modules.placement` / `modules.activation_trace` (assemble recipes). Inner shapes are handbook-defined; unknown module keys round-trip.
- **`extensions.<namespace>`** — required `ExtensionMap` on every durable data object; namespace keys are opaque product ids (`^[a-z][a-z0-9_-]*$`), values are opaque JSON. Adapters round-trip unknown namespaces and keys verbatim.

## Category rule

A **cross-product functional dialect** uses `modules.*`; **product data** uses `extensions.<product>`. Shared functional keys published under `extensions.*` would collide with the product-id reading of the namespace and with `HostCapabilityManifest.namespaces[]` exclusivity — so activation, pack, placement, and activation_trace live in `modules.*`.

## Host manifests

On `HostCapabilityManifest`, `extensions` carries deployment metadata; roles, capabilities, and namespace ownership are core manifest fields.

## Normative references

- [spoke-extension-modules.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-extension-modules.md) — the triad ADR: placement rules, round-trip, scope of authority
- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) — Extensions / Modules vocabulary
- [spoke-data-model.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-data-model.md) — extensions contract and ModuleMap on data objects
- [spoke-protocol-layers.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-protocol-layers.md) — `narrative-modules` capability flag
- [common.schema.json](https://github.com/42ch-dev/spoke/blob/main/schemas/common/common.schema.json) — `ExtensionMap` / `ModuleMap` definitions
