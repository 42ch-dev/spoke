/**
 * Capability-sliced adapter port contracts for injection orchestration.
 * Async SpokeResult surface — port methods return awaitable results; the
 * library stays I/O-free and only awaits injected ports.
 */

import type {
  ComputeRequest,
  ComputeResponse,
  ExtractRequest,
  Finding,
  ForkId,
  HostCapabilityManifest,
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
  Rule,
  Scope,
  TimelineEvent,
} from "@42ch/spoke-schemas";

import type { SpokeResult } from "../result.js";

/** Knowledge entry persistence — get / put by entry id. */
export interface KnowledgeEntryPort {
  getKnowledgeEntry(
    entryId: string,
  ): Promise<SpokeResult<KnowledgeEntry>>;
  /**
   * Persist a KnowledgeEntry with optimistic concurrency control.
   *
   * Adapters MUST treat `expectedBaseRevision` as the store’s required current
   * revision before accepting the write (conditional put / OCC / CAS).
   * `null` means the entry must be absent (create). A non-null value means the
   * store’s current revision for `entry.entry_id` MUST equal
   * `expectedBaseRevision`; otherwise reject with `STORED_REVISION_STALE` or
   * `REVISION_CONFLICT`. True concurrent safety requires atomic compare-and-put
   * in the adapter; the library stays I/O-free.
   */
  putKnowledgeEntry(
    entry: KnowledgeEntry,
    expectedBaseRevision: number | null,
  ): Promise<SpokeResult<KnowledgeEntry>>;
}

/** Relation persistence — get / put by relation id. */
export interface RelationPort {
  getRelation(relationId: string): Promise<SpokeResult<Relation>>;
  /**
   * Persist a Relation with optimistic concurrency control.
   *
   * Adapters MUST treat `expectedBaseRevision` as the store’s required current
   * revision before accepting the write (conditional put / OCC / CAS).
   * `null` means the relation must be absent (create). A non-null value means the
   * store’s current revision for `relation.relation_id` MUST equal
   * `expectedBaseRevision`; otherwise reject with `STORED_REVISION_STALE` or
   * `REVISION_CONFLICT`. True concurrent safety requires atomic compare-and-put
   * in the adapter; the library stays I/O-free.
   *
   * Revision assignment is adapter-owned: on create (`expectedBaseRevision`
   * `null`) the adapter MUST seed `revision = 1`; on an accepted update it MUST
   * persist `revision = stored + 1`. The returned Relation carries the assigned
   * revision — callers must not set it themselves.
   */
  putRelation(
    relation: Relation,
    expectedBaseRevision: number | null,
  ): Promise<SpokeResult<Relation>>;
}

/** Scope query for check / assemble — knowledge entries and timeline events. */
export interface ScopeQueryPort {
  listKnowledgeEntries(
    scope: Scope,
  ): Promise<SpokeResult<KnowledgeEntry[]>>;
  listTimelineEvents(scope: Scope): Promise<SpokeResult<TimelineEvent[]>>;
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
 * Optional `ke-extraction` family — the injected product boundary for
 * referenced-content extraction.
 *
 * Standalone like `ToolInvokePort`: `ExtractionPort` is NOT part of
 * `BaselinePorts` / `FullPorts` or the adapter aliases. The library contains no
 * source I/O and no extraction engine; it only awaits the injected port and
 * callback, then validates and assembles.
 */

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
  /** §7.1 `memory.describe` — provider self-description. **Grant:** none; authenticated principal only. Read-only. */
  describe(
    principal: Principal,
    request: MemoryDescribeRequest,
  ): Promise<SpokeResult<MemoryDescribeResponse>>;
  /** §7.2 `memory.spaces.list` — visible spaces; drain-barrier read. **Grant:** any valid grant in the space, or provider-configured visibility. Read-only. */
  listSpaces(
    principal: Principal,
    request: MemorySpacesListRequest,
  ): Promise<SpokeResult<MemorySpacesListResponse>>;
  /** §7.3 `memory.spaces.freeze`. **Grant:** `admin`. Idempotency key required. */
  freezeSpace(
    principal: Principal,
    request: MemorySpacesFreezeRequest,
  ): Promise<SpokeResult<MemorySpacesFreezeResponse>>;
  /** §7.4 `memory.spaces.unfreeze` — always allowed on a frozen space. **Grant:** `admin`. */
  unfreezeSpace(
    principal: Principal,
    request: MemorySpacesUnfreezeRequest,
  ): Promise<SpokeResult<MemorySpacesUnfreezeResponse>>;
  /** §7.5 `memory.subjects.ensure` — idempotent get-or-create; sole `idempotency_key` exemption (§7.0.5). **Grant:** `capture` (admin with coverage also satisfies). */
  ensureSubject(
    principal: Principal,
    request: MemorySubjectsEnsureRequest,
  ): Promise<SpokeResult<MemorySubjectsEnsureResponse>>;
  /** §7.6 `memory.subjects.get`. **Grant:** any capability on the subject. Read-only. */
  getSubject(
    principal: Principal,
    request: MemorySubjectsGetRequest,
  ): Promise<SpokeResult<MemorySubjectsGetResponse>>;
  /** §7.7 `memory.subjects.fence` — subject/context lifecycle fencing. **Grant:** `admin`. */
  fenceSubject(
    principal: Principal,
    request: MemorySubjectsFenceRequest,
  ): Promise<SpokeResult<MemorySubjectsFenceResponse>>;
  /** §7.8 `memory.grants.list`. **Grant:** `admin` for full/filtered views; self-view limited to the caller's own grants. Read-only. */
  listGrants(
    principal: Principal,
    request: MemoryGrantsListRequest,
  ): Promise<SpokeResult<MemoryGrantsListResponse>>;
  /** §7.9 `memory.grants.grant`. **Grant:** `admin`; the issued grant must be non-amplifying relative to the granter's coverage (§6.5). */
  grant(
    principal: Principal,
    request: MemoryGrantsGrantRequest,
  ): Promise<SpokeResult<MemoryGrantsGrantResponse>>;
  /** §7.10 `memory.grants.revoke` — allowed during freeze (§8.2 allowlist). **Grant:** `admin`. */
  revokeGrant(
    principal: Principal,
    request: MemoryGrantsRevokeRequest,
  ): Promise<SpokeResult<MemoryGrantsRevokeResponse>>;
  /** §7.11 `memory.capture` — never writes retained records (MEM-R012). **Grant:** `capture` covering the declared `capture_scope`. */
  capture(
    principal: Principal,
    request: MemoryCaptureRequest,
  ): Promise<SpokeResult<MemoryCaptureResponse>>;
  /** §7.12 `memory.candidates.list`. **Grant:** `review` covering `capture_scope`. Read-only. */
  listCandidates(
    principal: Principal,
    request: MemoryCandidatesListRequest,
  ): Promise<SpokeResult<MemoryCandidatesListResponse>>;
  /** §7.13 `memory.candidates.get`. **Grant:** `review` covering the candidate's `capture_scope`. Read-only. */
  getCandidate(
    principal: Principal,
    request: MemoryCandidatesGetRequest,
  ): Promise<SpokeResult<MemoryCandidatesGetResponse>>;
  /** §7.14 `memory.candidates.dismiss` — consumption without a review decision. **Grant:** `review` covering `capture_scope`. */
  dismissCandidate(
    principal: Principal,
    request: MemoryCandidatesDismissRequest,
  ): Promise<SpokeResult<MemoryCandidatesDismissResponse>>;
  /** §7.15 `memory.candidates.review` — the only candidate → record path. **Grant:** `review` + `records_write`, both covering `capture_scope`. */
  reviewCandidate(
    principal: Principal,
    request: MemoryCandidatesReviewRequest,
  ): Promise<SpokeResult<MemoryCandidatesReviewResponse>>;
  /** §7.16 `memory.records.create` — direct authored write. **Grant:** `records_write` covering the declared placement. */
  createRecord(
    principal: Principal,
    request: MemoryRecordsCreateRequest,
  ): Promise<SpokeResult<MemoryRecordsCreateResponse>>;
  /** §7.17 `memory.records.get`. **Grant:** `records_read` or `recall` covering placement; tombstoned reads require `admin`. Read-only. */
  getRecord(
    principal: Principal,
    request: MemoryRecordsGetRequest,
  ): Promise<SpokeResult<MemoryRecordsGetResponse>>;
  /** §7.18 `memory.records.update` — placement changes are NOT updates (see `promoteRecord`). **Grant:** `records_write` covering the current placement. */
  updateRecord(
    principal: Principal,
    request: MemoryRecordsUpdateRequest,
  ): Promise<SpokeResult<MemoryRecordsUpdateResponse>>;
  /** §7.19 `memory.records.delete` — tombstone retained, receipts unaffected. **Grant:** `records_write` covering the placement. */
  deleteRecord(
    principal: Principal,
    request: MemoryRecordsDeleteRequest,
  ): Promise<SpokeResult<MemoryRecordsDeleteResponse>>;
  /** §7.20 `memory.records.list`. **Grant:** `records_read` or `recall` covering the selection/filter; `include_tombstoned` requires `admin`. Read-only. */
  listRecords(
    principal: Principal,
    request: MemoryRecordsListRequest,
  ): Promise<SpokeResult<MemoryRecordsListResponse>>;
  /** §7.21 `memory.records.promote` — explicit placement change. **Grant:** `promote` + `records_write`, both covering source and target placements. */
  promoteRecord(
    principal: Principal,
    request: MemoryRecordsPromoteRequest,
  ): Promise<SpokeResult<MemoryRecordsPromoteResponse>>;
  /** §7.22 `memory.recall` — authorized, ranked, budgeted read. **Grant:** `recall` covering the selection/filter. Read-only. */
  recall(
    principal: Principal,
    request: MemoryRecallRequest,
  ): Promise<SpokeResult<MemoryRecallResponse>>;
  /** §7.23 `memory.profile.get` — absent kinds are honest-empty (§4.2). **Grant:** any capability on the subject with shared access or unrestricted coverage. Read-only. */
  getProfile(
    principal: Principal,
    request: MemoryProfileGetRequest,
  ): Promise<SpokeResult<MemoryProfileGetResponse>>;
  /** §7.24 `memory.profile.update`. **Grant:** `profile_write`. */
  updateProfile(
    principal: Principal,
    request: MemoryProfileUpdateRequest,
  ): Promise<SpokeResult<MemoryProfileUpdateResponse>>;
  /** §7.25 `memory.reflections.state` — capability `spoke-memory-reflection`. **Grant:** `reflect` covering the selection/filter. Read-only. */
  getReflectionState(
    principal: Principal,
    request: MemoryReflectionsStateRequest,
  ): Promise<SpokeResult<MemoryReflectionsStateResponse>>;
  /** §7.26 `memory.reflections.list` — capability `spoke-memory-reflection`. **Grant:** `reflect` covering the selection/filter. Read-only. */
  listReflections(
    principal: Principal,
    request: MemoryReflectionsListRequest,
  ): Promise<SpokeResult<MemoryReflectionsListResponse>>;
  /** §7.27 `memory.reflections.generate` — explicit cost-bearing generation; may complete asynchronously (§7.0.3). Capability `spoke-memory-reflection`. **Grant:** `reflect` covering the selection/filter. */
  generateReflection(
    principal: Principal,
    request: MemoryReflectionsGenerateRequest,
  ): Promise<SpokeResult<MemoryReflectionsGenerateResponse>>;
  /** §7.28 `memory.receipts.get` — sole uncertainty-resolution and pending-polling channel. **Grant:** current authorization as the receipt's `requester_principal_id`, or `admin` in the space. Read-only. */
  getReceipt(
    principal: Principal,
    request: MemoryReceiptsGetRequest,
  ): Promise<SpokeResult<MemoryReceiptsGetResponse>>;
  /** §7.29 `memory.exports.create` — capability `spoke-memory-transfer`; never fails `SPACE_FROZEN` (§8.2 allowlist). **Grant:** `transfer` covering the export scope; `include_tombstoned` requires `admin`. */
  createExport(
    principal: Principal,
    request: MemoryExportsCreateRequest,
  ): Promise<SpokeResult<MemoryExportsCreateResponse>>;
  /** §7.30 `memory.exports.get` — capability `spoke-memory-transfer`. **Grant:** `transfer` covering the export scope; `include_tombstoned` requires `admin`. Read-only. */
  getExport(
    principal: Principal,
    request: MemoryExportsGetRequest,
  ): Promise<SpokeResult<MemoryExportsGetResponse>>;
  /** §7.31 `memory.imports.stage` — per-chunk staging, strict ordinal order. Capability `spoke-memory-transfer`. **Grant:** `transfer` + `admin` on the target space. */
  stageImport(
    principal: Principal,
    request: MemoryImportsStageRequest,
  ): Promise<SpokeResult<MemoryImportsStageResponse>>;
  /** §7.32 `memory.imports.activate` — atomic visibility flip. Capability `spoke-memory-transfer`. **Grant:** `transfer` + `admin`. */
  activateImport(
    principal: Principal,
    request: MemoryImportsActivateRequest,
  ): Promise<SpokeResult<MemoryImportsActivateResponse>>;
  /** §7.33 `memory.imports.discard` — pre-activation rollback. Capability `spoke-memory-transfer`. **Grant:** `transfer` + `admin`. */
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
