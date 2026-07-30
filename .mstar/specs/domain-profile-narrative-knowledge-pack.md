# Domain Profile — Narrative Knowledge Pack

> **Status:** Domain Profile handbook (tracked result) — companion shapes **proposed**  
> **Document class:** Domain Profile — portable lore-bundle dialect over closed SPOKE envelopes  
> **Parent:** [`spoke-protocol-layers.md`](spoke-protocol-layers.md) §Domain Profile  
> **Bag placement authority:** [`spoke-extension-modules.md`](spoke-extension-modules.md)  
> **Wire SSOT:** `schemas/` (unchanged by this profile — no pack envelope and no `modules` property on the wire)

## Purpose

This Domain Profile documents how integrators express a **Narrative Knowledge Pack** — a portable lore bundle that ships an ordered set of KnowledgeEntries, Relations, and optional SourceAnchors between narrative hosts — using existing SPOKE wire atoms plus a shared **proposed** pack-level companion shape.

Integrators read this handbook to implement pack import/export, multi-pack compose (world pack + character pack), and the **Seed vs Pool** assemble candidate pattern. Matching engines, token budgets, ranking, and pack I/O stay **product-local**. The dialect is for **round-trip interchange** so lore graphs and fire conditions travel together.

Baseline schemas are unchanged. Pack atoms are existing wire objects. **Proposed** `modules.pack` ships on the wire only after the demand gate in [`spoke-extension-modules.md`](spoke-extension-modules.md).

---

## Placement — triad reminder

| Bag | Role for Knowledge Packs |
|-----|--------------------------|
| **Core fields** | Pack **atoms**: KnowledgeEntry, Relation, SourceAnchor identity and body — closed protocol objects |
| **Proposed `modules.pack`** | Cross-product **pack-level** metadata: title, version, creator, optional description |
| **Proposed `modules.activation`** | Per-entry fire conditions that travel with pack KnowledgeEntries — see [`domain-profile-lore-activation.md`](domain-profile-lore-activation.md) |
| **`extensions.<product>`** | One product’s private adapter state on atoms or pack sidecars (host-only filters, UI flags, interim staging) |

Category rule (normative triad ADR): shared functional dialects use **proposed** `modules.*`. Product data uses `extensions.<product>`. Publishing pack metadata as a shared key under `extensions.*` is a category error — see [`spoke-extension-modules.md`](spoke-extension-modules.md).

Until the demand gate opens, readers treat every **proposed** `modules.pack` mention as a **handbook companion shape**, not an optional JSON property available on committed schemas.

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

### Pack-level metadata — proposed `modules.pack`

**Status:** **Proposed** companion object. Not present on any committed schema. Integrators stage this shape in product storage or pack sidecars so multiple hosts converge before a schema slice.

| Field | Required | Type | Semantics |
|-------|----------|------|-----------|
| `title` | **yes** | `string` | Human pack name (world library, character lore set, scene kit, …) |
| `version` | **yes** | `string` | Pack author version string (integrator SemVer or product convention) |
| `creator` | **yes** | `string` | Authoring identity (person, studio, or product account id as open string) |
| `description` | no | `string` | Short human blurb for catalogs and import UIs |

### Illustrative pack shape (proposed companion — not wire schema)

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
| Metadata | **Proposed** `modules.pack` |
| Baseline schema | No pack envelope object; no `modules` key on committed schemas |
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

Stack policy (priority across packs, override wins, soft-delete) is **product-local**. The protocol supplies portable atoms + **proposed** metadata; compose is a host recipe, not a baseline op.

---

## Round-trip / preserve

| Surface | Rule |
|---------|------|
| `extensions.<product>` on pack atoms | Importers **MUST** round-trip unknown product namespaces and unknown keys **verbatim** ([`spoke-extension-modules.md`](spoke-extension-modules.md) §Round-trip; helpers in [`spoke-operations.md`](spoke-operations.md)) |
| **Proposed** `modules.pack` | Stage and re-export pack metadata with the handbook field names so hosts converge |
| **Proposed** `modules.activation` with entries | Preserve fire conditions beside each KnowledgeEntry in the pack companion ([`domain-profile-lore-activation.md`](domain-profile-lore-activation.md)) |
| Future shipped `modules` | When a demand-gated schema slice adds `modules`, adapters **MUST** round-trip the object the same way — unknown module keys and nested fields survive read/write |

Unknown open-string values (`entry_type`, `relation_type`, tags) round-trip without normalization per baseline data-model rules.

---

## Demand gate — pack envelope schema

A non-baseline pack **envelope** schema (zip/JSON container with a committed `modules` property) ships **only when** the triad demand gate opens:

1. **≥2 consumers** need an **identical** pack envelope / `modules.pack` shape on the wire, **or**
2. **Nexus + one external host** both require the same shipped shape.

Authority: [`spoke-extension-modules.md`](spoke-extension-modules.md) §Demand gate.

Until that gate opens:

| Surface | Expectation |
|---------|-------------|
| This handbook | Pack dialect + Seed/Pool pattern + field tables |
| `fixtures/toy-world/` | Conformance atoms and optional companion pack samples |
| `schemas/` | No pack envelope; no `modules` property |
| Integrators | Implement import/export against handbook atoms + **proposed** `modules.pack` |
| Capability flags | No baseline requirement to emit or parse pack envelopes |

Handbook + fixture samples are sufficient interchange documentation until the gate opens.

---

## Activation cross-link

Packs **MAY** transport **proposed** `modules.activation` with each KnowledgeEntry so fire conditions travel with the entry. The activation object stages in the **pack companion envelope** beside baseline-valid atoms — not as a property on the committed KnowledgeEntry schema (`additionalProperties: false` on KE root).

| Concern | Home |
|---------|------|
| Field table (`keys`, `secondary_keys`, `logic`, `constant`, `order`, `priority`, `position_hint`, `outlet`, `match`) | [`domain-profile-lore-activation.md`](domain-profile-lore-activation.md) |
| Standalone-snippet invariant (`body.summary` / assemble `snippet`) | Lore-activation handbook |
| Pack transport of activation | This handbook — preserve **proposed** `modules.activation` beside each exported KE in the companion envelope |
| Seed ↔ `constant` / Pool ↔ keyed | Short map in lore-activation; full assemble pattern **here** (§Seed vs Pool) |

Importers map activation companion fields into product engines. Engines stay product-local; `spoke-operations` gains no matchers.

### Illustrative entry with activation (proposed companion)

Baseline KnowledgeEntry atoms stay schema-valid. **Proposed** `modules.activation` stages **beside** those atoms in the pack companion envelope. Full Harbor sample: [`fixtures/toy-world/proposed/pack_tw_harbor_companion.json`](../../fixtures/toy-world/proposed/pack_tw_harbor_companion.json).

```text
// 1) Baseline KnowledgeEntry atom (committed schema — no modules property)
{
  "schema_version": 1,
  "entry_id": "kb_tw_harbor_rules",
  "entry_type": "rule",
  "canonical_name": "Harbor standing rules",
  "status": "confirmed",
  "body": {
    "summary": "Dockside law holds at dawn; captains answer to the harbor master."
  },
  "extensions": {}
}

// 2) Proposed companion annotation staged beside the atom in the pack envelope
//    (pack/companion level — not a KnowledgeEntry root field)
{
  "entry_id": "kb_tw_harbor_rules",
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

**Proposed** `modules.activation` is a companion shape staged **beside or around** valid KnowledgeEntry atoms until the demand gate. Baseline KnowledgeEntry validation uses committed schemas without a `modules` property.

---

## Seed vs Pool assemble pattern

**Primary home:** this handbook. Lore-activation only short-maps `constant` ↔ Seed and keyed entries ↔ Pool.

Assemble callers build an ordered KnowledgeEntry list, then call pure packet builders. Engines (match, scan, rank, budget) stay **outside** `@42ch/spoke-operations` / `spoke-operations`.

### Sets

| Set | Meaning | Typical sources | Activation map |
|-----|---------|-----------------|----------------|
| **Seed** | Always-on candidates for the current assemble | World rules, POV character facts, active scene anchors | **Proposed** `modules.activation.constant: true` |
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

1. A pack is an ordered list of KnowledgeEntry + Relation + optional SourceAnchor atoms with **proposed** `modules.pack` metadata.
2. Compose stacks world / character / session packs product-side; merge policy stays local.
3. Unknown `extensions.<product>` on atoms survive import/export verbatim; **proposed** `modules.*` round-trip with handbook names.
4. Packs **MAY** stage **proposed** `modules.activation` beside exported KnowledgeEntry atoms in the companion envelope; field semantics live in the lore-activation handbook.
5. Assemble callers build Seed then Pool, supply ordered candidates, and use `maxEntries` truncation on input order only.
6. Engines and pack I/O stay in the product; `spoke-operations` stays pure wire helpers.
7. Wire schemas remain closed; triad ADR governs promotion of pack envelope / `modules.pack` to schema.

---

## Acceptance (profile handbook)

- [x] Pack dialect defined: ordered KE + Relation + optional SourceAnchor atoms
- [x] **Proposed** `modules.pack` field table (title, version, creator, optional description)
- [x] Compose / stack model for multi-pack hosts (world + character + session)
- [x] Round-trip preserve for product `extensions` and proposed `modules.*`
- [x] Demand gate cited from triad ADR; handbook + fixture until gate
- [x] Cross-link to **proposed** `modules.activation` / lore-activation handbook
- [x] Seed vs Pool assemble pattern documented as primary home (caller order, `maxEntries`)
- [x] Engine boundary: product-local match/scan/budget/pack I/O; pure ops wire-only
- [x] Every `modules.pack` / pack `modules` mention marked **proposed**
- [x] No schema edits; no iteration ids; mechanisms only (clean-room public patterns)

---

## See also

| Doc | Topic |
|-----|-------|
| [`spoke-extension-modules.md`](spoke-extension-modules.md) | Core / proposed `modules.*` / `extensions.<product>` triad; demand gate; round-trip |
| [`domain-profile-lore-activation.md`](domain-profile-lore-activation.md) | **Proposed** `modules.activation` field table; standalone snippet; constant ↔ Seed pointer |
| [`domain-profile-narrative-structure.md`](domain-profile-narrative-structure.md) | Sister Domain Profile — Beat / structural mapping |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | Domain Profile principles; L8 AssemblePacket |
| [`spoke-data-model.md`](spoke-data-model.md) | KnowledgeEntry, Relation, SourceAnchor, AssemblePacket |
| [`spoke-ops.md`](spoke-ops.md) | `assemble` wire-only boundary; optional `max_entries` hint |
| [`spoke-operations.md`](spoke-operations.md) | `buildAssemblePacket`, extension preserve; no activation engines |
| [`fixtures/toy-world/`](../../fixtures/toy-world/) | Conformance atoms; optional companion pack samples |
| [`fixtures/toy-world/proposed/pack_tw_harbor_companion.json`](../../fixtures/toy-world/proposed/pack_tw_harbor_companion.json) | Harbor Narrative Knowledge Pack companion sample (proposed `modules.pack` + `modules.activation`) |
| [`CONCEPTS.md`](../../CONCEPTS.md) | Domain Profile; Modules (proposed); Extensions |
