# ADR: KE ownership and disclosure capability

Status: implemented — two optional KnowledgeEntry fields and one shared Scope viewpoint carry governance without a subject registry or a new entity

Date: 2026-09-17

## Problem

Knowledge interchange needs to distinguish who holds an entry from who may consume it. Losing character-private ownership can change a receiving product's interpretation from a subject's knowledge into world consensus. Governance therefore needs a shared carrier, distinct from provenance, ontology, and belief content.

## Decision

### 1.1 Capability flag: `ke-ownership`

**`ke-ownership`** is an optional KE governance capability. It covers `KnowledgeEntry.owner`, `KnowledgeEntry.disclosure`, and `Scope.viewpoint`. All three fields are wire-optional. A baseline claim does not require this capability, new ports, or an additional schema file.

The naming rule is **anchor the flag to the concern's actual carrier, not an invented layer**. Use `l<N>-<concern>` for a concern anchored to an existing layer (`l5-fork`, `l2-computable`); use a named cross-layer family when no such anchor is accurate (`narrative-modules`, `spoke-connect`, `spoke-cognition`). `ke-ownership` and [`ke-extraction`](2026-09-17-ke-extraction.md) use the same `ke-<concern>` family discipline. They are independent flags, not an umbrella capability and not prerequisites for each other.

L1 is ontology (`entry_type` and Domain Profiles), not governance. Ownership attaches to the KE envelope and is consumed by L7 check and L8 assemble. Calling it `l1-ownership` would incorrectly make governance an ontology classification; inventing L9 would incorrectly change the L0–L8 model. Register the flag consistently in the protocol umbrella, capability-level table, and host-capabilities table.

### 1.2 Holder-KE reference, no subject registry

| Field | Shape | Meaning |
|-------|-------|---------|
| `KnowledgeEntry.owner` | Optional non-empty string | Opaque `entry_id` of the holder KnowledgeEntry in the product's collaboration context |
| `KnowledgeEntry.disclosure` | Optional non-empty open string | Governance disclosure vocabulary; the sole core value is `owner-private` |

An absent `owner` means **unspecified ownership**, not a `world` owner, public truth, or world consensus. A world, actor, or group may be represented by an ordinary holder KE; the protocol reserves no subject-type enum or `world` token. Reference resolution remains product-owned; the wire need not embed the holder or prove that it is present in the same payload.

An absent `disclosure` means **shared within the already selected KB context**, not public access outside that context and not a claim about epistemic truth. `owner-private` requires a non-empty `owner`. Unknown disclosure strings remain round-trippable open vocabulary; Domain Profiles define any additional audience semantics. Core helpers must not reinterpret unknown values as shared.

The KE schema keeps `additionalProperties: false`. `owner` and `disclosure` are optional properties in the existing schema, not a new ownership object or module namespace. The `owner-private` → owner requirement is a cross-field normative invariant; the operations predicate fails closed for a malformed private entry. Do not close the disclosure vocabulary with an enum.

### 1.3 First-class shared `Scope.viewpoint`

**`Scope.viewpoint?: string`** is a non-empty holder-KE `entry_id`, defined once in `schemas/common/common.schema.json#/definitions/Scope`. Check and assemble consume it through their existing `$ref`; the same shared Scope carries the selector for both operations, and neither gains a request-only sibling selector.

`scope_id` stays opaque. It is neither parsed for viewpoint nor overloaded with an owner convention. Viewpoint is a reader context, not merely an owner-equality filter: a viewpoint may consume shared entries belonging to other holders as well as its own private entries.

Missing viewpoint names no subject and grants no private visibility. Existing entry-id, entry-type, and source refinements compose by AND with the disclosure predicate. TimelineEvent scope matching is unchanged: a KE governance field does not create an event-visibility model.

## Ownership boundary and operations contract

| Concern | Authority / owner |
|---------|-------------------|
| Entry governance | `owner` / `disclosure` on that KnowledgeEntry |
| Holder identity | Existing holder KnowledgeEntry, referenced by `entry_id` |
| Epistemic stance, including belief Access | Holder `modules.cognition` records; narrative content, not a grant of governance access |
| Temporal mental changes | `MindState`, strictly derivative; not an ownership registry |
| Storage write authority / OCC | Existing data-store revision contracts; unrelated to the entry's narrative owner |
| Identity authentication, authorization, view composition and audience evaluation | Product runtime / Domain Profile |

**Single authority per fact.** Do not mirror governance into cognition records or labels, `extensions`, a separate subject object, or a temporal record. A belief label may be mistaken or stale; it cannot override an entry's disclosure. A viewpoint string is not a credential or authorization proof.

### 2.1 Pure helper behavior

| Disclosure | Owner / viewpoint | Core visibility result |
|------------|-------------------|------------------------|
| Absent | Any, including unspecified | Include within the caller's pre-scoped KB |
| `owner-private` | Both present and exactly equal | Include |
| `owner-private` | Missing owner, missing viewpoint, or unequal | Exclude |
| Unknown non-empty value | Any | Exclude; a product must apply its explicitly understood profile outside the core predicate |

No holder lookup, type inference, belief evaluation, audience expansion, ranking, or I/O occurs. Comparison is exact; identifiers are not normalized. Core filtering is conservative, not a product authorization engine.

Frozen public signatures:

| TypeScript | Rust |
|------------|------|
| `getKnowledgeEntryOwner(entry: KnowledgeEntry): string \| undefined` | `get_knowledge_entry_owner(entry: &KnowledgeEntry) -> Option<&str>` |
| `knowledgeEntryVisibleToViewpoint(entry: KnowledgeEntry, viewpoint?: string): boolean` | `knowledge_entry_visible_to_viewpoint(entry: &KnowledgeEntry, viewpoint: Option<&str>) -> bool` |

Place these helpers with the existing KnowledgeEntry helpers. Reuse the predicate in the existing `knowledgeEntryPassesScopeAuthorization` (TypeScript) / `knowledge_entry_passes_scope_authorization` (Rust, crate-internal); reuse existing list filters rather than adding a parallel ownership-filter family. The Rust predicate borrows the identifier and entry; no cloning or additional allocation is required. Check/assemble and fork variants already consume scope filtering and must observe the same result.

### 2.2 Capability boundary

Hosts that exchange these fields or honor viewpoint semantics declare `ke-ownership`. A non-declaring host need not implement them, but must not accept a request requiring the capability and silently return an unfiltered success or strip governance before forwarding it. Capability selection/rejection belongs at the product boundary; this ADR adds no connect dispatch rule.

Pure helpers consume supplied data without fetching manifests or taking a new capability boolean. Calling the ownership-aware surface is the caller's opt-in; fields absent on baseline data retain baseline behavior. No new baseline port or changed check/assemble entrypoint signature is required.

## Alternatives considered

| Alternative | Reason |
|-------------|--------|
| `l1-ownership` | L1 names ontology, not the cross-layer governance concern; KE carrier naming is accurate. |
| New protocol layer / ownership Entity / subject registry | Adds a second identity or authority for a fact already carried by the KE. |
| Reserved `world` owner or closed `world \| character \| actor_world_binding` union | Bakes consumer taxonomy into core; a holder KE and product context already express it. |
| Cognition access labels as governance | Confuses narrative belief content with authoritative disclosure; creates dual SSOT. |
| Product extension/module bag only | Unknown keys cannot preserve agreed ownership semantics across products. |
| Vocabulary-only `entry_type` marker | Labels the entry but supplies neither holder reference nor reader selector. |
| Viewpoint encoded in `scope_id` | Violates opaque Scope neutrality and loses portable field semantics. |
| Request-only viewpoint siblings | Duplicates the selector on check/assemble and abandons the shared Scope carrier. |
| Required owner, default world owner, or baseline capability expansion | Breaks optionality and converts unspecified ownership into fabricated consensus. |
| Core audience-set, view-combination, or storage-lifecycle machinery | Product policy and persistence responsibilities, not this wire contract. |

## Evidence

| Decision basis | Repository evidence |
|----------------|---------------------|
| Loss of private ownership changes interpretation; holder references avoid new entities | Recorded design research; the rationale and check analysis are not retained as repository artifacts |
| L1 is ontology; cross-layer capability families already exist | [`spoke-protocol-layers.md`](../../../specs/protocol/spoke-protocol-layers.md), Layers and Optional flags |
| Shared Scope is the single selector carrier; requests already reference it | [`common.schema.json`](../../../../schemas/common/common.schema.json), `Scope`; [`check-request`](../../../../schemas/ops/check-request.schema.json); [`assemble-request`](../../../../schemas/ops/assemble-request.schema.json) |
| Holder authority and derivative temporal records must remain separate | [`2026-11-07-spoke-cognition-family.md`](../architecture/2026-11-07-spoke-cognition-family.md), authority and ownership |
| Optional fields preserve closed core and do not require a file-count bump | [`capability-flagged-optional-bag`](../../../knowledge/architecture-patterns/capability-flagged-optional-bag.md); [`spoke-codegen-pipeline`](../../../knowledge/architecture-patterns/spoke-codegen-pipeline.md) |
| Existing scope predicates are the mechanical consumer; orchestrators apply them | [`spoke-operations.md`](../../../specs/protocol/spoke-operations.md), Scope match and Injection Orchestration; [`scope/match.ts`](../../../../packages/spoke-operations/src/scope/match.ts) |

## Scope of authority

This ADR owns the flag, field placement, core disclosure semantics, and pure helper behavior. JSON Schema remains the executable wire SSOT; schema authoring and lockstep generation implement this contract. Ownership adds **zero schema files**.

Knowledge-boundary Rule/Finding vocabulary is a **Domain Profile** concern after the ownership carrier is available. Its completion criterion is a documented Rule-input / Finding-output vocabulary over this carrier, not a new checker engine or ownership registry. The core value `owner-private` does not itself define contradiction classes.

Connect/RemoteAdapter/FFI routing, subject registries, view composition, authentication, storage concurrency, and lifecycle variants are outside this contract. The two KE capabilities can be implemented independently.

## Consequences

- An absent `owner` means unspecified ownership, not a world owner or world consensus, and an absent `disclosure` means shared within the already selected KB context rather than public access; the closed KE schema gains optional properties, not a new object or module.
- Core filtering is conservative: absent disclosure includes, `owner-private` with exactly equal owner and viewpoint includes, every other combination excludes, and an unknown non-empty disclosure value excludes until a product applies its own profile outside the core predicate.
- The two helpers are pure and allocation-free where the language allows it; they add no holder lookup, type inference, belief evaluation, audience expansion, ranking or I/O, and existing scope-authorization and list filters reuse the same predicate.
- A non-declaring host must not answer a request that requires governance with an unfiltered success or governance-stripped forwarding; capability selection and rejection stay at the product boundary.
- Ownership adds zero schema files: JSON Schema remains the executable wire truth and schema authoring plus lockstep generation implement the contract.
- The wire carriers are `schemas/data/knowledge-entry.schema.json` and the shared `Scope` in `schemas/common/common.schema.json`; library behaviour is described in `.agents/specs/protocol/spoke-operations.md`.
