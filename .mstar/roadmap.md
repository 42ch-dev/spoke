# SPOKE Roadmap

> **Status:** Living product roadmap (durable result)  
> **Authority:** Direction for future iterations — not a substitute for per-iteration `delivery-compass.md`  
> **Research source:** Spoke Protocol Research canvas (nine protocol layers + dual standardization mandate)

This file records the **two durable thrusts** SPOKE must deliver. Iteration plans may slice work; they must not drop either thrust. Thrust A now spans **three delivery columns** (data wire, ops wire, ops behavior library).

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
| `@42ch/spoke-schema` / crate `spoke-schema` | **Generated** from `schemas/` | Types only — wire truth |
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
| Ops library | **Out of v0.1** — schemas + generated types only | `spoke-operations` deferred to a dedicated follow-on slice (see Sequencing) |

Later iterations deepen **all three** surfaces (e.g. Rule schema, richer check I/O, assemble profiles as *documented* product-local policy, and the hand-written operations library — still not a shared runtime).

---

## Thrust B — Nine protocol layers (realize on the wire)

Research framed SPOKE as **nine conceptual layers** (L0–L8). That model is interesting and **should become protocol reality** — not remain canvas-only vocabulary.

Read top-down: identity → ontology → body → provenance → graph → time → constraints → findings → AI packet.

Products may omit optional capabilities (e.g. L5 Fork, L2 computable state) and still claim SPOKE compliance at a **declared capability level** — once that capability model is specified.

| Layer | Concept | Protocol direction |
|-------|---------|--------------------|
| **L0 Envelope** | Identity + `schema_version` + scope | Required Keyblock envelope fields; immutable id + versioned content |
| **L1 Ontology** | `block_type` + Domain Profile | Open-string types + published core vocabulary; profiles as adapters (not forked cores) |
| **L2 Body** | summary / attributes / tags / optional state | Structured body; computable/WASM optional capability |
| **L3 Provenance** | SourceAnchor | Refs over full manuscript; excerpt/summary optional |
| **L4 Graph** | Relation (+ OCC as product concern) | Typed edges between Keyblocks (and anchors when needed) |
| **L5 Temporal** | Event + optional Fork | when-axis objects; Fork optional capability |
| **L6 Constraint** | Rule / Prohibition | First-class constraint objects (deferred in v0.1; must return) |
| **L7 Finding** | Check output lifecycle + evidence | Findings are not Keyblock bodies; status lifecycle on the wire |
| **L8 Context** | AssemblePacket | Shared packet shape for check / inference I/O; trim policy product-local |

**How Thrust B relates to Thrust A**

- Layers L0–L8 are the **conceptual map** of what the **data** surface must eventually express (and what **ops wire** must carry in and out).
- Thrust A is the **delivery cut**: every layer that is normative must appear as schemas + ops I/O, and cross-product **lifecycle rules** belong in `spoke-operations` when they cannot be expressed as schema alone.
- v0.1 **partially** covers L0–L4 (via Keyblock/Relation/SourceAnchor), L7, L8; L5–L6 and full L1 (Domain Profile) remain roadmap work; the behavior library is a follow-on slice.

---

## Sequencing (indicative)

Order is guidance for future compasses — adjust when grill locks say otherwise.

1. **v0.1 (delivered 2026-07-23)** — Bootstrap: data + ops **wire** schema SSOT, codegen `spoke-schema` packages, empty adapters, CI. Nine-layer model referenced here; not yet a normative L0–L8 spec section. **No** `spoke-operations` yet.
2. **Next — `spoke-operations` (hand-written)** — Stand up `@42ch/spoke-operations` on top of generated types: lifecycle helpers and protocol invariants only (see Package cut). Spec note / thin ADR in `{SPECS_DIR}` describing what is library vs wire vs adapter. Prefer this **before or tightly coupled with** first adapter code so Nexus/Creader do not fork promote/Finding rules.
3. **Adapters** — Implementable `adapters/nexus` + `adapters/creader` mapping product objects ↔ SPOKE; call `spoke-operations` for shared gates; optional conformance fixtures.
4. **Deepen surfaces** — Complete deferred data (`Rule`, temporal objects as needed); harden ops wire (`check`/`assemble` contracts); grow `spoke-operations` only for cross-product invariants; document capability levels for optional layers.
5. **Normative nine-layer chapter** — Promote L0–L8 into `{SPECS_DIR}` so compliance claims map layer ↔ schema ↔ op ↔ library helper (where applicable).
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
| [`.mstar/specs/spoke-protocol.md`](specs/spoke-protocol.md) | Umbrella — data + ops framing (v0.1) |
| [`.mstar/specs/spoke-data-model.md`](specs/spoke-data-model.md) | Data surface detail |
| [`.mstar/specs/spoke-ops.md`](specs/spoke-ops.md) | Ops **wire** surface detail |
| [`schemas/`](../schemas/) | Wire SSOT |
| Future `packages/spoke-operations/` | Hand-written behavior library (roadmap item; not in v0.1) |
| Iteration `delivery-compass.md` | Per-iteration scope (process; local) |
