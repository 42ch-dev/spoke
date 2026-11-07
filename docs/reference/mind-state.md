---
title: MindState
---

# MindState

First-class temporal mental-state record (L5, optional `spoke-cognition`) — a standalone wire object on the when-axis that records how a holder's mental state changes over time. Required: `schema_version`, `mind_state_id`, `holder_entry_id`, `extensions`.

## When to use

Use `MindState` when the temporal course of a mental state is itself the interchange fact: false-belief structures, dramatic-irony structures, or any "who knows, believes, perceives, wants, or feels what at time t" record. `MindState` is strictly **derivative** — the holder KnowledgeEntry's `modules.cognition` (standard cognition records) and `modules.mental` (the nine-field mental vocabulary: `identity`, `beliefs`, `attention`, `goals`, `intentions`, `emotions`, `dispositions`, `norms`, `constraints`) carry the authoritative values, and this record presents them over time. `modules.mental.beliefs` holds derivative summary, count or reference data.

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
      "path": "/modules/mental/emotions",
      "previous": [{ "emotion": "anxious", "intensity": 0.8 }],
      "next": [{ "emotion": "relief", "intensity": 0.7 }]
    }
  ],
  "extensions": {}
}
```

## Field table

| Field | Type | Notes |
|-------|------|-------|
| `schema_version` | SchemaVersion | Required |
| `mind_state_id` | string ≥ 1 char | Stable id, opaque to the protocol |
| `holder_entry_id` | string ≥ 1 char | The mental actor's KnowledgeEntry `entry_id`; the derivative source |
| `canonical_name` | string ≥ 1 char, optional | Human-stable label |
| `occurred_at` | Timestamp, optional | When the mental state held |
| `sort_key` | string, optional | Opaque ordering hint within a mental-state timeline |
| `snapshot` | MentalFieldMap, optional | Derivative full-field snapshot of the holder's mental state at this point |
| `deltas` | MindDelta[], optional | Derivative field-level change records since the prior mental state |
| `cognition` | CognitionProjection, optional | Derivative projection of the selected holder-side cognition authority, with all retained matching assessments and the current-view trace |
| `source_anchor` | SourceAnchor, optional | Manuscript or scene anchor |
| `created_at` / `updated_at` | Timestamp, optional | |
| `extensions` | ExtensionMap | Required |

Every `MindDelta.path` is an RFC 6901 JSON Pointer rooted at `/modules/mental/<field>` (any of the nine fields), `/modules/cognition/records/...` or `/modules/cognition/world_facts/...`. Deltas are presentation only.

## Capability: `spoke-cognition`

`spoke-cognition` is one optional capability flag, with no subflags. On this surface a host implements the `MindState` temporal record, `modules.cognition` on the holder KnowledgeEntry, `modules.observation` on `TimelineEvent.modules`, and `Scope.cognition` selection with attribution-preserving projection. A host that also serves Memory implements recorded-at-time `Content.cognition` validation and the `cognition_filter` reads. Advertisement covers only the surfaces a host actually serves, and the manifest string alone is not conformance evidence; negotiation reuses the signed-hello intersection, and an unsupported requirement rejects the whole request with `details.reason: "unsupported_cognition_requirement"`. Mental-state engines (belief revision, ToM inference, observation rendering) are product-owned. See [Protocol reference — Cognition negotiation and rejection](/reference/protocol#cognition-negotiation-and-rejection).

## Derivative cognition projection

`MindState.cognition` carries a `CognitionProjection` of the selected holder-side records. A `current` projection carries, per assessed item, the identified selection policy and the traced input/selected record ids; a `recorded_at` projection carries the historical records without a current selection. Every item keeps its own `assessment_context`, all retained matching assessments remain in `assessments`, and disagreements are preserved rather than averaged. An `assessed` item appears only for records the filter matched. A `no_record` item is not a matching record: it appears only when the filter exactly addresses holder, complete target identity, `assessment_context.context_id` and an explicit fork predicate, and the authorized source holds no record for that compound key in the selection's `view` before optional predicates — an omitted fork is a wildcard, and an ordinary empty result is `items: []` ([data model §Selection and projection](/reference/data-model#selection-and-projection)). Event `modules.observation` says who could perceive; a `PerceptionRecord` says what a holder actually captured.

## Field tables and handbook

Field-level detail lives on the reference and spec surfaces:

- [Data model reference — Cognition](/reference/data-model#cognition-spoke-cognition) — record shapes, `q` and optional metrics, filter grammar and projection rules.
- [Data-model field tables — §MindState](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/spoke-data-model.md) — required / optional fields and shared `MentalFieldMap` / `MindDelta` definitions.
- [Mental-state Domain Profile](https://github.com/42ch-dev/spoke/blob/main/.mstar/specs/domain-profile-mental-state.md) — `modules.mental` nine-field vocabulary, `modules.observation`, MindState record sketch.
- [MindState schema](https://github.com/42ch-dev/spoke/blob/main/schemas/data/mind-state.schema.json) — the committed wire schema.

## Related

- [Data model reference](/reference/data-model) — the durable objects, including TimelineEvent (when-axis), CognitionRecord and the MindState / ontology-label distinction.
- [Protocol reference](/reference/protocol) — capability flags, negotiation and rejection.
- [Concepts](/explanation/concepts) — layers and dual-concern pairs.
