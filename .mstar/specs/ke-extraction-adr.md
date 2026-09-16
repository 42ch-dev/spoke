# KE Extraction — Capability ADR

> **Status:** Normative ADR — accepted design contract
> **Document class:** Normative — optional extraction wire and injected execution boundary
> **Parent:** [`spoke-protocol.md`](spoke-protocol.md)
> **Wire SSOT:** `schemas/`; this decision specifies the contract, not a claim that generated packages already implement it.

## Purpose

An input-source host can propose provisional KnowledgeEntries, and promote governs admission to durable storage. Extraction needs its own interoperable action: referenced content and ranges in, provisional candidates out. The protocol standardizes this I/O, while the product owns source access and the extraction method.

## 1. Decision

### 1.1 Capability flag: `ke-extraction`

**`ke-extraction`** declares the optional `extract` operation and its standalone `ExtractionPort` plus injected `runExtractor` boundary. An offering host uses the existing `input-source` role and declares this flag in `HostCapabilityManifest.capabilities`. There is no new role, `extraction.*` sub-capability registry, or sixth baseline operation. Promote remains baseline.

Use the same **carrier-anchored naming discipline** as [`ke-ownership`](ke-ownership-disclosure-adr.md): `ke-<concern>` names a KE lifecycle family spanning layers. Source references are L3; candidate ontology/body are L1/L2; production is an operation, not a provenance record. Neither `l1-extraction` nor `l3-extraction` accurately names the whole capability. Existing layer-anchored flags retain `l<N>-<concern>`; the L0–L8 model is unchanged. The two KE flags are independent and neither requires the other.

### 1.2 OQ-EX-1: reference-only input

`ExtractRequest` has this closed top-level shape:

| Field | Required | Contract |
|-------|----------|----------|
| `run_id` | Yes | Non-empty opaque string supplied by the caller; correlation identity for this extraction batch |
| `sources` | Yes | Non-empty `SourceAnchor[]`, using `$ref` to the existing data schema |
| `entry_types` | No | Open string array of candidate-type hints; advisory, not a closed taxonomy or a post-filter |
| `extensions` | No | Existing `ExtensionMap` |

A complete SourceAnchor includes `schema_version`, `source_id`, and `extensions`; `span`, `label`, and `mime_type` keep their existing optionality. Extraction range is the selected source list plus each anchor's optional span. An absent span means the referenced artifact as a whole. Source locator grammar and byte-versus-character offset interpretation remain product-owned. Do not introduce a competing range object, required span, or a `Scope` that pretends source material is an already-existing KE collection.

There is **no inline-text field**, even for small fragments. Products can expose an ephemeral source locator for such material. A SourceAnchor label is descriptive metadata, not an alternative content channel. Product extensions must not be required as a hidden substitute for standardized input. Source loading occurs inside the host through the injected port, not on the extract request wire.

### 1.3 OQ-EX-2: one correlation id and minimal advisory metadata

`ExtractResponse` uses the existing closed `oneOf` success/error envelope:

| Branch | Required fields | Optional fields |
|--------|-----------------|-----------------|
| Success | `candidates: KnowledgeEntry[]`, `run: ExtractionRunMetadata` | `extensions: ExtensionMap` |
| Error | `error: ErrorEnvelope` | `extensions: ExtensionMap` |

`ExtractionRunMetadata` is a **local definition in `extract-response.schema.json`**, not a third schema file or durable entity:

| Field | Required | Contract |
|-------|----------|----------|
| `run_id` | Yes | Exact echo of request `run_id`; batch id and correlation id are one field, not two identities |
| `method` | No | Non-empty open string describing the product's method; no core method enum |
| `coverage_hint` | No | `$ref` to existing `OpaqueJson`; advisory and opaque |

`coverage_hint` has no standardized units, ordering, completeness score, comparison, or reconciliation meaning. Omission and JSON null both mean no advisory hint; other JSON values (scalars, arrays, or objects) remain opaque and are retained verbatim. Neither promote nor another core operation may require or interpret it. Product interpretation belongs under a mutually understood profile, not a core coverage protocol.

An empty candidate array is a successful zero-result run. Every returned candidate must be an ordinary KE with `status: "provisional"`. The schema references KnowledgeEntry rather than copying or adding fields to it; the operation invariant is enforced by orchestration. There are no core candidate-confidence fields, per-candidate run objects, suspicious-point fields, or Finding output. A candidate's `source_anchor`, when supplied, remains its provenance authority; batch metadata is not another content authority.

A failed response carries only the standard error branch, not a partial candidate list or a second run record. The caller associates failures with its invocation context. This contract adds no persistence, idempotency, retry, resume, incremental re-extraction, or run-lookup semantics.

## 2. Ownership boundary and frozen library contract

| Concern | Owner |
|---------|-------|
| Wire shape and provisional-result gate | Schemas and operations |
| Source access, credentials, location resolution | Injected product `ExtractionPort` |
| LLM, manual, or rule-based production | Injected product `runExtractor` |
| Batch association | Caller `run_id`, echoed mechanically by orchestration |
| Durable KE storage and revision assignment | Existing data-store through explicit promote / persistence contracts |
| Mental inference / checker Finding patterns | Product engines; not this extraction library |

### 2.1 Injection surface

The port loads product-defined JSON input for the requested references. That in-process value may contain source content; it is **not a new wire object** and never appears on ExtractRequest/ExtractResponse. One opaque payload avoids inventing a document parser or source registry in the library.

TypeScript contracts:

```typescript
interface ExtractionPort {
  loadExtractionInput(request: ExtractRequest): Promise<SpokeResult<OpaqueJson>>;
}

type ExtractRunInput = { request: ExtractRequest; input: OpaqueJson };
type ExtractionResult = {
  candidates: KnowledgeEntry[];
  method?: string;
  coverage_hint?: OpaqueJson;
};
type RunExtractor = (input: ExtractRunInput) => Promise<SpokeResult<ExtractionResult>>;

declare function orchestrateExtract(
  ports: ExtractionPort,
  request: ExtractRequest,
  runExtractor: RunExtractor,
): Promise<SpokeResult<ExtractResponse>>;
```

Rust contracts:

- `#[async_trait] pub trait ExtractionPort { async fn load_extraction_input(&self, request: &ExtractRequest) -> SpokeResult<serde_json::Value>; }` — Send future, no runtime dependency.
- `ExtractRunInput { request: ExtractRequest, input: serde_json::Value }`.
- `ExtractionResult { candidates: Vec<KnowledgeEntry>, method: Option<String>, coverage_hint: Option<serde_json::Value> }`. Null and absence have the same no-hint semantics; all non-null payloads are retained.
- `pub async fn orchestrate_extract<F, Fut>(ports: &dyn ExtractionPort, request: ExtractRequest, run_extractor: F) -> SpokeResult<ExtractResponse> where F: FnOnce(ExtractRunInput) -> Fut, Fut: Future<Output = SpokeResult<ExtractionResult>> + Send`.

`ExtractionPort` is standalone, like `ToolInvokePort`: do not add it to `BaselinePorts`, `FullPorts`, or their adapter aliases. Existing check callbacks remain synchronous and unchanged. The extraction callback is an **async injected product boundary**, because an extraction service can perform asynchronous work; neither a sync/async union nor a hidden blocking runtime is acceptable. The library only awaits injected work and performs deterministic validation/assembly; it contains no source I/O or extraction engine.

### 2.2 Required sequence and errors

1. Check non-empty `run_id` and a non-empty source list at the library boundary; invalid input returns `INVALID_INPUT`. Full structural JSON Schema validation remains at the caller/fixture boundary.
2. Verify the injected loading method is available at dynamic boundaries; absence returns `CAPABILITY_PORT_MISSING` with `details.capability = "ke-extraction"`, following optional-port practice. Rust's typed port is required; a missing-port double demonstrates the same rejection behavior without modifying baseline availability traits.
3. Await `loadExtractionInput(request)`. On rejection, return it unchanged and do not invoke the extractor.
4. Await `runExtractor({ request, input })` exactly once. On rejection, return it unchanged.
5. Check the candidate set before returning any success. A `merged` or `deleted` candidate returns `CANDIDATE_TERMINAL_STATUS`; another non-provisional status returns `CANDIDATE_NOT_PROVISIONAL`. Do not rewrite statuses, drop bad candidates, or return partial success. These codes retain their established meanings; promote logic itself is unchanged.
6. Return candidates and `run` with the caller's exact `run_id` plus supplied advisory metadata. The callback has no second batch-id field to disagree with. Use existing `toErrorEnvelope` / `fromErrorEnvelope` at the wire boundary; do not create another error taxonomy.

No get/put KnowledgeEntry, promote, revision increment, or status upgrade occurs in this sequence. The human-in-loop invariant remains intact; accepted extraction output is still only a provisional proposal. Capability selection belongs to the caller/host; the orchestrator does not auto-fetch manifests or add connect routing.

## 3. Rejected alternatives

| Alternative | Reason |
|-------------|--------|
| Research scheme A: vocabulary / provenance metadata only | Cannot express an extraction service request and result; leaves the production action missing. |
| Research scheme C: durable ExtractionRun + coverage reconciliation + incremental re-extraction | Adds lifecycle, authority, and coordination machinery not needed for request/result correlation. |
| Inline full text or a small-fragment exception | Creates a second input channel and a size-policy boundary; existing SourceAnchor references cover both. |
| A required span or flattened `source_id` / range siblings | Changes existing SourceAnchor optionality or duplicates its grammar. |
| Separate batch id and correlation id | Creates avoidable identity disagreement; a single echoed `run_id` is sufficient. |
| Closed extraction-method enum or standardized coverage score | Standardizes product cognition and quality semantics rather than interoperable I/O. |
| Extract writes durable/confirmed KEs | Bypasses explicit admission and human acceptance. |
| New confidence or run fields on core KE | Inflates every KE for one producer's metadata and introduces a second authority. |
| `l1-extraction`, `l3-extraction`, or `extraction.*` capability taxonomy | Misstates the cross-layer operation or creates unnecessary sub-capability registration. |
| Library extractor engine, belief revision, or suspicious-point checker | Violates the pure-library boundary; Finding remains checker output. |
| Synchronous-only extraction callback or `T \| Promise<T>` callback | Blocks a real asynchronous product service or doubles its contract; use one explicit async boundary. |

## 4. Evidence chain

| Decision basis | Repository evidence |
|----------------|---------------------|
| Production action gap; scheme B; A/C trade-offs | Local research corpus: `.mstar/projects/_default/research/ke-extraction/01-extract-half-gap.md`, lifecycle analysis and scheme comparison (harness-local, gitignored; supporting input only) |
| Input-source role exists; data-store owns settled writes | [`spoke-data-model.md`](spoke-data-model.md), Host roles and Authority |
| Full reference shape and optional span | [`source-anchor.schema.json`](../../schemas/data/source-anchor.schema.json); `SourceSpan` in [`common.schema.json`](../../schemas/common/common.schema.json) |
| Inject product computation after port reads | [`spoke-operations.md`](spoke-operations.md), `CheckRunInput` and `orchestrateCheck`; [`adapter-injection-orchestration`](../knowledge/architecture-patterns/adapter-injection-orchestration.md) |
| Honest async ports, stable SpokeResult, no compatibility shims | [`async-surface-migration-frozen-contract`](../knowledge/conventions/async-surface-migration-frozen-contract.md); the new callback's async choice does not change the existing checker contract |
| Pure plumbing is eligible; speculative mind-axis helpers and engines remain gated | [`mind-axis-ops-extraction-gate`](../knowledge/architecture-patterns/mind-axis-ops-extraction-gate.md), boundary and demand gate. That gate's two-consumer threshold governs mind-axis helper extraction, not this referenced-content operation. No two independent deployed consumers are claimed here. |
| Optional capabilities need not widen baseline | [`capability-flagged-optional-bag`](../knowledge/architecture-patterns/capability-flagged-optional-bag.md); [`spoke-protocol-layers.md`](spoke-protocol-layers.md), optional ops under `l2-computable` |
| Correlation without an additional durable domain object | [`spoke-protocol.md`](spoke-protocol.md), computable op `session_id` and Session lifecycle |
| Closed success/error envelopes, exact dual-generator inventory | [`check-response.schema.json`](../../schemas/ops/check-response.schema.json); [`spoke-codegen-pipeline`](../knowledge/architecture-patterns/spoke-codegen-pipeline.md); `EXPECTED_SCHEMA_COUNT` in [`assert-schema-count.mjs`](../../tooling/codegen/assert-schema-count.mjs) and [`rust-gen`](../../tooling/codegen/rust-gen/src/main.rs) |

## 5. Scope of authority

This ADR owns capability naming, reference-only request placement, minimal run metadata, provisional-result behavior, and the injected execution seam. JSON Schema remains executable wire truth. Exactly two schema files implement this operation: `extract-request.schema.json` and `extract-response.schema.json`. Shared definitions and metadata nested in those files do not add inventory entries.

There is no dependency on ownership fields or viewpoint. Connect/RemoteAdapter/responder/FFI integration is separate demand-gated work when a host requires remote extraction. Coverage reconciliation, durable run tracking, incremental re-extraction, full text on the wire, and cognitive-method standardization are outside this contract. Promote's `MERGE_TARGET_SELF`, terminal/provisional gates, and human-in-loop behavior remain unchanged.
