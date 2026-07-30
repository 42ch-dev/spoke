# SPOKE Extension Modules — Naming Triad

> **Status:** Normative ADR  
> **Document class:** Normative — bag placement authority  
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)  
> **Wire SSOT:** `schemas/` (this document does not change schema files)

## Purpose

State the three bags that place fields on SPOKE durable objects and ops envelopes, and the demand gate that promotes a proposed cross-product dialect onto the wire.

Integrators use this ADR to choose **core**, **proposed `modules.*`**, or **`extensions.<product>`** for every new field. Handbook authors cite it when documenting companion shapes that are not yet schema.

## The triad

| Bag | Audience | Wire today | Examples |
|-----|----------|------------|----------|
| **Core fields** | All baseline hosts | On the wire — closed protocol objects | `entry_id`, `body.summary`, `canonical_name`, `schema_version` |
| **Optional modules (`modules.*`)** | Cross-product **functional** dialects | **Proposed** — handbook companion shapes only until the demand gate | Proposed `modules.activation`, proposed `modules.pack` |
| **`extensions.<product>`** | One product / adapter | On the wire — required `ExtensionMap` on durable data objects | `extensions.nexus.world_id`, `extensions.toy.display_hint` |

### Core fields

Protocol-owned keys on closed objects (`additionalProperties: false` on the protocol object). Every baseline host reads and writes these shapes. Product-specific or lossy-round-trip data does not become a core field.

Field tables: [`spoke-data-model.md`](spoke-data-model.md), [`spoke-ops.md`](spoke-ops.md). Layer map: [`spoke-protocol-layers.md`](spoke-protocol-layers.md).

### Optional modules (`modules.*`) — proposed

Cross-product **functional** dialects shared by narrative hosts that need the same activation, pack, or similar companion shape. Examples under active handbook design:

| Proposed path | Role |
|---------------|------|
| `modules.activation` | Lore / knowledge activation keys, scan depth, and related trigger metadata |
| `modules.pack` | Narrative Knowledge Pack envelope metadata (title, version, creator) |

**Status on the wire:** `modules` is **proposed**. No `modules` key exists on any committed schema. Handbooks document companion field tables under `modules.*` so integrators implement identical shapes before a schema slice. Until the demand gate ships a schema property, readers treat every `modules.*` mention as a **proposed companion shape**, not an optional JSON property available now.

### `extensions.<product>`

Product- or adapter-owned bag. Namespace keys are opaque product / integrator ids matching `^[a-z][a-z0-9_-]*$`. Values are opaque JSON objects.

Shared schema fragment: [`schemas/common/common.schema.json#/definitions/ExtensionMap`](../../schemas/common/common.schema.json). Normative rules: [`spoke-data-model.md` §Extensions](spoke-data-model.md#extensions-normative), [`spoke-protocol.md` §Extensions](spoke-protocol.md#extensions).

On `HostCapabilityManifest`, `extensions` holds deployment metadata only — roles, capabilities, and namespace ownership are core manifest fields (`namespaces[]`), not KE product bags. See [`spoke-data-model.md` §HostCapabilityManifest](spoke-data-model.md#hostcapabilitymanifest-host-collaboration).

## Demand gate — promoting `modules` to the wire

A proposed `modules.*` dialect ships as a schema property **only when**:

1. **≥2 consumers** need an **identical** `modules` shape on the wire, **or**
2. **Nexus + one external host** both require the same shipped shape.

Until that gate opens:

| Surface | Expectation |
|---------|-------------|
| Handbooks | Publish proposed `modules.*` field tables and examples |
| `schemas/` | No `modules` property |
| Integrators | Stage functional dialects with the handbook shape; product-only query metadata stays in `extensions.<product>` |
| Capability flags | No baseline requirement to emit or parse `modules` |

When the gate opens, a later schema slice adds `modules` (capability or profile flag as designed then) and adapters round-trip the object per §Round-trip.

## Functional dialects vs product bags

| Placement | Correct for | Incorrect for |
|-----------|-------------|---------------|
| Core fields | Cross-host protocol identity and closed body envelope | Product DTO keys, one-host query filters |
| Proposed `modules.*` | Shared functional dialects (activation, pack, …) | One product’s private adapter state |
| `extensions.<product>` | One adapter’s product bag and product-local query metadata | Cross-product functional dialects shared by many hosts |

**Category rule:** a cross-product functional dialect uses **`modules.*`**. Product data uses **`extensions.<product>`**.

Publishing a shared functional key under `extensions.*` (for example treating `activation` or `pack` as an `extensions` namespace) is a **category error**:

| Why | Detail |
|-----|--------|
| Namespace exclusivity | `HostCapabilityManifest.namespaces[]` grants each product namespace to **at most one** `host_id` in a collaboration context. A shared functional name is not a product id and cannot own exclusivity the way `nexus` or `toy` does. |
| Reader expectation | `extensions.<ns>` reads as a **product / adapter** id. Functional dialect names collide with that reading. |
| Correct home | Functional dialects → proposed `modules.*`; product bags → `extensions.<product>`. |

Product interim storage of activation-like data under a **true product namespace** (e.g. `extensions.nexus.*`) remains product-local folklore until a shipped `modules` object exists; handbook shapes are the preferred staging path so multiple hosts converge on one companion layout.

## Round-trip / preserve

| Object | Rule |
|--------|------|
| `extensions.<product>` | Adapters **MUST** round-trip unknown namespaces and unknown keys inside a namespace **verbatim**. Merge/preserve helpers: [`spoke-operations.md`](spoke-operations.md) (`mergeExtensionMaps`, `preserveExtensionMaps`). |
| Future shipped `modules` | When a demand-gated schema slice adds `modules`, adapters **MUST** round-trip the `modules` object the same way — unknown module keys and nested fields survive read/write. |

Empty `extensions: {}` is valid. Core fields stay closed; extensions (and a future `modules` object) are the open product / dialect bags.

## Scope of authority

| This ADR owns | This ADR does not |
|---------------|-------------------|
| Normative **prose** placement of core / modules / extensions | Any new key in `schemas/**/*.json` |
| Demand gate for wire promotion of `modules` | Schema `description` field edits |
| Reject shared functional keys under `extensions.*` | Lore-activation or Knowledge Pack handbook field tables (sibling handbooks) |
| Round-trip expectation for extensions and future `modules` | Engines, pack I/O, registry behavior |

`extensions` on data objects is defined by `schemas/common/common.schema.json#/definitions/ExtensionMap`. Wire truth remains `schemas/`; this file is bag-placement authority only.

## Placement quick reference

| Need | Place it |
|------|----------|
| Identity / closed body / protocol selector | Core field (existing schema) |
| Lore activation keys shared across narrative hosts | Proposed `modules.activation` (handbook until demand gate) |
| Portable Knowledge Pack metadata | Proposed `modules.pack` (handbook until demand gate) |
| Product world id, UI hint, adapter DTO | `extensions.<your-product>` |
| Host roles / owned namespaces | `HostCapabilityManifest` core fields (`roles`, `namespaces[]`) |
| Deployment metadata on a host | `HostCapabilityManifest.extensions` |

## See also

| Doc | Topic |
|-----|-------|
| [`CONCEPTS.md`](../../CONCEPTS.md) | Vocabulary index — Extensions entry; Modules (proposed) when present |
| [`spoke-data-model.md`](spoke-data-model.md) | Data objects; §Extensions (normative); HostCapabilityManifest namespace exclusivity |
| [`spoke-protocol.md`](spoke-protocol.md) | Umbrella; §Extensions |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | L0–L8; Domain Profile vs core |
| [`spoke-ops.md`](spoke-ops.md) | Ops envelopes; optional top-level `extensions` |
| [`spoke-operations.md`](spoke-operations.md) | Extension merge/preserve helpers; host collaboration |
| Lore-activation handbook | Proposed `modules.activation` field tables (sibling handbook) |
| Narrative Knowledge Pack handbook | Proposed `modules.pack` + KE/Relation bundle (sibling handbook) |
