# SPOKE Roadmap

> **Status:** Living product roadmap (durable result)  
> **Authority:** Direction for future iterations — not a substitute for per-iteration `delivery-compass.md`  
> **Research source:** Spoke Protocol Research canvas (nine protocol layers + dual standardization mandate)

This file records the **two durable thrusts** SPOKE must deliver. Iteration plans may slice work; they must not drop either thrust.

---

## Thrust A — Dual standardization (must both land)

SPOKE is **not** a runtime. It is a shared wire dialect with **exactly two protocol surfaces**. Both are first-class; shipping only one is incomplete.

| Surface | Mandate | What “done” means |
|---------|---------|-------------------|
| **1. Data structure standardization** | Durable narrative atoms and related objects are defined once in `schemas/` (+ normative specs) | Products can round-trip Keyblock-shaped knowledge without inventing parallel DTOs; extensions preserve product-specific bags |
| **2. Operations / behavior standardization** | Transport-agnostic ops (request/response families) cover the lifecycle products already share | Upsert, extract→promote, relate, check, assemble (and future ops) have stable I/O shapes; products bind transport locally |

**Invariant:** Data schemas without ops leave products guessing how to mutate and check; ops without a shared data model become RPC folklore. Roadmap progress is measured on **both** columns.

### Current slice (v0.1)

| Surface | v0.1 intent | Explicit gaps (by design) |
|---------|-------------|---------------------------|
| Data | Five required objects: Keyblock, Relation, SourceAnchor, Finding, AssemblePacket + `extensions` | `Rule` deferred; Event/Fork not required fields; no fixtures |
| Ops | Five ops × request/response (10 schemas); `assemble` **wire-only** | No compute/ranking in protocol; no HTTP/MCP binding; adapters empty |

Later iterations deepen **both** surfaces (e.g. Rule schema, richer check I/O, assemble profiles as *documented* product-local policy — still not a shared runtime).

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

- Layers L0–L8 are the **conceptual map** of what the **data** surface must eventually express (and what **ops** must carry in and out).
- Thrust A is the **delivery cut**: every layer that is normative must appear as schemas + ops I/O, not only as prose.
- v0.1 **partially** covers L0–L4 (via Keyblock/Relation/SourceAnchor), L7, L8; L5–L6 and full L1 (Domain Profile) remain roadmap work.

---

## Sequencing (indicative)

Order is guidance for future compasses — adjust when grill locks say otherwise.

1. **v0.1 (in flight)** — Bootstrap: data + ops schema SSOT, codegen packages, empty adapters, CI. Nine-layer model referenced here; not yet a normative L0–L8 spec section.
2. **Next** — Adapters (`nexus`, `creader`) mapping product objects ↔ SPOKE; optional conformance fixtures.
3. **Deepen dual surfaces** — Complete deferred data (`Rule`, temporal objects as needed); harden ops (`check`/`assemble` contracts); document capability levels for optional layers.
4. **Normative nine-layer chapter** — Promote L0–L8 into `{SPECS_DIR}` (umbrella or dedicated spec) so compliance claims map layer ↔ schema ↔ op.
5. **North star** — Cross-product Keyblock dialect for consistency-check and context-assembly I/O **without** a shared runtime.

---

## Non-goals (durable)

These stay out of SPOKE itself unless a future grill explicitly reverses them:

- Shared daemon / MCP server / single runtime for all products
- Default shipping of full manuscript text on the wire
- Closed forever enums that freeze Nexus/Creader type growth
- Treating Creator Memory or unpromoted chat as Keyblock graph canon

---

## Related paths

| Path | Role |
|------|------|
| [`.mstar/specs/spoke-protocol.md`](specs/spoke-protocol.md) | Umbrella — data + ops framing (v0.1) |
| [`.mstar/specs/spoke-data-model.md`](specs/spoke-data-model.md) | Data surface detail |
| [`.mstar/specs/spoke-ops.md`](specs/spoke-ops.md) | Ops surface detail |
| [`schemas/`](../schemas/) | Wire SSOT |
| Iteration `delivery-compass.md` | Per-iteration scope (process; local) |
