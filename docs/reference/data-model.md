---
title: Data model reference
---

# Data model reference

The data layer defines the durable wire objects narrative products exchange. Objects are transport-agnostic; the required `extensions.<namespace>` bag and the closed-core rule are specified once in the [extensions contract](/reference/protocol#extensions-contract). Field tables below trace to the committed schemas in [`schemas/data/`](https://github.com/42ch-dev/spoke/tree/main/schemas/data) and [`schemas/common/`](https://github.com/42ch-dev/spoke/tree/main/schemas/common).

## Shared definitions

| Definition | Shape | Notes |
|------------|-------|-------|
| `SchemaVersion` | integer ≥ 1 | Wire schema version |
| `Timestamp` | string, RFC 3339 UTC | Created / updated / occurred times |
| `ExtensionMap` | object; keys `^[a-z][a-z0-9_-]*$`, values opaque JSON objects | Product namespace bag; round-trip preserves unknown namespaces and keys |
| `ModuleMap` | object; keys `^[a-z][a-z0-9_-]*$`, values structured JSON (object or array) | Cross-product functional-dialect bag; round-trip preserves unknown module namespaces |
| `SourceSpan` | `{ start, end }` (inclusive start, exclusive end) | Span within a source artifact |
| `TimelineScale` | open string; core list `brief`, `narrative`, `moment` | L5 projection tier |
| `ForkId` | string ≥ 1 char | Opaque world-history branch identity (`l5-fork`) |
| `Scope` | object; required `scope_id`; optional `viewpoint` (`ke-ownership`), `cognition` (`spoke-cognition`) | Shared ops selector — see [Ops wire reference](/reference/ops), [Ownership governance](#ownership-governance-ke-ownership) and [Cognition](#cognition-spoke-cognition) |
| `BodyAttribute` | `{ trait_type, value, display_type?, max_value? }` | ERC721-style trait item; duplicate `trait_type` allowed at array level |
| `ComputableFieldMap` | open map of field names to domain values | Shared by `body.state` and `body.computable` under `l2-computable` |
| `ComputableLogEntry` | `{ logged_at, entry_id, changes[] }` + optional `session_id` / `message` | Moment-scale presentation of computable field changes (`l2-computable`) |
| `MentalFieldMap` | open map of mental-field names to domain values | `MindState.snapshot`; matches the `modules.mental` nine-field vocabulary |
| `MindDelta` | `{ path, previous?, next? }` | `MindState.deltas[]`; `path` is an RFC 6901 JSON Pointer rooted at `/modules/mental/<field>`, `/modules/cognition/records/...` or `/modules/cognition/world_facts/...` |

## KnowledgeEntry

The atomic knowledge-base unit. Required: `schema_version`, `entry_id`, `entry_type`, `canonical_name`, `status`, `body`, `extensions`.

| Field | Type | Notes |
|-------|------|-------|
| `entry_id` | string | Stable id, opaque to the protocol |
| `entry_type` | open string | Core list (documented, not enforced): `character`, `location`, `event` (ontology label — distinct from the TimelineEvent wire object), `scene`, `act`, `organization`, `item`, `conflict`, `info_point`, `era`, `worldbuilding`, `note`, `research`, `ability`, `rule` (ontology label — distinct from the L6 Rule wire object). Products MAY emit values outside this list |
| `canonical_name` | string ≥ 1 char | Human-stable name |
| `status` | open string | Core list (documented, not enforced): `provisional`, `confirmed`, `deprecated`, `merged`, `deleted` |
| `body` | closed object | `summary?`, `tags[]?`, `attributes[]?` (BodyAttribute); `state?` / `computable?` (ComputableFieldMap) under `l2-computable` |
| `source_anchor` | SourceAnchor, optional | Provenance pointer |
| `revision` | integer ≥ 0 | Optimistic concurrency revision |
| `created_at` / `updated_at` | Timestamp | |
| `extensions` | ExtensionMap, required | |
| `modules` | ModuleMap, optional | Capability-flagged `narrative-modules`; carries per-entry dialects (e.g. `modules.activation`); `modules.cognition` carries the closed cognition module under `spoke-cognition` |
| `owner` | string ≥ 1 char, optional | Holder-KnowledgeEntry `entry_id` in the product's collaboration context (`ke-ownership`) |
| `disclosure` | open string, optional | Governance disclosure vocabulary; the sole core value is `owner-private` (`ke-ownership`) |

## Relation

Directed edge between two KnowledgeEntries (or a KnowledgeEntry and a source anchor). Required: `schema_version`, `relation_id`, `relation_type`, `from_id`, `to_id`, `extensions`.

| Field | Type | Notes |
|-------|------|-------|
| `relation_id` | string | Stable relation id |
| `relation_type` | open string | Core list (documented, not enforced): `related_to`, `parent_of`, `member_of`, `located_in`, `participates_in`, `causes`, `foreshadows` |
| `from_id` / `to_id` | string | Source / target endpoint ids |
| `label` | string, optional | Human label |
| `metadata` | open object, optional | |
| `revision` | integer ≥ 0 | Optimistic concurrency revision |
| `extensions` | ExtensionMap, required | |

## SourceAnchor

Pointer to a source artifact span (manuscript, scene, external locator). Required: `schema_version`, `source_id`, `extensions`.

| Field | Type | Notes |
|-------|------|-------|
| `source_id` | string | Opaque source locator; products define the grammar |
| `span` | SourceSpan, optional | Byte or character span within the source |
| `label` | string, optional | Human label |
| `mime_type` | string, optional | MIME type of the referenced source |
| `extensions` | ExtensionMap, required | |

## Finding

Checker output — a distinct artifact from a KnowledgeEntry `body`. Required: `schema_version`, `finding_id`, `severity`, `status`, `title`, `description`, `extensions`.

| Field | Type | Notes |
|-------|------|-------|
| `finding_id` | string | Stable finding id |
| `severity` | open string | Core list (documented, not enforced): `info`, `warning`, `error` |
| `status` | open string | Core list (documented, not enforced): `open`, `resolved`, `dismissed` |
| `title` / `description` | string | Short title and detail text |
| `kind` | string, optional | Checker kind or category |
| `target_entry_id` | string, optional | KnowledgeEntry the finding targets |
| `source_anchor` | SourceAnchor, optional | Provenance pointer |
| `suggested_fix` | string, optional | Suggested remediation text |
| `text_position` | object, optional | Position hint within source text |
| `extensions` | ExtensionMap, required | |

## AssemblePacket

Wire-only context-assembly payload. Required: `schema_version`, `packet_id`, `entries`, `extensions`.

| Field | Type | Notes |
|-------|------|-------|
| `packet_id` | string | Stable packet id |
| `entries` | array | Slim context entries (default); full KnowledgeEntry embedding is op-specific |
| `extensions` | ExtensionMap, required | |
| `modules` | ModuleMap, optional | Capability-flagged `narrative-modules`; carries packet-level recipes (`modules.placement`, `modules.activation_trace`) |
| `cognition` | CognitionProjection, optional | Derivative projection selected by `Scope.cognition` (`spoke-cognition`) |

## HostCapabilityManifest

Host self-description for in-process collaboration. Required: `schema_version`, `host_id`, `roles`, `capabilities`, `namespaces`, `extensions`.

| Field | Type | Notes |
|-------|------|-------|
| `host_id` | string ≥ 1 char | Stable host identity, opaque to the protocol |
| `roles` | string[], min 1, unique | Open vocabulary. Core list (documented, not enforced): `data-store`, `input-source`, `checker`, `assembler`, `computable-engine` |
| `capabilities` | string[], min 1, unique | Open string capability flags. Core list (documented, not enforced): `spoke-baseline`, `l2-computable`, `l5-fork`, `spoke-cognition`, `narrative-modules`, `ke-extraction`, `ke-ownership`, `spoke-connect`, `spoke-memory`, `spoke-memory-reflection`, `spoke-memory-transfer`. A manifest that exposes tools also lists each `tools.<ns>.<tool_id>` capability id |
| `namespaces` | string[], min 1, unique; keys `^[a-z][a-z0-9_-]*$` | Extension namespace keys this host owns in a collaboration context |
| `authority` | `{ scope_key }`, optional | Explicit single-writer authority scope; when absent with `data-store` in roles, implicit authority is this manifest's `host_id` |
| `extensions` | ExtensionMap, required | Deployment metadata — distinct surface from KnowledgeEntry `extensions` |

## ToolDescriptor

Closed descriptor of one remote-invokable tool, declared in `HostCapabilityManifest.tools[]`. Required: `schema_version`, `capability_id`, `op`, `description`, `input`, `output`. The descriptor has no `extensions` bag.

| Field | Type | Notes |
|-------|------|-------|
| `capability_id` | string | Tool capability string `tools.<ns>.<tool_id>` matching `^tools\.[a-z][a-z0-9_-]*\.[a-z0-9][a-z0-9_-]*$`; the namespace is owned by the declaring manifest (`namespaces[]` membership) |
| `op` | string | Wire op for the tool; MUST equal `capability_id` |
| `description` | string ≥ 1 char | Human-readable tool description |
| `input` | object | Opaque JSON Schema draft-07 subschema for tool arguments; `{}` declares an unconstrained ABI |
| `output` | object | Opaque JSON Schema draft-07 subschema for the success result; `{}` declares an unconstrained ABI |
| `idempotent` | boolean, optional | Advisory metadata (default `false`) |

Invoke and dispatch rules: [Connect reference — Tools](/reference/connect#tools-reverse-invokes).

## Rule

Declarative constraint input to `check` — never checker output. Required: `schema_version`, `rule_id`, `canonical_name`, `kind`, `extensions`.

| Field | Type | Notes |
|-------|------|-------|
| `rule_id` | string | Stable rule id, opaque to the protocol |
| `canonical_name` | string ≥ 1 char | Human-stable name |
| `kind` | open string | Core list (documented, not enforced): `rule`, `prohibition`, `style` |
| `statement` | string, optional | Declarative constraint text (human- or machine-readable; products choose the grammar) |
| `target_entry_types` | string[], optional | Ontology filter matching KnowledgeEntry `entry_type` vocabulary |
| `severity_hint` | open string, optional | Core list (documented, not enforced): `info`, `warning`, `error` |
| `status` | open string, optional | Core list (documented, not enforced): `draft`, `active`, `deprecated` |
| `source_anchor` | SourceAnchor, optional | When the rule is anchored to a manuscript |
| `extensions` | ExtensionMap, required | |

## TimelineEvent

First-class when-axis temporal object (L5). Required: `schema_version`, `timeline_event_id`, `canonical_name`, `extensions`.

| Field | Type | Notes |
|-------|------|-------|
| `timeline_event_id` | string | Stable id, opaque to the protocol |
| `canonical_name` | string | Human-stable label |
| `timeline_scale` | TimelineScale, optional | Projection tier: `brief`, `narrative`, `moment` |
| `occurred_at` | string | RFC 3339 or opaque fuzzy label (e.g. "Third Age") |
| `description` | string, optional | Longer narrative summary |
| `participant_entry_ids` | string[], optional | Related KnowledgeEntry ids |
| `source_anchor` | SourceAnchor, optional | |
| `sort_key` | string, optional | Opaque ordering hint within a timeline |
| `fork_id` / `parent_fork_id` | ForkId, optional | World-history branch metadata (`l5-fork`) |
| `computable_logs` | ComputableLogEntry[], optional | Moment-scale computable change history (`l2-computable`) |
| `modules` | ModuleMap, optional | Capability-flagged `narrative-modules`; `modules.observation` carries could-perceive observers and open access constraints (`spoke-cognition`) |
| `extensions` | ExtensionMap, required | |

`MindState` is the companion L5 temporal record for mental state on the same when-axis — see [MindState reference](/reference/mind-state).

## Cognition (`spoke-cognition`)

Standard cognition is one optional family — the flag `spoke-cognition`, with no subflags. A host serves it through the closed `modules.cognition` module on the holder KnowledgeEntry: `records[]` holds completed holder-side records, `world_facts[]` holds narrated world facts, and the module carries its own `extensions`. Four concepts ride three record categories. Belief is a `BeliefRecord`. Epistemic assessment is one `EpistemicAssessment` whose `state` distinguishes Knowledge (`known`) from Ignorance (`ignorant`) and from assessor indeterminacy (`undetermined`). Perception is a `PerceptionRecord` describing the content a holder actually captured. Perception and Belief are separate records and may coexist for the same holder and target.

### Record shapes

Every completed Belief, epistemic and Perception record carries a `q` whose `assertion` names the cognitive assertion it estimates. `record_id` is unique within its carrier.

| Record (`kind`) | Required core | Optional |
|-----------------|---------------|----------|
| Belief (`belief`) | `record_id`, `holder`, `target`, `assessor`, `assessor_role`, `basis`, `assessment_context`, `effective_at`, `assessed_at`, `view`, `order`, `q`, `extensions` | `criterion`, `metrics`, `truth`, `access`, `representation`, `content_type`, `source`, `context` |
| Epistemic (`epistemic`) | `record_id`, `holder`, `target`, `assessor`, `assessor_role`, `basis`, `assessment_context`, `effective_at`, `assessed_at`, `view`, `state` (`known` / `ignorant` / `undetermined`), `criterion`, `classification_policy` (`policy_id`, `policy_version`), `q`, `extensions` | `metrics` |
| Perception (`perception`) | `record_id`, `holder`, `target`, `assessor`, `assessor_role`, `basis`, `assessment_context`, `effective_at`, `assessed_at`, `view`, `episode`, `content` (required key), `q`, `extensions` | `access`, `criterion`, `metrics` |
| World fact (`world_facts[]`) | `fact_id`, `holder` (kind `world`), `target`, `order` = 0, `assessment_context`, `effective_at`, `recorded_at`, `extensions` | `truth`, `access`, `representation`, `content_type`, `source`, `context` |

World facts live on the designated world-state KnowledgeEntry; their `holder` is the world authority and never an actor. Actor Belief, epistemic and Perception records live under `modules.cognition.records[]` on the holder's KnowledgeEntry.

### Identity, attribution and `q`

| Element | Shape and rule |
|---------|----------------|
| Reference | `{ kind, id, namespace }`; `kind` is one of `knowledge_entry`, `timeline_event`, `memory_record`, `memory_subject`, `principal`, `world`, `external`. Identity is the complete `(kind, namespace, id)` tuple — never a bare id or equal text |
| Target | `target.carrier` (reference), `target_id` (stable within carrier), `target.kind` (`proposition`, `question`, `perceived_content`, `episode`), optional `text` (representational content, never identity), optional `about` (higher-order target) |
| Compound target identity | `(carrier, kind, target_id, assessment_context.context_id, assessment_context.fork_id)`. There is no target registry or lookup operation |
| `assessment_context` | `context_id` required, `fork_id` optional; the filter's fork predicate selects `{eq}` one fork or `{absent:true}` the no-fork context |
| `assessor` / `assessor_role` | A reference plus `holder_self_report`, `external_judgment` or `product_assessment`. Attribution states who assessed; it grants no authority over other assessments |
| `basis` | `{ kind: "evidence", evidence: [reference...] }` with at least one reference, or `{ kind: "prior", description }` with a non-empty declared subjective/prior/model basis. Either may carry `acquisition`. A declared basis may remain uncalibrated |
| `q` | `{ value (0–1), assertion, method, calibration, uncertainty? }`. `assertion` is `belief_attribution`, `actual_capture`, `knowledge_criterion`, `ignorance_criterion` or `assessor_indeterminacy`; `calibration` is `calibrated`, `uncalibrated` or `unknown`; `uncertainty` is `{ kind, lower, upper, level?, description? }` |
| Estimand identity | Holder, target, assessor, basis, criterion where applicable, assessment context, `effective_at` / `assessed_at`, and `q.assertion` together identify the estimated probability |
| Times | `effective_at` is when the assessed state applies; `assessed_at` is when the assessor made the assessment |
| `view` | `current` for holder-side current authority; `recorded_at` for Memory content and explicitly supplied historical input |

Belief `q` is the attribution confidence for that Belief record; a holder's own credence is the separate optional metric `b`. Perception `q` estimates actual capture; the same event's access metadata and optional correspondence metric `f` are separate. Epistemic `q` follows `state`: `knowledge_criterion` for `known`, `ignorance_criterion` for `ignorant`, `assessor_indeterminacy` for `undetermined`.

### Optional metrics

Metrics are distinct estimates. A key is present only when that estimate exists, and each metric repeats the attribution identity (`assessor`, `assessor_role`, `basis`, `context`, `effective_at`, `assessed_at`, `method`, `calibration`) plus its `estimand`.

| Key | `estimand` | Permitted category |
|-----|------------|--------------------|
| `b` | `holder_credence` | Belief |
| `p` | `capture` | Perception |
| `f` | `correspondence` | Perception |
| `k` | `knowledge_criterion` | Epistemic |
| `r` | `recording_fidelity` | Belief, epistemic, Perception |

`criterion` is required for `knowledge_criterion` and `correspondence` estimates. Product-local measurements belong in `extensions.<product>`.

### Belief vocabulary

`order` is `0`–`3` (higher-order depth; current actor Belief records use `1`–`3`, and `order` `0` narrated facts use world facts). The seven closed dimensions are `order`, `truth` (`True` / `False` / `Unknown`), `access` (`Private` / `Shared` / `Public`), `representation` (`Explicit` / `Implicit`), `content_type` (`Location`, `Contents/Physical State`, `Identity/Relation`, `Epistemic`, `Desire/Intention`, `Emotion`, `Trait/Value`, `Action/Event`), `source` (`Narration`, `Perception`, `Memory`, `Testimony`, `Inference`, `Imagination`, `Unknown`) and `context` (`Deceptive`, `Temporal`, `Counterfactual`, `Neutral`). `truth: "Unknown"` states the record's truth value and does not mean holder ignorance; `access` is a distribution label and never an authorization grant; `source: "Perception"` is an acquisition label and not proof that actual perception occurred.

### Observation and actual perception

`TimelineEvent.modules.observation` is a closed `ObservationModule`: `observers` is required and holds `CognitionReference` values of actors who could perceive the event, and `access` carries open perceptual-access constraints (`position`, `line_of_sight`, `hearing_range`, `modality`, plus unknown keys preserved on round-trip). Missing `modules.observation` means no observation metadata; `observers: []` states explicitly that no observer could perceive. A `PerceptionRecord` describes actual captured content — partial, misleading, illusory or disbelieved content included — and event access alone creates no Perception record and supplies no default probability.

### Selection and projection

`Scope.cognition` carries a `CognitionSelection`. The `current` form is `{ view: "current", filter, selection_policy }` and requires the negotiated `spoke-cognition` flag; it applies to holder-side current authority only. The `recorded_at` form is `{ view: "recorded_at", filter }` and is valid for explicitly supplied historical input; KnowledgeEntry carriers serve the `current` view, and Memory serves `recorded_at`. Requests carrying `Scope.cognition` or `cognition_filter` require the negotiated flag.

The filter has one closed grammar. Every supplied predicate must hold on the same record:

| Predicate | Matches |
|-----------|---------|
| `holder` | The complete holder reference |
| `target` | `carrier` (complete reference), `target_id`, `kind` |
| `assessment_context` | `context_id`; `fork_id` as `{ eq }` or `{ absent: true }` |
| `category` | `belief`, `epistemic` or `perception` |
| `state` | Epistemic `state` |
| `q` | Inclusive `min` / `max` over declared `q.value`; uncalibrated declared values participate identically |
| `order` | Belief `order` |
| `truth`, `access`, `representation`, `content_type`, `source`, `belief_context` | `{ eq }` for a present value or `{ absent: true }` for an explicitly absent optional field |

Predicates never join across records or targets, and reference predicates compare the complete tuple. The grammar has no OR, text similarity, ranking, default cutoff, optional-metric filter or implicit source/calibration/criterion constraint. A schema-unknown predicate, an unimplemented dimension or metric filter, or an unavailable requested view rejects the whole request as `unsupported_cognition_requirement` with `details.field` naming the requirement.

A `CognitionProjection` is `{ view, items[], extensions }`. Each item carries `holder`, `target`, `assessment_context`, `presence` (`assessed` or `no_record`) and `assessments[]` — every retained authorized matching record, with disagreements kept and never averaged or replaced. A `current` assessed item also carries `selection`: `{ policy: { policy_id, policy_version }, input_record_ids[], selected_record_id }`, where the selected id is one of the traced input ids and the product supplies the policy. A `recorded_at` projection has no current selection. `presence: "no_record"` is emitted only when the filter exactly addresses holder, complete target identity, `assessment_context.context_id` and an explicit fork predicate, and the authorized source holds no record for that compound key before optional predicates; an omitted fork is a wildcard and yields no `no_record` claim. An ordinary empty result is `items: []`.

### Memory cognition

Memory `Content.cognition` is an optional array of complete standard records; every entry carries `view: "recorded_at"` and keeps its own attribution, basis and times. An absent member, an explicitly present empty array and a non-empty array are distinct representations: absence records that no standard cognition was recorded, `[]` is preserved as its own wire value and is never produced by omission, and only a non-empty array carries cognition substance — an explicit empty array never becomes absent and never counts as non-empty content. A non-empty array counts as cognition content for classification, while every explicitly present member, `[]` included, is preserved and measured against the existing content byte budget. Cognition is validated on capture and on record creation or update. Review defaults to the captured proposed content where applicable; a `promote` decision may supply a new `content_override` and a `fragment` decision carries new fragment `content`, each independently validated by the same recorded-at-time rule before any effects. Existing grants, placement and the family boundary stay unchanged, review creates no KnowledgeEntry, and pending review confers no authority. Wire shapes and the review inputs are in the [Memory reference](/reference/memory#cognition-content-and-filters-spoke-cognition).

`cognition_filter` on `memory.recall` and `memory.records.list` is a hard filter applied after the authorized source set is resolved and before counts, ranking, pages and byte budgets: a record matches when at least one of its `Content.cognition` entries satisfies the complete conjunction. The filter binds cursor identity. `memory.reflections.state`, `memory.reflections.list` and `memory.reflections.generate` accept the same filter and pin it — together with the source id and revision set, profile/policy identity and cursor identity — into the reflection basis, so a changed filter or source set changes the basis and staleness. Export/import bundles and chunks include cognition in their canonical digests, and `(kind, namespace, id)` reference translation requires complete references and fails closed. Delete, scrub and tombstone clear cognition with the rest of the sensitive content; `memory.records.promote` changes placement and creates no KnowledgeEntry.

### Derivative MindState

`MindState.cognition` carries a derivative `CognitionProjection` of the selected holder-side authority, and `MindState.snapshot` / `deltas[]` present the mental-state trajectory. Every `MindDelta.path` is an RFC 6901 JSON Pointer rooted at `/modules/mental/<field>`, `/modules/cognition/records/...` or `/modules/cognition/world_facts/...`. The authoritative values remain on the holder KnowledgeEntry.

## Ownership governance (`ke-ownership`)

The optional `ke-ownership` capability carries ownership facts on the KnowledgeEntry envelope and a reader selector on the shared `Scope`. `KnowledgeEntry.owner` names the holder KnowledgeEntry (`entry_id`) in the product's collaboration context; when `owner` is absent, ownership is unspecified. `KnowledgeEntry.disclosure` carries the governance disclosure vocabulary, whose sole core value is `owner-private`; when `disclosure` is absent, the entry is shared within the already selected KB context. A holder is an ordinary KnowledgeEntry that the product resolves in its own context.

`Scope.viewpoint` is an optional non-empty string on the shared selector: the reader's holder-KnowledgeEntry `entry_id`. The core disclosure predicate admits an `owner-private` entry only when its `owner` equals the requesting `viewpoint` exactly — string equality on the ids as supplied; an entry without `disclosure` stays visible regardless of viewpoint.

The three Scope-bearing remote ops — `port.scope.list_knowledge_entries`, `port.scope.list_timeline_events`, `port.fork.list_timeline_events` — require `ke-ownership` in addition to their row capability when the request's Scope carries a non-empty `viewpoint`. See the [connect reference](/reference/connect#the-ownership-gate-ke-ownership).

## Open vocabulary

`entry_type`, `relation_type`, statuses, severities, and `kind` values are **open strings with documented core lists** — the schema keeps them open, and the core lists serve as reference values. Products emit their own values; Domain Profiles document published vocabulary (for example the profile-only `entry_type: "beat"`); adapters round-trip unknown values verbatim.

## Distinct artifacts

- Rule is declarative checker **input**; Finding is checker **output** — each keeps its own role.
- `TimelineEvent` is the L5 when-axis object; `entry_type: "event"` is an ontology label — one local concept may map to both (dual-concern).
- `MindState` is the L5 temporal mental-state record (`spoke-cognition`); `entry_type: "character"` / profile `mind` are ontology labels — the record is strictly derivative of the holder's `modules.cognition` / `modules.mental`, and the authoritative values stay on the holder KnowledgeEntry.
- `CognitionRecord` is the standard record shape (`spoke-cognition`); `modules.mental` carries the open nine-field mental vocabulary and its `beliefs` member is derivative summary, count or reference data.
- `modules.observation` states who could perceive an event; a `PerceptionRecord` states what a holder actually captured.
- `HostCapabilityManifest` carries host metadata (roles, capabilities, namespaces) on its dedicated surface, separate from KnowledgeEntry `extensions`.

## Related

- [Protocol reference](/reference/protocol) — schema inventory, extensions contract, capability flags.
- [Ops wire reference](/reference/ops) — `Scope` and the ops that read these objects.
- [Concepts](/explanation/concepts) — the layers each object belongs to.
- [Domain profiles](/explanation/domain-profiles) — open-string vocabulary published over these shapes.
- [MindState reference](/reference/mind-state) — the derivative L5 temporal record (`spoke-cognition`).
- [Cognition (`spoke-cognition`)](#cognition-spoke-cognition) — record shapes, `q` and metrics, filter grammar, projections and the Memory carrier.
