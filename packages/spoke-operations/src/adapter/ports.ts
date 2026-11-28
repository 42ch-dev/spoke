import type {
  ComputeRequest,
  ComputeResponse,
  ExtractRequest,
  Finding,
  ForkId,
  HostCapabilityManifest,
  KnowledgeAdoptRequest,
  KnowledgeAdoptResponse,
  KnowledgeEntry,
  MemoryCandidatesDismissRequest,
  MemoryCandidatesDismissResponse,
  MemoryCandidatesGetRequest,
  MemoryCandidatesGetResponse,
  MemoryCandidatesListRequest,
  MemoryCandidatesListResponse,
  MemoryCandidatesReviewRequest,
  MemoryCandidatesReviewResponse,
  MemoryCaptureRequest,
  MemoryCaptureResponse,
  MemoryDescribeRequest,
  MemoryDescribeResponse,
  MemoryExportsCreateRequest,
  MemoryExportsCreateResponse,
  MemoryExportsGetRequest,
  MemoryExportsGetResponse,
  MemoryGrantsGrantRequest,
  MemoryGrantsGrantResponse,
  MemoryGrantsListRequest,
  MemoryGrantsListResponse,
  MemoryGrantsRevokeRequest,
  MemoryGrantsRevokeResponse,
  MemoryImportsActivateRequest,
  MemoryImportsActivateResponse,
  MemoryImportsDiscardRequest,
  MemoryImportsDiscardResponse,
  MemoryImportsStageRequest,
  MemoryImportsStageResponse,
  MemoryProfileGetRequest,
  MemoryProfileGetResponse,
  MemoryProfileUpdateRequest,
  MemoryProfileUpdateResponse,
  MemoryRecallRequest,
  MemoryRecallResponse,
  MemoryReceiptsGetRequest,
  MemoryReceiptsGetResponse,
  MemoryRecordsCreateRequest,
  MemoryRecordsCreateResponse,
  MemoryRecordsDeleteRequest,
  MemoryRecordsDeleteResponse,
  MemoryRecordsGetRequest,
  MemoryRecordsGetResponse,
  MemoryRecordsListRequest,
  MemoryRecordsListResponse,
  MemoryRecordsPromoteRequest,
  MemoryRecordsPromoteResponse,
  MemoryRecordsUpdateRequest,
  MemoryRecordsUpdateResponse,
  MemoryReflectionsGenerateRequest,
  MemoryReflectionsGenerateResponse,
  MemoryReflectionsListRequest,
  MemoryReflectionsListResponse,
  MemoryReflectionsStateRequest,
  MemoryReflectionsStateResponse,
  MemorySpacesCreateRequest,
  MemorySpacesCreateResponse,
  MemorySpacesFreezeRequest,
  MemorySpacesFreezeResponse,
  MemorySpacesListRequest,
  MemorySpacesListResponse,
  MemorySpacesUnfreezeRequest,
  MemorySpacesUnfreezeResponse,
  MemorySubjectsEnsureRequest,
  MemorySubjectsEnsureResponse,
  MemorySubjectsFenceRequest,
  MemorySubjectsFenceResponse,
  MemorySubjectsGetRequest,
  MemorySubjectsGetResponse,
  Principal,
  ProjectRequest,
  ProjectResponse,
  Relation,
  RevisionSnapshot,
  Rule,
  Scope,
  TimelineEvent,
} from "@42ch/spoke-schemas";

import type { HistoryPruneResult } from "../history.js";
import type { SpokeResult } from "../result.js";

/** Knowledge entry persistence — get / put by entry id. */
export interface KnowledgeEntryPort {
  getKnowledgeEntry(
    entryId: string,
  ): Promise<SpokeResult<KnowledgeEntry>>;
  putKnowledgeEntry(
    entry: KnowledgeEntry,
    expectedBaseRevision: number | null,
  ): Promise<SpokeResult<KnowledgeEntry>>;
}

/** Relation persistence — get / put by relation id. */
export interface RelationPort {
  getRelation(relationId: string): Promise<SpokeResult<Relation>>;
  putRelation(
    relation: Relation,
    expectedBaseRevision: number | null,
  ): Promise<SpokeResult<Relation>>;
}

/** Scope query for check / assemble — timeline events only. */
export interface ScopeQueryPort {
  listTimelineEvents(scope: Scope): Promise<SpokeResult<TimelineEvent[]>>;
}

/**
 * Scope-scoped knowledge persistence — the serving face a host attaches to
 * answer the three remote knowledge ops. `scope` comes from the responder's
 * per-peer admission, never from the wire request.
 */
export interface ScopedKnowledgePort {
  getScopedKnowledgeEntry(
    scope: Scope,
    entryId: string,
  ): Promise<SpokeResult<KnowledgeEntry>>;
  putScopedKnowledgeEntry(
    scope: Scope,
    entry: KnowledgeEntry,
    expectedBaseRevision: number | null,
  ): Promise<SpokeResult<KnowledgeEntry>>;
  listScopedKnowledgeEntries(
    scope: Scope,
  ): Promise<SpokeResult<KnowledgeEntry[]>>;
}

/**
 * Unscoped knowledge listing — the read side `check` / `assemble` (and the
 * remote `port.scope.list_knowledge_entries` client) rely on. Write-side
 * knowledge ops stay on `KnowledgeEntryPort`.
 */
export interface KnowledgeQueryPort {
  listKnowledgeEntries(scope: Scope): Promise<SpokeResult<KnowledgeEntry[]>>;
}

/** Finding persistence. */
export interface FindingPort {
  putFindings(findings: Finding[]): Promise<SpokeResult<Finding[]>>;
}

/** Rule query by reference list. */
export interface RuleQueryPort {
  listRules(ruleRefs: string[]): Promise<SpokeResult<Rule[]>>;
}

/**
 * Host collaboration metadata — self manifest and product-known peer manifests.
 * Integrators call explicitly; orchestrators do not auto-fetch manifests.
 */
export interface HostManifestPort {
  getHostCapabilityManifest(): Promise<SpokeResult<HostCapabilityManifest>>;
  listPeerHostCapabilityManifests(): Promise<
    SpokeResult<HostCapabilityManifest[]>
  >;
}

/** Optional l2-computable session — project / compute. */
export interface ComputablePort {
  project(request: ProjectRequest): Promise<SpokeResult<ProjectResponse>>;
  compute(request: ComputeRequest): Promise<SpokeResult<ComputeResponse>>;
}

/**
 * Optional l5-fork timeline query.
 * Capability-specific refinement of ScopeQueryPort; one object MAY satisfy both.
 */
export interface ForkTimelineQueryPort {
  listForkTimelineEvents(
    scope: Scope & { fork_id: ForkId },
  ): Promise<SpokeResult<TimelineEvent[]>>;
}

/** Ports required for spoke-baseline orchestration. */
export type BaselinePorts = KnowledgeEntryPort &
  KnowledgeQueryPort &
  RelationPort &
  ScopeQueryPort &
  FindingPort &
  RuleQueryPort &
  HostManifestPort;

/** Baseline plus optional computable capability. */
export type ComputablePorts = BaselinePorts & ComputablePort;

/** Baseline plus optional fork timeline capability. */
export type ForkPorts = BaselinePorts & ForkTimelineQueryPort;

/** Full composition of baseline, computable, and fork ports. */
export type FullPorts = BaselinePorts & ComputablePort & ForkTimelineQueryPort;

/** Ergonomic alias for baseline adapter composition. */
export type BaselineAdapter = BaselinePorts;

/** Ergonomic alias for baseline plus computable adapter composition. */
export type ComputableAdapter = ComputablePorts;

/** Ergonomic alias for baseline plus fork adapter composition. */
export type ForkAdapter = ForkPorts;

/** Ergonomic alias for full adapter composition. */
export type FullAdapter = FullPorts;

/**
 * Opaque JSON (`common.schema.json#/definitions/OpaqueJson`) — any JSON value
 * retained verbatim, never narrowed to an object map.
 */
export type OpaqueJson = unknown;

/**
 * Injected product source loader. The in-process value it returns may carry
 * source content; it is not a wire object and never appears on
 * `ExtractRequest` / `ExtractResponse`.
 */
export interface ExtractionPort {
  loadExtractionInput(request: ExtractRequest): Promise<SpokeResult<OpaqueJson>>;
}

/** Extraction callback input — the request plus the loaded product input. */
export type ExtractRunInput = { request: ExtractRequest; input: OpaqueJson };

/** Product extraction output — candidate entries plus advisory run metadata. */
export type ExtractionResult = {
  candidates: KnowledgeEntry[];
  method?: string;
  coverage_hint?: OpaqueJson;
};

/**
 * Injected product extraction callback. Awaited exactly once by
 * `orchestrateExtract`; deliberately async because an extraction service can
 * perform asynchronous work (no sync/async union).
 */
export type RunExtractor = (
  input: ExtractRunInput,
) => Promise<SpokeResult<ExtractionResult>>;

/**
 * Optional `spoke-memory` family — the injected product boundary for the
 * memory-system operations (§7 of the memory protocol).
 *
 * Standalone like `ExtractionPort`: `MemoryPort` is NOT part of
 * `BaselinePorts` / `FullPorts` or the adapter aliases. One method per op
 * (33); every method takes the authenticated `Principal` as an explicit
 * admission-context parameter — it is never payload-asserted (MEM-R003).
 * The library contains no memory storage or enforcement; providers
 * implement the port and own storage, transactions, and every enforcement
 * duty. Rust parity: `crates/spoke-operations/src/adapter/ports.rs`
 * `MemoryPort`.
 *
 * Common contract (§7.0): mutating ops require `idempotency_key` except
 * `ensureSubject`; dedup namespace is `(space_id, requester_principal_id,
 * op, idempotency_key)`; list/recall ops paginate by `cursor` + `limit`.
 * Grant capabilities below are the minimum per op — an `admin` grant with
 * sufficient coverage also satisfies capability requirements (§6.5).
 */
export interface MemoryPort {
  describe(
    principal: Principal,
    request: MemoryDescribeRequest,
  ): Promise<SpokeResult<MemoryDescribeResponse>>;
  listSpaces(
    principal: Principal,
    request: MemorySpacesListRequest,
  ): Promise<SpokeResult<MemorySpacesListResponse>>;
  createSpace(
    principal: Principal,
    request: MemorySpacesCreateRequest,
  ): Promise<SpokeResult<MemorySpacesCreateResponse>>;
  freezeSpace(
    principal: Principal,
    request: MemorySpacesFreezeRequest,
  ): Promise<SpokeResult<MemorySpacesFreezeResponse>>;
  unfreezeSpace(
    principal: Principal,
    request: MemorySpacesUnfreezeRequest,
  ): Promise<SpokeResult<MemorySpacesUnfreezeResponse>>;
  ensureSubject(
    principal: Principal,
    request: MemorySubjectsEnsureRequest,
  ): Promise<SpokeResult<MemorySubjectsEnsureResponse>>;
  getSubject(
    principal: Principal,
    request: MemorySubjectsGetRequest,
  ): Promise<SpokeResult<MemorySubjectsGetResponse>>;
  fenceSubject(
    principal: Principal,
    request: MemorySubjectsFenceRequest,
  ): Promise<SpokeResult<MemorySubjectsFenceResponse>>;
  listGrants(
    principal: Principal,
    request: MemoryGrantsListRequest,
  ): Promise<SpokeResult<MemoryGrantsListResponse>>;
  grant(
    principal: Principal,
    request: MemoryGrantsGrantRequest,
  ): Promise<SpokeResult<MemoryGrantsGrantResponse>>;
  revokeGrant(
    principal: Principal,
    request: MemoryGrantsRevokeRequest,
  ): Promise<SpokeResult<MemoryGrantsRevokeResponse>>;
  capture(
    principal: Principal,
    request: MemoryCaptureRequest,
  ): Promise<SpokeResult<MemoryCaptureResponse>>;
  listCandidates(
    principal: Principal,
    request: MemoryCandidatesListRequest,
  ): Promise<SpokeResult<MemoryCandidatesListResponse>>;
  getCandidate(
    principal: Principal,
    request: MemoryCandidatesGetRequest,
  ): Promise<SpokeResult<MemoryCandidatesGetResponse>>;
  dismissCandidate(
    principal: Principal,
    request: MemoryCandidatesDismissRequest,
  ): Promise<SpokeResult<MemoryCandidatesDismissResponse>>;
  reviewCandidate(
    principal: Principal,
    request: MemoryCandidatesReviewRequest,
  ): Promise<SpokeResult<MemoryCandidatesReviewResponse>>;
  createRecord(
    principal: Principal,
    request: MemoryRecordsCreateRequest,
  ): Promise<SpokeResult<MemoryRecordsCreateResponse>>;
  getRecord(
    principal: Principal,
    request: MemoryRecordsGetRequest,
  ): Promise<SpokeResult<MemoryRecordsGetResponse>>;
  updateRecord(
    principal: Principal,
    request: MemoryRecordsUpdateRequest,
  ): Promise<SpokeResult<MemoryRecordsUpdateResponse>>;
  deleteRecord(
    principal: Principal,
    request: MemoryRecordsDeleteRequest,
  ): Promise<SpokeResult<MemoryRecordsDeleteResponse>>;
  listRecords(
    principal: Principal,
    request: MemoryRecordsListRequest,
  ): Promise<SpokeResult<MemoryRecordsListResponse>>;
  promoteRecord(
    principal: Principal,
    request: MemoryRecordsPromoteRequest,
  ): Promise<SpokeResult<MemoryRecordsPromoteResponse>>;
  recall(
    principal: Principal,
    request: MemoryRecallRequest,
  ): Promise<SpokeResult<MemoryRecallResponse>>;
  getProfile(
    principal: Principal,
    request: MemoryProfileGetRequest,
  ): Promise<SpokeResult<MemoryProfileGetResponse>>;
  updateProfile(
    principal: Principal,
    request: MemoryProfileUpdateRequest,
  ): Promise<SpokeResult<MemoryProfileUpdateResponse>>;
  getReflectionState(
    principal: Principal,
    request: MemoryReflectionsStateRequest,
  ): Promise<SpokeResult<MemoryReflectionsStateResponse>>;
  listReflections(
    principal: Principal,
    request: MemoryReflectionsListRequest,
  ): Promise<SpokeResult<MemoryReflectionsListResponse>>;
  generateReflection(
    principal: Principal,
    request: MemoryReflectionsGenerateRequest,
  ): Promise<SpokeResult<MemoryReflectionsGenerateResponse>>;
  getReceipt(
    principal: Principal,
    request: MemoryReceiptsGetRequest,
  ): Promise<SpokeResult<MemoryReceiptsGetResponse>>;
  createExport(
    principal: Principal,
    request: MemoryExportsCreateRequest,
  ): Promise<SpokeResult<MemoryExportsCreateResponse>>;
  getExport(
    principal: Principal,
    request: MemoryExportsGetRequest,
  ): Promise<SpokeResult<MemoryExportsGetResponse>>;
  stageImport(
    principal: Principal,
    request: MemoryImportsStageRequest,
  ): Promise<SpokeResult<MemoryImportsStageResponse>>;
  activateImport(
    principal: Principal,
    request: MemoryImportsActivateRequest,
  ): Promise<SpokeResult<MemoryImportsActivateResponse>>;
  discardImport(
    principal: Principal,
    request: MemoryImportsDiscardRequest,
  ): Promise<SpokeResult<MemoryImportsDiscardResponse>>;
}

/**
 * Availability probe for the standalone `spoke-memory` family, mirroring
 * `MemoryPorts` in Rust: adapters that serve memory expose `asMemory`
 * returning themselves; dynamically assembled boundaries omit it and the
 * probe reads as `undefined` (parity with `requirePortMethod` miss).
 */
export interface MemoryPorts {
  asMemory?: () => MemoryPort | undefined;
}

/**
 * Optional `spoke-history` family — the injected product boundary for
 * revision replay and explicit history pruning (spoke-history contract
 * §Standalone `HistoryPort`).
 *
 * Standalone like `ExtractionPort`: `HistoryPort` is NOT part of
 * `BaselinePorts` / `FullPorts` or the adapter aliases, and neither
 * `KnowledgeEntryPort` nor `RelationPort` inherits it. The library contains
 * no retention engine; providers own checkpoints, reconstruction, and every
 * refusal: a kind it does not serve answers `HISTORY_ENTITY_UNSUPPORTED`, a
 * revision below the retained floor answers `HISTORY_TRUNCATED`, and a
 * revision at or above the floor that cannot be reconstructed answers
 * `HISTORY_NOT_RETAINED`. A missing or non-disclosable entity keeps the
 * existing `NOT_FOUND` behavior, and `pruneHistory` authorization is
 * exactly the entity's write permission — there is no history-specific
 * grant or refusal code.
 *
 * Both methods take the authenticated `Principal` first and the required
 * effective `Scope` second (spoke-history contract §HistoryPort, mirroring
 * `MemoryPort` and MEM-R003): the Connect serving face resolves peer
 * admission and the optional wire `scope` into the admitted scope before
 * the call, so the port binds entity read/write authorization to the
 * caller rather than to payload-asserted identity. The connect
 * idempotency envelope is a connect-layer concern — `idempotency_key` is
 * deliberately not a port argument. Rust parity:
 * `crates/spoke-operations/src/adapter/ports.rs` `HistoryPort`.
 */
export interface HistoryPort {
  getHistorySnapshot(
    principal: Principal,
    scope: Scope,
    entityKind: string,
    entityId: string,
    revision: number,
  ): Promise<SpokeResult<RevisionSnapshot>>;
  pruneHistory(
    principal: Principal,
    scope: Scope,
    entityKind: string,
    entityId: string,
    keep: number,
  ): Promise<SpokeResult<HistoryPruneResult>>;
}

/**
 * Availability probe for the standalone `spoke-history` family, mirroring
 * `MemoryPorts`: adapters that serve history expose `asHistory` returning
 * themselves; dynamically assembled boundaries omit it and the probe reads
 * as `undefined` (parity with `requirePortMethod` miss).
 */
export interface HistoryPorts {
  asHistory?: () => HistoryPort | undefined;
}

/**
 * Optional `spoke-knowledge-adoption` family — the injected product
 * boundary for adopting a shared `KnowledgeEntry` into a destination
 * scope (spoke-knowledge-adoption contract §Standalone `AdoptionPort`).
 *
 * Standalone like `HistoryPort` and `MemoryPort`: `AdoptionPort` is NOT
 * part of `BaselinePorts` / `FullPorts` or the adapter aliases and
 * neither port inherits the other — `spoke-baseline` does not implicitly
 * provide adoption. The library contains no adoption engine; providers
 * own source visibility (a missing, out-of-scope, or owner-private source
 * answers one generic `NOT_FOUND`), retirement-chain resolution (≤32
 * edges; cycle / ambiguous / deleted terminal answers `SOURCE_RETIRED`
 * with `details.retirement_chain`), the copy rules, create-only OCC, the
 * anchor repeat rules (`already_present` at the same source revision,
 * `REVISION_CONFLICT` with `details.adopted_revision` at a different
 * one), and the destination write's atomicity.
 *
 * The method takes the authenticated `Principal` first and the admitted
 * `Scope` second (MEM-R003 parity with `HistoryPort`): the Connect
 * serving face resolves peer admission before the call, and the admitted
 * scope covers source visibility and destination authorization — the
 * wire request carries scope ids, never payload-asserted identity. The
 * connect idempotency envelope is a connect-layer concern —
 * `idempotency_key` is deliberately not a port argument. Rust parity:
 * `crates/spoke-operations/src/adapter/ports.rs` `AdoptionPort`.
 */
export interface AdoptionPort {
  adoptKnowledgeEntry(
    principal: Principal,
    scope: Scope,
    request: KnowledgeAdoptRequest,
  ): Promise<SpokeResult<KnowledgeAdoptResponse>>;
}

/**
 * Availability probe for the standalone `spoke-knowledge-adoption`
 * family, mirroring `HistoryPorts`: adapters that serve adoption expose
 * `asAdoption` returning themselves; dynamically assembled boundaries
 * omit it and the probe reads as `undefined` (parity with
 * `requirePortMethod` miss).
 */
export interface AdoptionPorts {
  asAdoption?: () => AdoptionPort | undefined;
}
