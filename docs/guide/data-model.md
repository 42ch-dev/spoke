---
title: Data model
---

# Data model

The data layer defines the durable wire objects narrative products exchange. All objects are transport-agnostic, carry required `extensions.<namespace>`, and keep core fields closed.

## Objects

- **KnowledgeEntry** — identity, open `entry_type` / `status`, closed `body` (summary, tags, `attributes[]` scalar traits), optional `source_anchor`, required `extensions`.
- **Relation** — directed edge (`from_id` / `to_id`) with open `relation_type`; optional `revision` for optimistic concurrency.
- **SourceAnchor** — source artifact span pointer.
- **Finding** — checker output with `status` vocabulary (`open`, `resolved`, `dismissed`).
- **AssemblePacket** — wire-only context-assembly payload (slim `entries[]`).
- **HostCapabilityManifest** — host roles, capability flags, owned namespaces for in-process collaboration.
- **Rule** — L6 declarative constraint input to `check` (kind, statement, target entry types).
- **TimelineEvent** — L5 when-axis object with optional `timeline_scale` and (under `l5-fork`) fork fields.

## Open vocabulary

`entry_type`, `relation_type`, and statuses are **open strings** with documented core lists — the schema does not close them with `enum`. Products emit their own values; Domain Profiles document published vocabulary (for example the profile-only `entry_type: "beat"`); adapters round-trip unknown values verbatim.

## Extensions contract

Every durable object carries `extensions: { "<namespace>": { } }`. Namespace keys are product-chosen ids; values are opaque JSON objects. Adapters preserve unknown namespaces and unknown keys inside a namespace on every read/write.

## Distinct artifacts

- Rule is declarative checker input; Finding is checker output — each artifact keeps its own role.
- TimelineEvent is the L5 when-axis object; `entry_type: "event"` is an ontology label — one local concept may map to both.
- Host metadata lives on `HostCapabilityManifest` (roles, capabilities, namespaces), not inside KnowledgeEntry `extensions`.

## Normative references

- [spoke-data-model.md](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-data-model.md) — field tables for all eight objects, extensions, open vocabulary
- [CONCEPTS.md](https://github.com/42ch-dev/spoke/blob/main/CONCEPTS.md) — vocabulary and dual-concern rules
- [schemas/README.md](https://github.com/42ch-dev/spoke/blob/main/schemas/README.md) — schema file inventory
- [schemas/data/](https://github.com/42ch-dev/spoke/tree/main/schemas/data) — committed data schemas
