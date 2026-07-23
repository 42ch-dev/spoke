# SPOKE Roadmap

> **Status:** Living product roadmap (durable result)  
> **Authority:** Direction for future work — not a substitute for local `delivery-compass.md` (process artifact)  
> **Research source:** Spoke Protocol Research canvas (nine protocol layers + dual standardization mandate)

This file records the **two durable thrusts** SPOKE must deliver. Work may slice delivery; neither thrust may be dropped. Thrust A now spans **three delivery columns** (data wire, ops wire, ops behavior library).

---

## Thrust A — Dual standardization (must all land)

SPOKE is **not** a runtime. It is a shared wire dialect with **data + ops** protocol surfaces, plus a thin **hand-written operations library** for lifecycle invariants. Shipping only schemas (or only adapters) is incomplete.

| Surface | Mandate | What “done” means |
|---------|---------|-------------------|
| **1. Data structure standardization** | Durable narrative atoms and related objects are defined once in `schemas/` (+ normative specs) | Products can round-trip Keyblock-shaped knowledge without inventing parallel DTOs; extensions preserve product-specific bags |
| **2. Operations wire standardization** | Transport-agnostic ops **request/response** families cover the lifecycle products already share | Upsert, extract→promote, relate, check, assemble (and future ops) have stable I/O shapes; products bind transport locally |
| **3. Operations behavior library** | Shared **hand-written** helpers encode protocol lifecycle invariants that JSON Schema cannot express | Products call the same pure functions for promote gates, revision/OCC checks, Finding status transitions, extension round-trip preserve, packet construction — without sharing a daemon |

**Invariant:** Data schemas without ops leave products guessing how to mutate and check; ops wire without shared behavior becomes folklore copy-paste; a behavior library that grows I/O or LLM calls becomes a forbidden third runtime. Roadmap progress is measured on **all three** columns.

### Package cut (durable)

| Package | Authored how | Role |
|---------|--------------|------|
| `@42ch/spoke-schemas` / crate `spoke-schemas` | **Generated** from `schemas/` | Types only — wire truth |
| `@42ch/spoke-operations` (TS first; Rust optional later) | **Hand-written** (not codegen) | Pure behavior on schema types — lifecycle & invariants |
| `adapters/nexus`, `adapters/creader` | Hand-written product maps | Product DTO ↔ SPOKE; may **call** `spoke-operations`, must not reimplement its invariants |

**`spoke-operations` boundary (hard):**

- **In:** pure functions / small state machines over SPOKE types (e.g. promote acceptance checks, Keyblock revision conflict rules, Finding lifecycle transitions, `extensions` merge/preserve, AssemblePacket builders that do **not** rank/retrieve).
- **Out:** storage, HTTP/MCP servers, LLM calls, ranking/retrieval engines, product-specific Guardian/detector logic, silent write paths that violate human-in-loop promote.

This is how research’s “standardize I/O **and lifecycle** — not a third runtime” becomes code without collapsing into adapters-only folklore.

### Current slice (v0.1 — delivered)

| Surface | v0.1 intent | Explicit gaps (by design) |
|---------|-------------|---------------------------|
| Data | Five required objects: Keyblock, Relation, SourceAnchor, Finding, AssemblePacket + `extensions` | `Rule` deferred; Event/Fork not required fields; no fixtures |
| Ops wire | Five ops × request/response (10 schemas); `assemble` **wire-only** | No compute/ranking in protocol; no HTTP/MCP binding; adapters empty |
| Ops library | **Out of v0.1** — schemas + generated types only | `spoke-operations` deferred to operations library first slice (see Sequencing) |

### Current slice (operations library first slice — delivered 2026-07-23)

| Surface | Intent | Explicit gaps (by design) |
|---------|--------|---------------------------|
| Ops library | `@42ch/spoke-operations` first slice: promote gate, Finding transitions, extension preserve, AssemblePacket builders | Rust crate deferred; full OCC deferred; no adapter code |
| Integrator docs | Consumer README EN/CN | Maintainer/harness process not in README body |

### Current slice (protocol layers + Rule/Event — delivered 2026-07-23)

| Surface | Intent | Explicit gaps (by design) |
|---------|--------|---------------------------|
| Data | + `Rule` (L6) + `Event` (L5); `TimelineScale` in `common.schema.json` | Fork optional; no fixtures yet |
| Ops wire | Scope neutrality; universal `error-envelope`; Rule-aware `check` | No transport binding; no new op families |
| Normative layers | [`spoke-protocol-layers.md`](specs/spoke-protocol-layers.md) — L0–L8 + capability levels | Adapter packages excluded per user lock |
| Ops library | No default growth unless pure invariant emerges | OCC emit, Keyblock status, Scope/upsert/relate gates deferred to operations library deepen |

### Current slice (operations library deepen + fixtures — delivered 2026-07-23)

| Surface | Intent | Explicit gaps (by design) |
|---------|--------|---------------------------|
| Ops library | OCC compare (emit reserved codes), Keyblock status transitions, active uniqueness; Scope match; upsert/relate gates; `SpokeReject`↔`error-envelope` map | Rust crate deferred; no storage fetch |
| Conformance | `fixtures/toy-world/` protocol JSON + CI schema validation (`fixtures/toy-world/tests/`; `@42ch/spoke-fixture-toy-world`) | No product DTO round-trip; no adapter packages |
| Integrator path | Callable actions + golden graph **before** adapters | Adapters, Fork, `project` op remain next |

Later work deepens adapters and optional capabilities (Fork, L2 computable) — still not a shared runtime.

---

## Thrust B — Nine protocol layers (realize on the wire)

Research framed SPOKE as **nine conceptual layers** (L0–L8). **Protocol layers + Rule/Event** promotes that model to normative protocol text in [`spoke-protocol-layers.md`](specs/spoke-protocol-layers.md) — integrators declare **baseline** vs optional **`l2-computable`** / **`l5-fork`** capability flags.

Read top-down: identity → ontology → body → provenance → graph → time → constraints → findings → AI packet.

Products may omit optional capabilities (e.g. L5 Fork, L2 computable state) and still claim SPOKE compliance at a **declared capability level** — see capability levels in the layers spec.

| Layer | Concept | Protocol direction |
|-------|---------|--------------------|
| **L0 Envelope** | Identity + `schema_version` + scope | Required Keyblock envelope fields; immutable id + versioned content |
| **L1 Ontology** | `block_type` + Domain Profile | Open-string types + published core vocabulary; profiles as adapters (not forked cores) |
| **L2 Body** | summary / attributes / tags / optional state | Structured body; computable/WASM optional capability |
| **L3 Provenance** | SourceAnchor | Refs over full manuscript; excerpt/summary optional |
| **L4 Graph** | Relation (+ OCC as product concern) | Typed edges between Keyblocks (and anchors when needed) |
| **L5 Temporal** | Event + Timeline tiers + optional Fork | when-axis `Event`; projection vocabulary `brief`/`narrative`/`moment`; Fork optional capability |
| **L6 Constraint** | Rule / Prohibition | First-class `Rule` wire object (protocol layers deepen) |
| **L7 Finding** | Check output lifecycle + evidence | Findings are not Keyblock bodies; status lifecycle on the wire |
| **L8 Context** | AssemblePacket | Shared context-assembly packet shape (`assemble` op); trim/rank policy product-local — distinct from `check` |

**How Thrust B relates to Thrust A**

- Layers L0–L8 are the **conceptual map** of what the **data** surface must eventually express (and what **ops wire** must carry in and out).
- Thrust A is the **delivery cut**: every layer that is normative must appear as schemas + ops I/O, and cross-product **lifecycle rules** belong in `spoke-operations` when they cannot be expressed as schema alone.
- v0.1 **partially** covered L0–L4 (via Keyblock/Relation/SourceAnchor), L7, L8; operations library first slice delivered the operations behavior library (column 3); protocol layers spec covers L0–L8 semantics + Domain Profile + L5 Timeline tiers — `Rule`/`Event` wire schemas delivered in **`rule-event`**.

---

## Sequencing (indicative)

Order is guidance for future compasses — adjust when grill locks say otherwise.

1. **v0.1 (delivered 2026-07-23)** — Bootstrap: data + ops **wire** schema SSOT, codegen `spoke-schemas` packages, empty adapters, CI. Nine-layer model referenced here (normative L0–L8 chapter delivered in protocol layers + Rule/Event). **No** `spoke-operations` yet.
2. **Operations library first slice (delivered 2026-07-23)** — `@42ch/spoke-operations` on generated types: lifecycle helpers and protocol invariants only (see Package cut). Integrator README EN/CN. Spec detail in [`spoke-operations.md`](specs/spoke-operations.md).
3. **Protocol layers + Rule/Event (delivered 2026-07-23)** — Normative L0–L8 + capability levels; `Rule` + `Event` data deepen (schemas in **`rule-event`**); ops wire harden (Scope, Check≠Assemble, error-envelope — committed wire in **`ops-harden`**). See [`spoke-protocol-layers.md`](specs/spoke-protocol-layers.md). **No** adapters.
4. **Operations library deepen + fixtures (delivered 2026-07-23)** — Deepen `@42ch/spoke-operations` with OCC emit, Keyblock status, uniqueness, Scope/upsert/relate gates, error-map helpers; **`fixtures/toy-world`** conformance graph + AJV/Vitest harness at `fixtures/toy-world/tests/` (`@42ch/spoke-fixture-toy-world`). **No** adapters — integrator value before product DTO maps.
5. **Next — Adapters (deferred)** — Implementable `adapters/nexus` + `adapters/creader` mapping product objects ↔ SPOKE; call `spoke-operations` for shared gates; optional product conformance round-trips atop fixtures.
6. **North star** — Cross-product Keyblock dialect for consistency-check and context-assembly I/O **without** a shared runtime.

---

## Non-goals (durable)

These stay out of SPOKE itself unless a future grill explicitly reverses them:

- Shared daemon / MCP server / single runtime for all products
- Putting I/O, LLM, ranking, or product detectors inside `spoke-operations` (that would be a runtime in disguise)
- Default shipping of full manuscript text on the wire
- Closed forever enums that freeze Nexus/Creader type growth
- Treating Creator Memory or unpromoted chat as Keyblock graph canon

---

## Related paths

| Path | Role |
|------|------|
| [`.mstar/specs/spoke-protocol.md`](specs/spoke-protocol.md) | Umbrella — data + ops framing |
| [`.mstar/specs/spoke-protocol-layers.md`](specs/spoke-protocol-layers.md) | Nine layers L0–L8, capability levels, TimelineScale |
| [`.mstar/specs/spoke-data-model.md`](specs/spoke-data-model.md) | Data surface detail |
| [`.mstar/specs/spoke-ops.md`](specs/spoke-ops.md) | Ops **wire** surface detail |
| [`schemas/`](../schemas/) | Wire SSOT |
| [`packages/spoke-operations/`](../packages/spoke-operations/) | Hand-written behavior library (delivered operations library first slice) |
| Local `delivery-compass.md` | Per-slice scope (process; gitignored) |
