# Timeline projection tiers (Brief / Narrative / Moment)

**Category:** architecture-patterns  
**Source:** compound 2026-07-23 (rule-event)  
**Status:** durable

## Problem

Nexus ships Timeline as three semantic-zoom layers (Brief / Narrative / Moment). Research originally treated the carriers as product-local. Without a protocol vocabulary, integrators invent parallel labels for the same when-axis scales.

## Decision

Under SPOKE **L5 Temporal**, standardize the Timeline **dimension** as open-string vocabulary:

| Wire value | Role |
|------------|------|
| `brief` | Coarse shape / era at a glance |
| `narrative` | Ordered story events |
| `moment` | Fine grain (scene / beat) |

Field name on wire: **`timeline_scale`** (common `TimelineScale` def; optional on `Event` and `Scope`).

Product UI may still spell Brief/Narrative/Moment. Fork remains optional capability `l5-fork`. L8 `AssemblePacket` ≠ L5 `moment` tier.

## Related

- Spec: `.mstar/specs/spoke-protocol-layers.md` § L5
- Schema: `schemas/common/common.schema.json` (`TimelineScale`), `schemas/data/event.schema.json`
- Research: Spoke Protocol Research canvas L5 row
