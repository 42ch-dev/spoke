# `spoke-cognition` consumer and conformance contract

> **Status:** Normative consumer contract

Data and authority rules: [`spoke-cognition-data-contract.md`](spoke-cognition-data-contract.md). Architecture decisions: [`spoke-cognition-family-adr.md`](spoke-cognition-family-adr.md).

## 1. Conformance on served surfaces

A host advertises `spoke-cognition` in `HostCapabilityManifest.capabilities` only when every mandatory rule applicable to its served surface is implemented. It is one family with no subflags; the manifest string alone is not conformance evidence.

| Served surface | Mandatory behavior | Not implied |
|---|---|---|
| KnowledgeEntry get/put | Validate and preserve closed `modules.cognition`; retain all nine open mental fields; one current holder authority | Memory behavior, timeline filters or historical reconstruction |
| Scope KnowledgeEntry list | Authorize/disclose first; validate CognitionSelection; conjunction on one holder/target/context; exact and absent categorical predicates; inclusive `q`; omitted-fork wildcard; explicit unsupported rejection | Ranking/retrieval or target resolution |
| Assemble | Build an attribution-preserving CognitionProjection from already-authorized entries before entry truncation; retain per-item source contexts, multi-context matches and disagreements; trace injected current policy | A protocol current-policy algorithm or second authority |
| TimelineEvent list/get | Preserve ObservationModule, unknown access keys and missing-versus-empty observers; actual Perception remains separate | Automatic Perception inference |
| MindState | Derivative snapshot/cognition projection and permitted JSON Pointer roots | Delta application, replay or history authority |
| Memory capture/review/create/update | Validate `Content.cognition`, preserve recorded-at-time attribution, and apply existing grants/placement/CAS/idempotency/admission bytes | Automatic KnowledgeEntry writes |
| Memory list/recall | One authorized filtered source set before counts/ranking/pages/budgets; declared `q`; stable cursors; emitted-wire byte accounting and oversized-first behavior | New ranking or retrieval policy |
| Memory reflection | Pin cognition filter, source id+revision set, profile/policy and cursor identity; basis changes with source or filter | Content-byte basis hashes or a generation engine |
| Memory export/import | Include cognition in canonical bundle/chunk digests; translate complete references fail-closed; preserve clearing | Grant adoption or cross-store authority by default |
| Delete/scrub/promote | Clear cognition with other sensitive content; promotion changes placement only | Cognition-based privileges or KnowledgeEntry mutation |

A KnowledgeEntry-only host can conform without Memory. A combined host advertises only after satisfying all applicable KnowledgeEntry and Memory rows. A host serving no relevant optional surface does not advertise.

## 2. Existing-carrier negotiation and rejection

### 2.1 Negotiation and dispatch

Signed-hello manifest intersection remains authoritative. TypeScript `negotiatedCapabilities(local, remote)` and the Rust manifest intersection include `spoke-cognition` only when both manifests declare it. Requests carrying `Scope.cognition` or Memory `cognition_filter` require the negotiated flag.

The existing operation/capability table is unchanged: baseline KnowledgeEntry and assemble operations retain `spoke-baseline`; Memory operations retain `spoke-memory` and its existing subflags. A payload-dependent guard adds the cognition requirement only when a cognitive selection/filter/projection is present. Capability-token grants and operation dispatch gates still apply independently.

When the family is absent, a request requiring it receives the existing `capability_missing` dispatch branch without handler side effects. Routing offers cognition-bearing requests only to peers declaring the family. Ordinary records without optional cognitive fields remain ordinary data and do not trigger the error. A requirement is present only when the declared carrier actually holds the family member: a `Scope` that is not a JSON object — or an object without `cognition` — declares no requirement, so a malformed `Scope` answers its baseline shape failure (`INVALID_INPUT`) first and never becomes this capability verdict.

### 2.2 Unsupported requirements

After schema decoding, an unknown predicate, known-but-unimplemented view/dimension/metric filter or unavailable requested source view rejects the entire request through the existing envelope:

```json
{
  "code": "invalid_input",
  "message": "Unsupported cognition requirement",
  "details": {
    "reason": "unsupported_cognition_requirement",
    "field": "<carrier-relative path to the requirement>"
  },
  "extensions": {}
}
```

No silent stripping, ignored conjunction, partial response or unfiltered fallback is permitted. The `field` value is relative to the API's declared carrier root: the request JSON path for the wire carriers (`scope.cognition`, or `cognition_filter` on Memory operations, each followed by the failing member) and the builder's own parameter name when the direct assemble builder receives the selection (`cognitionSelection` in TypeScript, `cognition_selection` in Rust). Valid fork wildcard, exact-fork, exact-no-fork and multi-context selections are served rather than narrowed by caller context. Invalid record data uses `details.reason = "invalid_cognition_record"`; missing optional data is not an error. Existing `fromErrorEnvelope` maps this branch to `INVALID_INPUT`; no ABI error type is added.

### 2.3 Compatibility boundary

Unknown `modules` and `extensions` namespaces retain their existing round-trip behavior. That transport property does not imply that another host understands the closed cognition fields, negotiates the family or interprets `q`. No calibration, old-host field-preservation or deployed-adoption guarantee follows.

## 3. Library and wire ownership

### 3.1 Hand-authored schemas and generated declarations

`schemas/data/cognition.schema.json` is the closed-record SSOT. Existing KnowledgeEntry, Scope, AssemblePacket, MindState/common, Memory Content/candidate/reflection/transfer and five Memory query-request schemas carry the family. Generated TypeScript and Rust outputs come from the existing generator and are never hand-edited.

Generated TypeScript declares the record and supporting types in `generated/data/cognition.ts`. The package's data barrel and package root export `CognitionRecord`; the other cognition member types are module declarations, not all package-root exports. `CognitionProjection` and `CognitionProjectionItem` are declared by the generated MindState, AssemblePacket and assemble-response carrier modules rather than by `cognition.ts`.

Generated Rust follows the same per-wire-module ownership. Canonical cognition imports come from `spoke_schemas::data::cognition`; the data barrel exports `CognitionRecord`, and generated/root barrels re-export the generated module families. Carrier support builders and presence-sensitive setters are generated members, not independent package-root APIs.

Memory Content decoders preserve absent, present-empty and non-empty `cognition`; explicit null is invalid. Generated carriers and their clone types use the same presence rule.

### 3.2 `spoke-operations` public surface

The TypeScript package root exports:

- `validateCognitionRecord`, `validateCognitionContent`, `validateCognitionModule`, `validateObservationModule`;
- `validateCognitionFilter`, `validateCognitionSelection`, `validateCognitionProjection`;
- `cognitionRecordMatchesFilter`, `selectCognitionRecords`, `projectCognitionRecords`;
- `CurrentViewPolicy` and helper-facing `CognitiveView`, `CognitionContext`, `CognitionTarget`, `CognitionProjection`, `CognitionProjectionItem`, `CognitionProjectionItemSelection` types;
- `assertSupportedCognitionSelection`, `cognitionRecordsOfAuthorizedEntries` and the existing scope/assemble entry points.

The Rust crate root exports the corresponding snake_case validators and helpers: `validate_cognition_record`, `validate_cognition_content`, `validate_cognition_module`, `validate_observation_module`, `validate_cognition_filter`, `validate_cognition_selection`, `validate_cognition_projection`, `cognition_record_matches_filter`, `select_cognition_records`, `project_cognition_records`, `CurrentViewPolicy`, `assert_supported_cognition_selection`, `cognition_records_of_authorized_entry` and `cognition_records_of_authorized_entries`.

The singular per-entry authorized-record helper is public in Rust; the TypeScript per-entry counterpart is module-internal and only the plural helper is exported. `validateMindState` / `validate_mind_state` retain their existing names and validate derivative cognition and pointer rules. All of these helpers are pure; they perform no storage, resolution, inference, negotiation, replay or network I/O.

Scope filtering applies cognition only after authorization/disclosure. Entries without carried cognition do not match a cognition selection. `assertSupportedCognitionSelection` rejects unavailable views, including `recorded_at` on the current KnowledgeEntry carrier. Assemble input has optional `cognitionSelection`/`cognition_selection` and `currentViewPolicy`/`current_view_policy`; there is no built-in default policy or context. Without a selection, packet behavior remains unchanged.

### 3.3 Connect public surface

TypeScript exports `CAPABILITY_SPOKE_COGNITION`, `cognitionRequirementField`, `cognitionRequirementPresent` and the existing `WIRE_INVALID_INPUT` constant. Rust exports `CAPABILITY_SPOKE_COGNITION`, `cognition_requirement_field` and `cognition_requirement_present`; its invalid-input vocabulary constant remains crate-internal. Existing session, responder, RemoteAdapter and embedded-node paths apply the same requirement without adding an operation, port method or envelope.

### 3.4 FFI, C API and bindings

Existing JSON-string request/response channels carry cognition fields without a typed callback or C ABI slot. `crates/spoke-connect/src/ffi.rs`, `crates/spoke-connect-capi/src/responder.rs`, its ABI layout and generated/native binding surfaces require no cognition-specific shape. Textual absence of a field in a binding is not runtime evidence; the unchanged JSON channel and existing ABI layout define the boundary.

## 4. Consumer responsibilities

| Consumer | Responsibility |
|---|---|
| KnowledgeEntry producer/adapter | Write closed current records to the holder's `modules.cognition`; retain open mental facets and unknown namespaces |
| World-state producer | Write order-0 WorldFact records without `q` or metrics to `world_facts` |
| Timeline producer | Record could-perceive access without claiming actual capture |
| MindState producer | Project authority derivatively; preserve per-item context and valid pointer roots |
| Memory provider | Preserve all Content states and attribution; authorize before filtering/ranking/pages/budgets; never mutate KnowledgeEntry |
| Memory transfer | Include cognition in bytes/digests and translate complete identities fail-closed |
| Reflection producer | Pin the authorized source id+revision set and cognition filter; detect basis changes |
| Connect host/responder | Advertise only supported served surfaces; negotiate and reject required features before side effects |
| Product policy/checker | Own current selection and Finding generation; retain disagreement and attribution |

Generic module/extension merge and preserve helpers need no cognition branch: opaque namespace transport is not belief merging. Memory review, retention and promotion are not knowledge promotion. No reusable cognition review-budget, delta-application, ordering, merge, scoring, retrieval or checker engine is part of this family.
