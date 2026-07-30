# AssemblePacket Module Recipes — Placement + Activation Trace

> **Status:** Domain Profile handbook (tracked result) — inner dialect shapes handbook-defined under shipped `modules`  
> **Document class:** Domain Profile — packet-level presentation and provenance recipes over closed AssemblePacket wire  
> **Parent:** [`spoke-protocol-layers.md`](spoke-protocol-layers.md) §L8 Context  
> **Bag placement authority:** [`spoke-extension-modules.md`](spoke-extension-modules.md)  
> **Wire SSOT:** `schemas/data/assemble-packet.schema.json` — optional `modules` (`ModuleMap`) shipped on AssemblePacket

## Purpose

This handbook documents two **packet-level** companion recipes under `AssemblePacket.modules`:

| Module key | Role |
|------------|------|
| **`modules.placement`** | Per-entry **where** hints — how each assembled entry prefers to inject into host context |
| **`modules.activation_trace`** | Per-entry **why** provenance — which fire path put the entry in the packet (debug / observability) |

Integrators implement emit and consume of these arrays from this handbook alone. Field names reuse the lore-activation vocabulary (`position_hint`, `outlet`, constant/key reasons) so authors learn one dialect across KnowledgeEntry fire conditions and AssemblePacket provenance.

Baseline `AssemblePacket` remains wire-only slim entries (`entry_id`, `entry_type`, `canonical_name`, optional `snippet`) plus required `extensions`. Ranking, retrieval, token budget, and matching stay **product-local**. `modules` is an optional, capability-flagged `ModuleMap` on `AssemblePacket` ([`spoke-extension-modules.md`](spoke-extension-modules.md)); inner array shapes stay handbook-defined (open bag).

---

## Placement — triad reminder

| Bag | Role for assemble recipes |
|-----|---------------------------|
| **Core fields** | `packet_id`, `entries[]` (`AssembleEntry`), `schema_version` — closed protocol objects |
| **`modules.placement` / `modules.activation_trace`** | Cross-product **functional** dialects: injection placement and activation provenance at packet scope |
| **`extensions.<product>`** | One product’s private assemble telemetry, host-only debug columns, interim staging |

Category rule (normative triad ADR): shared functional dialects use `modules.*`. Product data uses `extensions.<product>`. Publishing placement or activation-trace as a shared key under `extensions.*` is a category error — see [`spoke-extension-modules.md`](spoke-extension-modules.md).

`ModuleMap` is shipped; inner shapes (field tables) remain handbook-defined. Capability-flagged hosts emit/parse `AssemblePacket.modules`; baseline hosts leave `modules` absent.

---

## Envelope home — `AssemblePacket.modules`

**Status:** Optional, capability-flagged `ModuleMap` on `AssemblePacket`. Opt-in via `narrative-modules`.

| Fact | Contract |
|------|----------|
| Outer bag | Optional `modules` on `AssemblePacket` (capability-flagged `ModuleMap`) |
| This handbook’s keys | `placement` → array; `activation_trace` → array |
| Sibling dialects | `modules.activation` on KnowledgeEntry ([`domain-profile-lore-activation.md`](domain-profile-lore-activation.md)); `modules.pack` ([`domain-profile-narrative-knowledge-pack.md`](domain-profile-narrative-knowledge-pack.md)) |
| Wire envelope | Shipped optional `modules`; capability flag `narrative-modules` |
| Inner shapes | Handbook-defined (open bag — unknown module keys round-trip) |
| Scope | **Packet-level lists** first; per-entry `AssembleEntry.modules` is backlog only if packet lists fail product needs |

Integrators emit `modules` on a baseline-valid `AssemblePacket` when declaring `narrative-modules`. Absent and empty `modules` remain valid.

### Illustrative packet with modules (handbook-defined inner dialects)

```text
// AssemblePacket with optional modules (capability-flagged)
{
  "schema_version": 1,
  "packet_id": "pkt_tw_harbor_dawn",
  "entries": [
    {
      "entry_id": "kb_tw_harbor_rules",
      "entry_type": "rule",
      "canonical_name": "Harbor standing rules",
      "snippet": "Dockside law holds at dawn; captains answer to the harbor master."
    },
    {
      "entry_id": "kb_tw_harbor_master",
      "entry_type": "character",
      "canonical_name": "Harbor Master",
      "snippet": "The Harbor Master keeps the dawn ledger and names every berth."
    }
  ],
  "extensions": {},
  "modules": {
    "placement": [
      {
        "entry_id": "kb_tw_harbor_rules",
        "position_hint": "before_defs"
      },
      {
        "entry_id": "kb_tw_harbor_master",
        "position_hint": "after_defs"
      }
    ],
    "activation_trace": [
      {
        "entry_id": "kb_tw_harbor_rules",
        "reason": "constant"
      },
      {
        "entry_id": "kb_tw_harbor_master",
        "reason": "key",
        "matched_key": "Harbor"
      }
    ]
  }
}
```

---

## `modules.placement`

**Status:** Handbook-defined companion array under shipped optional `AssemblePacket.modules.placement`. Describes **where** each entry is injected. Caller-supplied array order is the interchange order; hosts apply product layout after reading the hints.

### Field table (per array element)

| Field | Required | Type | Semantics |
|-------|----------|------|-----------|
| `entry_id` | **yes** | `string` | KnowledgeEntry / `AssembleEntry.entry_id` this hint applies to |
| `position_hint` | **yes** | enum string | Preferred injection region — same vocabulary as lore-activation `position_hint` (see §Position hint values) |
| `depth` | no | `number` | Depth-relative offset when `position_hint` is `"depth"` (chat/history depth scale is product-local) |
| `outlet` | no | `string` | Named injection outlet id when `position_hint` is `"outlet"` (open string; product vocabulary) |

### Position hint values

Identical to [`domain-profile-lore-activation.md`](domain-profile-lore-activation.md) §Position hint:

| `position_hint` | Meaning |
|-----------------|---------|
| `before_defs` | Prefer placement before definition / system blocks |
| `after_defs` | Prefer placement after definition / system blocks |
| `depth` | Prefer depth-relative placement; pair with optional `depth` |
| `outlet` | Prefer a named outlet; pair with `outlet` |

### Integrator reading

| Rule | Detail |
|------|--------|
| Join key | Match `placement[].entry_id` to `entries[].entry_id` |
| Coverage | An entry **MAY** omit a placement row; hosts apply a local default |
| Multiplicity | At most one placement row per `entry_id` in a well-formed packet companion (hosts that emit duplicates define last-wins or reject) |
| Order | Array order is caller-supplied presentation order for the placement list; it does not replace `entries[]` order as the assemble candidate order |
| Source of truth for preference | Entry-level `modules.activation.position_hint` / `outlet` (lore-activation) is the durable authoring home; packet `placement[]` is the **assembled snapshot** for this packet |

### Illustrative shape

```text
// modules.placement — handbook-defined inner array under shipped ModuleMap
[
  {
    "entry_id": "kb_tw_harbor_rules",
    "position_hint": "before_defs"
  },
  {
    "entry_id": "kb_tw_scene_whisper",
    "position_hint": "depth",
    "depth": 4
  },
  {
    "entry_id": "kb_tw_outlet_memory",
    "position_hint": "outlet",
    "outlet": "memory_bank"
  }
]
```

---

## `modules.activation_trace`

**Status:** Handbook-defined companion array under shipped optional `AssemblePacket.modules.activation_trace`. Explains **why** each entry is in the packet — debug and observability provenance. It is **not** a ranking score, priority, or budget column.

### Field table (per array element)

| Field | Required | Type | Semantics |
|-------|----------|------|-----------|
| `entry_id` | **yes** | `string` | KnowledgeEntry / `AssembleEntry.entry_id` this trace applies to |
| `reason` | **yes** | enum string | Fire path that admitted the entry — see §Reason values |
| `matched_key` | no | `string` | Key string that matched when `reason` is `"key"` (product scanner surface) |
| `hop_count` | no | `number` | Relation hop distance when `reason` is `"relation_hop"` (non-negative integer; product expander scale) |

### Reason values

| `reason` | Meaning | Typical Seed / Pool map |
|----------|---------|-------------------------|
| `constant` | Always-on seed path (`modules.activation.constant: true`) | **Seed** |
| `seed` | Explicit seed-set membership (caller Seed list without requiring `constant` on the entry) | **Seed** |
| `key` | Keyword / selective activation match | **Pool** |
| `relation_hop` | Admitted via Relation graph expand | **Pool** |

### Integrator reading

| Rule | Detail |
|------|--------|
| Join key | Match `activation_trace[].entry_id` to `entries[].entry_id` |
| Observability | Traces feed inspectors, CLI debug, and cross-host telemetry with stable field names |
| Ranking | `reason`, `matched_key`, and `hop_count` are provenance only — hosts **do not** treat them as sort keys required by the protocol |
| Coverage | An entry **MAY** omit a trace row; a well-formed debug packet includes a row for every emitted entry |
| Multiplicity | One primary reason per `entry_id` in a well-formed companion; multi-path admission is product-local (choose primary, or emit product detail under `extensions.<product>`) |

### Illustrative shape

```text
// modules.activation_trace — handbook-defined inner array under shipped ModuleMap
[
  {
    "entry_id": "kb_tw_harbor_rules",
    "reason": "constant"
  },
  {
    "entry_id": "kb_tw_harbor_master",
    "reason": "key",
    "matched_key": "Harbor"
  },
  {
    "entry_id": "kb_tw_dawn_dock",
    "reason": "relation_hop",
    "hop_count": 1
  },
  {
    "entry_id": "kb_tw_pov_anchor",
    "reason": "seed"
  }
]
```

---

## Seed vs Pool cross-link

**Primary home** for the full Seed vs Pool assemble pattern (caller sets, candidate order, `maxEntries` truncation): [`domain-profile-narrative-knowledge-pack.md`](domain-profile-narrative-knowledge-pack.md) §Seed vs Pool.

This handbook maps packet **activation_trace** reasons onto that pattern:

| Assemble set | Lore-activation map | Packet `activation_trace.reason` |
|--------------|---------------------|----------------------------------|
| **Seed** | `modules.activation.constant: true` (always-on) | `"constant"` or `"seed"` |
| **Pool** | Keyed entries (`keys` non-empty; `constant` false or absent); relation expand | `"key"` or `"relation_hop"` |

Caller recipe (summary — full steps in Knowledge Pack handbook):

1. Collect **Seed** entries for the active Scope / scene.  
2. Run product activation / retrieval / relation expand → **Pool**.  
3. Concatenate candidates in **caller-chosen order** (common convention: Seed first, then Pool).  
4. Optional product budget trim **before** pure packet build.  
5. `buildAssemblePacket` — order-preserving wire shape only.  
6. Emit `modules.placement` + `modules.activation_trace` under optional `AssemblePacket.modules` for the same `entry_id` set when hosts need interchange of where/why.

---

## Engine boundary

| Concern | Owner |
|---------|-------|
| Keyword / selective matching | **Product-local** engine ([`domain-profile-lore-activation.md`](domain-profile-lore-activation.md)) |
| Relation hop expand, scan window, depth scale | **Product-local** |
| Token budget, ranking, embedding retrieval | **Product-local** |
| Candidate order + `maxEntries` truncate | Caller order + pure `buildAssemblePacket` ([`spoke-operations.md`](spoke-operations.md)) |
| Packet **placement** / **activation_trace** field names | **This handbook** (inner shapes under shipped envelope) |
| Pure library helpers | `@42ch/spoke-operations` / `spoke-operations` — **no** matching, scoring, ranking, or activation code |
| Baseline `AssemblePacket` | Slim entries + `extensions`; optional `modules` is **never required** |

Recipes are **presentation and provenance only**. They document how hosts describe injection layout and fire-path debug on the packet. They do not add ranking scores, token budgets, or retrieval metadata to baseline assemble wire ([`spoke-ops.md`](spoke-ops.md) §assemble wire-only; [`spoke-data-model.md`](spoke-data-model.md) §AssemblePacket).

Product-private assemble telemetry continues under `AssemblePacket.extensions.<product>`. Cross-product where/why dialects use `modules.placement` and `modules.activation_trace`.

---

## Relationship to wire

| Surface | Expectation |
|---------|-------------|
| `schemas/data/assemble-packet.schema.json` | Closed object: `schema_version`, `packet_id`, `entries`, `extensions`, optional `modules` (`ModuleMap`) |
| This handbook | Packet-level `placement[]` + `activation_trace[]` field tables and examples |
| Envelope status | Shipped optional, capability-flagged (`narrative-modules`) — [`spoke-extension-modules.md`](spoke-extension-modules.md) |
| Inner shapes | Handbook-defined; unknown module keys round-trip |
| Capability flags | `narrative-modules` opt-in; no baseline requirement to emit or parse these companions |

---

## Integrator checklist

An integrator can implement packet placement + activation-trace emit/consume from this handbook alone when:

1. Packet companions live as `modules.placement` (array) and `modules.activation_trace` (array) under optional `AssemblePacket.modules`.
2. `position_hint` / `outlet` values match the lore-activation handbook verbatim (`before_defs` \| `after_defs` \| `depth` \| `outlet`).
3. `activation_trace.reason` uses `"constant"` \| `"key"` \| `"relation_hop"` \| `"seed"`; optional `matched_key` / `hop_count` follow the field table.
4. Seed ↔ `constant`/`seed` and Pool ↔ `key`/`relation_hop` follow the Knowledge Pack Seed vs Pool pattern (cross-link only; full recipe there).
5. Arrays are presentation/provenance — no ranking scores or token budgets on the recipes.
6. Engines stay in the product; `spoke-operations` gains no matchers or scorers.
7. Optional `modules` is capability-flagged (`narrative-modules`); inner shapes stay handbook-defined.

---

## Acceptance (profile handbook)

- [x] `modules.placement` array documented (`entry_id`, `position_hint`, optional `depth` / `outlet`)
- [x] `modules.activation_trace` array documented (`entry_id`, `reason`, optional `matched_key` / `hop_count`)
- [x] Position-hint vocabulary aligned with lore-activation (`before_defs` / `after_defs` / `depth` / `outlet`)
- [x] Seed vs Pool cross-link to Knowledge Pack handbook; reason map stated
- [x] Engine boundary: presentation/provenance only; product-local match/scan/budget/rank; no ops matchers
- [x] Envelope home under shipped optional `AssemblePacket.modules`; inner shapes handbook-defined
- [x] Triad ADR cited; envelope capability-flagged (`narrative-modules`)
- [x] No field-table rewrite beyond status framing; no iteration ids; mechanisms only (clean-room public patterns)

---

## See also

| Doc | Topic |
|-----|-------|
| [`spoke-extension-modules.md`](spoke-extension-modules.md) | Core / `modules.*` / `extensions.<product>` triad; capability-flagged envelope; round-trip |
| [`domain-profile-lore-activation.md`](domain-profile-lore-activation.md) | `modules.activation` field table; `position_hint` / `outlet`; constant ↔ Seed pointer |
| [`domain-profile-narrative-knowledge-pack.md`](domain-profile-narrative-knowledge-pack.md) | Knowledge Pack envelope; **primary** Seed vs Pool assemble pattern; `modules.pack` |
| [`domain-profile-narrative-structure.md`](domain-profile-narrative-structure.md) | Sister Domain Profile — Beat / structural mapping |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | Domain Profile principles; L8 AssemblePacket; `narrative-modules` |
| [`spoke-data-model.md`](spoke-data-model.md) | AssemblePacket / AssembleEntry wire fields; ModuleMap |
| [`spoke-ops.md`](spoke-ops.md) | `assemble` wire-only boundary |
| [`spoke-operations.md`](spoke-operations.md) | `buildAssemblePacket`; extension + module merge/preserve; no activation engines |
| [`CONCEPTS.md`](../../CONCEPTS.md) | Domain Profile; Modules (capability-flagged); Extensions; AssemblePacket |
