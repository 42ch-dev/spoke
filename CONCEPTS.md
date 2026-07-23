# Concepts — SPOKE Domain Vocabulary

Core terms for the SPOKE protocol repository. Each entry defines what the term means *in SPOKE wire form*, not inside any single product runtime.

---

## Protocol layer

### Keyblock
The atomic narrative knowledge unit on the SPOKE wire. A Keyblock has stable identity (`keyblock_id`), open-string `block_type` and `status`, a structured `body`, optional provenance (`source_anchor`), and required `extensions`. Core `status` vocabulary: `provisional`, `confirmed`, `deprecated`, `merged`, `deleted` — cross-product transitions enforced by `@42ch/spoke-operations` (v0-iter004); `deprecated` → `merged` is excluded (restore to `confirmed` before absorb). Products map their local entities to Keyblocks via adapters (next iteration).

### Relation
A directed edge between two Keyblocks (or Keyblock ↔ source) identified by `relation_id` and open-string `relation_type`.

### SourceAnchor
A pointer to a source artifact span (manuscript, scene, external locator). Ties Keyblocks and Findings to provenance without embedding product file paths in protocol fields.

### Finding
Checker **output** — consistency, style, structure, or other analysis results. Not a Keyblock body and not a declarative rule definition.

### Rule
Declarative constraint **input** to `check` (L6). First-class wire object in v0-iter003 — `schemas/data/rule.schema.json` + field tables in [`spoke-data-model.md`](.mstar/specs/spoke-data-model.md). Distinct from Finding (checker output) and from Keyblock `block_type` strings products may use for ontology labels.

### Event
First-class **when-axis** temporal object (L5) in v0-iter003 — `schemas/data/event.schema.json` + field tables in [`spoke-data-model.md`](.mstar/specs/spoke-data-model.md). Optional `timeline_scale` tags the L5 projection tier (`brief`, `narrative`, `moment`). Distinct from Keyblock `block_type: "event"` ontology labels.

### TimelineScale
L5 Timeline projection tier vocabulary on the wire: core values `brief`, `narrative`, `moment` (open string). Field name **`timeline_scale`** on `Event` and optional `Scope` filters. Standardizes Timeline-dimension semantics — not product canvas surface names.

### Scope
Shared ops selector for `check` and `assemble`. Required `scope_id` (protocol-neutral opaque string) plus optional refinements (`keyblock_ids`, `block_types`, `event_ids`, `source_id`, `timeline_scale`). World/Book/product ids map via op `extensions` or adapters — not required `Scope` fields.

### Domain Profile
How a product publishes its ontology mapping without closing core protocol enums. Open `block_type` strings + published vocabulary tables in adapter specs — not closed `enum` in core schemas. See [`spoke-protocol-layers.md`](.mstar/specs/spoke-protocol-layers.md).

### spoke-baseline
Declared capability level for v0-iter003 SPOKE compliance: L0–L8 semantics per [`spoke-protocol-layers.md`](.mstar/specs/spoke-protocol-layers.md) §Baseline — excludes required Fork (`l5-fork`) and L2 computable state (`l2-computable`).

### AssemblePacket
Wire-only context-assembly payload: a list of slim entries (`keyblock_id`, `block_type`, `canonical_name`, optional `snippet`). Ranking, retrieval, and token budgeting are product-local; see [`spoke-ops.md` §assemble](.mstar/specs/spoke-ops.md#assemble-wire-only-boundary-normative).

### Extensions (`extensions.<namespace>`)
The sole product-specific bag on every data object. Namespace keys are product ids (`nexus`, `creader`, …). Adapters MUST round-trip unknown namespaces and keys verbatim.

---

## Boundaries (what SPOKE is not)

| Term | In SPOKE | Product-local (not redefined here) |
|------|----------|-------------------------------------|
| **Keyblock** | Portable wire envelope | — |
| **World KB** | Mapped *to* Keyblocks via adapters | Nexus world's structured knowledge store |
| **Author Memory** | May appear under `extensions.<namespace>` | Creator profile, SOUL, session memory pipelines |
| **Rule** | L6 declarative wire object + `check` input | Creader `KnowledgeEntryType: "rule"` mapping in adapters |
| **Event / Timeline tiers** | L5 `Event` + `timeline_scale` vocabulary | Nexus World/Work Canvas surfaces, carrier UI |
| **Fork** | Optional capability `l5-fork` (not baseline) | Nexus immutable world-history branch semantics |

**Invariant:** SPOKE standardizes interchange shapes. It does not own world history implementation, daemon routes, or checker engines.

---

## Spelling

| Context | Spelling |
|---------|----------|
| SPOKE protocol / schemas / packages | **Keyblock** |
| Nexus product code / UI | **KeyBlock** (product spelling) |
| Creader knowledge entries | **KnowledgeEntry** (mapped in adapters later) |
| TimelineScale wire values (`timeline_scale` field) | **`brief`**, **`narrative`**, **`moment`** (lowercase) |
| Nexus Timeline surfaces (product UI) | Brief, Narrative, Moment — map to wire values in adapters |

---

## See also

| Doc | Topic |
|-----|-------|
| [`STRATEGY.md`](STRATEGY.md) | Protocol-not-runtime positioning |
| [`.mstar/specs/spoke-protocol.md`](.mstar/specs/spoke-protocol.md) | Umbrella spec |
| [`.mstar/specs/spoke-protocol-layers.md`](.mstar/specs/spoke-protocol-layers.md) | L0–L8, capability levels, Timeline tiers |
| [`.mstar/specs/spoke-data-model.md`](.mstar/specs/spoke-data-model.md) | Data objects, Rule, Event, TimelineScale |
| [`.mstar/specs/spoke-ops.md`](.mstar/specs/spoke-ops.md) | Scope, check/assemble, error envelope |
