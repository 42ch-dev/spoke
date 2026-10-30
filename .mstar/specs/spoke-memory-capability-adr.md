# Memory Capability — Optional `spoke-memory` Family

> **Status:** Normative ADR — accepted design contract
> **Document class:** Normative — capability naming, family boundary, authority model, execution split
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)
> **Family contract:** [`spoke-memory.md`](spoke-memory.md)
> **Wire SSOT:** `schemas/`; this ADR specifies the contract, not a claim that a given product implements it.

## Purpose

Products need per-subject long-term memory — impressions, not canon — with explicit forgetting, provider-enforced authorization across multiple clients, and a replaceable provider. Three questions decide how that lands in SPOKE:

1. **Is memory a layer, a flag, or a family?** The layer model is L0–L8 and memory content spans them (L1/L2 records, L3 provenance, L7-equivalent review decisions, L8-equivalent read selection). A single layer cannot own it, and a `l<N>-<concern>` flag would misstate which artifacts it gates.
2. **Where does authority live?** Memory needs grant-checked admission, optimistic concurrency, idempotent replay, fencing, and atomic candidate review. Distributing those across clients produces irreconcilable duplicates.
3. **What is replaceable?** A provider swap must be a wire procedure with digests and verification, not a bespoke migration, and the reusable algorithm policy must be separable from storage and enforcement so it can be shared without owning state.

This ADR locks the naming, the family boundary, the single-authority model, the single async/outcome model, and the engine/provider split. Wire field semantics live in [`spoke-memory.md`](spoke-memory.md).

## 1. Decision

### 1.1 Capability family: `spoke-memory` (+ two optional sub-flags)

Memory is an **optional capability family**, not an extension of `spoke-baseline` and not a layer flag:

| Capability ID | Gates | Required for conformance |
|---------------|-------|--------------------------|
| `spoke-memory` | The 25 core `memory.*` ops | Yes, for the family |
| `spoke-memory-reflection` | `memory.reflections.state` / `.list` / `.generate` | Optional; requires `spoke-memory` |
| `spoke-memory-transfer` | `memory.exports.*` / `memory.imports.*` | Optional; requires `spoke-memory` |

**Naming.** `spoke-<family>` names a cross-layer capability family whose artifacts are not confined to one layer — the same discipline as `spoke-connect` for cross-process interaction. A `l2-*` or `l5-*` flag would name a layer that does not own the artifacts: records are L1/L2 shapes, provenance is L3, review decisions are Finding-like outputs, and read selection is L8-like. The layer model stays L0–L8 ([`spoke-protocol-layers.md`](spoke-protocol-layers.md)).

**No baseline expansion.** Declaring `spoke-memory` adds **no** field to any baseline object, no change to `HostCapabilityManifest`, no new baseline port, no `port.*` catalogue change, and no core op change:

| Surface | Baseline state | With memory |
|---------|----------------|-------------|
| `KnowledgeEntry` / `Relation` / `Finding` / `AssemblePacket` / `Rule` / `TimelineEvent` / `MindState` / `ToolDescriptor` | Unchanged | Unchanged |
| `HostCapabilityManifest` | `capabilities[]` open string vocabulary | Unchanged shape; hosts MAY list the three memory flags |
| Baseline port families (`KnowledgeEntryPort`, `RelationPort`, `ScopeQueryPort`, `FindingPort`, `RuleQueryPort`, `HostManifestPort`, `ComputablePort`, `ForkTimelineQueryPort`) | Six baseline families plus computable/fork composition | Unchanged; memory is not a member |
| Core op table (`upsert`, `promote`, `relate`, `check`, `assemble`, `project`, `compute`, `extract`) | Unchanged | Unchanged |
| `common.schema.json` definitions | Unchanged | Unchanged; memory reuses `SchemaVersion`, `Timestamp`, `ExtensionMap`, `OpaqueJson`, `SourceAnchor` |

The `MemoryPort` trait/interface is **standalone**, exactly like `ExtractionPort`: not in `BaselinePorts`, `FullPorts`, `ComputablePorts`, or `ForkPorts`, and not in their adapter aliases. Adapters that do not serve memory omit it; the availability probe (`as_memory()` / `asMemory()`) reports absence, and a caller that needs memory reports the missing capability (`MEM-R019`).

**Artifact placement.** Thirteen data schemas and 33 request/response pairs live in `schemas/data/` and `schemas/ops/` under the standard naming rules, generated into both language packages by the existing pipeline. The family adds no schema directory, no second codegen path, and no hand-edited generated code.

### 1.2 Single-authority model

One `MemorySpace` has exactly one authoritative provider for its whole lifetime (`MEM-R001`). The decision has three parts:

| Concern | Decision | Consequence |
|---------|----------|-------------|
| Identity | The principal comes from the authenticated admission context — the Connect peer identity mapped by provider-local configuration, or the in-process embedding product's context. Payload `principal_id` fields are target selectors only. | `MEM-R003`: no payload-supplied authority; a model or record body can never mint a grant. |
| Authorization | `MemoryGrant` is self-contained: capability set + subject/space scope + optional `context_ids` + `shared_access` + expiry, evaluated at admission by the provider. | `MEM-R020`: the memory grant model neither inherits nor consults `ke-ownership` / `Scope.viewpoint`. Memory ops accept no `Scope` as authorization input. |
| Consistency | `revision` CAS on every mutable object, durable payload-bound idempotency keyed on `(space_id, requester_principal_id, op, idempotency_key)`, and provider-side fencing for space freeze, subject fence, and context fence. | `MEM-R007`, `MEM-R008`, `MEM-R017`, `MEM-R024`: conflicting writes are either serialized by the authority or rejected with a stable code — never merged client-side. |

Multi-client semantics are therefore admission-time properties of the provider, not conventions layered by clients: revocation applies to subsequent admissions while already-admitted ops complete (`MEM-R018`); a fence commit and the admission-gate update are one transaction; a cursor bound to a snapshot is invalidated by any state or grant change (`MEM-R026`).

### 1.3 Receipt async model

There is exactly one async/outcome model: a durable, payload-bound, content-free `MemoryReceipt`.

| Property | Decision | Consequence |
|----------|----------|-------------|
| Uniform return | Every mutating op returns a receipt — `committed`, `pending`, or `failed` — and `result` carries only IDs, revisions, counts, outcome enums, digests, and cost facts. | `MEM-R022`: live objects are read through read ops, so receipts survive erasure and remain immutable audit. |
| Pending set | Only `memory.reflections.generate`, `memory.exports.create`, the final chunk of `memory.imports.stage`, and `memory.imports.activate` may return `pending`. Completion is observed exclusively through `memory.receipts.get`. | No webhooks, streams, or callbacks; no second notification surface. |
| Uncertainty | A timeout or transport failure resolves by same-key replay or by polling `memory.receipts.get` (`MEM-R010`, `MEM-R021`). Replay returns the stored receipt without re-executing and without re-evaluating superseded preconditions. | Idempotency is the recovery channel; `OUTCOME_UNCERTAIN` is a terminal observation, not a second commit path. |
| Retention | Receipt bodies retire after `receipt_retention_seconds`; a content-free dedup tombstone survives `dedup_retention_seconds`. Replay against an evicted body fails `RECEIPT_EXPIRED`. | Dedup outlives disclosure; erasure and audit coexist. |

Non-mutating ops return payloads directly and never mint receipts.

### 1.4 Engine / provider split

Algorithm policy and enforcement are separated because they have different owners and different failure modes:

| Concern | Owner | State | Contract |
|---------|-------|-------|----------|
| Classification, ranking, budget/truncation, reflection assembly and staleness, transfer partitioning/digests/verification/translation | `spoke-fixture-memory-engine` crate (`MemoryEngine`, four pure modules) — unpublished conformance fixture at `fixtures/memory-engine/rust`, not a shipped artifact; the published memory surface is `spoke-schemas` + `spoke-operations` | None | Pure functions over already-authorized inputs; deterministic per `ENGINE_POLICY_VERSION`; no storage abstraction, no I/O, no authorization, no CAS, no transactions |
| Storage, authorization, grants, fencing, idempotency, transactions, durability | The provider behind `MemoryPort` | Provider-owned | Enforces §5 invariants inside its own transaction layer; never delegates enforcement to the engine |
| Wire shapes | `schemas/` + generated packages | — | The only normative structural truth |

Consequences:

- The engine is **not a protocol participant**: two providers may run different engines (or none) and stay wire-compatible; `engine_policy_version` is the algorithm axis, `memory_protocol_version` the wire axis.
- The library contains helpers but no policy engine: request validators, selection invariants, grant-coverage evaluation, freeze allowlist, and JCS/SHA-256 digests. Validation at the library boundary is structural and deterministic — authorization remains the provider's duty.
- A provider swap changes only the provider; the engine and the wire are unchanged.

### 1.5 Conformance providers (dual-provider decision)

A single-provider conformance suite proves wire-shape agreement but not **provider replaceability**: the export/import and cutover contract is exercised only when two distinct providers implement the same wire and the same verification rules independently.

Decision: the family's conformance scenarios run against **two** providers over Connect loopback:

1. a **reference-policy loopback provider** driven by `MemoryEngine` — proves the algorithm policy and the pure-helper gates;
2. a **repository-local persistent reference provider** with its own storage and transaction handling — proves storage-backed authority, durability of receipts and dedup tombstones, staged import durability, and digest-verified chunked transfer.

Scenarios pass only when both produce the identical outcome required by the anchor they cite (§17 of [`spoke-memory.md`](spoke-memory.md)). Algorithm-variable surfaces (classification dispositions, ranking order, reflection text, scoring) are excluded from the identity requirement.

The second provider is conformance infrastructure inside this repository: it is not a shipped runtime, daemon, or hosted service, and it is not the protocol's reference implementation for products. Product adoption, data migration, and cutover remain consumer-repository work.

### 1.6 Semantic locks carried by the family

The family's locked semantics are normative in [`spoke-memory.md`](spoke-memory.md) §15 and summarized here as design decisions that constrain any future wire change:

| Axis | Lock | Wire home | Rejected home |
|------|------|-----------|---------------|
| Subjectivity | Memory is per-subject impression, never KnowledgeEntry canon | Separate envelopes and grant model | Reusing `KnowledgeEntry` or `modules.memory` |
| Forgetting | Explicit only: drop/dismiss, tombstone delete, archive, import discard | `memory.candidates.*`, `memory.records.delete`, `memory.subjects.fence`, `memory.imports.discard` | Record-level `expires_at` or time-based fading |
| Affect | Opaque convention via `content.structured` | `content.structured` + open `kind` vocabulary | First-class `affect` wire field or affect enumeration |
| Viewer impressions | Flat subjects plus placement and read-selection axes | `scope` + `ReadSelection` + `ProvenanceFilter` + context grants/fences | `about_subject_ids[]`, hidden graph semantics |
| Personality basis | Authored trusted profile documents, never authorization | `ProfileDocument` (`personality`, `preferences`) | Presence of a profile implying capability |

## 2. Ownership boundary

| Concern | Owner |
|---------|-------|
| Wire shapes and family op table | `schemas/data/`, `schemas/ops/`, [`spoke-memory.md`](spoke-memory.md) |
| Structural/selection/grant/digest helpers | `packages/spoke-operations/src/memory/`, `crates/spoke-operations/src/memory/` |
| Provider admission, storage, transactions, durability | Provider behind `MemoryPort` |
| Algorithm policy | `fixtures/memory-engine/rust/` (unpublished fixture crate `spoke-fixture-memory-engine`) |
| Capability negotiation, dispatch gate, serving/client seams | `crates/spoke-connect/`, `packages/spoke-connect-ts/` |
| Space provisioning and initial `admin` grant | Provider operator, out of band (§7.0.6 of the family spec) |
| Identity mapping of peers to principals | Provider-local configuration (`MemoryAdmission` in the Connect serving seam) |
| Product memory policy (mood taxonomy, retention policy beyond the wire, cross-subject references) | Product code under `content.structured` and `extensions.<namespace>` |

### 2.1 Injection and serving surface

| Surface | Shape | Rule |
|---------|-------|------|
| `MemoryPort` | Typed per-op methods, principal passed explicitly | Standalone optional port; the only public replacement boundary for an embedded provider |
| `MemoryPorts` / `asMemory` | Availability probe | Absence means "not served"; callers report the missing capability instead of falling back |
| `RemoteMemoryService` | One callable per `memory.*` op | Serving-host face; no second 33-method serving trait, because the typed contract already exists on `MemoryPort` |
| Client mapping | `MemoryPort` method → `memory.*` op table | Parity-tested in both languages; exported so the mapping cannot drift |

## 3. Rejected alternatives

| Alternative | Reason |
|-------------|--------|
| Reuse `KnowledgeEntry` + `modules.memory` | KnowledgeEntry is a narrative KB envelope (canonical name, `ke-ownership` disclosure semantics); memory needs candidates, grants, fencing, receipts, and a self-contained authorization model. Emulation is prohibited by `MEM-R019`. |
| Extend `BaselinePorts` / `FullPorts` | Memory is optional and independent; a baseline member would force every baseline adapter to implement or stub 33 ops (mirrors the `ExtractionPort` precedent). |
| `l2-memory` / `l5-memory` layer flag | The artifacts span L1–L5 and L8-like read selection; a layer flag misstates the gated set. |
| `memory-axis` style flag | Breaks the `l<N>-<concern>` / `spoke-<family>` naming discipline without naming what is gated. |
| Public storage SPI behind the engine | The replacement boundary is `MemoryPort`; storage and transactions stay provider-private, and the engine stays pure. |
| Engine performing authorization or CAS | Enforcement in a reusable library invites partial compliance and duplicate authority; the provider must own admission inside its transaction boundary. |
| CRDT / offline multi-master / federated recall | Contradicts `MEM-R001` and the provider-enforced fencing, atomic review, and drain-barrier guarantees. |
| Webhooks / streaming / callback async | One durable-receipt model covers every long-running op; a second notification surface doubles the outcome contract. |
| Inheriting `Scope.viewpoint` authorization | It is a KB disclosure carrier, not a grant system; inherited semantics would make memory authorization product-ambiguous (`MEM-R020`). |
| Chunked export with the header enumeration only (no `extensions` carrier) | The bundle requires `extensions` while the header carries scalars; without a carrier a valid bundle cannot be chunked and reassembled losslessly, breaking the digest round-trip. The header therefore carries optional `extensions` on chunk 0. |
| Unchunked single-message export | Connect invoke payloads are bounded and memory exports must never silently truncate (`MEM-R015`). |
| Durable `ExportRun` / `ImportRun` domain objects | Receipts already correlate, bound, and replay these operations; a second run object would duplicate authority without adding a wire need. |
| Single-provider conformance | Proves wire agreement only; replaceability and the transfer contract need a second independent provider (§1.5). |
| Product adoption or data migration inside this repository | Consumer-repository work; this repository ships wire, library, engine, Connect seams, and conformance infrastructure. |

## 4. Evidence chain

| Decision basis | Repository evidence |
|----------------|---------------------|
| Optional families already exist outside baseline (`spoke-connect`, `l2-computable`, `ke-extraction`, `l5-mind`, `ke-ownership`, `narrative-modules`) | [`spoke-protocol-layers.md`](spoke-protocol-layers.md) §Optional flags; [`spoke-protocol.md`](spoke-protocol.md) |
| Standalone optional port precedent (`ExtractionPort`, not in `BaselinePorts` / `FullPorts`) | `crates/spoke-operations/src/adapter/ports.rs`; [`ke-extraction-adr.md`](ke-extraction-adr.md) §2.1 |
| Capability gate fails closed; rows per op | `crates/spoke-connect/src/core/dispatch.rs` (`CAPABILITY_SPOKE_MEMORY`, `…_REFLECTION`, `…_TRANSFER`); TypeScript parity in `packages/spoke-connect-ts/tests/dispatch.test.ts` |
| Serving seam mirrors the extraction precedent | `crates/spoke-connect/src/remote/serve_ports.rs` (`RemoteMemoryService`, `MemoryPortService`, `MemoryAdmission`, `RemoteServePortsComposite::with_memory`) |
| Client mapping precedent (`port.*` invoke mapping) | `crates/spoke-connect/src/remote/remote_adapter.rs`; `packages/spoke-connect-ts/src/remote/remote-adapter.ts` |
| Durable payload-bound idempotency is new for this family; baseline `idempotency_key` carries no server semantics | `schemas/ops/upsert-request.schema.json`; family contract §7.0.5 |
| OCC precedent (`expected_revision` conditional writes) | `packages/spoke-operations/src/adapter/ports.ts`, `crates/spoke-operations/src/adapter/ports.rs` |
| Content-free receipt + dedup tombstone pattern | `schemas/data/memory-receipt.schema.json`; family contract §6.10, §14.3 |
| Author/derivative split precedent (authority vs derived record) | [`l5-mind-capability-adr.md`](l5-mind-capability-adr.md) §Ownership boundary (`modules.mental` authority vs `MindState` derivative) |
| Schema conventions: draft-07, closed objects, required `extensions`, `<op>-request`/`<op>-response` pairing, one type per file | [`schemas/README.md`](../../schemas/README.md); `EXPECTED_SCHEMA_COUNT` in `tooling/codegen/assert-schema-count.mjs` |
| Transfer verification is algorithmic, not storage-bound | `fixtures/memory-engine/rust/src/transfer.rs` (`bundle_digest`, `chunk_digest`, `partition_bundle`, `reassemble`, `verify_reassembly`, `build_translation_maps`, `adopt_grants`, `staged_report`) |
| Manifest carrier needs no new field | `schemas/data/host-capability-manifest.schema.json` `capabilities[]` |

## 5. Scope of authority

| This ADR owns | This ADR does not |
|---------------|-------------------|
| Capability naming, family boundary, and the no-baseline-expansion rule | Field-level tables — [`spoke-memory.md`](spoke-memory.md) §6 |
| Single-authority model and its admission-time consequences | Per-op request/result/error detail — [`spoke-memory.md`](spoke-memory.md) §7, §14 |
| Receipt as the sole async/outcome model | Retry-class mappings — [`spoke-memory.md`](spoke-memory.md) §14.2 |
| Engine/provider split and the engine's non-participant status | Engine function signatures — `fixtures/memory-engine/rust/` |
| Conformance provider selection and the identity/algorithm-variable split | Scenario bodies and expected vectors — the repository conformance fixtures |
| Standalone port and Connect seam placement | Product adapter behavior, identity provisioning, and cutover execution — consumer repositories |
| Semantic locks as design constraints | Mood taxonomies, retention policy, and cross-subject reference conventions — product policy |

## See also

| Doc | Topic |
|-----|-------|
| [`spoke-memory.md`](spoke-memory.md) | Normative family contract — flags, invariants, data models, ops, errors, transfer, ProfileDocument |
| [`spoke-protocol.md`](spoke-protocol.md) | Umbrella — three columns and capability flags |
| [`spoke-protocol-layers.md`](spoke-protocol-layers.md) | L0–L8 layer model; optional families |
| [`spoke-ops.md`](spoke-ops.md) | Op wire families; error envelope attachment |
| [`spoke-connect.md`](spoke-connect.md) | Connect envelopes carrying `memory.*` payloads |
| [`ke-extraction-adr.md`](ke-extraction-adr.md) | Optional family + standalone port precedent |
| [`l5-mind-capability-adr.md`](l5-mind-capability-adr.md) | Authority vs derivative split precedent |
| [`schemas/README.md`](../../schemas/README.md) | Schema inventory and authoring rules |
