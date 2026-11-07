# Domain Profile — Mental State

> **Status:** Domain Profile
> **Document class:** Handbook — mental-state vocabulary and product-checker outputs
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)
> **Capabilities:** `spoke-cognition` + `narrative-modules`
> **Bag placement authority:** [`spoke-extension-modules.md`](spoke-extension-modules.md)
> **Wire SSOT:** `schemas/`

## Purpose

Narrative hosts exchange what an actor or group believes, attends to, wants, intends, feels, is disposed toward, regards as normatively binding and is constrained by without sharing an inference engine. Standard cognition is attributable data; belief revision, theory-of-mind inference, observation rendering, branch evaluation and transition simulation remain product-local.

The normative family and data contracts are [`spoke-cognition-family-adr.md`](spoke-cognition-family-adr.md) and [`spoke-cognition-data-contract.md`](spoke-cognition-data-contract.md). This profile retains the open mental-field vocabulary and product-checker patterns.

## Placement

| Concern | Home | Role |
|---|---|---|
| Standard Belief, epistemic assessment and actual Perception | Holder KnowledgeEntry `modules.cognition.records` | Closed, attributable current records with mandatory `q` |
| Narrated-world facts | Designated world-state KnowledgeEntry `modules.cognition.world_facts` | Separate order-0 WorldFact records without `q` |
| Other mental facets | Holder KnowledgeEntry `modules.mental` | Open nine-field vocabulary |
| Could-perceive metadata | `TimelineEvent.modules.observation` | Observer/access metadata, not actual-capture proof |
| Temporal presentation | Standalone `MindState` | Derivative snapshot, projection and deltas; never authority |
| Product-private state | `extensions.<product>` | Product-owned data outside the shared dialect |

Shared functional dialects use `modules.*`; product-private data uses `extensions.*`. Unknown module/extension namespaces round-trip under the existing bag contract. `modules.mental.beliefs` may hold only a derivative summary, count or references; standard Belief authority is `modules.cognition.records`.

## `modules.mental` — nine open fields

All nine fields are optional. Individual and group holders reuse the same shapes; scalar `body.attributes[]` mirrors and nested module values remain valid under their existing owners.

| Field | Value shape | Semantics |
|---|---|---|
| `identity` | object or scalar | Self-concept, occupation or role-class; distinct from wire `entry_id` |
| `beliefs` | array, count, summary or reference list | Derivative presentation of standard Belief; never current authority |
| `attention` | scalar or object | Current focus of perception or thought; linked to, not replaced by, Perception |
| `goals` | scalar or object array | Desired ends; no epistemic or `q` coercion |
| `intentions` | scalar or object array | Planned action, distinct from goals and Belief |
| `emotions` | scalar, array or object | Affective state |
| `dispositions` | scalar, array or object | Preferences and traits |
| `norms` | string/object array | Rules the holder regards as binding; no authorization effect |
| `constraints` | string/object array | Obligations and prohibitions; no governance side effect |

## Standard Belief and WorldFact

Belief and WorldFact share seven categorical dimensions:

| Dimension | Values |
|---|---|
| `order` | `0`, `1`, `2`, `3`; higher-order Belief retains its `target.about` chain |
| `truth` | `True`, `False`, `Unknown` |
| `access` | `Private`, `Shared`, `Public`; distribution label, never authorization |
| `representation` | `Explicit`, `Implicit` |
| `content_type` | `Location`, `Contents/Physical State`, `Identity/Relation`, `Epistemic`, `Desire/Intention`, `Emotion`, `Trait/Value`, `Action/Event` |
| `source` | `Narration`, `Perception`, `Memory`, `Testimony`, `Inference`, `Imagination`, `Unknown` |
| `context` | `Deceptive`, `Temporal`, `Counterfactual`, `Neutral` |

A Belief is a completed attributed assessment with mandatory `q` for belief attribution. Optional holder credence `b` is a separate metric. `truth = "Unknown"` is not holder ignorance. `source = "Perception"` is an acquisition label, not evidence that an actual Perception record exists.

A WorldFact has `holder.kind = "world"` and `order = 0`, lives in `world_facts`, and has no `q`, assessor, basis or cognition metrics. A false-belief structure can pair a divergent actor Belief and a WorldFact for the same stable proposition without merging their authority.

## `modules.observation` and actual Perception

`ObservationModule.observers` is a required array of namespace-bearing CognitionReferences. Optional open `access` supports `position`, `line_of_sight`, `hearing_range`, `modality` and unknown extension keys. `TimelineEvent.participant_entry_ids` remains the separate participation list.

Three states are distinct:

1. Missing `modules.observation`: no observation metadata recorded.
2. `observers: []`: explicitly empty could-perceive set.
3. A Perception record: attributed actual experienced/captured content.

Could-perceive access alone never proves or disproves actual capture. Actual Perception may be partial, misleading, illusory or disbelieved; optional correspondence `f` is a distinct assessment. A product checker may conclude only that a belief was not updated by an event the holder could not perceive, not that the holder actually did not perceive it.

## MindState and MindDelta

`MindState` is a standalone L5 temporal derivative. It retains optional `canonical_name` and `occurred_at`, open `snapshot`, `deltas`, `source_anchor`, and optional `cognition` projection. Every projection item retains its source `assessment_context`; there is no projection-global effective context. Exact-key `no_record`, fork wildcard and optional-predicate behavior follow the data contract.

`MindDelta` is `{ path, previous?, next? }`. `path` is an RFC 6901 JSON Pointer rooted at `/modules/mental/<field>`, `/modules/cognition/records` or `/modules/cognition/world_facts`. The record supplies no delta application, replay, ordering or history engine.

## Product-checker Finding patterns

These are output contracts for product-local checkers, not engines supplied by `spoke-operations`.

| Finding `kind` | Inputs and product interpretation |
|---|---|
| `stale_belief_drift` | Compare current Belief and WorldFact for a stable proposition, event timeline and could-perceive access. Do not infer actual non-capture solely from access. |
| `dramatic_irony_asymmetry` | Combine WorldFact, divergent holder Belief, explicit observer sets and access-modality gaps. |
| `access_violation` | Compare Belief access/source labels, could-perceive constraints and any separately attributed actual Perception. |
| `action_content_mismatch` | Compare event physical-carrier metadata with associated Belief, intention and perceived content. |

Findings never mutate the authority records. Their `kind`, target, severity, description and optional suggested fix follow the existing Finding schema.

## Integrator checklist

1. Store current standard cognition on the holder's `modules.cognition.records`, with complete attribution and mandatory `q`.
2. Keep narrated WorldFact separate in `world_facts`, without treating it as actor cognition.
3. Preserve all nine open mental fields; keep `mental.beliefs` derivative.
4. Preserve observation missing/empty/actual-capture distinctions and unknown access keys.
5. Use MindState only for derivative temporal presentation and valid JSON Pointer paths.
6. Keep inference, revision, selection policy and Finding generation product-local.
7. Advertise `spoke-cognition` only for served surfaces that satisfy the consumer contract.

## See also

| Doc | Topic |
|---|---|
| [`spoke-cognition-family-adr.md`](spoke-cognition-family-adr.md) | Family authority and ownership |
| [`spoke-cognition-data-contract.md`](spoke-cognition-data-contract.md) | Exact records, metrics, filters, projections and Memory data |
| [`spoke-cognition-consumer-contract.md`](spoke-cognition-consumer-contract.md) | Served-surface and library conformance |
| [`spoke-extension-modules.md`](spoke-extension-modules.md) | Core/modules/extensions placement |
| [`spoke-data-model.md`](spoke-data-model.md) | KnowledgeEntry, TimelineEvent and MindState wire models |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | Layer and optional-capability model |
| [`CONCEPTS.md`](../../CONCEPTS.md) | Shared protocol vocabulary |
