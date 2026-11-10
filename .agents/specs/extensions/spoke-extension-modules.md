# SPOKE Extension Modules — Naming Triad

> **Status:** Normative  
> **Document class:** Normative — bag placement authority  
> **Parent:** [`spoke-protocol.md`](../protocol/spoke-protocol.md)  
> **Wire SSOT:** `schemas/` (optional `modules` / `ModuleMap` on KnowledgeEntry + AssemblePacket + TimelineEvent)

## Scope

This page is the bag-placement authority for SPOKE durable objects and ops envelopes: which fields belong in core, which belong in the optional capability-flagged `modules.*` bag, and which belong to a single product in `extensions.<product>`. It records the demand gate that put the optional `modules` envelope on the wire, the round-trip expectation for both open bags, and the quick-reference placement table integrators use to choose a home for a new field. Inner dialect field tables (activation, placement, cognition, mental state) belong to the sibling handbooks and are out of scope here.

## Current state

### Purpose

State the three bags that place fields on SPOKE durable objects and ops envelopes, and the rule that keeps cross-product functional dialects in `modules.*` while product bags stay in `extensions.<product>`.

Integrators use this ADR to choose **core**, **`modules.*`**, or **`extensions.<product>`** for every new field. Handbook authors cite it when documenting inner dialect field tables under the shipped `modules` envelope.

### The triad

| Bag | Audience | Wire today | Examples |
|-----|----------|------------|----------|
| **Core fields** | All baseline hosts | On the wire — closed protocol objects | `entry_id`, `body.summary`, `canonical_name`, `schema_version` |
| **Optional modules (`modules.*`)** | Cross-product **functional** dialects | **Optional, capability-flagged (`narrative-modules`; `spoke-cognition` supplies the closed cognition/observation semantics)** — open `ModuleMap` on KnowledgeEntry + AssemblePacket + TimelineEvent | `modules.activation`, `modules.placement`, `modules.activation_trace`, `modules.cognition`, `modules.mental`, `modules.observation` |
| **`extensions.<product>`** | One product / adapter | On the wire — required `ExtensionMap` on durable data objects | `extensions.nexus.world_id`, `extensions.toy.display_hint` |

#### Core fields

Protocol-owned keys on closed objects (`additionalProperties: false` on the protocol object). Every baseline host reads and writes these shapes. Product-specific or lossy-round-trip data does not become a core field.

Field tables: [`spoke-data-model.md`](../protocol/spoke-data-model.md), [`spoke-ops.md`](../protocol/spoke-ops.md). Layer map: [`spoke-protocol-layers.md`](../protocol/spoke-protocol-layers.md).

#### Optional modules (`modules.*`) — capability-flagged

Cross-product **functional** dialects shared by narrative hosts that need the same activation, placement, or similar companion shape. Examples under handbook-defined inner shapes:

| Path | Role |
|------|------|
| `modules.activation` | Lore / knowledge activation keys and related trigger metadata (KnowledgeEntry) |
| `modules.placement` | Packet-level injection placement hints (AssemblePacket) |
| `modules.activation_trace` | Packet-level activation provenance (AssemblePacket) |
| `modules.cognition` | Closed standard cognition records and narrated WorldFact records; current holder/world authority (KnowledgeEntry) |
| `modules.mental` | Actor / group open nine-field mental-state vocabulary (KnowledgeEntry); `beliefs` is derivative only |
| `modules.observation` | Event could-perceive observers and open access metadata (TimelineEvent) |

**Not a `modules.*` dialect:** Narrative Knowledge Pack **catalog metadata** (`title` / `version` / `creator`) lives on the **product transport envelope** that wraps atoms for import/export. Pack ≠ AssemblePacket (durable library interchange vs ephemeral assemble output). Do not place pack catalog fields on KnowledgeEntry or AssemblePacket `modules`. See [`domain-profile-narrative-knowledge-pack.md`](../domain-profiles/domain-profile-narrative-knowledge-pack.md).

**Status on the wire:** optional `modules` (`ModuleMap`) is **shipped** on `KnowledgeEntry`, `AssemblePacket`, and `TimelineEvent` (event observation). The bag is **capability-flagged** (`narrative-modules` in [`spoke-protocol-layers.md`](../protocol/spoke-protocol-layers.md); `spoke-cognition` supplies the closed cognition and observation semantics): opt-in; absent and empty `modules` are valid; baseline hosts need not emit or parse it. Unknown module keys round-trip, while `modules.cognition` itself is a closed dialect. Schema fragment: [`schemas/common/common.schema.json#/definitions/ModuleMap`](../../../schemas/common/common.schema.json).

#### `extensions.<product>`

Product- or adapter-owned bag. Namespace keys are opaque product / integrator ids matching `^[a-z][a-z0-9_-]*$`. Values are opaque JSON objects.

Shared schema fragment: [`schemas/common/common.schema.json#/definitions/ExtensionMap`](../../../schemas/common/common.schema.json). Normative rules: [`spoke-data-model.md` §Extensions](../protocol/spoke-data-model.md#extensions-normative), [`spoke-protocol.md` §Extensions](../protocol/spoke-protocol.md#extensions).

On `HostCapabilityManifest`, `extensions` holds deployment metadata only — roles, capabilities, and namespace ownership are core manifest fields (`namespaces[]`), not KE product bags. See [`spoke-data-model.md` §HostCapabilityManifest](../protocol/spoke-data-model.md#hostcapabilitymanifest-host-collaboration).

### Demand gate — promoting `modules` to the wire

The optional `modules` (`ModuleMap`) bag is on the wire under capability flag **`narrative-modules`** (same opt-in pattern as `l2-computable` / `l5-fork`). Baseline hosts ignore the bag unless they declare the flag.

| Surface | Expectation |
|---------|-------------|
| Handbooks | Publish `modules.*` **inner** field tables and examples (activation, placement, activation_trace, …) |
| `schemas/` | Optional `modules` (`ModuleMap`) on KnowledgeEntry + AssemblePacket + TimelineEvent; not required; open bag |
| Integrators | Emit/parse `modules.<functional-ns>` when declaring `narrative-modules`; product-only query metadata stays in `extensions.<product>` |
| Capability flags | `narrative-modules` — opt-in; no baseline requirement to emit or parse `modules` |

Inner dialect shapes remain handbook-defined. Freezing a specific inner field table into schema is a separate, later decision when consumers need a closed dialect.

### Functional dialects vs product bags

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

### Round-trip / preserve

| Object | Rule |
|--------|------|
| `extensions.<product>` | Adapters **MUST** round-trip unknown namespaces and unknown keys inside a namespace **verbatim**. Merge/preserve helpers: [`spoke-operations.md`](../protocol/spoke-operations.md) (`mergeExtensionMaps`, `preserveExtensionMaps`). |
| Shipped `modules` (capability-flagged) | Adapters **MUST** round-trip the `modules` object the same way — unknown module keys and nested fields survive read/write. Helpers: `mergeModuleMaps`, `preserveModuleMaps`. |

Empty `extensions: {}` is valid. Absent or empty `modules` is valid. Core fields stay closed; extensions and `modules` are the open product / dialect bags.

### Scope of authority

| This page owns | This page does not |
|---------------|-------------------|
| Normative **prose** placement of core / modules / extensions | Hard-coding inner dialect field tables into `schemas/**/*.json` |
| Demand-gate resolution for the optional `modules` envelope | Schema `description` field edits |
| Reject shared functional keys under `extensions.*` | Lore-activation or Knowledge Pack handbook field tables (sibling handbooks) |
| Round-trip expectation for extensions and shipped `modules` | Engines, pack I/O, registry behavior |

`extensions` on data objects is defined by `schemas/common/common.schema.json#/definitions/ExtensionMap`. `modules` is defined by `schemas/common/common.schema.json#/definitions/ModuleMap`. Wire truth remains `schemas/`; this file is bag-placement authority only.

### Placement quick reference

| Need | Place it |
|------|----------|
| Identity / closed body / protocol selector | Core field (existing schema) |
| Lore activation keys shared across narrative hosts | `modules.activation` (inner shape handbook-defined) |
| Portable Knowledge Pack catalog metadata | Product transport envelope (not `modules.*` on KE / AssemblePacket) |
| Packet placement / activation provenance | `modules.placement` / `modules.activation_trace` (inner shapes handbook-defined) |
| Standard Belief / epistemic / actual-Perception records and WorldFact | `modules.cognition` (holder or designated world-state KnowledgeEntry; closed shapes; `narrative-modules` + `spoke-cognition`) |
| Other actor / group mental-state fields (nine fields) | `modules.mental` (holder KnowledgeEntry; inner shape handbook-defined; `narrative-modules`) |
| Event could-perceive metadata and access constraints | `modules.observation` (`TimelineEvent.modules`; closed observer shape with open access keys; `narrative-modules` + `spoke-cognition`) |
| Product world id, UI hint, adapter DTO | `extensions.<your-product>` |
| Host roles / owned namespaces | `HostCapabilityManifest` core fields (`roles`, `namespaces[]`) |
| Deployment metadata on a host | `HostCapabilityManifest.extensions` |

### See also

| Doc | Topic |
|-----|-------|
| [`CONCEPTS.md`](../../../CONCEPTS.md) | Vocabulary index — Extensions; Modules (capability-flagged) |
| [`spoke-data-model.md`](../protocol/spoke-data-model.md) | Data objects; §Extensions; ModuleMap; HostCapabilityManifest namespace exclusivity |
| [`spoke-protocol.md`](../protocol/spoke-protocol.md) | Umbrella; §Extensions |
| [`spoke-protocol-layers.md`](../protocol/spoke-protocol-layers.md) | L0–L8; Domain Profile vs core; `narrative-modules` capability |
| [`spoke-ops.md`](../protocol/spoke-ops.md) | Ops envelopes; optional top-level `extensions`; AssemblePacket `modules` |
| [`spoke-operations.md`](../protocol/spoke-operations.md) | Extension + module merge/preserve helpers; host collaboration |
| Lore-activation handbook | `modules.activation` field tables (sibling handbook) |
| Narrative Knowledge Pack handbook | KE/Relation bundle + product envelope catalog metadata; Seed vs Pool (sibling handbook) |
| Assemble module recipes handbook | `modules.placement` + `modules.activation_trace` (sibling handbook) |
| Mental-state handbook | `modules.cognition` / `modules.mental` / `modules.observation` vocabulary and checker outputs |
| Cognition family ADR / data contract | [`2026-11-07-spoke-cognition-family.md`](../../adr/implemented/architecture/2026-11-07-spoke-cognition-family.md); [`spoke-cognition-data-contract.md`](../cognition/spoke-cognition-data-contract.md) |

## Invariants

- Core fields are protocol-owned keys on closed objects (`additionalProperties: false`); product-specific or lossy-round-trip data never becomes a core field.
- `modules.*` is a cross-product **functional** dialect bag only: optional, capability-flagged (`narrative-modules`; `spoke-cognition` supplies the closed cognition and observation semantics), shipped on `KnowledgeEntry`, `AssemblePacket`, and `TimelineEvent`. Absent and empty `modules` are valid and baseline hosts need not emit or parse it.
- Unknown module keys round-trip, while `modules.cognition` itself is a closed dialect with consumer-contract conformance conditions.
- `extensions.<product>` namespace keys are opaque product / integrator ids matching `^[a-z][a-z0-9_-]*$` with opaque JSON object values; adapters **MUST** round-trip unknown namespaces and unknown keys inside a namespace verbatim.
- A cross-product functional dialect published under `extensions.*` is a category error: `HostCapabilityManifest.namespaces[]` grants each product namespace to at most one `host_id`, and a functional name cannot own that exclusivity.
- `extensions` on `HostCapabilityManifest` holds deployment metadata only — roles, capabilities, and namespace ownership are core manifest fields.
- Publishing inner dialect field tables and examples is the handbooks' job; freezing a specific inner field table into schema is a separate, later decision.
- Wire truth remains `schemas/`; this page is bag-placement authority only.

## Where it lives

| Surface | Path |
|---------|------|
| `ModuleMap` definition | `schemas/common/common.schema.json#/definitions/ModuleMap` |
| `ExtensionMap` definition | `schemas/common/common.schema.json#/definitions/ExtensionMap` |
| Closed protocol objects and their field tables | `.agents/specs/protocol/spoke-data-model.md` |
| Ops envelopes and optional top-level `extensions` | `.agents/specs/protocol/spoke-ops.md` |
| Umbrella protocol and `§Extensions` rules | `.agents/specs/protocol/spoke-protocol.md` |
| Layer map, Domain Profile vs core, `narrative-modules` capability | `.agents/specs/protocol/spoke-protocol-layers.md` |
| `mergeExtensionMaps`, `preserveExtensionMaps`, `mergeModuleMaps`, `preserveModuleMaps` | `.agents/specs/protocol/spoke-operations.md` |
| Activation dialect field tables | `.agents/specs/domain-profiles/domain-profile-lore-activation.md` |
| Placement / activation-trace dialects | `.agents/specs/domain-profiles/assemble-module-recipes.md` |
| Cognition, mental-state, observation dialects | `.agents/specs/domain-profiles/domain-profile-mental-state.md` |

## Change obligations

- Placing any new field → record the chosen bag in the triad and quick-reference tables of §Current state here same PR, and point at the owning handbook for its inner shape.
- Adding a `modules.<functional-ns>` dialect → add a row to the `modules.*` role table here and to the placement quick reference, plus the sibling handbook that owns its field table, in the same PR.
- Changing the capability flag, or making any `modules` sub-bag closed → update this page and `.agents/specs/protocol/spoke-protocol-layers.md` capability table in the same PR; a closed inner dialect also updates the owning handbook and `schemas/common/common.schema.json`.
- Changing a round-trip rule or helper name → update §Current state here and `.agents/specs/protocol/spoke-operations.md` same PR.
- Changing `ExtensionMap` key grammar → update `schemas/common/common.schema.json` and the table here in the same PR.
