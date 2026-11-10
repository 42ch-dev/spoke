# ADR: Place shared functional dialects in a capability-flagged `modules` bag

Status: implemented — cross-product functional dialects live in an optional open `ModuleMap`; product data stays in `extensions.<product>`

Date: 2026-07-30

## Problem

SPOKE durable objects and ops envelopes had exactly two homes: closed protocol core fields and product-owned `extensions.<product>`. Narrative hosts then needed the same functional companions independently — lore activation keys, packet placement and activation provenance, standard cognition records, open mental-state fields, event observation metadata. Published under an `extensions.<ns>`, a shared dialect cannot be shared: `HostCapabilityManifest.namespaces[]` grants each product namespace to at most one `host_id`, and every reader expects an `extensions` key to be a product or adapter id. Promoting the same shapes into core fields would make every baseline host read and write product vocabulary on closed objects, and would freeze inner field tables into schema before any two consumers had converged. A third bag with an explicit admission rule was needed.

## Decision

Every new field lands in one of three bags, and the bag is chosen by audience rather than convenience:

- **Core fields** stay protocol-owned keys on closed objects (`additionalProperties: false`). Product-specific or lossy-round-trip data never becomes a core field.
- **`modules.*`** carries cross-product functional dialects. It ships as an optional, open `ModuleMap` on `KnowledgeEntry`, `AssemblePacket`, and `TimelineEvent`, capability-flagged `narrative-modules`; `spoke-cognition` supplies the closed cognition and observation semantics inside it. Absent and empty `modules` are valid, baseline hosts need not emit or parse it, and unknown module keys round-trip.
- **`extensions.<product>`** carries one product's or adapter's data: opaque namespace ids matching `^[a-z][a-z0-9_-]*$` with opaque JSON object values, round-tripped verbatim together with any unknown keys inside the namespace.

The demand gate that blocked the envelope is resolved by shipping it optional and open rather than closed: inner dialect field tables stay handbook-defined, and freezing a specific inner table into schema is a separate, later decision that belongs to the consumers who need a closed dialect. Publishing a shared functional dialect under `extensions.*` is a category error, not a style choice.

## Alternatives considered

**A shared dialect under an `extensions.<functional-ns>` namespace.** Rejected on three grounds that hold independently: namespace exclusivity makes a shared name unownable next to a real product id; `extensions.<ns>` already reads as a product or adapter id, so a functional name misleads every consumer; and the correct-home rule sends functional dialects to `modules.*` and product bags to `extensions.<product>`. Product interim storage of activation-like data under a true product namespace stays product-local, but the shipped bag is the home where hosts converge.

**Promoting the dialects to core fields.** Rejected because core objects are closed and read/written by every baseline host: product vocabulary and lossy-round-trip values would become protocol obligations on hosts that never needed them, and the versioning cost would land on the whole wire instead of one opt-in bag.

**Hard-coding the inner field tables into `schemas/` at the same time.** Rejected because no closed inner layout had converged; freezing one would have destroyed unknown-key round-trip on a bag whose contents were still moving. The envelope ships open, the handbooks own the field tables, and schema `description` text lists names only.

**Leaving the envelope off the wire until demand is proven.** Rejected because hosts were already storing the same dialects inside their own product namespaces, which is exactly the duplication the triad forbids; an optional flag plus an open bag admits them without creating a baseline requirement.

**Placing Narrative Knowledge Pack catalog metadata on `KnowledgeEntry` or `AssemblePacket` `modules`.** Rejected because pack catalog is transport-envelope metadata for durable library interchange, while `modules` carries functional dialects on protocol objects; keeping them apart is what makes Pack ≠ AssemblePacket.

## Consequences

- Baseline is untouched: no schema requirement, absent `modules` valid, and a baseline host that ignores the bag still interoperates.
- The open bag buys lock-step-free evolution — unknown module keys and nested fields survive read/write — and costs schema-level validation of inner shapes: the handbooks carry the field tables, and adapters that skip `mergeModuleMaps` / `preserveModuleMaps` silently lose data.
- Convergence is a handbook duty, not an enforced one. Two hosts can both declare `narrative-modules` and still disagree on an inner field name; the shared dialect tables and the conformance samples are the only place that is caught early.
- `spoke-cognition` is the deliberate exception: its cognition and observation dialects are closed, and conformance is enforced through the consumer contract's conformance conditions rather than through the module-key enum.
- Any change to a bag's status — closing an inner dialect, changing the capability flag, moving a dialect between bags — has to update the placement authority page and the layer capability table in the same change, because the flag and the bag are the only admission points.
- Products that staged functional data under their own namespace keep it working but carry one migration to the shared bag, after which their private namespace is free for actual product data.
