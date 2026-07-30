# Domain Profile — Lore Activation

> **Status:** Domain Profile handbook (tracked result) — companion shapes **proposed**  
> **Document class:** Domain Profile — portable lore fire-condition vocabulary over closed SPOKE envelopes  
> **Parent:** [`spoke-protocol-layers.md`](spoke-protocol-layers.md) §Domain Profile  
> **Bag placement authority:** [`spoke-extension-modules.md`](spoke-extension-modules.md)  
> **Wire SSOT:** `schemas/` (unchanged by this profile — no `modules` property on the wire)

## Purpose

This Domain Profile documents how integrators express **lore activation** — the fire conditions under which a KnowledgeEntry prefers to surface into assembled context — using a shared **proposed** companion shape on KnowledgeEntry.

Integrators read this handbook to implement a keyword / selective activation engine and pack import/export mapping against **proposed** `modules.activation`. Matching, scan depth, token budget, and ranking stay **product-local**. The module vocabulary is for **round-trip and pack import** so fire conditions travel between narrative hosts.

Baseline KnowledgeEntry schema is unchanged. **Proposed** `modules.activation` ships on the wire only after the demand gate in [`spoke-extension-modules.md`](spoke-extension-modules.md).

---

## Placement — triad reminder

| Bag | Role for lore activation |
|-----|--------------------------|
| **Core fields** | Identity and body (`entry_id`, `canonical_name`, `body.summary`, …) — closed protocol objects |
| **Proposed `modules.activation`** | Cross-product **functional** dialect: keys, logic, constant/seed hint, order, placement hints |
| **`extensions.<product>`** | One product’s private adapter state (host-only filters, UI flags, interim staging) |

Category rule (normative triad ADR): shared functional dialects use **proposed** `modules.*`. Product data uses `extensions.<product>`. Publishing activation as a shared key under `extensions.*` is a category error — see [`spoke-extension-modules.md`](spoke-extension-modules.md).

Until the demand gate opens, readers treat every **proposed** `modules.activation` mention as a **handbook companion shape**, not an optional JSON property available on committed schemas.

---

## Proposed `modules.activation` — portable subset

**Status:** **Proposed** companion object. Not present on any committed schema. Integrators stage this shape in product storage or pack sidecars so multiple hosts converge before a schema slice.

### Field table

| Field | Required | Type | Semantics |
|-------|----------|------|-----------|
| `keys` | **yes** | `string[]` | Primary activation triggers (aliases, names, phrases). Empty array is valid only when `constant` is `true` (always-on entry). |
| `secondary_keys` | no | `string[]` | Secondary / selective triggers evaluated with `logic` against the primary set |
| `logic` | no | enum string | How primary and secondary keys combine — see §Logic values. Default integrator reading when omitted and secondary keys are present: `and_any` |
| `constant` | no | `boolean` | When `true`, entry is an always-on **seed** candidate (see §Seed vs constant) |
| `order` | no | `number` | Insertion / scan order hint (lower first is the common integrator convention; product engines define sort) |
| `priority` | no | `number` | Tie-break or budget preference among activated entries (product-defined scale) |
| `position_hint` | no | enum string | Preferred placement relative to definitions / depth / outlet — see §Position hint |
| `outlet` | no | `string` | Named injection outlet id when `position_hint` is `"outlet"` (open string; product vocabulary) |
| `match` | no | enum string | How key strings match scanned context — see §Match mode |

### Logic values

| `logic` | Meaning |
|---------|---------|
| `and_any` | Activate when **any** primary key matches **and** **any** secondary key matches (selective AND-of-anys). Default integrator reading when secondary keys are present and `logic` is omitted. |
| `and_all` | Activate when **all** listed primary keys match **and** **all** secondary keys match |
| `not_any` | Activate when primary keys match under product primary rules **and** **no** secondary key matches |
| `not_all` | Activate when primary keys match **and** it is **false** that every secondary key matches |

When `secondary_keys` is absent or empty, only primary `keys` participate; `logic` is ignored.

### Position hint values

| `position_hint` | Meaning |
|-----------------|---------|
| `before_defs` | Prefer placement before definition / system blocks |
| `after_defs` | Prefer placement after definition / system blocks |
| `depth` | Prefer depth-relative placement (chat/history depth is product-local) |
| `outlet` | Prefer a named outlet; pair with `outlet` |

### Match mode values

| `match` | Meaning |
|---------|---------|
| `literal` | Substring or exact-token match per product scanner (default integrator reading when omitted) |
| `regex` | Keys are regular expressions (flavor is product-local) |
| `whole_word` | Whole-word / boundary-aware match |

### Illustrative shape (proposed companion — not wire schema)

```text
// proposed modules.activation — handbook companion only
{
  "keys": ["Harbor", "dawn dock"],
  "secondary_keys": ["chapter 1"],
  "logic": "and_any",
  "constant": false,
  "order": 100,
  "priority": 10,
  "position_hint": "after_defs",
  "match": "literal"
}
```

```text
// proposed modules.activation — always-on seed candidate
{
  "keys": [],
  "constant": true,
  "order": 0,
  "priority": 100,
  "position_hint": "before_defs"
}
```

Product-private fields stay in `extensions.<product>`. They do not extend this portable subset.

---

## Standalone-snippet invariant

Activation keys and titles are **trigger metadata**. Injected prose must stand alone.

| Surface | Contract |
|---------|----------|
| `body.summary` | **MUST** read as a complete lore fact without requiring the reader to know trigger keys |
| `AssemblePacket` entry `snippet` | **MUST** be self-contained context for the model; same rule as summary |
| Triggers / aliases | Live in **proposed** `modules.activation.keys` (preferred) or a documented tag convention (e.g. `alias:…` on `body.tags`) |

Integrators map display identity to `canonical_name` and activation aliases to **proposed** `modules.activation.keys`. Comma-joined triggers do not belong in `canonical_name`.

This invariant raises check and assemble quality across products without schema churn.

---

## Seed vs constant (short pointer)

| Activation field | Assemble role |
|------------------|---------------|
| `constant: true` | **Seed** — always-on candidates (world rules, POV, active scene anchors) |
| Keyed entries (`keys` non-empty, `constant` false/absent) | **Pool** — activation- and relation-expand candidates |

Full **Seed vs Pool** assemble pattern (caller sets, packet order, `maxEntries` truncation) lives in the companion Narrative Knowledge Pack handbook, `domain-profile-narrative-knowledge-pack.md`. This profile only maps `constant` ↔ seed and keyed entries ↔ pool so pack importers preserve the distinction.

---

## Relation-first recursion

Lore adjacency expands best as a **graph**, not as string mentions inside Content.

| Step | Guidance |
|------|----------|
| 1 | Emit `Relation` edges for lore adjacency (`related_to`, `located_in`, `member_of`, profile-open types, …) |
| 2 | Product expanders hop Relations under **local** budget and scan policy |
| 3 | Importers **MAY** synthesize Relations from legacy key-mention recursion as a **one-shot migration** tool |

Prefer Relation hop-expand over scanning activated Content for further key strings. Stringly key-mention recursion remains a product migration path; portable packs encode durable links as `Relation` objects on existing wire shapes ([`spoke-data-model.md`](spoke-data-model.md) §Relation).

---

## Engine boundary

| Concern | Owner |
|---------|-------|
| Keyword / selective matching | **Product-local** engine |
| Scan window, depth, token budget, ranking | **Product-local** |
| Round-trip field names and pack import mapping | **Proposed** `modules.activation` (this handbook) |
| Pure library helpers | `@42ch/spoke-operations` / `spoke-operations` — **no** matching, scan, or activation code |
| Baseline KnowledgeEntry | **Proposed** module fields are **never required** |

`AssemblePacket` remains wire-only assemble I/O ([`spoke-ops.md`](spoke-ops.md), [`spoke-operations.md`](spoke-operations.md)). Placement traces and activation debug on the packet belong to future packet-module recipes or `AssemblePacket.extensions.<product>` — outside this handbook’s KE field table.

---

## Integrator checklist

An integrator can implement activation storage + pack mapping from this handbook alone when:

1. Per-entry fire conditions stage as **proposed** `modules.activation` (field table above).
2. `body.summary` / assemble `snippet` satisfy the standalone-snippet invariant.
3. `constant: true` entries feed the seed set; keyed entries feed the pool (pack handbook for full assemble recipe).
4. Graph expansion uses `Relation` edges; key-mention recursion is migration-only.
5. Engines stay in the product; `spoke-operations` gains no matchers.
6. Wire schemas remain closed; triad ADR governs promotion of `modules.activation` (proposed) to schema.

---

## Acceptance (profile handbook)

- [x] **Proposed** `modules.activation` portable subset documented (keys, secondary_keys, logic, constant, order, priority, position_hint, outlet, match)
- [x] Standalone-snippet invariant stated for `body.summary` and AssemblePacket `snippet`
- [x] Seed ↔ `constant` / Pool ↔ keyed pointer with cross-link to Knowledge Pack handbook
- [x] Relation-first recursion recommendation; one-shot importer synthesis allowed
- [x] Engine boundary: product-local match/scan/budget; no ops matchers; module never required on baseline KE
- [x] Triad ADR cited; every `modules.activation` mention marked **proposed**
- [x] No schema edits; no iteration ids; mechanisms only (clean-room public patterns)

---

## See also

| Doc | Topic |
|-----|-------|
| [`spoke-extension-modules.md`](spoke-extension-modules.md) | Core / proposed `modules.*` / `extensions.<product>` triad; demand gate |
| `domain-profile-narrative-knowledge-pack.md` (companion handbook) | Knowledge Pack envelope; full Seed vs Pool assemble pattern; proposed `modules.pack` |
| [`assemble-module-recipes.md`](assemble-module-recipes.md) | Packet-level proposed `modules.placement` + `modules.activation_trace` (where/why companions to per-entry activation) |
| [`domain-profile-narrative-structure.md`](domain-profile-narrative-structure.md) | Sister Domain Profile — Beat / structural mapping |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | Domain Profile principles; L8 AssemblePacket |
| [`spoke-data-model.md`](spoke-data-model.md) | KnowledgeEntry, Relation, BodyAttribute, Extensions |
| [`spoke-operations.md`](spoke-operations.md) | Pure assemble helpers (wire-only; no activation engines) |
| [`CONCEPTS.md`](../../CONCEPTS.md) | Domain Profile; Modules (proposed); Extensions |
