# Architecture ADR: `spoke-cognition` family

> **Status:** Normative ADR
> **Document class:** Normative — cognition authority, semantics and ownership boundary
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)
> **Wire SSOT:** `schemas/`

Companion contracts: [`spoke-cognition-data-contract.md`](spoke-cognition-data-contract.md) defines the wire data, predicates, projections and Memory lifecycle; [`spoke-cognition-consumer-contract.md`](spoke-cognition-consumer-contract.md) defines served-surface conformance and public library boundaries.

## Decision

SPOKE represents four cognitive concepts as attributable wire data: Belief, epistemic assessment (`known`, `ignorant`, `undetermined`), actual Perception, and narrated WorldFact. The first three are completed standard `CognitionRecord` variants with a mandatory foundational `q`; WorldFact is a separate record and does not acquire `q` or cognition metrics.

The family uses existing KnowledgeEntry, TimelineEvent, MindState, AssemblePacket and Memory carriers. It adds no cognitive layer, inference or revision engine, scoring or calibration model, target registry, reference-resolution I/O, retrieval engine, replay engine, global store, operation, port, dispatch family, typed callback, C ABI slot or native-binding surface.

## Authority and ownership

| Concern | Authority or carrier | Rule |
|---|---|---|
| Current Belief, epistemic assessment and actual Perception | Holder KnowledgeEntry `modules.cognition.records` | Authoritative only for that holder, target and assessment context |
| Narrated-world facts | Designated world-state KnowledgeEntry `modules.cognition.world_facts` | `holder.kind = "world"`, `order = 0`; not an actor belief |
| Other mental facets | Holder KnowledgeEntry `modules.mental` | Open nine-field vocabulary; `beliefs` is derivative summary/count/reference data only |
| Could-perceive metadata | `TimelineEvent.modules.observation` | Access is not evidence of actual capture |
| Temporal presentation | `MindState` / `MindDelta` | Strictly derivative; never authority, a revision log or replay engine |
| Recorded cognition | Memory `Content.cognition` | Independent recorded-at-time assessments; never synchronized into holder-side current authority |

A holder may be an individual or group. Multiple assessments of the same target remain separate attributed records while retained by their carrier. Assessment roles describe attribution only and grant no write, disclosure, placement or world-truth privilege.

## Semantic decisions

### Complete assessments and distinct estimands

Every completed standard assessment identifies holder, target, assessor, assessor role, basis, assessment context, effective/assessment times and a declared `q`. Its basis is either non-empty evidence or an explicit declared subjective/prior/model basis. Raw or unassessed source material is not a completed cognition record. Declared but uncalibrated estimates remain valid without fabricated evidence, defaults, intervals or calibration.

For assessor `A`, basis `E`, context `ξ` and assessment time `τ`, `q = Pr_A(G | E, ξ, τ)` for the assertion identified by the record:

| Record | Assertion `G` | Metric boundary |
|---|---|---|
| Belief | The holder holds the stated stance toward the target | `q` is attribution confidence; optional holder credence `b` is distinct |
| Perception | The holder actually perceived the identified content in the episode | `q` may reuse capture `p` only for the identical event and complete conditioning |
| `known` | The holder satisfies the identified binary criterion | `q = k` only for the identical criterion and estimand |
| `ignorant` | The holder does not satisfy that same criterion | `q = 1-k` only for that same binary estimand |
| `undetermined` | The assessor cannot determine the holder's state under the criterion and policy | `q` is assessor indeterminacy, not `k`, `1-k`, a knowledge bound or endorsement of another metric |

Optional `p`, `f`, `k` and `r` respectively represent capture, correspondence, knowledge-criterion and recording-fidelity estimates. Task success, extraction confidence and product predictions are separate product estimands. No cross-estimand arithmetic, complement, averaging, independence, threshold or calibration is inferred.

### Identity, contexts and absence

Every reference has a complete `(kind, namespace, id)` identity. Equal bare ids or text across identity realms do not establish equality. Target identity includes carrier, target kind, target id and assessment context; higher-order Belief retains its target chain and order 0–3. Validation checks shape and internal consistency only, without resolution or I/O.

`known`, `ignorant`, `undetermined`, no-record, missing metadata and explicit field absence are distinct. `truth = "Unknown"` is not holder ignorance. Missing is not zero, non-perception or indeterminacy. A projection reports `no_record` only for an exact holder/target/context key whose authorized selected view has no source record before optional predicates; a filter mismatch never converts an existing record into no-record.

A current view is an explicitly requested derivative projection. Its product-selected policy identity and per-item input/selected record trace are recorded. The protocol supplies no latest-wins, averaging, consensus or source-privilege algorithm. Projection never rewrites authority.

### Actual Perception and observation access

Actual Perception records experienced or captured content, including partial, misleading, illusory or disbelieved content. `TimelineEvent.modules.observation` separately records could-perceive observers and open access metadata. Missing observation metadata, `observers: []` and an actual Perception record are three different facts. Belief `source = "Perception"` is an acquisition label, not proof of an actual Perception record. Optional correspondence `f` is assessed independently from capture.

### Retained vocabulary and derivatives

The nine optional mental fields are `identity`, `beliefs`, `attention`, `goals`, `intentions`, `emotions`, `dispositions`, `norms` and `constraints`. Standard Belief and WorldFact retain seven dimensions: `order`, `truth`, `access`, `representation`, `content_type`, `source` and `context`. Their exact vocabularies are defined by the data contract. Attitude/role `Relation` edges, scene/event atmosphere, body scalar mirrors and nested module values retain their existing owners.

`MindState.snapshot` remains an open derivative map. Optional `MindState.cognition` carries a selected projection. `MindDelta.path` is an RFC 6901 JSON Pointer rooted at `/modules/mental`, `/modules/cognition/records` or `/modules/cognition/world_facts`. No delta application, replay, ordering or history reconstruction is implied.

### Time

Cognition timestamps may describe fictional time. Their order within an identified context is their primary use; the protocol defines no cross-context total order or time engine. Validators apply deterministic offline RFC 3339 structural checks. A leap second is accepted only at a structurally possible UTC month-end after offset adjustment; no historical IERS announcement lookup is required.

### Memory

Memory cognition is recorded-at-time and independently authorized. Updates preserve original holder, assessor, criterion/policy, basis, context and times; reassessment creates a separately attributed assessment. Review, retention, transfer, reflection and promotion never create or mutate a KnowledgeEntry. Existing grants, placement, CAS, idempotency, retention, scrub/tombstone and byte-accounting rules remain authoritative. Transfer uses complete namespace-bearing references and fails closed rather than matching bare ids.

## Module boundaries

| Module | Owns | Must not own |
|---|---|---|
| `schemas/` and generated schema packages | Closed standard shapes and open-facet boundaries | Runtime policy or hand-edited generated types |
| `spoke-operations` cognition validators | Pure shape and conditional validation | I/O, inference, negotiation, authorization, merging or replay |
| `spoke-operations` selection/assembly | Deterministic predicates and traceable derivative projection with injected current policy | Storage, ranking or a universal selection algorithm |
| Memory provider/reference engine | Authorized source sets, existing ranking/budgets, transfer/digest and reflection basis | New grants, CAS/storage semantics or automatic KnowledgeEntry writes |
| Connect/RemoteAdapter | Existing manifest intersection and payload-dependent requirement guards | New operations, ports, ABI or silent feature stripping |
| Product checker/policy | Finding output and named current-view policy | Protocol authority or privilege expansion |

**Ablation:** closed standard records preserve mandatory `q`, estimands and exact vocabularies. **Ablation:** typed namespace-bearing references prevent cross-system identity conflation. **Ablation:** policy identity and trace prevent implicit record selection. **Ablation:** separate Observation and Perception shapes prevent access from being treated as capture. **Ablation:** Memory filter identity in cursors/reflection basis prevents reuse over a different source set.

## Conformance

`spoke-cognition` is one conditional capability covering the mandatory behavior on the surfaces a host actually serves. KnowledgeEntry-only hosts need no Memory ports. A combined host must satisfy both applicable KnowledgeEntry and Memory behavior before advertising the family. Unsupported requested features reject explicitly; absent optional data follows missing-data and projection rules rather than producing an unsupported-feature error.
