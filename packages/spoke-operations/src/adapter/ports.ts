/**
 * note: Capability-sliced adapter port contracts for injection orchestration.
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
  /**
   * note: Persist a KnowledgeEntry with optimistic concurrency control.
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
   * note: Persist a Relation with optimistic concurrency control.
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
 * note: Optional `ke-extraction` family — the injected product boundary for
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
  /** note: §7.1 `memory.describe` — provider self-description. **Grant:** none; authenticated principal only. Read-only. */
  describe(
    principal: Principal,
    request: MemoryDescribeRequest,
  ): Promise<SpokeResult<MemoryDescribeResponse>>;
  /** note: §7.2 `memory.spaces.list` — visible spaces; drain-barrier read. **Grant:** any valid grant in the space, or provider-configured visibility. Read-only. */
  listSpaces(
    principal: Principal,
    request: MemorySpacesListRequest,
  ): Promise<SpokeResult<MemorySpacesListResponse>>;
  /**
   * note: `memory.spaces.create` — create a space (a Mind) with a caller-named
   * `space_id`; establishes no subject. **Grant:** none for a new id
   * (authenticated principal only, MEM-R003); on an existing space the
   * requester MUST hold `admin` or the call fails `NOT_AUTHORIZED` — never
   * `NOT_FOUND`, so the op is not an existence oracle. Idempotency key
   * required (§7.0.5) over `(space_id, requester_principal_id, op,
   * idempotency_key)`: a replay returns the stored receipt, and a repeat
   * create by an admin answers `created: false` without writing. The adapter
   * MUST write the space row and the creator's default grant row in one
   * transaction.
   */
  createSpace(
    principal: Principal,
    request: MemorySpacesCreateRequest,
  ): Promise<SpokeResult<MemorySpacesCreateResponse>>;
  /** note: §7.3 `memory.spaces.freeze`. **Grant:** `admin`. Idempotency key required. */
  freezeSpace(
    principal: Principal,
    request: MemorySpacesFreezeRequest,
  ): Promise<SpokeResult<MemorySpacesFreezeResponse>>;
  /** note: §7.4 `memory.spaces.unfreeze` — always allowed on a frozen space. **Grant:** `admin`. */
  unfreezeSpace(
    principal: Principal,
    request: MemorySpacesUnfreezeRequest,
  ): Promise<SpokeResult<MemorySpacesUnfreezeResponse>>;
  /** note: §7.5 `memory.subjects.ensure` — idempotent get-or-create; sole `idempotency_key` exemption (§7.0.5). **Grant:** `capture` (admin with coverage also satisfies). */
  ensureSubject(
    principal: Principal,
    request: MemorySubjectsEnsureRequest,
  ): Promise<SpokeResult<MemorySubjectsEnsureResponse>>;
  /** note: §7.6 `memory.subjects.get`. **Grant:** any capability on the subject. Read-only. */
  getSubject(
    principal: Principal,
    request: MemorySubjectsGetRequest,
  ): Promise<SpokeResult<MemorySubjectsGetResponse>>;
  /** note: §7.7 `memory.subjects.fence` — subject/context lifecycle fencing. **Grant:** `admin`. */
  fenceSubject(
    principal: Principal,
    request: MemorySubjectsFenceRequest,
  ): Promise<SpokeResult<MemorySubjectsFenceResponse>>;
  /** note: §7.8 `memory.grants.list`. **Grant:** `admin` for full/filtered views; self-view limited to the caller's own grants. Read-only. */
  listGrants(
    principal: Principal,
    request: MemoryGrantsListRequest,
  ): Promise<SpokeResult<MemoryGrantsListResponse>>;
  /** note: §7.9 `memory.grants.grant`. **Grant:** `admin`; the issued grant must be non-amplifying relative to the granter's coverage (§6.5). */
  grant(
    principal: Principal,
    request: MemoryGrantsGrantRequest,
  ): Promise<SpokeResult<MemoryGrantsGrantResponse>>;
  /** note: §7.10 `memory.grants.revoke` — allowed during freeze (§8.2 allowlist). **Grant:** `admin`. */
  revokeGrant(
    principal: Principal,
    request: MemoryGrantsRevokeRequest,
  ): Promise<SpokeResult<MemoryGrantsRevokeResponse>>;
  /** note: §7.11 `memory.capture` — never writes retained records (MEM-R012). **Grant:** `capture` covering the declared `capture_scope`. */
  capture(
    principal: Principal,
    request: MemoryCaptureRequest,
  ): Promise<SpokeResult<MemoryCaptureResponse>>;
  /** note: §7.12 `memory.candidates.list`. **Grant:** `review` covering `capture_scope`. Read-only. */
  listCandidates(
    principal: Principal,
    request: MemoryCandidatesListRequest,
  ): Promise<SpokeResult<MemoryCandidatesListResponse>>;
  /** note: §7.13 `memory.candidates.get`. **Grant:** `review` covering the candidate's `capture_scope`. Read-only. */
  getCandidate(
    principal: Principal,
    request: MemoryCandidatesGetRequest,
  ): Promise<SpokeResult<MemoryCandidatesGetResponse>>;
  /** note: §7.14 `memory.candidates.dismiss` — consumption without a review decision. **Grant:** `review` covering `capture_scope`. */
  dismissCandidate(
    principal: Principal,
    request: MemoryCandidatesDismissRequest,
  ): Promise<SpokeResult<MemoryCandidatesDismissResponse>>;
  /** note: §7.15 `memory.candidates.review` — the only candidate → record path. **Grant:** `review` + `records_write`, both covering `capture_scope`. */
  reviewCandidate(
    principal: Principal,
    request: MemoryCandidatesReviewRequest,
  ): Promise<SpokeResult<MemoryCandidatesReviewResponse>>;
  /** note: §7.16 `memory.records.create` — direct authored write. **Grant:** `records_write` covering the declared placement. */
  createRecord(
    principal: Principal,
    request: MemoryRecordsCreateRequest,
  ): Promise<SpokeResult<MemoryRecordsCreateResponse>>;
  /** note: §7.17 `memory.records.get`. **Grant:** `records_read` or `recall` covering placement; tombstoned reads require `admin`. Read-only. */
  getRecord(
    principal: Principal,
    request: MemoryRecordsGetRequest,
  ): Promise<SpokeResult<MemoryRecordsGetResponse>>;
  /** note: §7.18 `memory.records.update` — placement changes are NOT updates (see `promoteRecord`). **Grant:** `records_write` covering the current placement. */
  updateRecord(
    principal: Principal,
    request: MemoryRecordsUpdateRequest,
  ): Promise<SpokeResult<MemoryRecordsUpdateResponse>>;
  /** note: §7.19 `memory.records.delete` — tombstone retained, receipts unaffected. **Grant:** `records_write` covering the placement. */
  deleteRecord(
    principal: Principal,
    request: MemoryRecordsDeleteRequest,
  ): Promise<SpokeResult<MemoryRecordsDeleteResponse>>;
  /** note: §7.20 `memory.records.list`. **Grant:** `records_read` or `recall` covering the selection/filter; `include_tombstoned` requires `admin`. Read-only. */
  listRecords(
    principal: Principal,
    request: MemoryRecordsListRequest,
  ): Promise<SpokeResult<MemoryRecordsListResponse>>;
  /** note: §7.21 `memory.records.promote` — explicit placement change. **Grant:** `promote` + `records_write`, both covering source and target placements. */
  promoteRecord(
    principal: Principal,
    request: MemoryRecordsPromoteRequest,
  ): Promise<SpokeResult<MemoryRecordsPromoteResponse>>;
  /** note: §7.22 `memory.recall` — authorized, ranked, budgeted read. **Grant:** `recall` covering the selection/filter. Read-only. */
  recall(
    principal: Principal,
    request: MemoryRecallRequest,
  ): Promise<SpokeResult<MemoryRecallResponse>>;
  /** note: §7.23 `memory.profile.get` — absent kinds are honest-empty (§4.2). **Grant:** any capability on the subject with shared access or unrestricted coverage. Read-only. */
  getProfile(
    principal: Principal,
    request: MemoryProfileGetRequest,
  ): Promise<SpokeResult<MemoryProfileGetResponse>>;
  /** note: §7.24 `memory.profile.update`. **Grant:** `profile_write`. */
  updateProfile(
    principal: Principal,
    request: MemoryProfileUpdateRequest,
  ): Promise<SpokeResult<MemoryProfileUpdateResponse>>;
  /** note: §7.25 `memory.reflections.state` — capability `spoke-memory-reflection`. **Grant:** `reflect` covering the selection/filter. Read-only. */
  getReflectionState(
    principal: Principal,
    request: MemoryReflectionsStateRequest,
  ): Promise<SpokeResult<MemoryReflectionsStateResponse>>;
  /** note: §7.26 `memory.reflections.list` — capability `spoke-memory-reflection`. **Grant:** `reflect` covering the selection/filter. Read-only. */
  listReflections(
    principal: Principal,
    request: MemoryReflectionsListRequest,
  ): Promise<SpokeResult<MemoryReflectionsListResponse>>;
  /** note: §7.27 `memory.reflections.generate` — explicit cost-bearing generation; may complete asynchronously (§7.0.3). Capability `spoke-memory-reflection`. **Grant:** `reflect` covering the selection/filter. */
  generateReflection(
    principal: Principal,
    request: MemoryReflectionsGenerateRequest,
  ): Promise<SpokeResult<MemoryReflectionsGenerateResponse>>;
  /** note: §7.28 `memory.receipts.get` — sole uncertainty-resolution and pending-polling channel. **Grant:** current authorization as the receipt's `requester_principal_id`, or `admin` in the space. Read-only. */
  getReceipt(
    principal: Principal,
    request: MemoryReceiptsGetRequest,
  ): Promise<SpokeResult<MemoryReceiptsGetResponse>>;
  /** note: §7.29 `memory.exports.create` — capability `spoke-memory-transfer`; never fails `SPACE_FROZEN` (§8.2 allowlist). **Grant:** `transfer` covering the export scope; `include_tombstoned` requires `admin`. */
  createExport(
    principal: Principal,
    request: MemoryExportsCreateRequest,
  ): Promise<SpokeResult<MemoryExportsCreateResponse>>;
  /** note: §7.30 `memory.exports.get` — capability `spoke-memory-transfer`. **Grant:** `transfer` covering the export scope; `include_tombstoned` requires `admin`. Read-only. */
  getExport(
    principal: Principal,
    request: MemoryExportsGetRequest,
  ): Promise<SpokeResult<MemoryExportsGetResponse>>;
  /** note: §7.31 `memory.imports.stage` — per-chunk staging, strict ordinal order. Capability `spoke-memory-transfer`. **Grant:** `transfer` + `admin` on the target space. */
  stageImport(
    principal: Principal,
    request: MemoryImportsStageRequest,
  ): Promise<SpokeResult<MemoryImportsStageResponse>>;
  /** note: §7.32 `memory.imports.activate` — atomic visibility flip. Capability `spoke-memory-transfer`. **Grant:** `transfer` + `admin`. */
  activateImport(
    principal: Principal,
    request: MemoryImportsActivateRequest,
  ): Promise<SpokeResult<MemoryImportsActivateResponse>>;
  /** note: §7.33 `memory.imports.discard` — pre-activation rollback. Capability `spoke-memory-transfer`. **Grant:** `transfer` + `admin`. */
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
  /** note: `history.revisions.get` — replay the stored entity at exactly `revision`. Read-only. */
  getHistorySnapshot(
    principal: Principal,
    scope: Scope,
    entityKind: string,
    entityId: string,
    revision: number,
  ): Promise<SpokeResult<RevisionSnapshot>>;
  /**
   * note: `history.revisions.prune` — retain the newest `keep` checkpoints
   * of one `(scope, entity_kind, entity_id)` target and delete every older
   * checkpoint. The live row is not a checkpoint and never consumes `keep`;
   * no write path performs this pruning implicitly. Prune authorization is
   * the selected entity's write permission in `scope`.
   */
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
  /**
   * note: `knowledge.adopt` — adopt a shared `KnowledgeEntry` into the
   * destination scope, writing `adoption_anchor` with the resolved
   * terminal source scope/id/revision. Mutating op.
   */
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
