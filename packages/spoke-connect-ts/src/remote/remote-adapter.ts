/**
 * `RemoteAdapter` — drop-in async `BaselinePorts` over a connect session
 * (frozen contract: `.agents/specs/adapters/spoke-remote-adapter.md`).
 *
 * PUBLIC surface: the async `BaselinePorts` (six families) + the optional
 * `l2-computable` / `l5-fork` port methods (`project` / `compute` /
 * `listForkTimelineEvents`) + the `connectRemoteAdapter` dial entrypoint +
 * read-only session info (`sessionId`, `remotePeerId`, `remoteManifest`,
 * `state`) + `close`.
 *
 * INTERNAL (encapsulated — consumers never touch these): hello sign/verify,
 * allowlist, nonce single-use, sequence allocate/advance, `request_id`
 * correlation, the dispatch gate, optional capability-token attach, the
 * receive-loop demux, and invoke timeout timers. All of it reuses the pure
 * session-core (`src/core/`) — nothing is reimplemented here.
 *
 * Each `BaselinePorts` method maps to a reserved `port.*` product op with an
 * opaque JSON payload (frozen contract §5.2), sent as a `ConnectInvokeRequest`
 * over the `Transport`, awaited via the adapter-owned receive loop, and
 * deserialized back to `SpokeResult`. The WebSocket implementation is
 * consumer-side; only the loopback `Transport` ships in-repo.
 */

import type {
  ComputeRequest,
  ComputeResponse,
  ConnectInvokeRequest,
  ConnectInvokeResponse,
  ErrorEnvelope,
  ExtractRequest,
  ExtractResponse,
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
import {
  fromErrorEnvelope,
  isStrictTimestamp,
  parseToolCapabilityId,
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  toErrorEnvelope,
  type AdoptionPort,
  type AdoptionPorts,
  type BaselinePorts,
  type HistoryPort,
  type HistoryPruneResult,
  type MemoryPort,
  type SpokeReject,
  type SpokeResult,
} from "@42ch/spoke-operations";

import { getPublicKeyEd25519 } from "../crypto.js";
import { isAllowlisted } from "../core/allowlist.js";
import type { CapabilityTokenProof } from "../core/capability-token.js";
import {
  checkResponseCorrelation,
  correlationFromRequest,
  correlationFromResponse,
  type Correlation,
} from "../core/correlate.js";
import {
  authenticateInvokeRequest,
  authenticateInvokeResponse,
  authenticateSession,
  EnvelopeAuthError,
  verifyInvokeRequestAuth,
  verifyInvokeResponseAuth,
  verifySessionAuth,
  type InvokeRequestSignInput,
} from "../core/envelope-auth.js";
import {
  CAPABILITY_SPOKE_KNOWLEDGE_SCOPED,
  WIRE_INVALID_INPUT,
} from "../core/dispatch.js";
import { CoreError } from "../core/error.js";
import { signHelloEd25519, verifyHelloEd25519 } from "../core/hello.js";
import { generateNonce, NonceStore } from "../core/nonce.js";
import { negotiatedCapabilities, Session } from "../core/session.js";
import { decodeJsonMessage, encodeJsonMessage } from "../framing.js";
import { derivePeerIdFromEd25519Pubkey } from "../identity.js";
import {
  isConnectHello,
  isConnectInvokeRequest,
  isConnectInvokeResponse,
  isConnectSession,
} from "./guards.js";
import {
  carriesAdoptFailureMember,
  isAdoptFailurePayload,
  isValidAdoptSuccessPayload,
  isValidRevisionSnapshot,
  isValidSuccessPayload,
} from "./payload.js";
import type { EnvelopeBytes, Transport } from "./transport.js";

const DEFAULT_INVOKE_TIMEOUT_MS = 5000;

let acceptedServerHellos = new NonceStore();

/**
 * @internal test-only — not part of the RemoteAdapter public API.
 */
export function __resetAcceptedServerHellosForTest(): void {
  acceptedServerHellos = new NonceStore();
}

const textEncoder = new TextEncoder();

function encodeEnvelope(doc: unknown): EnvelopeBytes {
  return textEncoder.encode(encodeJsonMessage(doc));
}

function withTimeout<T>(promise: Promise<T>, ms: number, what: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error(`connect: ${what} timed out after ${ms}ms`)),
      ms,
    );
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
}

type RemoteErrorKind =
  | "transport"
  | "session_closed"
  | "timeout"
  | "correlation_mismatch"
  | "sequence_exhausted"
  // Envelope-auth rejections (envelope-auth contract §8): the locked
  // `details.kind` vocabulary, surfaced via `INTERNAL_ERROR` SpokeRejects.
  | "envelope_auth_missing"
  | "envelope_auth_invalid"
  | "envelope_auth_session_unbound";

class RemoteError extends Error {
  readonly kind: RemoteErrorKind;

  constructor(kind: RemoteErrorKind, message: string) {
    super(message);
    this.name = "RemoteError";
    this.kind = kind;
  }
}

function internalError(kind: RemoteErrorKind, message: string): SpokeReject {
  return spokeReject(SpokeRejectCode.INTERNAL_ERROR, message, { kind });
}

/**
 * Dispatch-deny wire codes (contract §8.2): the host answered that the op or
 * its required capability is not available → `CAPABILITY_PORT_MISSING`.
 */
const DISPATCH_DENY_CODES = new Set(["op_unsupported", "capability_missing"]);

/** Map an error-branch envelope to a `SpokeResult` reject (contract §8.2). */
function mapErrorEnvelope(error: ErrorEnvelope): SpokeReject {
  if (DISPATCH_DENY_CODES.has(error.code)) {
    return spokeReject(SpokeRejectCode.CAPABILITY_PORT_MISSING, error.message, {
      ...(error.details ?? {}),
      wire_code: error.code,
    });
  }
  // Envelope-auth rejection (contract §8): the host answered `auth_failed`
  // for a request that failed envelope verification. `auth_failed` is not a
  // SpokeRejectCode, so `fromErrorEnvelope` would map it to INVALID_INPUT;
  // the locked mapping is `INTERNAL_ERROR` with the envelope-auth
  // `details.kind` surfaced verbatim.
  if (error.code === "auth_failed") {
    return spokeReject(SpokeRejectCode.INTERNAL_ERROR, error.message, {
      ...(error.details ?? {}),
    });
  }
  // Dispatch-layer validation verdict (consumer contract §2.2): the host
  // spelled the wire code in the Connect dispatch vocabulary, which is not a
  // `SpokeRejectCode` literal. Map it to the internal `INVALID_INPUT` and
  // preserve the verdict's `details` verbatim — `fromErrorEnvelope`'s
  // unknown-code branch would replace the message and drop `reason`/`field`.
  if (error.code === WIRE_INVALID_INPUT) {
    return spokeReject(SpokeRejectCode.INVALID_INPUT, error.message, {
      ...(error.details ?? {}),
    });
  }
  return fromErrorEnvelope(error);
}

/** Port-method → `port.*` product-op catalogue (frozen contract §5.2). */
const PORT_OPS = {
  getKnowledgeEntry: "port.knowledge.get",
  putKnowledgeEntry: "port.knowledge.put",
  getRelation: "port.relation.get",
  putRelation: "port.relation.put",
  listKnowledgeEntries: "port.scope.list_knowledge_entries",
  listTimelineEvents: "port.scope.list_timeline_events",
  putFindings: "port.finding.put",
  listRules: "port.rule.list",
  listPeerHostCapabilityManifests: "port.host.list_peer_manifests",
  // Shipped optional families (frozen contract §5.2 — capability-gated
  // remotely): project / compute (`l2-computable`) and fork timeline query
  // (`l5-fork`).
  project: "port.computable.project",
  compute: "port.computable.compute",
  listForkTimelineEvents: "port.fork.list_timeline_events",
} as const;

export type RemoteAdapterState =
  | "Disconnected"
  | "Handshaking"
  | "Established"
  | "Closed";

/**
 * Memory-port method → `memory.*` op (protocol §13). Exported so the
 * cross-language golden vectors can assert the adapter's op table row-for-row
 * against the Rust `MemoryPort for RemoteAdapter` impl.
 */
export const MEMORY_OPS = {
  describe: "memory.describe",
  listSpaces: "memory.spaces.list",
  createSpace: "memory.spaces.create",
  freezeSpace: "memory.spaces.freeze",
  unfreezeSpace: "memory.spaces.unfreeze",
  ensureSubject: "memory.subjects.ensure",
  getSubject: "memory.subjects.get",
  fenceSubject: "memory.subjects.fence",
  listGrants: "memory.grants.list",
  grant: "memory.grants.grant",
  revokeGrant: "memory.grants.revoke",
  capture: "memory.capture",
  listCandidates: "memory.candidates.list",
  getCandidate: "memory.candidates.get",
  dismissCandidate: "memory.candidates.dismiss",
  reviewCandidate: "memory.candidates.review",
  createRecord: "memory.records.create",
  getRecord: "memory.records.get",
  updateRecord: "memory.records.update",
  deleteRecord: "memory.records.delete",
  listRecords: "memory.records.list",
  promoteRecord: "memory.records.promote",
  recall: "memory.recall",
  getProfile: "memory.profile.get",
  updateProfile: "memory.profile.update",
  getReflectionState: "memory.reflections.state",
  listReflections: "memory.reflections.list",
  generateReflection: "memory.reflections.generate",
  getReceipt: "memory.receipts.get",
  createExport: "memory.exports.create",
  getExport: "memory.exports.get",
  stageImport: "memory.imports.stage",
  activateImport: "memory.imports.activate",
  discardImport: "memory.imports.discard",
} as const;

/**
 * History-port method → `history.*` op (spoke-history contract §Connect
 * operations). Exported so the cross-language golden vectors can assert the
 * adapter's op table row-for-row against the Rust `HistoryPort for
 * RemoteAdapter` impl.
 */
export const HISTORY_OPS = {
  getHistorySnapshot: "history.revisions.get",
  pruneHistory: "history.revisions.prune",
} as const;

/**
 * Adoption-port method → `knowledge.*` op (spoke-knowledge-adoption
 * contract §Connect operation). Exported so the cross-language golden
 * vectors can assert the adapter's op table row-for-row against the Rust
 * `AdoptionPort for RemoteAdapter` impl.
 */
export const ADOPTION_OPS = {
  adoptKnowledgeEntry: "knowledge.adopt",
} as const;

/** Raw Ed25519 keypair material for the local connect peer. */
export interface RemoteIdentity {
  seed: Uint8Array;
}

export interface RemoteAdapterOptions {
  transport: Transport;
  localIdentity: RemoteIdentity;
  localManifest: HostCapabilityManifest;
  remotePubkey: Uint8Array;
  allowlist: readonly string[];
  invokeTimeoutMs?: number;
  capabilityToken?: CapabilityTokenProof;
}

interface PendingInvoke {
  correlation: Correlation;
  op: string;
  resolve: (response: ConnectInvokeResponse) => void;
  reject: (error: RemoteError) => void;
  timer: ReturnType<typeof setTimeout>;
}

type ReverseGateResult =
  | { ok: true }
  | {
      ok: false;
      code: string;
      message: string;
      details?: Record<string, unknown>;
    }
  | null;

/**
 * Registered tool handler (frozen contract §1/§6): receives the tool's
 * `arguments` object from the request payload and resolves with the tool
 * result as a `SpokeResult`. A throwing/rejecting handler answers the error
 * branch (mapped via `toErrorEnvelope`) and never crashes the receive loop.
 *
 * The frozen contract writes the parameter as `arguments`; TypeScript
 * rejects `arguments` as a binding identifier in strict mode (TS1215), so
 * the TS surface names it `args` — the contract is positional, and
 * consumers passing a handler of the frozen shape type-check unchanged.
 */
export type ToolHandler = (
  args: Record<string, unknown>,
) => Promise<SpokeResult<unknown>>;

/**
 * Single-peer async `BaselinePorts` proxy over an established connect
 * session. Construct via `connectRemoteAdapter` — the constructor alone
 * yields an un-established adapter that fails closed on every port call.
 */
export class RemoteAdapter implements BaselinePorts, MemoryPort, HistoryPort, AdoptionPort, AdoptionPorts {
  readonly #transport: Transport;
  readonly #invokeTimeoutMs: number;
  readonly #secret: Uint8Array;
  readonly #remotePubkey: Uint8Array;
  #capabilityToken?: CapabilityTokenProof;

  #stateInternal: RemoteAdapterState = "Disconnected";
  #session: Session | null = null;
  #remoteManifestInternal: HostCapabilityManifest | null = null;
  #receiveLoopRunning = false;
  #pending = new Map<string, PendingInvoke>();
  #toolHandlers = new Map<string, ToolHandler>();
  #sendTail: Promise<void> = Promise.resolve();

  constructor(
    transport: Transport,
    secret: Uint8Array,
    remotePubkey: Uint8Array,
    invokeTimeoutMs: number = DEFAULT_INVOKE_TIMEOUT_MS,
    capabilityToken?: CapabilityTokenProof,
  ) {
    this.#transport = transport;
    this.#secret = secret;
    this.#remotePubkey = remotePubkey;
    this.#invokeTimeoutMs = invokeTimeoutMs;
    this.#capabilityToken = capabilityToken;
  }

  get state(): RemoteAdapterState {
    return this.#stateInternal;
  }

  get sessionId(): string {
    return this.#session?.session_id ?? "";
  }

  get remotePeerId(): string {
    return this.#session?.responder_peer_id ?? "";
  }

  get remoteManifest(): HostCapabilityManifest {
    if (this.#remoteManifestInternal === null) {
      throw new Error("connect session is not established — remote manifest unavailable");
    }
    return this.#remoteManifestInternal;
  }

  async getKnowledgeEntry(entryId: string): Promise<SpokeResult<KnowledgeEntry>> {
    return this.#invokeMapped<KnowledgeEntry>(PORT_OPS.getKnowledgeEntry, {
      entry_id: entryId,
    });
  }

  async putKnowledgeEntry(
    entry: KnowledgeEntry,
    expectedBaseRevision: number | null,
  ): Promise<SpokeResult<KnowledgeEntry>> {
    return this.#invokeMapped<KnowledgeEntry>(PORT_OPS.putKnowledgeEntry, {
      entry,
      expected_base_revision: expectedBaseRevision,
    });
  }

  /**
   * Remote `port.knowledge.get` naming the scope to read in (A2). The member
   * is sent only when both sides negotiated `spoke-knowledge-scoped`; a
   * caller-named scope without the marker is refused locally without
   * sending, so a scope is never silently dropped (A2-D5). A `null` scope
   * keeps the `0.18.1` payload.
   */
  async getKnowledgeEntryInScope(
    entryId: string,
    scope: Scope | null,
  ): Promise<SpokeResult<KnowledgeEntry>> {
    const payload = this.#scopedKnowledgePayload(
      PORT_OPS.getKnowledgeEntry,
      { entry_id: entryId },
      scope,
    );
    if (!payload.ok) {
      return payload;
    }
    return this.#invokeMapped<KnowledgeEntry>(
      PORT_OPS.getKnowledgeEntry,
      payload.value,
    );
  }

  /** Remote `port.knowledge.put` naming the scope to write in (A2) — the same marker rule and local refusal as {@link getKnowledgeEntryInScope}. */
  async putKnowledgeEntryInScope(
    entry: KnowledgeEntry,
    expectedBaseRevision: number | null,
    scope: Scope | null,
  ): Promise<SpokeResult<KnowledgeEntry>> {
    const payload = this.#scopedKnowledgePayload(
      PORT_OPS.putKnowledgeEntry,
      { entry, expected_base_revision: expectedBaseRevision },
      scope,
    );
    if (!payload.ok) {
      return payload;
    }
    return this.#invokeMapped<KnowledgeEntry>(
      PORT_OPS.putKnowledgeEntry,
      payload.value,
    );
  }

  /**
   * Attach `scope` to a knowledge payload only when the session negotiated
   * the marker; otherwise refuse locally with the dispatch-deny class the
   * responder would answer (`op_unsupported` → `CAPABILITY_PORT_MISSING`),
   * never sending a scope the peer would ignore (A2-D5).
   */
  #scopedKnowledgePayload(
    op: string,
    payload: Record<string, unknown>,
    scope: Scope | null,
  ): SpokeResult<Record<string, unknown>> {
    if (scope === null) {
      return spokeOk(payload);
    }
    const negotiated =
      this.#session?.negotiated_capabilities.includes(
        CAPABILITY_SPOKE_KNOWLEDGE_SCOPED,
      ) ?? false;
    if (!negotiated) {
      return spokeReject(
        SpokeRejectCode.CAPABILITY_PORT_MISSING,
        `op ${op} requires capability ${CAPABILITY_SPOKE_KNOWLEDGE_SCOPED} for its declared scope`,
        { wire_code: "op_unsupported" },
      );
    }
    return spokeOk({ ...payload, scope });
  }

  async getRelation(relationId: string): Promise<SpokeResult<Relation>> {
    return this.#invokeMapped<Relation>(PORT_OPS.getRelation, {
      relation_id: relationId,
    });
  }

  async putRelation(
    relation: Relation,
    expectedBaseRevision: number | null,
  ): Promise<SpokeResult<Relation>> {
    return this.#invokeMapped<Relation>(PORT_OPS.putRelation, {
      relation,
      expected_base_revision: expectedBaseRevision,
    });
  }

  async listKnowledgeEntries(scope: Scope): Promise<SpokeResult<KnowledgeEntry[]>> {
    return this.#invokeMapped<KnowledgeEntry[]>(PORT_OPS.listKnowledgeEntries, {
      scope,
    });
  }

  async listTimelineEvents(scope: Scope): Promise<SpokeResult<TimelineEvent[]>> {
    return this.#invokeMapped<TimelineEvent[]>(PORT_OPS.listTimelineEvents, {
      scope,
    });
  }

  async putFindings(findings: Finding[]): Promise<SpokeResult<Finding[]>> {
    return this.#invokeMapped<Finding[]>(PORT_OPS.putFindings, { findings });
  }

  async listRules(ruleRefs: string[]): Promise<SpokeResult<Rule[]>> {
    return this.#invokeMapped<Rule[]>(PORT_OPS.listRules, { rule_refs: ruleRefs });
  }

  /**
   * Optional l2-computable face (frozen contract §5.2): project the
   * session's computable view on the remote peer. No local pre-gate —
   * mirrors the baseline port methods: the responder's dispatch gate
   * denies (wire `op_unsupported` / `capability_missing`) and the D7 row
   * maps to `CAPABILITY_PORT_MISSING` with `details.wire_code` preserved.
   */
  async project(request: ProjectRequest): Promise<SpokeResult<ProjectResponse>> {
    return this.#invokeMapped<ProjectResponse>(PORT_OPS.project, {
      ...request,
    });
  }

  /**
   * Optional l2-computable face (frozen contract §5.2): apply / settle
   * computable updates on the remote peer. Same deny mapping as `project`.
   */
  async compute(request: ComputeRequest): Promise<SpokeResult<ComputeResponse>> {
    return this.#invokeMapped<ComputeResponse>(PORT_OPS.compute, {
      ...request,
    });
  }

  /**
   * Optional l5-fork face (frozen contract §5.2): query the remote peer's
   * fork-branch timeline, scoped like `listTimelineEvents` with the
   * `fork_id` carried on the `Scope`. Same deny mapping as the baseline
   * port methods (no local pre-gate).
   */
  async listForkTimelineEvents(
    scope: Scope & { fork_id: ForkId },
  ): Promise<SpokeResult<TimelineEvent[]>> {
    return this.#invokeMapped<TimelineEvent[]>(
      PORT_OPS.listForkTimelineEvents,
      { scope },
    );
  }


  async extract(request: ExtractRequest): Promise<SpokeResult<ExtractResponse>> {
    return this.#invokeExtractMapped(request);
  }

  async getHostCapabilityManifest(): Promise<SpokeResult<HostCapabilityManifest>> {
    const manifest = this.#remoteManifestInternal;
    if (manifest === null) {
      return internalError("session_closed", "connect session is not established");
    }
    return spokeOk(structuredClone(manifest));
  }

  /** Proxy to the remote's product-seeded peer list (contract §7). */
  async listPeerHostCapabilityManifests(): Promise<
    SpokeResult<HostCapabilityManifest[]>
  > {
    return this.#invokeMapped<HostCapabilityManifest[]>(
      PORT_OPS.listPeerHostCapabilityManifests,
      {},
    );
  }

  async describe(
    _principal: Principal,
    request: MemoryDescribeRequest,
  ): Promise<SpokeResult<MemoryDescribeResponse>> {
    return this.#invokeMapped<MemoryDescribeResponse>(MEMORY_OPS.describe, {
      ...request,
    });
  }

  async listSpaces(
    _principal: Principal,
    request: MemorySpacesListRequest,
  ): Promise<SpokeResult<MemorySpacesListResponse>> {
    return this.#invokeMapped<MemorySpacesListResponse>(MEMORY_OPS.listSpaces, {
      ...request,
    });
  }

  async createSpace(
    _principal: Principal,
    request: MemorySpacesCreateRequest,
  ): Promise<SpokeResult<MemorySpacesCreateResponse>> {
    return this.#invokeMapped<MemorySpacesCreateResponse>(
      MEMORY_OPS.createSpace,
      { ...request },
    );
  }

  async freezeSpace(
    _principal: Principal,
    request: MemorySpacesFreezeRequest,
  ): Promise<SpokeResult<MemorySpacesFreezeResponse>> {
    return this.#invokeMapped<MemorySpacesFreezeResponse>(
      MEMORY_OPS.freezeSpace,
      { ...request },
    );
  }

  async unfreezeSpace(
    _principal: Principal,
    request: MemorySpacesUnfreezeRequest,
  ): Promise<SpokeResult<MemorySpacesUnfreezeResponse>> {
    return this.#invokeMapped<MemorySpacesUnfreezeResponse>(
      MEMORY_OPS.unfreezeSpace,
      { ...request },
    );
  }

  async ensureSubject(
    _principal: Principal,
    request: MemorySubjectsEnsureRequest,
  ): Promise<SpokeResult<MemorySubjectsEnsureResponse>> {
    return this.#invokeMapped<MemorySubjectsEnsureResponse>(
      MEMORY_OPS.ensureSubject,
      { ...request },
    );
  }

  async getSubject(
    _principal: Principal,
    request: MemorySubjectsGetRequest,
  ): Promise<SpokeResult<MemorySubjectsGetResponse>> {
    return this.#invokeMapped<MemorySubjectsGetResponse>(MEMORY_OPS.getSubject, {
      ...request,
    });
  }

  async fenceSubject(
    _principal: Principal,
    request: MemorySubjectsFenceRequest,
  ): Promise<SpokeResult<MemorySubjectsFenceResponse>> {
    return this.#invokeMapped<MemorySubjectsFenceResponse>(
      MEMORY_OPS.fenceSubject,
      { ...request },
    );
  }

  async listGrants(
    _principal: Principal,
    request: MemoryGrantsListRequest,
  ): Promise<SpokeResult<MemoryGrantsListResponse>> {
    return this.#invokeMapped<MemoryGrantsListResponse>(MEMORY_OPS.listGrants, {
      ...request,
    });
  }

  async grant(
    _principal: Principal,
    request: MemoryGrantsGrantRequest,
  ): Promise<SpokeResult<MemoryGrantsGrantResponse>> {
    return this.#invokeMapped<MemoryGrantsGrantResponse>(MEMORY_OPS.grant, {
      ...request,
    });
  }

  async revokeGrant(
    _principal: Principal,
    request: MemoryGrantsRevokeRequest,
  ): Promise<SpokeResult<MemoryGrantsRevokeResponse>> {
    return this.#invokeMapped<MemoryGrantsRevokeResponse>(
      MEMORY_OPS.revokeGrant,
      { ...request },
    );
  }

  async capture(
    _principal: Principal,
    request: MemoryCaptureRequest,
  ): Promise<SpokeResult<MemoryCaptureResponse>> {
    return this.#invokeMapped<MemoryCaptureResponse>(MEMORY_OPS.capture, {
      ...request,
    });
  }

  async listCandidates(
    _principal: Principal,
    request: MemoryCandidatesListRequest,
  ): Promise<SpokeResult<MemoryCandidatesListResponse>> {
    return this.#invokeMapped<MemoryCandidatesListResponse>(
      MEMORY_OPS.listCandidates,
      { ...request },
    );
  }

  async getCandidate(
    _principal: Principal,
    request: MemoryCandidatesGetRequest,
  ): Promise<SpokeResult<MemoryCandidatesGetResponse>> {
    return this.#invokeMapped<MemoryCandidatesGetResponse>(
      MEMORY_OPS.getCandidate,
      { ...request },
    );
  }

  async dismissCandidate(
    _principal: Principal,
    request: MemoryCandidatesDismissRequest,
  ): Promise<SpokeResult<MemoryCandidatesDismissResponse>> {
    return this.#invokeMapped<MemoryCandidatesDismissResponse>(
      MEMORY_OPS.dismissCandidate,
      { ...request },
    );
  }

  async reviewCandidate(
    _principal: Principal,
    request: MemoryCandidatesReviewRequest,
  ): Promise<SpokeResult<MemoryCandidatesReviewResponse>> {
    return this.#invokeMapped<MemoryCandidatesReviewResponse>(
      MEMORY_OPS.reviewCandidate,
      { ...request },
    );
  }

  async createRecord(
    _principal: Principal,
    request: MemoryRecordsCreateRequest,
  ): Promise<SpokeResult<MemoryRecordsCreateResponse>> {
    return this.#invokeMapped<MemoryRecordsCreateResponse>(
      MEMORY_OPS.createRecord,
      { ...request },
    );
  }

  async getRecord(
    _principal: Principal,
    request: MemoryRecordsGetRequest,
  ): Promise<SpokeResult<MemoryRecordsGetResponse>> {
    return this.#invokeMapped<MemoryRecordsGetResponse>(MEMORY_OPS.getRecord, {
      ...request,
    });
  }

  async updateRecord(
    _principal: Principal,
    request: MemoryRecordsUpdateRequest,
  ): Promise<SpokeResult<MemoryRecordsUpdateResponse>> {
    return this.#invokeMapped<MemoryRecordsUpdateResponse>(
      MEMORY_OPS.updateRecord,
      { ...request },
    );
  }

  async deleteRecord(
    _principal: Principal,
    request: MemoryRecordsDeleteRequest,
  ): Promise<SpokeResult<MemoryRecordsDeleteResponse>> {
    return this.#invokeMapped<MemoryRecordsDeleteResponse>(
      MEMORY_OPS.deleteRecord,
      { ...request },
    );
  }

  async listRecords(
    _principal: Principal,
    request: MemoryRecordsListRequest,
  ): Promise<SpokeResult<MemoryRecordsListResponse>> {
    return this.#invokeMapped<MemoryRecordsListResponse>(MEMORY_OPS.listRecords, {
      ...request,
    });
  }

  async promoteRecord(
    _principal: Principal,
    request: MemoryRecordsPromoteRequest,
  ): Promise<SpokeResult<MemoryRecordsPromoteResponse>> {
    return this.#invokeMapped<MemoryRecordsPromoteResponse>(
      MEMORY_OPS.promoteRecord,
      { ...request },
    );
  }

  async recall(
    _principal: Principal,
    request: MemoryRecallRequest,
  ): Promise<SpokeResult<MemoryRecallResponse>> {
    return this.#invokeMapped<MemoryRecallResponse>(MEMORY_OPS.recall, {
      ...request,
    });
  }

  async getProfile(
    _principal: Principal,
    request: MemoryProfileGetRequest,
  ): Promise<SpokeResult<MemoryProfileGetResponse>> {
    return this.#invokeMapped<MemoryProfileGetResponse>(MEMORY_OPS.getProfile, {
      ...request,
    });
  }

  async updateProfile(
    _principal: Principal,
    request: MemoryProfileUpdateRequest,
  ): Promise<SpokeResult<MemoryProfileUpdateResponse>> {
    return this.#invokeMapped<MemoryProfileUpdateResponse>(
      MEMORY_OPS.updateProfile,
      { ...request },
    );
  }

  async getReflectionState(
    _principal: Principal,
    request: MemoryReflectionsStateRequest,
  ): Promise<SpokeResult<MemoryReflectionsStateResponse>> {
    return this.#invokeMapped<MemoryReflectionsStateResponse>(
      MEMORY_OPS.getReflectionState,
      { ...request },
    );
  }

  async listReflections(
    _principal: Principal,
    request: MemoryReflectionsListRequest,
  ): Promise<SpokeResult<MemoryReflectionsListResponse>> {
    return this.#invokeMapped<MemoryReflectionsListResponse>(
      MEMORY_OPS.listReflections,
      { ...request },
    );
  }

  async generateReflection(
    _principal: Principal,
    request: MemoryReflectionsGenerateRequest,
  ): Promise<SpokeResult<MemoryReflectionsGenerateResponse>> {
    return this.#invokeMapped<MemoryReflectionsGenerateResponse>(
      MEMORY_OPS.generateReflection,
      { ...request },
    );
  }

  async getReceipt(
    _principal: Principal,
    request: MemoryReceiptsGetRequest,
  ): Promise<SpokeResult<MemoryReceiptsGetResponse>> {
    return this.#invokeMapped<MemoryReceiptsGetResponse>(MEMORY_OPS.getReceipt, {
      ...request,
    });
  }

  async createExport(
    _principal: Principal,
    request: MemoryExportsCreateRequest,
  ): Promise<SpokeResult<MemoryExportsCreateResponse>> {
    return this.#invokeMapped<MemoryExportsCreateResponse>(
      MEMORY_OPS.createExport,
      { ...request },
    );
  }

  async getExport(
    _principal: Principal,
    request: MemoryExportsGetRequest,
  ): Promise<SpokeResult<MemoryExportsGetResponse>> {
    return this.#invokeMapped<MemoryExportsGetResponse>(MEMORY_OPS.getExport, {
      ...request,
    });
  }

  async stageImport(
    _principal: Principal,
    request: MemoryImportsStageRequest,
  ): Promise<SpokeResult<MemoryImportsStageResponse>> {
    return this.#invokeMapped<MemoryImportsStageResponse>(
      MEMORY_OPS.stageImport,
      { ...request },
    );
  }

  async activateImport(
    _principal: Principal,
    request: MemoryImportsActivateRequest,
  ): Promise<SpokeResult<MemoryImportsActivateResponse>> {
    return this.#invokeMapped<MemoryImportsActivateResponse>(
      MEMORY_OPS.activateImport,
      { ...request },
    );
  }

  async discardImport(
    _principal: Principal,
    request: MemoryImportsDiscardRequest,
  ): Promise<SpokeResult<MemoryImportsDiscardResponse>> {
    return this.#invokeMapped<MemoryImportsDiscardResponse>(
      MEMORY_OPS.discardImport,
      { ...request },
    );
  }

  async getHistorySnapshot(
    _principal: Principal,
    scope: Scope,
    entityKind: string,
    entityId: string,
    revision: number,
  ): Promise<SpokeResult<RevisionSnapshot>> {
    const request: Record<string, unknown> = {
      entity_kind: entityKind,
      entity_id: entityId,
      revision,
      scope,
    };
    const result = await this.#invokeMapped<Record<string, unknown>>(
      HISTORY_OPS.getHistorySnapshot,
      request,
    );
    if (!result.ok) {
      return result;
    }
    const snapshot = result.value.snapshot;
    if (
      !isValidRevisionSnapshot(snapshot, {
        entity_kind: entityKind,
        entity_id: entityId,
        revision,
      })
    ) {
      return internalError(
        "transport",
        "history.revisions.get response snapshot does not match the request selectors",
      );
    }
    return spokeOk(snapshot as RevisionSnapshot);
  }

  async pruneHistory(
    _principal: Principal,
    scope: Scope,
    entityKind: string,
    entityId: string,
    keep: number,
    options?: { idempotencyKey?: string },
  ): Promise<SpokeResult<HistoryPruneResult>> {
    const request: Record<string, unknown> = {
      entity_kind: entityKind,
      entity_id: entityId,
      keep,
      idempotency_key: options?.idempotencyKey ?? globalThis.crypto.randomUUID(),
      scope,
    };
    return this.#invokeMapped<HistoryPruneResult>(
      HISTORY_OPS.pruneHistory,
      request,
    );
  }

  async adoptKnowledgeEntry(
    _principal: Principal,
    _scope: Scope,
    request: KnowledgeAdoptRequest,
  ): Promise<SpokeResult<KnowledgeAdoptResponse>> {
    return this.#invokeMapped<KnowledgeAdoptResponse>(
      ADOPTION_OPS.adoptKnowledgeEntry,
      { ...request },
    );
  }

  asAdoption(): AdoptionPort {
    return this;
  }

  close(): void {
    this.#closeSession("local shutdown");
  }

  // see contract §6 — tool serving (reverse invokes)

  registerToolHandler(capabilityId: string, handler: ToolHandler): void {
    const parsed = parseToolCapabilityId(capabilityId);
    if (!parsed.ok) {
      throw new Error(parsed.message);
    }
    this.#toolHandlers.set(capabilityId, handler);
  }

  async invokeTool(
    capabilityId: string,
    args: Record<string, unknown>,
  ): Promise<SpokeResult<unknown>> {
    const parsed = parseToolCapabilityId(capabilityId);
    if (!parsed.ok) {
      return parsed;
    }
    try {
      const response = await this.#invokeOp(capabilityId, { arguments: args });
      if ("error" in response) {
        return mapErrorEnvelope(response.error);
      }
      if (
        typeof response.payload !== "object" ||
        response.payload === null ||
        !("result" in response.payload)
      ) {
        return internalError(
          "transport",
          `response payload decode failed: payload does not match the ${capabilityId} success shape`,
        );
      }
      return spokeOk(response.payload.result);
    } catch (error) {
      if (error instanceof RemoteError) {
        return internalError(error.kind, error.message);
      }
      return internalError(
        "transport",
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  #beginHandshake(): void {
    this.#stateInternal = "Handshaking";
  }

  async #sendEnvelope(doc: unknown): Promise<void> {
    await this.#transport.send(encodeEnvelope(doc));
  }

  async #recvEnvelope(): Promise<unknown> {
    return decodeJsonMessage(await this.#transport.recv());
  }

  #establish(session: Session, remoteManifest: HostCapabilityManifest): void {
    this.#session = session;
    this.#remoteManifestInternal = remoteManifest;
    this.#stateInternal = "Established";
    this.#runReceiveLoop();
  }

  #closeSession(reason: string): void {
    if (this.#stateInternal === "Closed") {
      return;
    }
    this.#stateInternal = "Closed";
    let closeResult: unknown;
    try {
      closeResult = this.#transport.close?.();
    } catch {
      closeResult = undefined;
    }
    if (closeResult !== undefined) {
      void Promise.resolve(closeResult).catch(() => {});
    }
    this.#failAllPending(
      new RemoteError("session_closed", `connect session closed: ${reason}`),
    );
  }

  #failAllPending(error: RemoteError): void {
    for (const entry of this.#pending.values()) {
      clearTimeout(entry.timer);
      entry.reject(error);
    }
    this.#pending.clear();
  }

  #runReceiveLoop(): void {
    if (this.#receiveLoopRunning) {
      return;
    }
    this.#receiveLoopRunning = true;
    void this.#receiveLoop().finally(() => {
      this.#receiveLoopRunning = false;
    });
  }

  async #receiveLoop(): Promise<void> {
    while (this.#stateInternal === "Established") {
      const session = this.#session;
      if (session === null) {
        return;
      }
      let doc: unknown;
      try {
        doc = decodeJsonMessage(await this.#transport.recv());
      } catch (error) {
        this.#closeSession(
          `transport loss: ${error instanceof Error ? error.message : String(error)}`,
        );
        return;
      }

      if (isConnectInvokeRequest(doc)) {
        try {
          await this.#serveReverseInvoke(doc);
        } catch (error) {
          this.#closeSession(
            `reverse invoke serving failed: ${error instanceof Error ? error.message : String(error)}`,
          );
          return;
        }
        continue;
      }

      if (isConnectInvokeResponse(doc)) {
        const entry = this.#pending.get(doc.request_id);
        if (entry === undefined) {
          continue;
        }
        clearTimeout(entry.timer);
        this.#pending.delete(doc.request_id);
        try {
          checkResponseCorrelation(entry.correlation, correlationFromResponse(doc));
          await verifyInvokeResponseAuth(
            this.#remotePubkey,
            doc,
            session.session_id,
            entry.op,
          );
          entry.resolve(doc);
        } catch (error) {
          if (error instanceof EnvelopeAuthError) {
            entry.reject(new RemoteError(error.kind, error.message));
          } else if (error instanceof CoreError && error.code === "crypto") {
            entry.reject(new RemoteError("envelope_auth_invalid", error.message));
          } else {
            entry.reject(
              new RemoteError(
                "correlation_mismatch",
                "response echo fields do not match the request",
              ),
            );
          }
        }
        continue;
      }
    }
  }

  // see contract §4 — reverse invoke serving pipeline

  #sendReverseResponse(doc: unknown): void {
    void this.#transport.send(encodeEnvelope(doc)).catch(() => {});
  }

  async #sendReverseErrorEnvelope(
    doc: ConnectInvokeRequest,
    error: ErrorEnvelope,
  ): Promise<void> {
    const session = this.#session;
    if (session === null) {
      return;
    }
    this.#sendReverseResponse(
      await authenticateInvokeResponse(
        this.#secret,
        {
          session_id: doc.session_id,
          sequence: doc.sequence,
          request_id: doc.request_id,
          error,
        },
        {},
        doc.op,
      ),
    );
  }

  async #serveReverseInvoke(doc: ConnectInvokeRequest): Promise<void> {
    const gate = await this.#runReverseGate(doc);
    if (gate === null) {
      return;
    }
    if (!gate.ok) {
      await this.#sendReverseErrorEnvelope(doc, {
        code: gate.code,
        message: gate.message,
        ...(gate.details !== undefined ? { details: gate.details } : {}),
        extensions: {},
      });
      return;
    }
    void this.#dispatchReverseInvoke(doc);
  }

  async #runReverseGate(doc: ConnectInvokeRequest): Promise<ReverseGateResult> {
    const session = this.#session;
    if (session === null) {
      return null;
    }
    try {
      session.peekInboundSequence(doc.sequence);
    } catch {
      return {
        ok: false,
        code: "invalid_sequence",
        message: `inbound sequence ${doc.sequence} is not the next expected`,
      };
    }
    try {
      await verifyInvokeRequestAuth(
        this.#remotePubkey,
        doc,
        session.session_id,
      );
    } catch (error) {
      if (error instanceof EnvelopeAuthError) {
        return {
          ok: false,
          code: "auth_failed",
          message: error.message,
          details: { kind: error.kind },
        };
      }
      throw error;
    }
    session.acceptInboundSequence(doc.sequence);
    return { ok: true };
  }

  async #dispatchReverseInvoke(doc: ConnectInvokeRequest): Promise<void> {
    const session = this.#session;
    if (session === null) {
      return;
    }
    try {
      if (!session.dispatchAllowed(doc.op)) {
        await this.#sendReverseErrorEnvelope(doc, {
          code: "op_unsupported",
          message: `op ${doc.op} is not authorized by this session`,
          extensions: {},
        });
        return;
      }
      const handler = this.#toolHandlers.get(doc.op);
      if (handler === undefined) {
        await this.#sendReverseErrorEnvelope(doc, {
          code: "op_unsupported",
          message: `no handler registered for ${doc.op}`,
          extensions: {},
        });
        return;
      }
      const payload = doc.payload as { arguments?: unknown };
      const argumentsField = payload.arguments;
      const handlerArgs: Record<string, unknown> =
        typeof argumentsField === "object" &&
        argumentsField !== null &&
        !Array.isArray(argumentsField)
          ? (argumentsField as Record<string, unknown>)
          : {};
      let result: SpokeResult<unknown>;
      try {
        result = await handler(handlerArgs);
      } catch (error) {
        result = spokeReject(
          SpokeRejectCode.INTERNAL_ERROR,
          error instanceof Error ? error.message : String(error),
        );
      }
      if (result.ok) {
        this.#sendReverseResponse(
          await authenticateInvokeResponse(
            this.#secret,
            {
              session_id: doc.session_id,
              sequence: doc.sequence,
              request_id: doc.request_id,
              payload: {
                result: "value" in result ? result.value : undefined,
              },
            },
            {},
            doc.op,
          ),
        );
      } else {
        await this.#sendReverseErrorEnvelope(doc, toErrorEnvelope(result));
      }
    } catch (error) {
      try {
        await this.#sendReverseErrorEnvelope(doc, {
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: `tool invoke failed: ${error instanceof Error ? error.message : String(error)}`,
          extensions: {},
        });
      } catch {
      }
    }
  }

  async #invokeOp(
    op: string,
    payload: Record<string, unknown>,
  ): Promise<ConnectInvokeResponse> {
    if (this.#stateInternal !== "Established" || this.#session === null) {
      throw new RemoteError(
        "session_closed",
        `connect session is not established (state ${this.#stateInternal})`,
      );
    }
    const session = this.#session;
    let sequence: number;
    try {
      sequence = session.allocateOutboundSequence();
    } catch (error) {
      this.#closeSession("outbound sequence exhausted");
      throw new RemoteError(
        "sequence_exhausted",
        "outbound sequence space exhausted — reopen session",
      );
    }
    // The wire request to sign (`spoke-connect-invoke-request-jcs-v1`,
    // envelope-auth contract §2/§3): the signed object covers
    // `{session_id, sequence, request_id, op, payload}` plus `auth` when a
    // capability token is attached.
    const unsigned: InvokeRequestSignInput = {
      session_id: session.session_id,
      sequence,
      request_id: globalThis.crypto.randomUUID(),
      op,
      payload,
      ...(this.#capabilityToken !== undefined
        ? { auth: this.#capabilityToken as unknown as Record<string, unknown> }
        : {}),
    };
    // The waiter is registered SYNCHRONOUSLY, before the async sign: from
    // the caller's perspective the invoke is in flight immediately, so a
    // transport close during signing fails it with `session_closed`
    // (contract §8.2 mid-flight) — not a send-after-close `transport`
    // error. Signing failures (non-JSON-serializable payload, bad key) are
    // local adapter bugs and reject with the same `transport` kind as the
    // encode-failure path (the key is adapter-supplied, never wire material).
    return new Promise<ConnectInvokeResponse>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.#pending.delete(unsigned.request_id);
        reject(
          new RemoteError(
            "timeout",
            `invoke ${op} (${unsigned.request_id}) timed out after ${this.#invokeTimeoutMs}ms`,
          ),
        );
      }, this.#invokeTimeoutMs);
      this.#pending.set(unsigned.request_id, {
        correlation: {
          session_id: unsigned.session_id,
          sequence: unsigned.sequence,
          request_id: unsigned.request_id,
        },
        op,
        resolve,
        reject,
        timer,
      });
      const prevSend = this.#sendTail;
      let releaseSend: () => void = () => {};
      this.#sendTail = new Promise<void>((resolveTail) => {
        releaseSend = resolveTail;
      });
      void authenticateInvokeRequest(this.#secret, unsigned).then(
        (signed) => {
          void prevSend
            .then(() => {
              if (!this.#pending.has(unsigned.request_id)) {
                this.#closeSession(
                  "deferred invoke skipped (settled while queued) — outbound sequence never transmitted",
                );
                return;
              }
              try {
                return this.#transport.send(encodeEnvelope(signed));
              } catch (error) {
                throw new RemoteError(
                  "transport",
                  `invoke encode failed: ${error instanceof Error ? error.message : String(error)}`,
                );
              }
            })
            .then(
              () => {
                releaseSend();
              },
              (sendError) => {
                releaseSend();
                clearTimeout(timer);
                this.#pending.delete(unsigned.request_id);
                reject(
                  sendError instanceof RemoteError
                    ? sendError
                    : new RemoteError(
                        "transport",
                        `invoke send failed: ${sendError instanceof Error ? sendError.message : String(sendError)}`,
                      ),
                );
              },
            );
        },
        (error) => {
          releaseSend();
          clearTimeout(timer);
          this.#pending.delete(unsigned.request_id);
          reject(
            new RemoteError(
              "transport",
              `invoke sign failed: ${error instanceof Error ? error.message : String(error)}`,
            ),
          );
        },
      );
    });
  }

  async #invokeExtractMapped(
    request: ExtractRequest,
  ): Promise<SpokeResult<ExtractResponse>> {
    try {
      const response = await this.#invokeOp(
        "extract",
        request as unknown as Record<string, unknown>,
      );
      if ("error" in response) {
        return mapErrorEnvelope(response.error);
      }
      if (!isValidSuccessPayload("extract", response.payload)) {
        return internalError(
          "transport",
          "response payload decode failed: payload does not match the extract success shape",
        );
      }
      const payload = response.payload as ExtractResponse;
      if (
        typeof payload === "object" &&
        payload !== null &&
        "error" in payload
      ) {
        return fromErrorEnvelope(payload.error);
      }
      return spokeOk(payload);
    } catch (error) {
      if (error instanceof RemoteError) {
        return internalError(error.kind, error.message);
      }
      return internalError(
        "transport",
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  /** Invoke a port op and map the response to `SpokeResult` (contract §5.3/§8). */
  async #invokeMapped<T>(
    op: string,
    payload: Record<string, unknown>,
  ): Promise<SpokeResult<T>> {
    try {
      const response = await this.#invokeOp(op, payload);
      if ("error" in response) {
        return mapErrorEnvelope(response.error);
      }
      // Two-branch served payload (contract E3): `knowledge.adopt` returns
      // its provider refusal as `{error, extensions?}` — a successful port
      // outcome, not a transport error (qc2 QC2-21 / F-030; Rust
      // `invoke_mapped<KnowledgeAdoptResponse>` decodes both branches).
      if (op === ADOPTION_OPS.adoptKnowledgeEntry) {
        // A payload carrying BOTH branch shapes is neither: the typed Rust
        // untagged decode refuses it (qc1 F-032).
        if (carriesAdoptFailureMember(response.payload)) {
          if (!isAdoptFailurePayload(response.payload)) {
            return internalError(
              "transport",
              "response payload decode failed: payload is not a valid knowledge.adopt failure envelope (a mixed envelope is never valid)",
            );
          }
          return spokeOk(response.payload as unknown as T);
        }
        if (!isValidAdoptSuccessPayload(response.payload)) {
          return internalError(
            "transport",
            "response payload decode failed: payload does not match the knowledge.adopt success shape (entry must be a complete KnowledgeEntry with its adoption_anchor)",
          );
        }
        return spokeOk(response.payload as unknown as T);
      }
      // Success-payload shape gate (Rust parity, contract §8.2): a malformed
      // payload must reject with `INTERNAL_ERROR` `details.kind = "transport"`
      // instead of surfacing `spokeOk(garbage)`.
      if (!isValidSuccessPayload(op, response.payload)) {
        return internalError(
          "transport",
          `response payload decode failed: payload does not match the ${op} success shape`,
        );
      }
      return spokeOk(response.payload as unknown as T);
    } catch (error) {
      if (error instanceof RemoteError) {
        return internalError(error.kind, error.message);
      }
      return internalError(
        "transport",
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  /**
   * @internal test-only — the sequence-exhaustion fixture. Not part of the
   * RemoteAdapter public API.
   */
  setOutboundNextForTest(next: number): void {
    if (this.#stateInternal !== "Established" || this.#session === null) {
      throw new Error("setOutboundNextForTest requires an Established session");
    }
    const session = this.#session as unknown as {
      outbound: { setNext(value: number): void };
    };
    session.outbound.setNext(next);
  }

  /**
   * Dial a remote peer over `transport`: perform the signed hello exchange +
   * session snapshot, then return an `Established` adapter. Throws on any
   * handshake / allowlist / verification failure (contract §3.3/§8.2 — no
   * half-open `BaselinePorts` instance).
   *
   * Lives on the class because the session-lifecycle methods are `#`-private
   * and only reachable from inside the class body; the module-level
   * `connectRemoteAdapter` below is the same entrypoint for consumers.
   */
  static async connectRemoteAdapter(
    options: RemoteAdapterOptions,
  ): Promise<RemoteAdapter> {
    const { transport, localIdentity, localManifest, remotePubkey, allowlist } =
      options;
    const invokeTimeoutMs = options.invokeTimeoutMs ?? DEFAULT_INVOKE_TIMEOUT_MS;
    if (localIdentity.seed.length !== 32) {
      throw new Error("localIdentity.seed must be 32 bytes");
    }
    if (remotePubkey.length !== 32) {
      throw new Error("remotePubkey must be 32 bytes");
    }
    const remotePeerId = derivePeerIdFromEd25519Pubkey(remotePubkey);
    if (!isAllowlisted(allowlist, remotePeerId)) {
      throw new Error(
        `remote peer ${remotePeerId} is not on the allowlist (fail-closed)`,
      );
    }
    const localPeerId = derivePeerIdFromEd25519Pubkey(
      getPublicKeyEd25519(localIdentity.seed),
    );

    const adapter = new RemoteAdapter(
      transport,
      localIdentity.seed,
      remotePubkey,
      invokeTimeoutMs,
      options.capabilityToken,
    );
    adapter.#beginHandshake();

    try {
      //    The initiator hello signs the 4-field object (no `peer_nonce`);
      //    the nonce is kept for the responder-hello dial-binding assert.
      const initiatorNonce = generateNonce();
      await withTimeout(
        adapter.#sendEnvelope(
          await signHelloEd25519(
            localIdentity.seed,
            initiatorNonce,
            localManifest,
          ),
        ),
        invokeTimeoutMs,
        "hello send",
      );

      //    Dial binding: the responder signs the 5-field object incl.
      //    `peer_nonce` = our nonce; a replayed responder hello (signed over
      //    a different or absent initiator nonce — e.g. captured on an
      //    earlier dial and replayed after a process restart resets the
      //    nonce store) fails this assert before any session state exists.
      const helloDoc = await withTimeout(
        adapter.#recvEnvelope(),
        invokeTimeoutMs,
        "server hello",
      );
      if (!isConnectHello(helloDoc)) {
        throw new Error("expected ConnectHello from server");
      }
      await verifyHelloEd25519(
        remotePubkey,
        remotePeerId,
        helloDoc,
        initiatorNonce,
      );
      if (!acceptedServerHellos.checkAndRecord(remotePeerId, helloDoc.nonce)) {
        throw new Error(
          `server hello replay rejected: (peer_id ${remotePeerId}, nonce ${helloDoc.nonce}) was already accepted`,
        );
      }
      const remoteManifest: HostCapabilityManifest = helloDoc.host;

      //    verify runs on the wire form BEFORE the typed checks
      //    (`spoke-connect-session-jcs-v1` against the responder's hello
      //    key; the step-6 peer-id binding assert replaces the old manual
      //    comparison). A missing/invalid signature or peer-id mismatch
      //    fails the dial closed — no session state exists yet.
      const sessionDoc = await withTimeout(
        adapter.#recvEnvelope(),
        invokeTimeoutMs,
        "session snapshot",
      );
      if (!isConnectSession(sessionDoc)) {
        throw new Error("expected ConnectSession snapshot after server hello");
      }
      await verifySessionAuth(
        remotePubkey,
        sessionDoc,
        localPeerId,
        remotePeerId,
      );
      if (sessionDoc.session_id.length === 0) {
        throw new Error("session snapshot session_id must not be empty");
      }
      if (sessionDoc.initial_sequence !== 0) {
        throw new Error("session snapshot initial_sequence must be 0");
      }
      if (
        typeof sessionDoc.opened_at !== "string" ||
        !isStrictTimestamp(sessionDoc.opened_at)
      ) {
        throw new Error(
          `session snapshot opened_at must be a strict RFC 3339 timestamp with an explicit offset (got ${sessionDoc.opened_at})`,
        );
      }

      adapter.#establish(
        new Session({
          session_id: sessionDoc.session_id,
          initiator_peer_id: localPeerId,
          responder_peer_id: remotePeerId,
          negotiated_capabilities: negotiatedCapabilities(
            localManifest.capabilities,
            remoteManifest.capabilities,
          ),
        }),
        remoteManifest,
      );
      return adapter;
    } catch (error) {
      adapter.close();
      throw error;
    }
  }
}

/**
 * Dial a remote peer over `transport`: perform the signed hello exchange +
 * session snapshot, then return an `Established` adapter. Throws on any
 * handshake / allowlist / verification failure (contract §3.3/§8.2 — no
 * half-open `BaselinePorts` instance).
 */
export async function connectRemoteAdapter(
  options: RemoteAdapterOptions,
): Promise<RemoteAdapter> {
  return RemoteAdapter.connectRemoteAdapter(options);
}
