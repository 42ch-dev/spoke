# Optional l2-computable Session model (wire)

## Problem

Products that need programmable KnowledgeEntry body state invent private shapes for static vs dynamic fields, Session correlation, and how Moment TimelineEvents present change history. Baseline consumers must remain able to omit the capability.

## Decision

| Concern | Wire / rule |
|---------|-------------|
| Capability flag | Single optional **`l2-computable`** (covers body + Moment logs + `project`/`compute` ops) |
| Static state | `KnowledgeEntry.body.state` → `ComputableFieldMap` |
| Dynamic projection | `KnowledgeEntry.body.computable` → same map |
| Session | **Lifecycle + op `session_id`**, not a durable Session schema object |
| Moment logs | `TimelineEvent.computable_logs[]` of `ComputableLogEntry` (not Finding-shaped) |
| Ops | Optional `project` (state → computable) and `compute` (apply / `settle: true` → merged state) |
| Engines | Product-owned; `@42ch/spoke-operations` only validates shapes |

## Lifecycle (normative)

1. Pre-Session: `state` authoritative; `computable` absent/inert  
2. Session start: `project` materializes `computable` from `state`  
3. Mid-Session: mutate `computable` only; do not silently rewrite `state`  
4. Session end: `compute` with `settle: true` merges into `state`  
5. History: Moment-scale TimelineEvent may carry `computable_logs` presentation only  

## Dual-concern

- KnowledgeEntry `entry_type: "event"` ≠ TimelineEvent ≠ Session  
- Moment `computable_logs` ≠ Finding  

## Related

- Specs: `spoke-data-model.md`, `spoke-ops.md`, `spoke-protocol-layers.md`  
- Schemas: `common.schema.json` defs; `knowledge-entry` / `timeline-event`; `project-*` / `compute-*` ops  
- Library: `validateComputableFieldMap`, `validateComputableLogEntry`, `validateProjectRequest`, `validateComputeRequest`
