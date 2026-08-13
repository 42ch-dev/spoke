# SPOKE Extension Modules — Naming Triad

> **Status:** Normative ADR  
> **Document class:** Normative — bag placement authority  
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)  
> **Wire SSOT:** `schemas/` (optional `modules` / `ModuleMap` on KnowledgeEntry + AssemblePacket + TimelineEvent)

## Purpose

State the three bags that place fields on SPOKE durable objects and ops envelopes, and the rule that keeps cross-product functional dialects in `modules.*` while product bags stay in `extensions.<product>`.

Integrators use this ADR to choose **core**, **`modules.*`**, or **`extensions.<product>`** for every new field. Handbook authors cite it when documenting inner dialect field tables under the shipped `modules` envelope.

## The triad

| Bag | Audience | Wire today | Examples |
|-----|----------|------------|----------|
| **Core fields** | All baseline hosts | On the wire — closed protocol objects | `entry_id`, `body.summary`, `canonical_name`, `schema_version` |
| **Optional modules (`modules.*`)** | Cross-product **functional** dialects | **Optional, capability-flagged (`narrative-modules`; `l5-mind` for event observation)** — open `ModuleMap` on KnowledgeEntry + AssemblePacket + TimelineEvent | `modules.activation`, `modules.placement`, `modules.activation_trace`, `modules.mental`, `modules.belief`, `modules.observation` |
| **`extensions.<product>`** | One product / adapter | On the wire — required `ExtensionMap` on durable data objects | `extensions.nexus.world_id`, `extensions.toy.display_hint` |

### Core fields

Protocol-owned keys on closed objects (`additionalProperties: false` on the protocol object). Every baseline host reads and writes these shapes. Product-specific or lossy-round-trip data does not become a core field.

Field tables: [`spoke-data-model.md`](spoke-data-model.md), [`spoke-ops.md`](spoke-ops.md). Layer map: [`spoke-protocol-layers.md`](spoke-protocol-layers.md).

### Optional modules (`modules.*`) — capability-flagged

Cross-product **functional** dialects shared by narrative hosts that need the same activation, placement, or similar companion shape. Examples under handbook-defined inner shapes:

| Path | Role |
|------|------|
| `modules.activation` | Lore / knowledge activation keys and related trigger metadata (KnowledgeEntry) |
| `modules.placement` | Packet-level injection placement hints (AssemblePacket) |
| `modules.activation_trace` | Packet-level activation provenance (AssemblePacket) |
| `modules.mental` | Actor / group mental-state fields, nine-field vocabulary — durable authority (KnowledgeEntry) |
| `modules.belief` | Per-proposition belief labels, seven closed dimensions — durable authority (KnowledgeEntry) |
| `modules.observation` | Event observation metadata — who could perceive + access constraints (TimelineEvent) |

**Not a `modules.*` dialect:** Narrative Knowledge Pack **catalog metadata** (`title` / `version` / `creator`) lives on the **product transport envelope** that wraps atoms for import/export. Pack ≠ AssemblePacket (durable library interchange vs ephemeral assemble output). Do not place pack catalog fields on KnowledgeEntry or AssemblePacket `modules`. See [`domain-profile-narrative-knowledge-pack.md`](domain-profile-narrative-knowledge-pack.md).

**Status on the wire:** optional `modules` (`ModuleMap`) is **shipped** on `KnowledgeEntry`, `AssemblePacket`, and `TimelineEvent` (event observation). The bag is **capability-flagged** (`narrative-modules` in [`spoke-protocol-layers.md`](spoke-protocol-layers.md); `l5-mind` for event-observation semantics): opt-in; absent and empty `modules` are valid; baseline hosts need not emit or parse it. Inner dialect field tables stay **handbook-defined** (open bag — unknown module keys round-trip). Schema fragment: [`schemas/common/common.schema.json#/definitions/ModuleMap`](../../schemas/common/common.schema.json).

### `extensions.<product>`

Product- or adapter-owned bag. Namespace keys are opaque product / integrator ids matching `^[a-z][a-z0-9_-]*$`. Values are opaque JSON objects.

Shared schema fragment: [`schemas/common/common.schema.json#/definitions/ExtensionMap`](../../schemas/common/common.schema.json). Normative rules: [`spoke-data-model.md` §Extensions](spoke-data-model.md#extensions-normative), [`spoke-protocol.md` §Extensions](spoke-protocol.md#extensions).

On `HostCapabilityManifest`, `extensions` holds deployment metadata only — roles, capabilities, and namespace ownership are core manifest fields (`namespaces[]`), not KE product bags. See [`spoke-data-model.md` §HostCapabilityManifest](spoke-data-model.md#hostcapabilitymanifest-host-collaboration).

## Demand gate — promoting `modules` to the wire

**Resolved — shipped as optional capability-flagged envelope; demand gate satisfied.**

Optional `modules` (`ModuleMap`) is on the wire under capability flag **`narrative-modules`** (same opt-in pattern as `l2-computable` / `l5-fork`). Baseline hosts ignore the bag unless they declare the flag.

| Surface | Expectation |
|---------|-------------|
| Handbooks | Publish `modules.*` **inner** field tables and examples (activation, placement, activation_trace, …) |
| `schemas/` | Optional `modules` (`ModuleMap`) on KnowledgeEntry + AssemblePacket + TimelineEvent; not required; open bag |
| Integrators | Emit/parse `modules.<functional-ns>` when declaring `narrative-modules`; product-only query metadata stays in `extensions.<product>` |
| Capability flags | `narrative-modules` — opt-in; no baseline requirement to emit or parse `modules` |

Inner dialect shapes remain handbook-defined. Freezing a specific inner field table into schema is a separate, later decision when consumers need a closed dialect.

## Functional dialects vs product bags

| Placement | Correct for | Incorrect for |
|-----------|-------------|---------------|
| Core fields | Cross-host protocol identity and closed body envelope | Product DTO keys, one-host query filters |
| `modules.*` | Shared functional dialects (activation, placement, activation_trace, …) | One product’s private adapter state |
| `extensions.<product>` | One adapter’s product bag and product-local query metadata | Cross-product functional dialects shared by many hosts |

**Category rule:** a cross-product functional dialect uses **`modules.*`**. Product data uses **`extensions.<product>`**.

Publishing a shared functional key under `extensions.*` (for example treating `activation` or `pack` as an `extensions` namespace) is a **category error**:

| Why | Detail |
|-----|--------|
| Namespace exclusivity | `HostCapabilityManifest.namespaces[]` grants each product namespace to **at most one** `host_id` in a collaboration context. A shared functional name is not a product id and cannot own exclusivity the way `nexus` or `toy` does. |
| Reader expectation | `extensions.<ns>` reads as a **product / adapter** id. Functional dialect names collide with that reading. |
| Correct home | Functional dialects → `modules.*`; product bags → `extensions.<product>`. |

Product interim storage of activation-like data under a **true product namespace** (e.g. `extensions.nexus.*`) remains product-local; the shipped `modules` bag is the preferred home so multiple hosts converge on one layout.

## Round-trip / preserve

| Object | Rule |
|--------|------|
| `extensions.<product>` | Adapters **MUST** round-trip unknown namespaces and unknown keys inside a namespace **verbatim**. Merge/preserve helpers: [`spoke-operations.md`](spoke-operations.md) (`mergeExtensionMaps`, `preserveExtensionMaps`). |
| Shipped `modules` (capability-flagged) | Adapters **MUST** round-trip the `modules` object the same way — unknown module keys and nested fields survive read/write. Helpers: `mergeModuleMaps`, `preserveModuleMaps`. |

Empty `extensions: {}` is valid. Absent or empty `modules` is valid. Core fields stay closed; extensions and `modules` are the open product / dialect bags.

## Scope of authority

| This ADR owns | This ADR does not |
|---------------|-------------------|
| Normative **prose** placement of core / modules / extensions | Hard-coding inner dialect field tables into `schemas/**/*.json` |
| Demand-gate resolution for the optional `modules` envelope | Schema `description` field edits |
| Reject shared functional keys under `extensions.*` | Lore-activation or Knowledge Pack handbook field tables (sibling handbooks) |
| Round-trip expectation for extensions and shipped `modules` | Engines, pack I/O, registry behavior |

`extensions` on data objects is defined by `schemas/common/common.schema.json#/definitions/ExtensionMap`. `modules` is defined by `schemas/common/common.schema.json#/definitions/ModuleMap`. Wire truth remains `schemas/`; this file is bag-placement authority only.

## Placement quick reference

| Need | Place it |
|------|----------|
| Identity / closed body / protocol selector | Core field (existing schema) |
| Lore activation keys shared across narrative hosts | `modules.activation` (inner shape handbook-defined) |
| Portable Knowledge Pack catalog metadata | Product transport envelope (not `modules.*` on KE / AssemblePacket) |
| Packet placement / activation provenance | `modules.placement` / `modules.activation_trace` (inner shapes handbook-defined) |
| Actor / group mental-state fields (nine fields) | `modules.mental` (holder KnowledgeEntry; inner shape handbook-defined; `narrative-modules`) |
| Per-proposition belief labels (seven dimensions) | `modules.belief` (holder KnowledgeEntry; inner shape handbook-defined; `narrative-modules`) |
| Event observation metadata (who perceived an event + access constraints) | `modules.observation` (`TimelineEvent.modules`; inner shape handbook-defined; `narrative-modules` bag, `l5-mind` semantics) |
| Product world id, UI hint, adapter DTO | `extensions.<your-product>` |
| Host roles / owned namespaces | `HostCapabilityManifest` core fields (`roles`, `namespaces[]`) |
| Deployment metadata on a host | `HostCapabilityManifest.extensions` |

## See also

| Doc | Topic |
|-----|-------|
| [`CONCEPTS.md`](../../CONCEPTS.md) | Vocabulary index — Extensions; Modules (capability-flagged) |
| [`spoke-data-model.md`](spoke-data-model.md) | Data objects; §Extensions; ModuleMap; HostCapabilityManifest namespace exclusivity |
| [`spoke-protocol.md`](spoke-protocol.md) | Umbrella; §Extensions |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | L0–L8; Domain Profile vs core; `narrative-modules` capability |
| [`spoke-ops.md`](spoke-ops.md) | Ops envelopes; optional top-level `extensions`; AssemblePacket `modules` |
| [`spoke-operations.md`](spoke-operations.md) | Extension + module merge/preserve helpers; host collaboration |
| Lore-activation handbook | `modules.activation` field tables (sibling handbook) |
| Narrative Knowledge Pack handbook | KE/Relation bundle + product envelope catalog metadata; Seed vs Pool (sibling handbook) |
| Assemble module recipes handbook | `modules.placement` + `modules.activation_trace` (sibling handbook) |
| Mental-state handbook | `modules.mental` / `modules.belief` / `modules.observation` field tables (sibling handbook) |
| l5-mind ADR | `l5-mind` flag; `MindState` naming, placement, ownership boundary |
