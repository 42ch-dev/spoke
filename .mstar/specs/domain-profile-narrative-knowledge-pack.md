# Domain Profile — Narrative Knowledge Pack

> **Status:** Domain Profile handbook (tracked result) — inner dialect shapes handbook-defined under shipped `modules`  
> **Document class:** Domain Profile — portable lore-bundle dialect over closed SPOKE envelopes  
> **Parent:** [`spoke-protocol-layers.md`](spoke-protocol-layers.md) §Domain Profile  
> **Bag placement authority:** [`spoke-extension-modules.md`](spoke-extension-modules.md)  
> **Wire SSOT:** `schemas/` — optional `modules` (`ModuleMap`) on KnowledgeEntry and AssemblePacket

## Purpose

This Domain Profile documents how integrators express a **Narrative Knowledge Pack** — a portable lore bundle that ships an ordered set of KnowledgeEntries, Relations, and optional SourceAnchors between narrative hosts — using existing SPOKE wire atoms plus a shared pack-level companion shape under `modules.pack`.

Integrators read this handbook to implement pack import/export, multi-pack compose (world pack + character pack), and the **Seed vs Pool** assemble candidate pattern. Matching engines, token budgets, ranking, and pack I/O stay **product-local**. The dialect is for **round-trip interchange** so lore graphs and fire conditions travel together.

Pack atoms are existing wire objects. Optional `modules` (`ModuleMap`) is shipped as optional, capability-flagged (`narrative-modules`) — see [`spoke-extension-modules.md`](spoke-extension-modules.md). Inner pack field tables remain handbook-defined. A dedicated pack **container** envelope (zip/JSON transport object) stays product-local.

---

## Placement — triad reminder

| Bag | Role for Knowledge Packs |
|-----|--------------------------|
| **Core fields** | Pack **atoms**: KnowledgeEntry, Relation, SourceAnchor identity and body — closed protocol objects |
| **`modules.pack`** | Cross-product **pack-level** metadata: title, version, creator, optional description |
| **`modules.activation`** | Per-entry fire conditions that travel with pack KnowledgeEntries — see [`domain-profile-lore-activation.md`](domain-profile-lore-activation.md) |
| **`extensions.<product>`** | One product’s private adapter state on atoms or pack sidecars (host-only filters, UI flags, interim staging) |

Category rule (normative triad ADR): shared functional dialects use `modules.*`. Product data uses `extensions.<product>`. Publishing pack metadata as a shared key under `extensions.*` is a category error — see [`spoke-extension-modules.md`](spoke-extension-modules.md).

`ModuleMap` is shipped; inner shapes (field tables) remain handbook-defined. Capability-flagged hosts emit/parse `modules.pack` / `modules.activation`; baseline hosts leave `modules` absent.

---

## Pack dialect

A **Narrative Knowledge Pack** is a handbook dialect + sample layout over existing wire atoms. It is **not** a new baseline schema object.

### Atoms (existing wire)

| Atom | Role in a pack |
|------|----------------|
| **KnowledgeEntry** | Lore nodes (world rules, characters, locations, events, profile types, …) — ordered list |
| **Relation** | Graph edges among pack entries (`related_to`, `located_in`, `member_of`, profile-open types, …) |
| **SourceAnchor** (optional) | Provenance spans when the pack carries manuscript or source grounding |

Atoms validate against committed schemas in `schemas/data/`. Field tables: [`spoke-data-model.md`](spoke-data-model.md).

### Pack-level metadata — `modules.pack`

**Status:** Handbook-defined inner object under shipped optional `modules` (`ModuleMap`). Capability-flagged (`narrative-modules`); opt-in. Integrators use this field table so multiple hosts converge on one pack-metadata layout.

| Field | Required | Type | Semantics |
|-------|----------|------|-----------|
| `title` | **yes** | `string` | Human pack name (world library, character lore set, scene kit, …) |
| `version` | **yes** | `string` | Pack author version string (integrator SemVer or product convention) |
| `creator` | **yes** | `string` | Authoring identity (person, studio, or product account id as open string) |
| `description` | no | `string` | Short human blurb for catalogs and import UIs |

### Illustrative pack shape (handbook-defined inner dialect)

```text
// Narrative Knowledge Pack — handbook companion layout
{
  "modules": {
    "pack": {
      "title": "Harbor World Lore",
      "version": "1.0.0",
      "creator": "toy-world",
      "description": "Harbor setting facts for cross-host interchange"
    }
  },
  "entries": [ /* KnowledgeEntry[] in stable export order */ ],
  "relations": [ /* Relation[] among entry ids in this pack */ ],
  "source_anchors": [ /* optional SourceAnchor[] */ ]
}
```

Product-private pack transport fields (zip layout, checksums, registry ids) stay in `extensions.<product>` or product envelope layers outside this dialect.

### What the pack is

| Property | Contract |
|----------|----------|
| Dialect home | This handbook + conformance samples under `fixtures/toy-world/` |
| Atoms | Existing KnowledgeEntry / Relation / SourceAnchor wire shapes |
| Metadata | `modules.pack` (inner shape handbook-defined) under optional shipped `ModuleMap` |
| Baseline schema | No dedicated pack **container** object; optional `modules` on KE + AssemblePacket |
| Registry / CLI | Product-owned pack I/O (import, export, catalog) |

---

## Compose / stack model

Narrative hosts commonly maintain **multiple** lore libraries (world facts, character lore, persona kits, session overlays). A host **stacks** or **merges** packs product-side:

| Layer (illustrative) | Typical pack content |
|----------------------|----------------------|
| World pack | Setting rules, locations, factions, always-on world constants |
| Character pack | Character facts, voice anchors, character-local lore |
| Session / work pack | Chapter-scoped or run-scoped facts (product Scope binding) |

### Compose guidance

| Step | Guidance |
|------|----------|
| 1 | Import each pack’s atoms into product storage; preserve export order when useful for stable re-export |
| 2 | Merge Relation graphs by id; product policy resolves id collisions (rename, namespace prefix, or reject) |
| 3 | Union Seed sets and Pool sets across stacked packs (see §Seed vs Pool) |
| 4 | Run product activation / Scope filter / budget **after** stack merge |
| 5 | Emit `AssemblePacket` via pure helpers with **caller-supplied** candidate order |

Stack policy (priority across packs, override wins, soft-delete) is **product-local**. The protocol supplies portable atoms + `modules.pack` / `modules.activation` metadata; compose is a host recipe, not a baseline op.

---

## Round-trip / preserve

| Surface | Rule |
|---------|------|
| `extensions.<product>` on pack atoms | Importers **MUST** round-trip unknown product namespaces and unknown keys **verbatim** ([`spoke-extension-modules.md`](spoke-extension-modules.md) §Round-trip; helpers in [`spoke-operations.md`](spoke-operations.md)) |
| `modules.pack` | Stage and re-export pack metadata with the handbook field names so hosts converge |
| `modules.activation` with entries | Preserve fire conditions on each KnowledgeEntry under optional `modules` ([`domain-profile-lore-activation.md`](domain-profile-lore-activation.md)) |
| Shipped `modules` (capability-flagged) | Adapters **MUST** round-trip the object — unknown module keys and nested fields survive read/write (`mergeModuleMaps` / `preserveModuleMaps`) |

Unknown open-string values (`entry_type`, `relation_type`, tags) round-trip without normalization per baseline data-model rules.

---

## Pack container envelope

Optional `modules` (`ModuleMap`) is shipped on KnowledgeEntry and AssemblePacket (capability-flagged `narrative-modules`). A dedicated pack **container** schema (zip/JSON transport object that wraps ordered atoms) remains **product-local** — this handbook documents the dialect + atom layout, not a baseline container type.

| Surface | Expectation |
|---------|-------------|
| This handbook | Pack dialect + Seed/Pool pattern + field tables |
| `fixtures/toy-world/` | Conformance atoms and optional companion pack samples |
| `schemas/` | Optional `modules` on KE + AssemblePacket; no dedicated pack container object |
| Integrators | Implement import/export against handbook atoms + `modules.pack` / `modules.activation` |
| Capability flags | `narrative-modules` opt-in; no baseline requirement to emit pack containers |

Authority for bag placement: [`spoke-extension-modules.md`](spoke-extension-modules.md).

---

## Activation cross-link

Packs **MAY** transport `modules.activation` with each KnowledgeEntry so fire conditions travel with the entry. Optional `modules` (`ModuleMap`) is a capability-flagged root property on KnowledgeEntry; inner activation shapes stay handbook-defined.

| Concern | Home |
|---------|------|
| Field table (`keys`, `secondary_keys`, `logic`, `constant`, `order`, `priority`, `position_hint`, `outlet`, `match`) | [`domain-profile-lore-activation.md`](domain-profile-lore-activation.md) |
| Standalone-snippet invariant (`body.summary` / assemble `snippet`) | Lore-activation handbook |
| Pack transport of activation | This handbook — preserve `modules.activation` on each exported KE |
| Seed ↔ `constant` / Pool ↔ keyed | Short map in lore-activation; full assemble pattern **here** (§Seed vs Pool) |

Importers map activation fields into product engines. Engines stay product-local; `spoke-operations` gains no matchers.

### Illustrative entry with activation

Baseline KnowledgeEntry atoms stay schema-valid with or without optional `modules`. Full Harbor sample: [`fixtures/toy-world/proposed/pack_tw_harbor_companion.json`](../../fixtures/toy-world/proposed/pack_tw_harbor_companion.json).

```text
// KnowledgeEntry with optional modules.activation (capability-flagged)
{
  "schema_version": 1,
  "entry_id": "kb_tw_harbor_rules",
  "entry_type": "rule",
  "canonical_name": "Harbor standing rules",
  "status": "confirmed",
  "body": {
    "summary": "Dockside law holds at dawn; captains answer to the harbor master."
  },
  "extensions": {},
  "modules": {
    "activation": {
      "keys": [],
      "constant": true,
      "order": 0,
      "priority": 100,
      "position_hint": "before_defs"
    }
  }
}
```

Optional `modules` is capability-flagged (`narrative-modules`); absent and empty are valid. Inner `modules.activation` field tables remain handbook-defined.

---

## Seed vs Pool assemble pattern

**Primary home:** this handbook. Lore-activation only short-maps `constant` ↔ Seed and keyed entries ↔ Pool.

Assemble callers build an ordered KnowledgeEntry list, then call pure packet builders. Engines (match, scan, rank, budget) stay **outside** `@42ch/spoke-operations` / `spoke-operations`.

### Sets

| Set | Meaning | Typical sources | Activation map |
|-----|---------|-----------------|----------------|
| **Seed** | Always-on candidates for the current assemble | World rules, POV character facts, active scene anchors | `modules.activation.constant: true` |
| **Pool** | On-demand candidates | Key-triggered lore, relation-hop expansions, Scope-filtered hits | Keyed entries (`keys` non-empty; `constant` false or absent) |

### Caller recipe

| Step | Action | Owner |
|------|--------|-------|
| 1 | Collect **Seed** entries for the active Scope / scene | Product |
| 2 | Run activation / retrieval / relation expand → **Pool** | Product engine |
| 3 | Concatenate candidates in **caller-chosen order** (common convention: Seed first, then Pool by product priority) | Product |
| 4 | Optional product budget trim (tokens, depth) **before** the library call | Product |
| 5 | `buildAssemblePacket({ packetId, knowledgeEntries, maxEntries? })` | Pure ops — [`spoke-operations.md`](spoke-operations.md) §AssemblePacket builder |
| 6 | When `maxEntries` is a positive integer, keep the **first n** entries in **input order** only | Pure ops (no sort/rank) |

### Wire and ops facts

| Fact | Reference |
|------|-----------|
| `assemble` request optional `max_entries` is a selector **hint** | [`spoke-ops.md`](spoke-ops.md) §assemble |
| `buildAssemblePacket` `maxEntries` truncates **input order** only | [`spoke-operations.md`](spoke-operations.md) |
| `AssemblePacket` is wire-only context shape | [`spoke-data-model.md`](spoke-data-model.md) §AssemblePacket; L8 in [`spoke-protocol-layers.md`](spoke-protocol-layers.md) |
| Ranking, embedding search, token counting | Product-local — outside pure ops |

### Relation expand into Pool

| Step | Guidance |
|------|----------|
| 1 | Prefer `Relation` edges for lore adjacency (pack `relations[]`) |
| 2 | Product expanders hop Relations under **local** budget into the Pool |
| 3 | Importers **MAY** synthesize Relations from legacy key-mention graphs as a **one-shot migration** |

Relation-first recursion detail: [`domain-profile-lore-activation.md`](domain-profile-lore-activation.md) §Relation-first recursion.

### Engine boundary

| Concern | Owner |
|---------|-------|
| Seed / Pool set construction | **Product-local** |
| Keyword / selective matching | **Product-local** (activation handbook vocabulary) |
| Scan window, depth, token budget, ranking | **Product-local** |
| Packet order + `maxEntries` truncate | Caller order + pure `buildAssemblePacket` |
| Pack import/export I/O | **Product-local** |
| Pure library helpers | `@42ch/spoke-operations` / `spoke-operations` — **no** matching, scan, activation, or pack I/O |

---

## Integrator checklist

An integrator can implement pack import/export from this handbook (and the lore-activation handbook for fire-condition fields) when:

1. A pack is an ordered list of KnowledgeEntry + Relation + optional SourceAnchor atoms with `modules.pack` metadata.
2. Compose stacks world / character / session packs product-side; merge policy stays local.
3. Unknown `extensions.<product>` on atoms survive import/export verbatim; `modules.*` round-trip with handbook names (`mergeModuleMaps` / `preserveModuleMaps`).
4. Packs **MAY** carry `modules.activation` on exported KnowledgeEntry atoms; field semantics live in the lore-activation handbook.
5. Assemble callers build Seed then Pool, supply ordered candidates, and use `maxEntries` truncation on input order only.
6. Engines and pack I/O stay in the product; `spoke-operations` stays pure wire helpers.
7. Optional `modules` is capability-flagged (`narrative-modules`); inner pack shapes stay handbook-defined; pack **container** transport stays product-local.

---

## Acceptance (profile handbook)

- [x] Pack dialect defined: ordered KE + Relation + optional SourceAnchor atoms
- [x] `modules.pack` field table (title, version, creator, optional description)
- [x] Compose / stack model for multi-pack hosts (world + character + session)
- [x] Round-trip preserve for product `extensions` and shipped `modules.*`
- [x] Envelope shipped capability-flagged; pack container remains product-local
- [x] Cross-link to `modules.activation` / lore-activation handbook
- [x] Seed vs Pool assemble pattern documented as primary home (caller order, `maxEntries`)
- [x] Engine boundary: product-local match/scan/budget/pack I/O; pure ops wire-only
- [x] Envelope status present-tense shipped; inner shapes handbook-defined
- [x] No field-table rewrite beyond status framing; no iteration ids; mechanisms only (clean-room public patterns)

---

## See also

| Doc | Topic |
|-----|-------|
| [`spoke-extension-modules.md`](spoke-extension-modules.md) | Core / `modules.*` / `extensions.<product>` triad; capability-flagged envelope; round-trip |
| [`domain-profile-lore-activation.md`](domain-profile-lore-activation.md) | `modules.activation` field table; standalone snippet; constant ↔ Seed pointer |
| [`domain-profile-narrative-structure.md`](domain-profile-narrative-structure.md) | Sister Domain Profile — Beat / structural mapping |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | Domain Profile principles; L8 AssemblePacket; `narrative-modules` |
| [`spoke-data-model.md`](spoke-data-model.md) | KnowledgeEntry, Relation, SourceAnchor, AssemblePacket, ModuleMap |
| [`spoke-ops.md`](spoke-ops.md) | `assemble` wire-only boundary; optional `max_entries` hint |
| [`spoke-operations.md`](spoke-operations.md) | `buildAssemblePacket`, extension + module preserve; no activation engines |
| [`fixtures/toy-world/`](../../fixtures/toy-world/) | Conformance atoms; optional companion pack samples |
| [`fixtures/toy-world/proposed/pack_tw_harbor_companion.json`](../../fixtures/toy-world/proposed/pack_tw_harbor_companion.json) | Harbor Narrative Knowledge Pack companion sample (`modules.pack` + `modules.activation`) |
| [`CONCEPTS.md`](../../CONCEPTS.md) | Domain Profile; Modules (capability-flagged); Extensions |
