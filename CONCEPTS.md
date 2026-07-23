# Concepts — SPOKE Domain Vocabulary

Core terms for the SPOKE protocol repository. Each entry defines what the term means *in SPOKE wire form*.

**Product name:** **SPOKE** = **Standardized Programmable Ontology Knowledge Engine** (acronym unchanged).

---

## Protocol layer

### KnowledgeEntry

The atomic **Knowledge Base entry** on the SPOKE wire (L0–L1). A KnowledgeEntry has stable identity (`entry_id`), open-string `entry_type` and `status`, a structured `body`, optional provenance (`source_anchor`), and required `extensions`. Core `status` vocabulary: `provisional`, `confirmed`, `deprecated`, `merged`, `deleted` — transitions enforced by `@42ch/spoke-operations`; `deprecated` → `merged` is excluded (restore to `confirmed` before absorb).

### Relation

A directed edge between two KnowledgeEntries (or KnowledgeEntry ↔ source) identified by `relation_id` and open-string `relation_type`.

### SourceAnchor

A pointer to a source artifact span (manuscript, scene, external locator). Ties KnowledgeEntries and Findings to provenance without embedding product file paths in protocol fields.

### Finding

Checker **output** — consistency, style, structure, or other analysis results. Not a KnowledgeEntry body and not a declarative rule definition.

### Rule

Declarative constraint **input** to `check` (L6). First-class wire object — `schemas/data/rule.schema.json` + field tables in [`spoke-data-model.md`](.mstar/specs/spoke-data-model.md). Distinct from Finding (checker output) and from KnowledgeEntry `entry_type` strings used as ontology labels.

### TimelineEvent

First-class **when-axis** temporal object (L5) — `schemas/data/timeline-event.schema.json` + field tables in [`spoke-data-model.md`](.mstar/specs/spoke-data-model.md). Optional `timeline_scale` tags the L5 projection tier (`brief`, `narrative`, `moment`). **Distinct from** KnowledgeEntry `entry_type: "event"` (ontology label on a KB entry body).

### TimelineScale

L5 Timeline projection tier vocabulary on the wire: core values `brief`, `narrative`, `moment` (open string). Field name **`timeline_scale`** on `TimelineEvent` and optional `Scope` filters.

### Scope

Shared ops selector for `check` and `assemble`. Required `scope_id` (protocol-neutral opaque string) plus optional refinements (`entry_ids`, `entry_types`, `timeline_event_ids`, `source_id`, `timeline_scale`). Product-local scope ids map via op `extensions` or adapters — not required `Scope` fields.

### Domain Profile

How an integrator publishes ontology vocabulary without closing core protocol enums. Open `entry_type` strings + published vocabulary tables in adapter specs — not closed `enum` in core schemas. See [`spoke-protocol-layers.md`](.mstar/specs/spoke-protocol-layers.md).

### spoke-baseline

Declared capability level for spoke-baseline SPOKE compliance: L0–L8 semantics per [`spoke-protocol-layers.md`](.mstar/specs/spoke-protocol-layers.md) §Baseline — excludes required Fork (`l5-fork`) and L2 computable state (`l2-computable`).

### AssemblePacket

Wire-only context-assembly payload: a list of slim entries (`entry_id`, `entry_type`, `canonical_name`, optional `snippet`). Ranking, retrieval, and token budgeting are product-local; see [`spoke-ops.md` §assemble](.mstar/specs/spoke-ops.md#assemble-wire-only-boundary-normative).

### Extensions (`extensions.<namespace>`)

The sole product-specific bag on every data object. Namespace keys are opaque product / integrator ids. Adapters MUST round-trip unknown namespaces and keys verbatim.

---

## Core `entry_type` vocabulary (documented, not enforced)

Open string on `KnowledgeEntry.entry_type`. Full table with typical-use rows: [`spoke-data-model.md` §Open vocabulary](.mstar/specs/spoke-data-model.md#open-vocabulary). Schema `description` core list in `knowledge-entry.schema.json` MUST match that table.

**In the core table:** includes `ability` (skill / power / capability) and `rule` (world-rule ontology label — see dual-concern below), among other documented values.

**Profile-only (not in core table or schema description list):** e.g. `dialogue`, `beat`, `species`, `magic_system` — publish via Domain Profile / adapter specs.

---

## Dual-concern: ontology `"event"` vs TimelineEvent

| Concern | Wire artifact | Example |
|---------|---------------|---------|
| **Ontology / KB fact** | `KnowledgeEntry` with `entry_type: "event"` | “The Battle of Five Armies” as a typed KB node |
| **Timeline / when-axis** | `TimelineEvent` with `timeline_event_id` | Same story beat placed on the Timeline with `timeline_scale: "narrative"` |

Integrators may map one local concept to one or both wire shapes. SPOKE keeps the names separate so check/assemble selectors stay unambiguous.

---

## Dual-concern: ontology `"rule"` vs L6 `Rule`

| Concern | Wire artifact | Example |
|---------|---------------|---------|
| **Ontology / KB label** | `KnowledgeEntry` with `entry_type: "rule"` | “No resurrection without foreshadowing” as a typed KB node |
| **Declarative checker input** | L6 `Rule` with `rule_id`, `kind`, `statement`, `target_entry_types` | Same constraint as portable `check` input via `rules[]` |

`Rule.target_entry_types` filters KnowledgeEntry **`entry_type`** strings (e.g. `character`, `event`) — not kinds of `Rule` objects. `Scope.entry_types` filters KnowledgeEntry labels; `Scope.timeline_event_ids` filters `TimelineEvent` ids — do not cross-wire.

---

## Boundaries

| Term | In SPOKE |
|------|----------|
| **KnowledgeEntry** | Portable wire envelope for one KB entry |
| **World KB / Author Memory** | Product-local stores; mapped via adapters — not redefined as protocol types |
| **Rule** | L6 declarative wire object + `check` input |
| **TimelineEvent / TimelineScale** | L5 when-axis object + `brief` / `narrative` / `moment` vocabulary |
| **Fork** | Optional capability `l5-fork` (not baseline) |

**Invariant:** SPOKE standardizes interchange shapes. It does not own world history implementation, daemon routes, or checker engines.

---

## Spelling

| Context | Spelling |
|---------|----------|
| SPOKE protocol / schemas / packages | **KnowledgeEntry**, **TimelineEvent** |
| TimelineScale wire values (`timeline_scale` field) | **`brief`**, **`narrative`**, **`moment`** (lowercase) |
| Ontology label on a KB entry | `entry_type: "event"` (string value — **not** the `TimelineEvent` type) |
| Ontology label vs L6 object | `entry_type: "rule"` — **not** the L6 `Rule` type |

---

## See also

| Doc | Topic |
|-----|-------|
| [`STRATEGY.md`](STRATEGY.md) | Protocol-not-runtime positioning |
| [`.mstar/specs/spoke-protocol.md`](.mstar/specs/spoke-protocol.md) | Umbrella spec |
| [`.mstar/specs/spoke-protocol-layers.md`](.mstar/specs/spoke-protocol-layers.md) | L0–L8, capability levels, Timeline tiers |
| [`.mstar/specs/spoke-data-model.md`](.mstar/specs/spoke-data-model.md) | Data objects, Rule, TimelineEvent, TimelineScale |
| [`.mstar/specs/spoke-ops.md`](.mstar/specs/spoke-ops.md) | Scope, check/assemble, error envelope |
