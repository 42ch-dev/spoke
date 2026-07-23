# SPOKE Protocol Layers (L0–L8)

> **Status:** Normative (v0-iter003) — architect-locked 2026-07-23  
> **Document class:** Normative — layer model + capability levels  
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)  
> **Research source:** Spoke Protocol Research canvas (nine layers)

## Purpose

Define the **nine conceptual layers** of the SPOKE wire dialect and how products **claim compliance** at a **declared capability level**.

This document is the integrator-facing map from research vocabulary to wire artifacts. It does **not** duplicate field-level JSON Schema tables — those live in [`spoke-data-model.md`](spoke-data-model.md), [`spoke-ops.md`](spoke-ops.md), and `schemas/`.

## User value

| Without this spec | With this spec |
|-------------------|----------------|
| “Nine layers” is canvas-only | Integrators declare **baseline** vs **optional** capabilities |
| Layer ↔ file mapping is tribal knowledge | Each layer points to schema / op / library helper |
| Domain Profile sounds like a fork | Profile is an **ontology adapter** — core stays open vocabulary |

## Layers

Read top-down: identity → ontology → body → provenance → graph → time → constraints → findings → context packet.

| Layer | Concept | Baseline (v0-iter003 target) | Optional capability | Primary wire artifacts |
|-------|---------|------------------------------|---------------------|------------------------|
| **L0 Envelope** | Identity + `schema_version` | Required on all durable objects | — | All `schemas/data/*`; `schema_version` in `common.schema.json` |
| **L1 Ontology** | `block_type` + Domain Profile | Open `block_type` string + published core vocabulary | Profile-specific type tables (adapter/docs) | `keyblock.schema.json`; `CONCEPTS.md`; future adapter specs |
| **L2 Body** | summary / attributes / tags | Structured `body` (`additionalProperties: true` subtree) | Computable / WASM `state` in body | `keyblock.schema.json` `body` |
| **L3 Provenance** | SourceAnchor | Refs over full manuscript | — | `source-anchor.schema.json`; optional on Keyblock |
| **L4 Graph** | Relation | Typed directed edges | OCC / revision as product concern | `relation.schema.json`; `relate` op |
| **L5 Temporal** | Timeline dimension + Event + optional Fork | **`Event` wire object** (when-axis) + **Timeline projection tiers** (`brief` / `narrative` / `moment`) as structured Timeline vocabulary | **Fork** (world-history branch) — not required for baseline | `event.schema.json` (**`rule-event` plan** — architect-locked target wire, not yet committed); tier vocabulary in this spec + data-model; Fork deferred optional |
| **L6 Constraint** | Rule / Prohibition | **`Rule` wire object** | Prohibition variants as open vocabulary | `rule.schema.json` (**`rule-event` plan** — architect-locked target wire, not yet committed); `check` op |
| **L7 Finding** | Checker output lifecycle | `Finding` + status vocabulary | Richer product overlays in `extensions` | `finding.schema.json`; `check` op; `spoke-operations` transitions |
| **L8 Context** | AssemblePacket | Shared packet shape | Trim/rank policy — product-local | `assemble-packet.schema.json`; `assemble` op (wire-only) |

### Hard boundaries (product)

| Pair | Rule |
|------|------|
| **Rule vs Finding** | `Rule` = declarative **input** to checkers; `Finding` = checker **output**. Never interchange types. |
| **Check vs Assemble** | `check` returns `Finding[]`; `assemble` returns `AssemblePacket` only. No merged op. |
| **Event vs Keyblock `block_type: event`** | `Event` is a first-class temporal object (L5). Keyblock `block_type` may still use `"event"` as an ontology label — different concerns. |
| **Timeline tiers vs product canvas** | `brief` / `narrative` / `moment` are **protocol Timeline-dimension labels** (semantic zoom of the same when-axis). They are **not** a requirement to ship Nexus World/Work Canvas surfaces or Creader UI. |
| **Scope** | Ops selectors use shared `Scope` with required **`scope_id`** (protocol-neutral opaque string). World/Book/product ids belong in op **`extensions`** or adapters — not `Scope` required fields. |

## L5 — Timeline projection tiers (normative vocabulary)

Spoke Protocol Research maps Nexus product Timeline surfaces (**Brief / Narrative / Moment** — UI labels) onto L5 Temporal. **v0-iter003 product lock:** those three names are **standardizable** on the wire as lowercase **`brief` / `narrative` / `moment`** — structured expression of the **Timeline dimension**, not product-local forever.

| Tier | Role on the when-axis | Typical carrier (informative — architect locks wire) |
|------|----------------------|------------------------------------------------------|
| **`brief`** | Coarse shape / era / age-at-a-glance | Often era-scale markers (e.g. Keyblock `block_type` vocabulary such as `era`) |
| **`narrative`** | Ordered story events on the Timeline | First-class `Event` objects and/or event-shaped Keyblocks |
| **`moment`** | Fine grain (scene / beat / beat-local) | Finer temporal units; product may keep some carriers local until wire exists |

**Wire field (architect-locked target wire):** optional `timeline_scale` on `Event` and as an optional `Scope` refinement filter — values `brief` | `narrative` | `moment` (open string; core vocabulary documented, not `enum`). Shared def: `common.schema.json#/definitions/TimelineScale` — committed in sibling plan **`ops-harden`** (not yet in tree).

**Rules:**

1. Tier names on the wire (when present) MUST use the open vocabulary **`brief` | `narrative` | `moment`** (lowercase) — not product UI strings.
2. A product MAY implement a subset of tiers and still claim baseline if it ships **`Event`** for the when-axis; declaring which tiers it exposes is part of capability / profile docs.
3. Tier ≠ Fork. Fork remains optional capability `l5-fork`.
4. Tier ≠ AssemblePacket. **L8 context assembly** consumes temporal scope; it does not redefine Timeline tiers (including the `moment` tier).
5. Research canvas historically listed Timeline Brief/Narrative/Moment (product carriers) under “keep local” — **superseded for SPOKE wire vocabulary** by this iteration lock; carrier mapping details stay adapter/product concerns.

## Capability levels

Products MUST declare which level they implement when claiming SPOKE compliance.

### Baseline (`spoke-baseline`)

Required for “SPOKE baseline” claims in v0-iter003:

| Includes | Excludes |
|----------|----------|
| L0–L4 via Keyblock, Relation, SourceAnchor | Required Fork |
| L5 `Event` wire object | Required WASM / computable body state |
| L6 `Rule` wire object | Adapter packages |
| L7 Finding + core status vocabulary | Shared runtime / daemon |
| L8 AssemblePacket wire shape | Ranking/retrieval in assemble protocol fields |
| Five ops wire families + `error-envelope` on failures | Transport bindings (HTTP/MCP) |

### Optional flags (declare explicitly)

| Flag | Layer | Meaning |
|------|-------|---------|
| `l2-computable` | L2 | Keyblock `body` may carry computable/WASM `state` per architect-locked schema optional fields |
| `l5-fork` | L5 | Product implements Fork semantics (immutable world history branches) — wire shape TBD in a future iteration |

Baseline compliance MUST NOT require either flag.

## Domain Profile

**Domain Profile** is how a product or integration publishes its ontology mapping **without** closing the core protocol.

| Principle | Detail |
|-----------|--------|
| Core stays open | `block_type`, statuses, relation types remain open strings in schemas |
| Profile documents vocabulary | Published tables (e.g. Nexus world KB types, Creader knowledge entry types) live in **adapter specs** or product docs — not closed `enum` in core |
| Profile is not a fork | Products MUST NOT fork `keyblock.schema.json` for profile-specific types; use open strings + `extensions.<namespace>` |
| Adapter role (future) | `adapters/*` maps product DTOs ↔ SPOKE; calls `@42ch/spoke-operations` for shared gates |

## Layer ↔ artifact map

| Layer | Data schema (`schemas/data/` or `common/`) | Op wire (`schemas/ops/`) | Library (`@42ch/spoke-operations`) |
|-------|--------------------------------------------|--------------------------|-------------------------------------|
| **L0 Envelope** | `common/common.schema.json` (`SchemaVersion`, `ExtensionMap`, `Timestamp`, `SourceSpan`); `schema_version` on all data objects | All ops request/response envelopes; `common/error-envelope.schema.json` | — |
| **L1 Ontology** | `data/keyblock.schema.json` (`block_type`, `canonical_name`, `status`) | `upsert-*`, `promote-*` | `validatePromoteRequest` |
| **L2 Body** | `keyblock.schema.json` → `body` subtree (`additionalProperties: true`) | `upsert-*` | — |
| **L3 Provenance** | `data/source-anchor.schema.json`; optional on Keyblock / Finding / Event / Rule | `promote-*`; `check-*` / `assemble-*` via `Scope.source_id` refinement | — |
| **L4 Graph** | `data/relation.schema.json` | `relate-*` | — |
| **L5 Temporal** | `data/event.schema.json` (**`rule-event` plan**); `common/…#/definitions/TimelineScale` (**`ops-harden` plan**); tier vocabulary §L5 | `check-*`, `assemble-*` via `Scope.event_ids` / `Scope.timeline_scale` refinements (**`ops-harden` plan**); Event upsert via product binding or future op — **no new op in v0-iter003** | — |
| **L6 Constraint** | `data/rule.schema.json` (**`rule-event` plan**) | `check-*` (`rule_refs` + embedded `rules[]`) | — (no helper this iteration) |
| **L7 Finding** | `data/finding.schema.json` | `check-*` response `findings[]` | `isValidFindingStatusTransition`, `transitionFindingStatus` |
| **L8 Context** | `data/assemble-packet.schema.json` | `assemble-*` | `buildAssemblePacket`, `keyblockToAssembleEntry` |

### Shared cross-layer defs (`schemas/common/common.schema.json`)

| Definition | Used by | Role |
|------------|---------|------|
| `Scope` | `check-request`, `assemble-request` | Protocol-neutral selector; required `scope_id` — **`ops-harden` plan** (architect-locked target wire) |
| `TimelineScale` | `Event.timeline_scale`, `Scope.timeline_scale` | L5 tier vocabulary (`brief` / `narrative` / `moment`) — **`ops-harden` plan** (architect-locked target wire) |
| `ExtensionMap` | All data objects + all ops | Product namespace bag |
| `ErrorEnvelope` | `schemas/common/error-envelope.schema.json` | All ops failure branch (`error` attachment) |

Field-level tables: [`spoke-data-model.md`](spoke-data-model.md) (Rule, Event, TimelineScale, §Rule vs Finding) and [`spoke-ops.md`](spoke-ops.md) (Scope, error envelope, check `rule_refs` + `rules[]`, assemble).

## Acceptance (layers spec)

- [x] Integrator can name baseline vs optional flags without reading research canvas
- [x] Every baseline layer row maps to normative semantics (field tables + layer rules in this doc, data-model, ops)
- [ ] Every baseline layer row maps to at least one **committed** schema or op family — **L5/L6** (`event.schema.json`, `rule.schema.json`) and **shared `Scope` / `TimelineScale`** defs pending sibling plans **`rule-event`** and **`ops-harden`**; L5 Event consumers may use product binding until `rule-event` lands
- [x] Rule vs Finding and Check vs Assemble boundaries appear in this doc and cross-link data/ops specs
- [x] Domain Profile section prevents “closed enum in core” misread
- [x] Layer ↔ artifact matrix is complete (schema / op / library helper per layer; pending commits tagged with sibling plan)

## Non-goals (this spec)

- Adapter package implementations or field-map tables
- Conformance fixtures / golden toy-world
- Fork wire schema (optional flag only)
- HTTP/gRPC/MCP route tables
- Closed forever enums for ontology

## See also

| Doc | Topic |
|-----|-------|
| [`spoke-protocol.md`](spoke-protocol.md) | Umbrella — three columns (data / ops wire / operations library) |
| [`spoke-data-model.md`](spoke-data-model.md) | Data object field detail |
| [`spoke-ops.md`](spoke-ops.md) | Ops wire + Scope + error envelope |
| [`spoke-operations.md`](spoke-operations.md) | Hand-written lifecycle helpers |
| [`.mstar/roadmap.md`](../roadmap.md) | Thrust B — nine layers on the wire |
| [`CONCEPTS.md`](../../CONCEPTS.md) | Scope, Domain Profile, Event, Rule vocabulary |
