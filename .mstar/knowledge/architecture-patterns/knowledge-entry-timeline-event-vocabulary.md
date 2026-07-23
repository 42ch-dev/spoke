# Protocol vocabulary: KnowledgeEntry and TimelineEvent

> Category: architecture-patterns  
> Updated: 2026-07-24

## Facts

| Concern | Wire name | Identity field |
|---------|-----------|----------------|
| L0–L1 atomic Knowledge Base entry | `KnowledgeEntry` | `entry_id` |
| L5 Timeline when-axis | `TimelineEvent` | `timeline_event_id` |

Product expand string: **Standardized Programmable Ontology Knowledge Engine** (acronym **SPOKE**). Package ids stay `@42ch/spoke-*` / `spoke-schemas`.

Ontology type on a KnowledgeEntry is the open string field **`entry_type`**. Related filters:

| Location | Field |
|----------|-------|
| KnowledgeEntry / AssembleEntry | `entry_type` |
| Scope (`check` / `assemble`) | `entry_types` |
| Rule | `target_entry_types` |

Core documented `entry_type` values live in `.mstar/specs/spoke-data-model.md` (open string — not a closed schema `enum`).

## Dual-concern rule

- `entry_type: "event"` on a **KnowledgeEntry** is a KB ontology / fact label.
- **TimelineEvent** is a Timeline when-axis object (optional `timeline_scale`).
- `entry_type: "rule"` on a **KnowledgeEntry** is an ontology label — distinct from the L6 **`Rule`** wire object.
- One story beat MAY map to KnowledgeEntry and/or TimelineEvent; protocol keeps the names separate.
- Toy-world pair: `kb_tw_harbor_dawn_event` + `evt_tw_harbor_dawn` (`extensions.spoke.timeline_entry_id`).

## Pointers

- Specs: `.mstar/specs/spoke-data-model.md`, `spoke-protocol-layers.md`
- Schemas: `schemas/data/knowledge-entry.schema.json`, `timeline-event.schema.json`
- CONCEPTS.md spelling table
