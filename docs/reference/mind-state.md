---
title: MindState
---

# MindState

First-class temporal mental-state record (L5, optional `l5-mind`) — a standalone wire object on the when-axis that records how a holder's mental fields change over time. Required: `schema_version`, `mind_state_id`, `holder_entry_id`, `extensions`.

## When to use

Use `MindState` when the temporal course of a mental state is itself the interchange fact: false-belief structures, dramatic-irony structures, or any "who believes / wants / feels what at time t" record. `MindState` is strictly **derivative** — the holder KnowledgeEntry's `modules.mental` / `modules.belief` (settled home) stay the single authority, and this record is never a second source of truth.

## Minimal example

```json
{
  "schema_version": 1,
  "mind_state_id": "ms_01HXYZ",
  "holder_entry_id": "kb_mira",
  "canonical_name": "Mira — relieved after the Treaty of Ashford",
  "occurred_at": "1421-06-03T12:00:00Z",
  "snapshot": {
    "emotions": [{ "emotion": "relief", "intensity": 0.7 }],
    "goals": [{ "goal": "secure the harbor charter", "status": "active" }]
  },
  "deltas": [
    {
      "path": "modules.mental.emotions",
      "previous": [{ "emotion": "anxious", "intensity": 0.8 }],
      "next": [{ "emotion": "relief", "intensity": 0.7 }]
    }
  ],
  "extensions": {}
}
```

## Capability: `l5-mind`

`l5-mind` is an optional capability flag on the L5 Temporal layer — declaring it means a product implements the `MindState` temporal record and `modules.observation` on `TimelineEvent.modules`. It is not `spoke-baseline`; mental-state engines (belief revision, ToM inference, observation rendering) are product-owned. The settled home for mental fields is `modules.mental` / `modules.belief` on the holder KnowledgeEntry (`narrative-modules` bag); `MindState` records only snapshot / delta changes over the when-axis.

## Field tables and handbook

Field-level detail lives in the spec corpus (SSOT) — not duplicated here:

- [Data-model field tables — §MindState](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-data-model.md) — required / optional fields, shared `MentalFieldMap` / `MindDelta` definitions, dual-concern table.
- [Mental-state Domain Profile](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/domain-profile-mental-state.md) — `modules.mental` nine-field vocabulary, `modules.belief` label space, `modules.observation`, MindState record sketch.
- [MindState schema](https://github.com/42ch-dev/spoke/blob/main/schemas/data/mind-state.schema.json) — the committed wire schema.

## Related

- [Data model reference](/reference/data-model) — the durable objects, including TimelineEvent (when-axis) and the MindState / ontology-label distinction.
- [Protocol reference](/reference/protocol) — capability flags, including `l5-mind`.
- [Concepts](/explanation/concepts) — layers and dual-concern pairs.
