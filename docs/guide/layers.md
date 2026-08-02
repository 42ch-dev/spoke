---
title: Layers & capabilities
---

# Layers & capabilities

The protocol organizes its wire dialect into **nine conceptual layers** (L0–L8), from envelope identity to context packet. Products claim compliance at a declared capability level: **baseline** (`spoke-baseline`) covers the required layers, and optional flags extend specific layers.

## Layers at a glance

- **L0 Envelope** — identity + `schema_version` on all durable objects
- **L1 Ontology** — open `entry_type` strings + Domain Profiles
- **L2 Body** — closed `body`: summary, tags, scalar `attributes` (optional `state` / `computable` under `l2-computable`)
- **L3 Provenance** — SourceAnchor pointers
- **L4 Graph** — typed directed Relations (with optional `revision` OCC)
- **L5 Temporal** — TimelineEvent when-axis + projection tiers `brief` / `narrative` / `moment` (optional Fork under `l5-fork`)
- **L6 Constraint** — Rule declarative input to `check`
- **L7 Finding** — checker output + status lifecycle
- **L8 Context** — AssemblePacket wire shape (assembly itself stays product-local)

## Capability levels

- **`spoke-baseline`** — L0–L8 semantics via the five ops wire families, `HostCapabilityManifest` + baseline `HostManifestPort`, and the shared `Scope` / `error-envelope` defs. Optional flags are additive — baseline compliance stands alone.
- **`l2-computable`** — optional `body.state` / `body.computable`, `TimelineEvent.computable_logs`, and `project` / `compute` ops.
- **`l5-fork`** — optional `fork_id` / `parent_fork_id` branch metadata on TimelineEvent and `Scope.fork_id` filtering.
- **`narrative-modules`** — optional `modules` (`ModuleMap`) on KnowledgeEntry + AssemblePacket for cross-product functional dialects.
- **`spoke-connect`** — the opt-in interaction envelope family; hosts that speak it list the flag in `HostCapabilityManifest.capabilities`.

## Hard boundaries

- Rule is declarative input; Finding is checker output — two distinct artifacts.
- `check` returns findings; `assemble` returns a packet — each op returns its own artifact.
- Timeline tiers (`brief` / `narrative` / `moment`) are protocol when-axis labels, distinct from L8 context assembly and from Fork branches.

## Domain Profiles

A Domain Profile publishes ontology vocabulary while core schemas stay closed: profile type tables live in adapter specs and handbooks (narrative-structure beat mapping, lore activation), and `entry_type` and `relation_type` remain open strings on the wire.

## Normative references

- [spoke-protocol-layers.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-protocol-layers.md) — full layer table, capability levels, layer ↔ artifact map
- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) — spoke-baseline, Domain Profile, capability flags
- [spoke-data-model.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-data-model.md) — field tables per layer
- [spoke-ops.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-ops.md) — ops wire per layer
- [spoke-connect.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-connect.md) — connect capability flag
