# Protocol vocabulary: KnowledgeEntry and TimelineEvent

> Category: architecture-patterns  
> Updated: 2026-07-24

## Problem

Early SPOKE drafts used **Keyblock** as the atomic wire unit. That name invited “KB = Key Blocks,” while product language (especially Creader) already treats **KB = Knowledge Base** and the atom as **KnowledgeEntry**. L5’s first-class when-axis object was named **Event**, which collided with ontology `entry_type: "event"` and with DOM/`Event` in TypeScript ecosystems.

## Decision

Pre-1.0 clean break (no dual-name aliases):

| Concern | Wire name | Identity field |
|---------|-----------|----------------|
| L0–L1 atomic KB entry | `KnowledgeEntry` | `knowledge_entry_id` |
| L5 Timeline when-axis | `TimelineEvent` | `timeline_event_id` |

Product expand string: **Standardized Programmable Ontology Knowledge Engine** — acronym **SPOKE** unchanged; npm/crate package ids `@42ch/spoke-*` / `spoke-schemas` unchanged.

## Dual-concern rule

- Ontology label `entry_type: "event"` on a **KnowledgeEntry** is a KB fact node — distinct from the L5 **TimelineEvent** wire object.
- **TimelineEvent** is a Timeline when-axis placement (optional `timeline_scale`).
- One story beat MAY map to either or both; protocol keeps the names separate.
- Toy-world exercises this with `kb_tw_harbor_dawn_event` + `evt_tw_harbor_dawn` (`extensions.spoke.timeline_knowledge_entry_id`).


## Ontology field: `entry_type`

After the KnowledgeEntry type rename, residual wire fields still said `block_type` / `block_types` / `target_block_types`. Pre-1.0 follow-up renamed them to **`entry_type` / `entry_types` / `target_entry_types`** (no dual aliases). Values are unchanged open strings; core documented vocabulary lives in `.mstar/specs/spoke-data-model.md`.

Also: ontology `entry_type: "rule"` is **not** the L6 `Rule` wire object.

## What not to do

- Do not reintroduce `Keyblock` / bare wire `Event` types or dual aliases.
- Do not put migration shims in `@42ch/spoke-operations` for the old names.
- Do not rename the SPOKE acronym or package scope for this vocabulary change.

## Pointers

- Specs: `.mstar/specs/spoke-data-model.md`, `spoke-protocol-layers.md`
- Schemas: `schemas/data/knowledge-entry.schema.json`, `timeline-event.schema.json`
- CONCEPTS.md spelling table
