# `spoke-cognition` data contract

> **Status:** Normative data contract
> **Wire SSOT:** `schemas/`, especially `schemas/data/cognition.schema.json`

Authority and rationale: [`spoke-cognition-family-adr.md`](spoke-cognition-family-adr.md). Served-surface behavior: [`spoke-cognition-consumer-contract.md`](spoke-cognition-consumer-contract.md).

## 1. Common wire notation

The TypeScript-like notation below documents the landed JSON shapes; JSON Schema remains the wire authority. `?` means the key may be absent. Absent is not serialized as `null`. Objects in the closed cognition family use `additionalProperties: false`, except `ObservationAccess`, `MentalFieldMap` and existing extension/opaque-content surfaces. Arrays preserve input order unless an existing consumer owns a deterministic order.

```ts
type Timestamp = string;            // RFC 3339 date-time with offset
type OpaqueJson = unknown;
type ExtensionMap = Record<string, Record<string, unknown>>;

type CognitionReferenceKind =
  | "knowledge_entry" | "timeline_event" | "memory_record"
  | "memory_subject" | "principal" | "world" | "external";

type AssessorRole = "holder_self_report" | "external_judgment" | "product_assessment";
type AcquisitionPath =
  | "Narration" | "Perception" | "Memory" | "Testimony"
  | "Inference" | "Imagination" | "Unknown";
type CalibrationStatus = "calibrated" | "uncalibrated" | "unknown";
type CognitiveView = "current" | "recorded_at";
```

Cognition timestamps may denote fictional time and are primarily ordered within an identified context. Validation is deterministic and offline: calendar/offset syntax is checked, and second `60` is admitted only when offset adjustment places it at a structurally possible UTC month-end. No historical IERS lookup or cross-context total order is implied.

### 1.1 References and targets

```ts
interface CognitionReference {
  kind: CognitionReferenceKind;
  id: string;                 // non-empty, opaque within kind/namespace
  namespace: string;          // ^[a-z][a-z0-9_-]*$
}

interface CognitionContext {
  context_id: string;         // non-empty; product-selected
  fork_id?: string;           // opaque L5 fork identity
}

type CognitionTargetKind = "proposition" | "question" | "perceived_content" | "episode";

interface CognitionTarget {
  carrier: CognitionReference;
  target_id: string;          // non-empty and stable within carrier
  kind: CognitionTargetKind;
  text?: string;              // content, never identity
  about?: CognitionTarget;    // higher-order target, bounded by Belief order <= 3
}
```

Reference identity is the complete `(kind, namespace, id)` tuple. Target identity is `(carrier, kind, target_id, assessment_context.context_id, assessment_context.fork_id)`. Pure validation never resolves a registry or performs I/O. `Memory.subject_id` is not implicitly a holder; use `memory_subject` explicitly when intended. `world` is invalid as the holder of Belief, epistemic or Perception records. Evidence references identify support but neither authenticate it nor grant access.

## 2. Attribution, probability and metrics

### 2.1 Basis and criterion

```ts
type AssessmentBasis =
  | { kind: "evidence"; evidence: CognitionReference[]; acquisition?: AcquisitionPath }
  | { kind: "prior"; description: string; acquisition?: AcquisitionPath };

interface CognitionCriterion {
  criterion_id: string;
  expression?: string;        // readable rule, not executable protocol logic
}

interface ProbabilityUncertainty {
  kind: "subjective_credible" | "statistical_confidence" | "other";
  lower: number;              // 0 <= lower <= upper <= 1
  upper: number;
  level?: number;             // 0 < level < 1 when declared by kind
  description?: string;       // required for kind = other
}
```

Every completed assessment has non-empty `evidence` or a non-empty explicit subjective/prior/model `prior.description`. Raw/unassessed material has no `AssessmentBasis` and is not a completed record. Uncertainty is optional; absence never fabricates a zero-width interval. Calibration declares estimator/method status and supplies no default probability.

### 2.2 Mandatory `q`

```ts
type CognitiveAssertion =
  | "belief_attribution" | "actual_capture" | "knowledge_criterion"
  | "ignorance_criterion" | "assessor_indeterminacy";

interface DeclaredProbability {
  value: number;              // finite, 0..1
  assertion: CognitiveAssertion;
  method: string;             // non-empty
  calibration: CalibrationStatus;
  uncertainty?: ProbabilityUncertainty;
}
```

`q` is mandatory for every Belief, epistemic and Perception record. Its assertion, enclosing holder/target/assessor/basis/context/criterion/times and view form the estimand identity. `q` never means world truth. Values 0 and 1 are deliberate estimates, not categorical defaults.

### 2.3 Optional metrics

```ts
type MetricEstimand =
  | "holder_credence" | "capture" | "correspondence"
  | "knowledge_criterion" | "recording_fidelity";

interface CognitionMetric {
  value: number;              // finite, 0..1
  estimand: MetricEstimand;
  assessor: CognitionReference;
  assessor_role: AssessorRole;
  basis: AssessmentBasis;
  context: CognitionContext;
  effective_at: Timestamp;
  assessed_at: Timestamp;
  method: string;
  criterion?: CognitionCriterion; // required for knowledge_criterion/correspondence
  calibration: CalibrationStatus;
  uncertainty?: ProbabilityUncertainty;
}

interface CognitionMetrics {
  b?: CognitionMetric;         // holder_credence
  p?: CognitionMetric;         // capture
  f?: CognitionMetric;         // correspondence
  k?: CognitionMetric;         // knowledge_criterion
  r?: CognitionMetric;         // recording_fidelity
}
```

A key exists only when its distinct estimate exists. Belief permits `b`/`r`; Perception permits `p`/`f`/`r`; epistemic permits `k`/`r`. Duplicate same-estimand and category-incompatible metrics reject. `q` may reuse `p`, `k` or `1-k` only for the exact same estimand; the duplicate metric is then absent. No arithmetic, complement, independence, cutoff or calibration is inferred across metrics.

## 3. Completed standard records

All three variants carry these members:

```ts
interface AssessmentMembers {
  record_id: string;          // non-empty, unique within carrier
  holder: CognitionReference;
  target: CognitionTarget;
  assessor: CognitionReference;
  assessor_role: AssessorRole;
  basis: AssessmentBasis;
  assessment_context: CognitionContext;
  effective_at: Timestamp;
  assessed_at: Timestamp;
  view: CognitiveView;
  criterion?: CognitionCriterion;
  q: DeclaredProbability;
  metrics?: CognitionMetrics;
  extensions: ExtensionMap;
}
```

`view = "current"` is valid only in holder-side current input/authority. `view = "recorded_at"` is required in Memory and valid for explicitly supplied historical input. The carrier guards reject a repeated `record_id` within `modules.cognition.records` and within one `Content.cognition` array; ids are never compared across carriers.

### 3.1 Belief

```ts
type BeliefOrder = 0 | 1 | 2 | 3;
type Truth = "True" | "False" | "Unknown";
type BeliefAccess = "Private" | "Shared" | "Public";
type Representation = "Explicit" | "Implicit";
type BeliefContentType =
  | "Location" | "Contents/Physical State" | "Identity/Relation" | "Epistemic"
  | "Desire/Intention" | "Emotion" | "Trait/Value" | "Action/Event";
type BeliefSource = AcquisitionPath;
type BeliefContext = "Deceptive" | "Temporal" | "Counterfactual" | "Neutral";

interface BeliefRecord extends AssessmentMembers {
  kind: "belief";
  target: CognitionTarget;    // proposition or higher-order proposition
  order: BeliefOrder;
  truth?: Truth;
  access?: BeliefAccess;      // distribution label, not authorization
  representation?: Representation;
  content_type?: BeliefContentType;
  source?: BeliefSource;      // acquisition label, not actual-capture proof
  context?: BeliefContext;
  q: DeclaredProbability;     // assertion = belief_attribution
}
```

For order 1, `target.about` is absent. Orders 2 and 3 require a consistent `about` chain; the chain never exceeds 3. Actor Belief normally uses orders 1–3; order 0 is retained for the shared vocabulary and narrated facts use `WorldFactRecord`.

### 3.2 Epistemic assessment

```ts
type EpistemicState = "known" | "ignorant" | "undetermined";

interface EpistemicAssessment extends AssessmentMembers {
  kind: "epistemic";
  target: CognitionTarget;    // proposition or question
  state: EpistemicState;
  criterion: CognitionCriterion;
  classification_policy: { policy_id: string; policy_version: string };
  q: DeclaredProbability;
}
```

`known` uses `knowledge_criterion`, `ignorant` uses `ignorance_criterion`, and `undetermined` uses `assessor_indeterminacy`. No-record is a projection state, not a fourth enum. Undetermined supplies no knowledge bound and does not validate another metric.

### 3.3 Perception

```ts
interface PerceptionRecord extends AssessmentMembers {
  kind: "perception";
  target: CognitionTarget;    // perceived_content or episode
  episode: CognitionReference; // timeline_event, memory_record or external
  content: OpaqueJson;        // required actual experienced/captured content
  access?: ObservationAccess; // actual-capture constraints, not event observability
  q: DeclaredProbability;     // assertion = actual_capture
}
```

Target carrier/episode identity agrees with `episode`. Partial, misleading, illusory and disbelieved content remain valid; correspondence is separately assessed by optional `metrics.f`.

## 4. Authority, WorldFact, observation and temporal derivatives

### 4.1 KnowledgeEntry modules

```ts
interface CognitionModule {
  records: CognitionRecord[];
  world_facts: WorldFactRecord[];
  extensions: ExtensionMap;
}

type CognitionRecord = BeliefRecord | EpistemicAssessment | PerceptionRecord;

interface WorldFactRecord {
  fact_id: string;
  holder: CognitionReference; // kind = world
  target: CognitionTarget;    // proposition
  order: 0;
  truth?: Truth;
  access?: BeliefAccess;
  representation?: Representation;
  content_type?: BeliefContentType;
  source?: BeliefSource;
  context?: BeliefContext;
  assessment_context: CognitionContext;
  effective_at: Timestamp;
  recorded_at: Timestamp;
  extensions: ExtensionMap;
}
```

`KnowledgeEntry.modules.cognition` carries this closed module. Current actor records live on the holder's entry. World facts live on the designated world-state entry and have no `q`, `metrics`, assessor or assessment basis. `modules.mental` retains open optional `identity`, `beliefs`, `attention`, `goals`, `intentions`, `emotions`, `dispositions`, `norms` and `constraints`; only `beliefs` is restricted to derivative summary/count/reference data.

### 4.2 Observation access

```ts
interface ObservationModule {
  observers: CognitionReference[];
  access?: ObservationAccess;
}

interface ObservationAccess {
  position?: string | Record<string, unknown>;
  line_of_sight?: boolean | Record<string, unknown>;
  hearing_range?: boolean | Record<string, unknown>;
  modality?: string[];
  [extension_key: string]: unknown;
}
```

`TimelineEvent.modules.observation` records could-perceive metadata. Missing module means no metadata, `observers: []` means an explicit empty could-perceive set, and a Perception record means actual capture. None normalizes into another. Unknown access keys round-trip verbatim.

### 4.3 MindState and MindDelta

`MindState` retains its ids, holder, timing, open `snapshot`, `deltas`, `source_anchor` and extensions, plus optional `cognition?: CognitionProjection`. `canonical_name` and `occurred_at` remain optional. `snapshot` uses the nine-field vocabulary; `snapshot.beliefs` is derivative only.

`MindDelta.path` is an RFC 6901 JSON Pointer rooted at one of:

- `/modules/mental/<field>...`;
- `/modules/cognition/records/...`;
- `/modules/cognition/world_facts/...`.

Validation covers the pointer/root and previous/next shapes only. MindState never applies deltas, replays history or becomes authority.

## 5. Selection and projection

### 5.1 Filter grammar

```ts
type CategoricalPredicate<T> = { eq: T } | { absent: true };

interface TargetPredicate {
  carrier?: CognitionReference;
  target_id?: string;
  kind?: CognitionTargetKind;
}

interface ContextPredicate {
  context_id?: string;
  fork_id?: CategoricalPredicate<string>;
}

interface QRange {
  min?: number;               // inclusive, finite 0..1
  max?: number;               // inclusive, finite 0..1; min <= max
}                             // at least one bound required

interface SelectionPolicyIdentity {
  policy_id: string;
  policy_version: string;
}

interface CognitionFilter {
  holder?: CognitionReference;
  target?: TargetPredicate;
  assessment_context?: ContextPredicate;
  category?: "epistemic" | "perception" | "belief";
  state?: EpistemicState;
  q?: QRange;
  order?: BeliefOrder;
  truth?: CategoricalPredicate<Truth>;
  access?: CategoricalPredicate<BeliefAccess>;
  representation?: CategoricalPredicate<Representation>;
  content_type?: CategoricalPredicate<BeliefContentType>;
  source?: CategoricalPredicate<BeliefSource>;
  belief_context?: CategoricalPredicate<BeliefContext>;
}

type CognitionSelection =
  | { view: "current"; filter: CognitionFilter; selection_policy: SelectionPolicyIdentity }
  | { view: "recorded_at"; filter: CognitionFilter };
```

All predicates AND on one record with the same holder/target/context. Reference comparisons use the complete tuple. `q` bounds are inclusive over declared `q.value`, including uncalibrated values. The core grammar has no OR, text similarity, ranking, default cutoff, optional-metric filter or implicit source/calibration/criterion constraint.

`assessment_context.context_id` is exact. Omitted `fork_id` is a wildcard over forked and no-fork contexts; `{eq: fork}` selects one fork and `{absent: true}` selects exactly the no-fork identity. The `eq` and `absent` arms are mutually exclusive. For optional categorical fields, `{eq}` excludes a missing field and `{absent:true}` matches only explicit absence. Required fields cannot use `absent`.

### 5.2 Unsupported versus missing

Unknown predicates, known-but-unimplemented views/dimensions/metric filters and unavailable source views reject the whole request with `invalid_input`, `details.reason = "unsupported_cognition_requirement"` and the requirement's carrier-relative path in `details.field`. No filter is stripped and no partial/unfiltered success is returned. Invalid completed records instead use `details.reason = "invalid_cognition_record"`.

Absent optional fields, absent observation metadata and no matching records are ordinary missing data, not unsupported features. `no_record` follows only §5.3.

### 5.3 Attribution-preserving projection

```ts
interface CognitionProjectionItem {
  holder: CognitionReference;
  target: CognitionTarget;
  assessment_context: CognitionContext;
  presence: "assessed" | "no_record";
  assessments: CognitionRecord[];
  selection?: {
    policy: SelectionPolicyIdentity;
    input_record_ids: string[];
    selected_record_id: string;
  };
}

interface CognitionProjection {
  view: CognitiveView;
  items: CognitionProjectionItem[];
  extensions: ExtensionMap;
}
```

Every item carries its own source `assessment_context`; there is no projection-global effective or assessment context. Multi-context matches remain separate, and an empty projection infers no context. Records group by `(holder, target.carrier, target.kind, target.target_id, assessment_context.context_id, assessment_context.fork_id)`. Groups retain first-match order and assessments retain input order.

For current assessed items, `selection` is required and identifies the explicitly injected product policy; `selected_record_id` must be one of `input_record_ids`. All attributed disagreements remain in `assessments`. Recorded-at-time projection has no current selection. The protocol supplies no policy algorithm. One item stays self-consistent: every assessment carries the item's holder, target identity and assessment context, `input_record_ids` name exactly that item's assessments, and `selected_record_id` names one of them. `record_id` stays carrier-local, so two separate carriers may each contribute the same `record_id` to one item; ids are never compared across items or carriers.

A filter exactly addresses a source key only when it supplies the complete holder and target identities, `assessment_context.context_id`, and an explicit fork predicate. Before optional `category`, `state`, `q`, `order` or Belief-dimension predicates, the authorized source is checked for the selected `view` and complete key. If absent, exactly one `no_record` item carries the addressed holder/target/context, `assessments: []` and no `selection`. If any source record exists, `no_record` is never emitted even when optional predicates exclude every record; the result is then an empty projection for that key. Wildcard or partially addressed empty results are always `items: []`. Emitted `no_record` items follow assessed items.

## 6. Memory content and lifecycle

### 6.1 Recorded-at-time carrier and Content states

`Content` has optional `cognition?: CognitionRecord[]`, with every entry `view = "recorded_at"`. Three wire states are distinct: absent, explicitly present `[]`, and non-empty. In the in-process TypeScript API a key present with value `undefined` is the absent state, as for every other optional member. Explicit `null` is invalid. Present `[]` is preserved and included in byte accounting, but is not cognition substance for classification and cannot keep a contentless candidate alive. Placement alone never converts raw material into completed cognition.

Updates preserve original holder, assessor, criterion/policy, basis/evidence, context, `effective_at` and `assessed_at`. Reassessment appends a separately identified and attributed recorded-at-time assessment; it neither rewrites retained attribution nor mutates/synchronizes holder-side current authority.

### 6.2 Lifecycle

| Surface | Cognition behavior |
|---|---|
| `memory.capture` | Validate before admission; existing grant/scope/idempotency apply; no retained record or KnowledgeEntry is created |
| Pending candidate | `proposed.content` uses Content; non-empty cognition counts as content; pending review confers no authority |
| Candidate review | Validate every `content_override` and fragment before state change; existing grants and immutable capture scope remain; a valid override wins |
| Consumed candidate | Scrubbed shell has no `proposed.content`; existing digest still commits to consumed content |
| Record create/update | Validate every entry; existing records_write, placement, CAS, idempotency and revision rules apply |
| Promote/retention | Placement changes only; no KnowledgeEntry output or mutation |
| Delete/scrub/tombstone | Tombstone content is exactly `{text: ""}` with no structured or cognition content; later exports contain none |
| Export/chunks | Cognition participates in canonical JCS bundle/chunk bytes and SHA-256 digests |
| Import/stage | Validate translated Content and all cognition references before activation |
| List/get | Return authorized selected recorded-at-time content under existing exclusions |

Cognition grants no additional permission, placement, CAS, idempotency or disclosure privilege. Assessment roles are not Memory roles.

### 6.3 Transfer reference translation

Existing `TranslationMaps.subject_id_map`, `record_id_map` and `candidate_id_map` retain their purposes. Transfer also accepts explicit `(from, to)` CognitionReference pairs containing complete `(kind, namespace, id)` identities. Translation recursively covers records, targets/`about`, assessors, basis evidence, metric attribution/basis, episode references and projection traces where present.

Exact pairs translate every identity component, including namespace. `memory_subject` and `memory_record` id maps apply only within an unchanged namespace; missing or ambiguous assignments fail. Other changed references require exact pairs. Unpaired `external` references may remain unchanged with their complete identity preserved. Record, target, criterion and policy ids remain stable within the translated carrier. Bare-id matching and silent preservation of an unresolved source id as a target id are forbidden.

### 6.4 Byte accounting

Admission `max_content_bytes` uses complete canonical Content bytes, including cognition, with top-level explicit null members omitted by the canonical measurement form. Recall `budget.max_bytes` uses emitted engine-wire Content bytes, also including cognition. Neither path substitutes text-only length or drops cognition. Over-budget admission uses the existing `INVALID_INPUT` and `details.limit = "max_content_bytes"` branch.

## 7. Authorized source sets, cursors and reflection

`MemoryRecordsListRequest`, `MemoryRecallRequest`, `MemoryReflectionsStateRequest`, `MemoryReflectionsGenerateRequest` and `MemoryReflectionsListRequest` have optional `cognition_filter?: CognitionFilter`. `Reflection` optionally pins that filter in its basis. Existing selection and provenance filters compose by AND.

For fixed principal, space, subject, selection, provenance filter, cognition filter, snapshot and engine policy:

1. Exclude unauthorized, placement/provenance-ineligible, candidate and tombstoned records under existing rules.
2. Apply the cognition filter to each record's entries. One entry must satisfy the entire conjunction; entries never jointly satisfy different conjuncts.
3. The resulting single authorized filtered source set feeds counts, list pages, recall and reflection. Only then apply provider ranking/tie-break, cursor offset, item limits and byte budgets.
4. `total_authorized` counts this eligible set; returned pages/recall may be bounded subsets.
5. Cursor identity binds canonical cognition filter with existing principal/space/subject/selection/filter/snapshot/policy identity. Selection-affecting changes invalidate reuse with `CURSOR_INVALID`; unchanged pinned pages are stable and non-overlapping.
6. `ReflectionBasis` includes `cognition_filter`, `engine_policy_version`, provenance `filter`, `profile_revisions`, `selection` and sorted source `{record_id, revision}` entries. `source_set_digest` remains id+revision; `state_digest` changes when filter or source set changes. Content revision already changes the basis; no content-byte hash is added.
7. If the first ranked eligible record exceeds `max_bytes`, return that whole record, set `overflow_record_id` and mark truncation under existing rules; never drop it or substitute a lower-ranked record.

## 8. Pure validation and helper boundary

Pure validators enforce required/variant fields, closed/open keys, enum/range/timestamp rules, namespaces, target/order consistency, basis, assertion/metric compatibility, view/placement, filter grammar, projection trace and Memory recorded-at-time shape. Selection helpers evaluate supplied records only. They never resolve references, judge evidence sufficiency, calculate probabilities, merge disagreements, replay deltas, negotiate capabilities, rank/retrieve, access storage or perform network calls.

The family includes only the validators, predicates, projection and carrier-integration helpers required by these consumers. It adds no delta application, cognition ordering, merge, scoring, retrieval or Finding-checker helper family.
