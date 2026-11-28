import type {
  ComputeRequest,
  ConnectInvokeRequest,
  ConnectInvokeResponse,
  ErrorEnvelope,
  ExtractRequest,
  ExtractResponse,
  Finding,
  ForkId,
  HostCapabilityManifest,
  KnowledgeEntry,
  ProjectRequest,
  Relation,
  Rule,
  Scope,
} from "@42ch/spoke-schemas";
import {
  fromErrorEnvelope,
  assertAdoptionAnchorPreserved,
  parseToolCapabilityId,
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  toErrorEnvelope,
  validateCognitionFilter,
  validateCognitionSelection,
  type BaselinePorts,
  type ComputablePort,
  type ForkTimelineQueryPort,
  type ScopedKnowledgePort,
  type SpokeReject,
  type SpokeResult,
} from "@42ch/spoke-operations";

import { getPublicKeyEd25519 } from "../crypto.js";
import { isAllowlisted } from "../core/allowlist.js";
import {
  checkResponseCorrelation,
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
} from "../core/envelope-auth.js";
import {
  CAPABILITY_SPOKE_COGNITION,
  CAPABILITY_SPOKE_KNOWLEDGE,
  CAPABILITY_SPOKE_KNOWLEDGE_SCOPED,
  capabilitySatisfies,
  cognitionRequirementField,
  cognitionRequirementPresent,
  dispatchAllowed,
  requiredCapability,
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
} from "./guards.js";
import {
  isValidKnowledgeEntryDeep,
  isValidSuccessPayload,
  isRevisionInteger,
} from "./payload.js";
import type { RemoteAdoptionService, RemoteHistoryService } from "./serve_ports.js";
import type { RemoteIdentity, ToolHandler } from "./remote-adapter.js";
import type { EnvelopeBytes, Transport } from "./transport.js";

const DEFAULT_INVOKE_TIMEOUT_MS = 5000;

const SESSION_ID_PREFIX = "connect-responder-session-";

const textEncoder = new TextEncoder();

function encodeEnvelope(doc: unknown): EnvelopeBytes {
  return textEncoder.encode(encodeJsonMessage(doc));
}

type ResponderErrorKind =
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

class ResponderError extends Error {
  readonly kind: ResponderErrorKind;

  constructor(kind: ResponderErrorKind, message: string) {
    super(message);
    this.name = "ResponderError";
    this.kind = kind;
  }
}

function internalError(kind: ResponderErrorKind, message: string): SpokeReject {
  return spokeReject(SpokeRejectCode.INTERNAL_ERROR, message, { kind });
}

const SERVE_BUDGET_EXPIRED = Symbol("serve-budget-expired");

const SERVE_BUDGET_CLOSED = Symbol("serve-budget-closed");

const DISPATCH_DENY_CODES = new Set(["op_unsupported", "capability_missing"]);

function mapErrorEnvelope(error: ErrorEnvelope): SpokeReject {
  if (DISPATCH_DENY_CODES.has(error.code)) {
    return spokeReject(SpokeRejectCode.CAPABILITY_PORT_MISSING, error.message, {
      ...(error.details ?? {}),
      wire_code: error.code,
    });
  }
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

/**
 * Product `op_capability_requirements` map (D4): every baseline `port.*`
 * op requires `spoke-baseline`; the optional families require their
 * negotiated family capability (`l2-computable` / `l5-fork`). The core
 * `requiredCapability` table returns `undefined` for `port.*`, so WITHOUT
 * this map every port invoke would be denied `op_unsupported`.
 *
 * Exported because the multi-peer router selects peers for the same
 * catalogue and must read the same rows — one product map, two readers, so
 * routing cannot drift from serving.
 */
export const PORT_OP_CAPABILITY_REQUIREMENTS: Readonly<Record<string, string>> = {
  "port.knowledge.get": CAPABILITY_SPOKE_KNOWLEDGE,
  "port.knowledge.put": CAPABILITY_SPOKE_KNOWLEDGE,
  "port.relation.get": "spoke-baseline",
  "port.relation.put": "spoke-baseline",
  "port.scope.list_knowledge_entries": CAPABILITY_SPOKE_KNOWLEDGE,
  "port.scope.list_timeline_events": "spoke-baseline",
  "port.finding.put": "spoke-baseline",
  "port.rule.list": "spoke-baseline",
  "port.host.list_peer_manifests": "spoke-baseline",
  "port.computable.project": "l2-computable",
  "port.computable.compute": "l2-computable",
  "port.fork.list_timeline_events": "l5-fork",
};

/**
 * The responder's combined serve gate for one op: the core op table first
 * (which also covers the self-describing `tools.*` family), then the product
 * `port.*` map above.
 *
 * `productMap` defaults to the shipped map; a host injecting its own
 * `op_capability_requirements` composition passes it here — the gate's
 * reserved-prefix short-circuit runs before EITHER map, so a conflicting
 * row can never advance a reserved `knowledge.*` unknown.
 *
 * Named and pure so the serve decision is asserted directly from the shared
 * capability matrix instead of only through the per-table lookups.
 */
export function portDispatchAllowed(
  op: string,
  negotiated: readonly string[],
  productMap: Readonly<Record<string, string>> = PORT_OP_CAPABILITY_REQUIREMENTS,
): boolean {
  if (op.startsWith("knowledge.")) {
    return dispatchAllowed(op, negotiated);
  }
  if (dispatchAllowed(op, negotiated)) {
    return true;
  }
  const required = productMap[op];
  return required !== undefined && capabilitySatisfies(negotiated, required);
}

/** Capability required when a Scope-bearing remote op carries viewpoint. */
export const CAPABILITY_KE_OWNERSHIP = "ke-ownership";

const SCOPE_BEARING_OPS = new Set([
  "port.scope.list_knowledge_entries",
  "port.scope.list_timeline_events",
  "port.fork.list_timeline_events",
  "port.knowledge.get",
  "port.knowledge.put",
]);

/** The subset whose wire request requires `scope`; the rest accept it optionally. */
const REQUIRED_SCOPE_OPS = new Set([
  "port.scope.list_knowledge_entries",
  "port.scope.list_timeline_events",
  "port.fork.list_timeline_events",
]);

/** Whether `op` accepts an optional declared `Scope` (the A2 knowledge ops). */
function optionalScopeOp(op: string): boolean {
  return SCOPE_BEARING_OPS.has(op) && !REQUIRED_SCOPE_OPS.has(op);
}

/**
 * Whether `op` + `payload` require `ke-ownership` in the negotiated set (F2).
 * Malformed Scope payloads return false here — `validateScopeOpPayload` rejects
 * them with `INVALID_INPUT` before provider calls. A non-object payload has no
 * declared viewpoint: the optional-scope knowledge ops still reach this
 * probe (their input failure is answered after the ports presence check), so
 * the read is structural, never a member access on a scalar/`null`.
 */
export function scopeOpRequiresOwnershipCapability(
  op: string,
  payload: Record<string, unknown>,
): boolean {
  if (!SCOPE_BEARING_OPS.has(op) || !isJsonObject(payload)) {
    return false;
  }
  const scope = payload.scope;
  if (typeof scope !== "object" || scope === null || Array.isArray(scope)) {
    return false;
  }
  const viewpoint = (scope as Record<string, unknown>).viewpoint;
  return typeof viewpoint === "string" && viewpoint.length > 0;
}

export function isJsonObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** qc1 F-031 / qc3 F-032 — the adoption Reject channel's revision-typed
 * detail (`details.adopted_revision`) obeys the same `[0, 2^53]`
 * f64-observed domain as the served branches. The guard lives here, at the
 * lower layer both boundaries import, so the serving face AND the responder
 * apply one rule: the responder never trusts an arbitrary injected
 * `RemoteAdoptionService`. An out-of-domain value settles as a
 * provider-fault `INTERNAL_ERROR` (transport kind) instead of travelling
 * on the wire; the receipt is otherwise returned verbatim (JS numbers are
 * already the f64-observed identity). */
export function validateAdoptRejectReceipt(reject: SpokeReject): SpokeReject {
  const details = reject.details;
  if (
    isJsonObject(details) &&
    "adopted_revision" in details &&
    !isRevisionInteger(details.adopted_revision)
  ) {
    return spokeReject(
      SpokeRejectCode.INTERNAL_ERROR,
      "adoption provider returned an error details.adopted_revision outside [0, 2^53]",
      { kind: "transport" },
    );
  }
  return reject;
}

const UNSUPPORTED_COGNITION_MESSAGE = "Unsupported cognition requirement";

const UNAVAILABLE_COGNITION_VIEW_MESSAGE =
  "Unsupported cognition requirement: recorded-at-time source is unavailable";

/**
 * Raw unsupported-requirement diagnosis (consumer contract §2.2 / data
 * contract §5.2).
 *
 * Runs on the RAW payload — before any typed decode could drop or rename an
 * unknown predicate — and rejects the whole request through the dispatch
 * envelope `code = WIRE_INVALID_INPUT` with `details.reason =
 * "unsupported_cognition_requirement"` and `details.field` set to the actual
 * request path. Returns the message and field for that envelope; the caller
 * stamps the wire code, which is not the ops-protocol `SpokeRejectCode`
 * spelling (consumer contract §2.2). The closed grammar itself stays C5's
 * `validateCognitionFilter` / `validateCognitionSelection`, which also names
 * the failing member relative to the validated value; this module only
 * prefixes the carrier location, so no second predicate grammar exists here.
 *
 * Only the dispatched op's declared carrier is inspected
 * (`cognitionRequirementField`), so unrelated payload data is never judged.
 * The cognition carriers serve current-authority KE sources only: the
 * `recorded_at` placement is a Memory source view, so it is an unavailable
 * requested view on these carriers — not an empty result (consumer contract
 * §1, data contract §5.2).
 */
function unsupportedCognitionRequirement(
  op: string,
  payload: Record<string, unknown>,
): { message: string; field: string } | null {
  const location = cognitionRequirementField(op, payload);
  if (location === undefined) {
    return null;
  }
  const scope = payload.scope;
  const value =
    location === "cognition_filter"
      ? payload.cognition_filter
      : isJsonObject(scope)
        ? scope.cognition
        : undefined;
  const validated =
    location === "cognition_filter"
      ? validateCognitionFilter(value)
      : validateCognitionSelection(value);
  if (validated.ok) {
    return location !== "cognition_filter" &&
      isJsonObject(value) &&
      value.view === "recorded_at"
      ? {
          message: UNAVAILABLE_COGNITION_VIEW_MESSAGE,
          field: `${location}.view`,
        }
      : null;
  }
  const relative = validated.details?.field;
  return {
    message: UNSUPPORTED_COGNITION_MESSAGE,
    field:
      typeof relative === "string"
        ? relative.startsWith("[")
          ? `${location}${relative}`
          : `${location}.${relative}`
        : location,
  };
}

function scopePayloadReject(op: string, detail: string): SpokeReject {
  return spokeReject(
    SpokeRejectCode.INVALID_INPUT,
    `invalid ${op} payload: ${detail}`,
    { op },
  );
}

const SCOPE_WIRE_KEYS = new Set([
  "scope_id",
  "cognition",
  "entry_ids",
  "entry_types",
  "timeline_event_ids",
  "source_id",
  "timeline_scale",
  "fork_id",
  "viewpoint",
  "extensions",
]);

const EXTRACT_REQUEST_WIRE_KEYS = new Set([
  "run_id",
  "sources",
  "entry_types",
  "extensions",
]);

const EXTENSION_MAP_KEY_PATTERN = /^[a-z][a-z0-9_-]*$/;

const SOURCE_ANCHOR_WIRE_KEYS = new Set([
  "schema_version",
  "source_id",
  "span",
  "label",
  "mime_type",
  "extensions",
]);

const SOURCE_SPAN_WIRE_KEYS = new Set(["start", "end"]);

function isValidExtensionMap(value: unknown): boolean {
  if (!isJsonObject(value)) {
    return false;
  }
  for (const [key, namespaceValue] of Object.entries(value)) {
    if (!EXTENSION_MAP_KEY_PATTERN.test(key)) {
      return false;
    }
    if (!isJsonObject(namespaceValue)) {
      return false;
    }
  }
  return true;
}

function isStringArray(value: unknown): boolean {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}

export function validateDeclaredScope(scope: Record<string, unknown>): string | null {
  for (const key of Object.keys(scope)) {
    if (!SCOPE_WIRE_KEYS.has(key)) {
      return `unknown property \`${key}\` at scope`;
    }
  }
  if (typeof scope.scope_id !== "string") {
    return "missing field `scope_id`";
  }
  if (
    "entry_ids" in scope &&
    scope.entry_ids !== undefined &&
    !isStringArray(scope.entry_ids)
  ) {
    return "invalid type for scope.entry_ids, expected an array of strings";
  }
  if (
    "entry_types" in scope &&
    scope.entry_types !== undefined &&
    !isStringArray(scope.entry_types)
  ) {
    return "invalid type for scope.entry_types, expected an array of strings";
  }
  if (
    "timeline_event_ids" in scope &&
    scope.timeline_event_ids !== undefined &&
    !isStringArray(scope.timeline_event_ids)
  ) {
    return "invalid type for scope.timeline_event_ids, expected an array of strings";
  }
  if (
    "source_id" in scope &&
    scope.source_id !== undefined &&
    typeof scope.source_id !== "string"
  ) {
    return "invalid type for scope.source_id, expected a string";
  }
  if (
    "timeline_scale" in scope &&
    scope.timeline_scale !== undefined &&
    typeof scope.timeline_scale !== "string"
  ) {
    return "invalid type for scope.timeline_scale, expected a string";
  }
  if (
    "fork_id" in scope &&
    scope.fork_id !== undefined &&
    (typeof scope.fork_id !== "string" || scope.fork_id.length === 0)
  ) {
    return "invalid type for scope.fork_id, expected a non-empty string";
  }
  if (
    "extensions" in scope &&
    scope.extensions !== undefined &&
    !isValidExtensionMap(scope.extensions)
  ) {
    return "invalid type for scope.extensions, expected an ExtensionMap object";
  }
  return null;
}

/** Scope validation for the five remote catalogue ops (F2 malformed rows). */
export function validateScopeOpPayload(
  op: string,
  payload: Record<string, unknown>,
): SpokeReject | null {
  if (!SCOPE_BEARING_OPS.has(op)) {
    return null;
  }
  if (!isJsonObject(payload)) {
    return REQUIRED_SCOPE_OPS.has(op)
      ? scopePayloadReject(op, "payload must be an object")
      : null;
  }
  if (!("scope" in payload)) {
    return REQUIRED_SCOPE_OPS.has(op)
      ? scopePayloadReject(op, "missing scope")
      : null;
  }
  const scope = payload.scope;
  if (!isJsonObject(scope)) {
    return scopePayloadReject(op, "scope must be an object");
  }
  if ("viewpoint" in scope) {
    const viewpoint = scope.viewpoint;
    if (!(typeof viewpoint === "string" && viewpoint.length > 0)) {
      return scopePayloadReject(
        op,
        "scope.viewpoint must be a non-empty string",
      );
    }
  }
  const scopeError = validateDeclaredScope(scope);
  if (scopeError !== null) {
    return scopePayloadReject(op, scopeError);
  }
  if (optionalScopeOp(op) && "cognition" in scope) {
    const selection = validateCognitionSelection(scope.cognition);
    if (!selection.ok) {
      return scopePayloadReject(op, selection.message);
    }
  }
  return null;
}

function validateKnowledgeEntryFields(
  entry: Record<string, unknown>,
): string | null {
  const schemaVersion = entry.schema_version;
  if (
    typeof schemaVersion !== "number" ||
    !Number.isSafeInteger(schemaVersion) ||
    schemaVersion < 1
  ) {
    return "entry.schema_version must be an integer >= 1";
  }
  for (const name of ["entry_id", "canonical_name"]) {
    const value = entry[name];
    if (typeof value !== "string" || value.length === 0) {
      return `entry.${name} must be a non-empty string`;
    }
  }
  for (const name of ["entry_type", "status"]) {
    if (typeof entry[name] !== "string") {
      return `entry.${name} must be a string`;
    }
  }
  for (const name of ["body", "extensions"]) {
    if (!isJsonObject(entry[name])) {
      return `entry.${name} must be an object`;
    }
  }
  if (!isValidKnowledgeEntryDeep(entry)) {
    return "entry does not satisfy the KnowledgeEntry wire shape";
  }
  return null;
}

/**
 * Payload pre-validation for the three knowledge ops, mirroring Rust
 * `decode_knowledge_call`: malformed get/put payloads answer `INVALID_INPUT`
 * before the face probe, admission, and any store call. The list op keeps
 * the existing Scope shape check (`validateScopeOpPayload`, which runs
 * earlier in the port dispatch).
 */
export function validateKnowledgeOpPayload(
  op: string,
  payload: Record<string, unknown>,
): SpokeReject | null {
  if (!isJsonObject(payload)) {
    return scopePayloadReject(op, "payload must be an object");
  }
  if (op === "port.knowledge.get") {
    const entryId = payload.entry_id;
    if (!(typeof entryId === "string" && entryId.length > 0)) {
      return scopePayloadReject(op, "entry_id must be a non-empty string");
    }
    return null;
  }
  if (op === "port.knowledge.put") {
    if (payload.destination_entry_id === null) {
      return scopePayloadReject(
        op,
        "destination_entry_id must be a string or absent, not null",
      );
    }
    if (!isJsonObject(payload.entry)) {
      return scopePayloadReject(op, "entry must be an object");
    }
    const entryError = validateKnowledgeEntryFields(payload.entry);
    if (entryError !== null) {
      return scopePayloadReject(op, entryError);
    }
    const expected = payload.expected_base_revision;
    if (
      expected !== undefined &&
      expected !== null &&
      !isRevisionInteger(expected)
    ) {
      return scopePayloadReject(
        op,
        "expected_base_revision must be a non-negative integer within [0, 2^53] or null",
      );
    }
  }
  return null;
}

function isValidSourceSpan(value: unknown): boolean {
  if (!isJsonObject(value)) {
    return false;
  }
  for (const key of Object.keys(value)) {
    if (!SOURCE_SPAN_WIRE_KEYS.has(key)) {
      return false;
    }
  }
  return (
    typeof value.start === "number" &&
    Number.isFinite(value.start) &&
    typeof value.end === "number" &&
    Number.isFinite(value.end)
  );
}

function isValidSourceAnchor(value: unknown): boolean {
  if (!isJsonObject(value)) {
    return false;
  }
  for (const key of Object.keys(value)) {
    if (!SOURCE_ANCHOR_WIRE_KEYS.has(key)) {
      return false;
    }
  }
  const schemaVersion = value.schema_version;
  if (
    typeof schemaVersion !== "number" ||
    !Number.isSafeInteger(schemaVersion) ||
    schemaVersion < 1
  ) {
    return false;
  }
  if (typeof value.source_id !== "string") {
    return false;
  }
  if (!isValidExtensionMap(value.extensions)) {
    return false;
  }
  if ("span" in value && value.span !== undefined && !isValidSourceSpan(value.span)) {
    return false;
  }
  if ("label" in value && value.label !== undefined && typeof value.label !== "string") {
    return false;
  }
  if (
    "mime_type" in value &&
    value.mime_type !== undefined &&
    typeof value.mime_type !== "string"
  ) {
    return false;
  }
  return true;
}

/** Runtime decode/validate for the `extract` invoke payload (F1/F3 serving order). */
export function validateExtractRequestPayload(
  payload: unknown,
): SpokeReject | null {
  if (!isJsonObject(payload)) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "invalid extract payload: expected a JSON object",
      { op: "extract" },
    );
  }
  for (const key of Object.keys(payload)) {
    if (!EXTRACT_REQUEST_WIRE_KEYS.has(key)) {
      return spokeReject(
        SpokeRejectCode.INVALID_INPUT,
        `invalid extract payload: unknown property \`${key}\``,
        { op: "extract" },
      );
    }
  }
  if (typeof payload.run_id !== "string" || payload.run_id.length === 0) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "ExtractRequest run_id must be a non-empty string",
      { field: "run_id", op: "extract" },
    );
  }
  const sources = payload.sources;
  if (!Array.isArray(sources) || sources.length === 0) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "ExtractRequest sources must be a non-empty SourceAnchor list",
      { field: "sources", op: "extract" },
    );
  }
  if (!sources.every(isValidSourceAnchor)) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "ExtractRequest sources must be a non-empty SourceAnchor list",
      { field: "sources", op: "extract" },
    );
  }
  if (
    "entry_types" in payload &&
    payload.entry_types !== undefined &&
    (!Array.isArray(payload.entry_types) ||
      payload.entry_types.some((entryType) => typeof entryType !== "string"))
  ) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "ExtractRequest entry_types must be an array of strings when present",
      { field: "entry_types", op: "extract" },
    );
  }
  if (
    "extensions" in payload &&
    payload.extensions !== undefined &&
    !isValidExtensionMap(payload.extensions)
  ) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "ExtractRequest extensions must be an ExtensionMap object when present",
      { field: "extensions", op: "extract" },
    );
  }
  return null;
}

/** Remote-only extract service seam (F3). */
export interface RemoteExtractService {
  extract(request: ExtractRequest): Promise<SpokeResult<ExtractResponse>>;
}

/**
 * Remote-only memory service seam (memory protocol §13): the serving host
 * owns the provider, the admission context that turns the authenticated peer
 * into a `Principal` (MEM-R003) and the per-op wire mapping. One callable
 * serves every `memory.*` op; `payload` is the op's request object and the
 * returned value is its response object (Rust parity:
 * `RemoteMemoryService` in `crates/spoke-connect/src/remote/serve_ports.rs`).
 */
export interface RemoteMemoryService {
  invoke(
    peerId: string,
    op: string,
    payload: Record<string, unknown>,
  ): Promise<SpokeResult<unknown>>;
}

/**
 * Remote-only history and adoption service seams (spoke-history contract
 * §Connect operations; spoke-knowledge-adoption contract §Connect
 * operation): the serving faces a responder host attaches to answer the
 * `history.*` ops and `knowledge.adopt`. Each owns peer admission and its
 * connect-layer dedup — the connect idempotency envelope and the closed
 * request decode never reach the provider port. Re-exported from
 * `serve_ports.ts`, which also ships `HistoryPortService` and
 * `AdoptionPortService` adapting a provider port to each seam (mirroring
 * `RemoteMemoryService` / `MemoryPortService`; Rust parity:
 * `RemoteServePorts::as_history` / `::as_adoption`).
 */
export type {
  AdoptionAdmission,
  RemoteAdoptionService,
  HistoryAdmission,
  HistoryCall,
  HistoryDedupStore,
  RemoteHistoryService,
} from "./serve_ports.js";

/**
 * The knowledge serving face: a scoped store plus per-peer admission.
 * `admission(peerId, scopeId)` returns the peer's admitted `Scope` or rejects —
 * the responder then serves only inside that admitted scope; the wire
 * request never carries a scope for the get/put ops and the list op's
 * scope must match the admission exactly. `scopeId` is the scope selector the
 * request declares (`undefined` for an op that names none) and is advisory,
 * never an authorization grant; a one-argument callback stays assignable
 * (Rust parity: `ScopedKnowledgeAdmission`).
 */
export interface RemoteKnowledgeFace {
  store: ScopedKnowledgePort;
  admission: (peerId: string, scopeId?: string) => SpokeResult<Scope>;
}

/**
 * The serving faces a responder host may inject. Baseline ops are served
 * through `baseline`; the three knowledge ops through `knowledge`. An
 * absent face keeps the documented dispatch-deny answer for its family —
 * a knowledge-only host injects only `knowledge`.
 */
export interface RemoteServePorts {
  baseline?: BaselinePorts;
  knowledge?: RemoteKnowledgeFace;
  asHistory?: () => RemoteHistoryService | undefined;
  asAdoption?: () => RemoteAdoptionService | undefined;
}

/** Knowledge-only serve ports: no baseline face, so `assemble` and every other baseline op stays denied. */
export function knowledgeOnlyServePorts(face: RemoteKnowledgeFace): RemoteServePorts {
  return { knowledge: face };
}

/** Attach the knowledge face to existing serve ports (returns a new object). */
export function withKnowledge(ports: RemoteServePorts, face: RemoteKnowledgeFace): RemoteServePorts {
  return { ...ports, knowledge: face };
}

const KNOWLEDGE_OPS: Readonly<Record<string, true>> = {
  "port.knowledge.get": true,
  "port.knowledge.put": true,
  "port.scope.list_knowledge_entries": true,
};

function scopeMatchesAdmission(
  requested: Scope,
  admitted: Scope,
): boolean {
  return (
    requested.scope_id === admitted.scope_id &&
    (requested.viewpoint ?? undefined) === (admitted.viewpoint ?? undefined)
  );
}

const WIDENING = Symbol("widening");

function adoptedConstraint<T>(
  admitted: T | undefined,
  requested: T | undefined,
  equals: (a: T, b: T) => boolean = (a, b) => a === b,
): T | undefined | typeof WIDENING {
  if (admitted === undefined) {
    return requested;
  }
  if (requested === undefined || equals(admitted, requested)) {
    return admitted;
  }
  return WIDENING;
}

function adoptedSetConstraint(
  admitted: string[] | undefined,
  requested: string[] | undefined,
): string[] | undefined | typeof WIDENING {
  if (admitted === undefined) {
    return requested;
  }
  if (requested === undefined) {
    return admitted;
  }
  return requested.every((value) => admitted.includes(value)) ? requested : WIDENING;
}

function intersectedIds(
  admitted: string[] | undefined,
  requested: string[] | undefined,
): string[] | undefined {
  if (admitted === undefined) {
    return requested;
  }
  return requested === undefined
    ? admitted
    : requested.filter((id) => admitted.includes(id));
}

function effectiveScope(requested: Scope, admitted: Scope): Scope | null {
  const entryTypes = adoptedSetConstraint(
    admitted.entry_types,
    requested.entry_types,
  );
  const sourceId = adoptedConstraint(admitted.source_id, requested.source_id);
  const timelineScale = adoptedConstraint(
    admitted.timeline_scale,
    requested.timeline_scale,
  );
  const forkId = adoptedConstraint(admitted.fork_id, requested.fork_id);
  if (
    entryTypes === WIDENING ||
    sourceId === WIDENING ||
    timelineScale === WIDENING ||
    forkId === WIDENING
  ) {
    return null;
  }
  const scope: Scope = {
    ...requested,
    scope_id: admitted.scope_id,
    viewpoint: admitted.viewpoint,
    entry_ids: intersectedIds(admitted.entry_ids, requested.entry_ids),
    timeline_event_ids: intersectedIds(
      admitted.timeline_event_ids,
      requested.timeline_event_ids,
    ),
    entry_types: entryTypes,
    source_id: sourceId,
    timeline_scale: timelineScale,
    fork_id: forkId,
  };
  if (admitted.extensions !== undefined) {
    scope.extensions = { ...requested.extensions, ...admitted.extensions };
  }
  if (scope.viewpoint === undefined) {
    delete scope.viewpoint;
  }
  return scope;
}

const OPTIONAL_PORT_METHODS: Readonly<Record<string, string>> = {
  "port.computable.project": "project",
  "port.computable.compute": "compute",
  "port.fork.list_timeline_events": "listForkTimelineEvents",
};

function probeOptionalPortMethod(
  ports: BaselinePorts,
  op: string,
): string | null {
  const method = OPTIONAL_PORT_METHODS[op];
  if (method === undefined) {
    return null;
  }
  return typeof (ports as unknown as Record<string, unknown>)[method] ===
    "function"
    ? null
    : method;
}

function dispatchPortOp(
  op: string,
  payload: Record<string, unknown>,
  ports: BaselinePorts,
): Promise<SpokeResult<unknown>> {
  switch (op) {
    case "port.relation.get":
      return ports.getRelation(payload.relation_id as string);
    case "port.relation.put":
      {
        const relation = payload.relation;
        if (!isValidSuccessPayload("port.relation.put", relation)) {
          return Promise.resolve(
            spokeReject(
              SpokeRejectCode.INVALID_INPUT,
              "relation must be a well-formed Relation with a revision in [0, 2^53]",
              { field: "relation" },
            ),
          );
        }
        if (!("expected_base_revision" in payload)) {
          return Promise.resolve(
            spokeReject(
              SpokeRejectCode.INVALID_INPUT,
              "expected_base_revision is required (nullable) on port.relation.put",
              { field: "expected_base_revision" },
            ),
          );
        }
        const expected = payload.expected_base_revision;
        if (expected !== null && !isRevisionInteger(expected)) {
          return Promise.resolve(
            spokeReject(
              SpokeRejectCode.INVALID_INPUT,
              "expected_base_revision must be a non-negative integer within [0, 2^53] or null",
              { field: "expected_base_revision" },
            ),
          );
        }
        return ports.putRelation(
          relation as Relation,
          expected as number | null,
        );
      }
    case "port.scope.list_timeline_events":
      return ports.listTimelineEvents(payload.scope as Scope);
    case "port.finding.put":
      return ports.putFindings(payload.findings as Finding[]);
    case "port.rule.list":
      return ports.listRules(payload.rule_refs as string[]);
    case "port.host.list_peer_manifests":
      return ports.listPeerHostCapabilityManifests();
    case "port.computable.project":
      return (ports as unknown as ComputablePort).project(
        payload as unknown as ProjectRequest,
      );
    case "port.computable.compute":
      return (ports as unknown as ComputablePort).compute(
        payload as unknown as ComputeRequest,
      );
    case "port.fork.list_timeline_events":
      return (ports as unknown as ForkTimelineQueryPort).listForkTimelineEvents(
        payload.scope as unknown as Scope & { fork_id: ForkId },
      );
    default:
      return Promise.resolve(
        spokeReject(
          SpokeRejectCode.CAPABILITY_PORT_MISSING,
          `unimplemented port op ${op}`,
          { op },
        ),
      );
  }
}

export type ConnectResponderState =
  | "Disconnected"
  | "Handshaking"
  | "Established"
  | "Closed";

export interface ConnectResponderOptions {
  transport: Transport;
  identity: RemoteIdentity;
  manifest: HostCapabilityManifest;
  allowlist: readonly string[];
  peerKeys: Readonly<Record<string, Uint8Array>>;
  ports?: RemoteServePorts & Partial<RemoteExtractService> & Partial<RemoteMemoryService>;
  invokeTimeoutMs?: number;
}

interface PendingReverseInvoke {
  correlation: Correlation;
  op: string;
  resolve: (response: ConnectInvokeResponse) => void;
  reject: (error: ResponderError) => void;
  timer: ReturnType<typeof setTimeout>;
}

type ServeGateResult =
  | { ok: true }
  | {
      ok: false;
      code: string;
      message: string;
      details?: Record<string, unknown>;
    }
  | null;

/**
 * Connect responder over one established session (frozen contract §6).
 * Construct via `connectResponder` — the factory alone yields an
 * un-established responder whose handshake runs in the background; the
 * dialer's hello is the synchronization point.
 */
export class ConnectResponder {
  readonly #transport: Transport;
  readonly #secret: Uint8Array;
  readonly #manifest: HostCapabilityManifest;
  readonly #allowlist: readonly string[];
  readonly #peerKeys: Readonly<Record<string, Uint8Array>>;
  readonly #ports:
    | (RemoteServePorts & Partial<RemoteExtractService> & Partial<RemoteMemoryService>)
    | undefined;
  readonly #invokeTimeoutMs: number;
  readonly #nonceStore = new NonceStore();

  #stateInternal: ConnectResponderState = "Disconnected";
  #session: Session | null = null;
  #clientPubkey: Uint8Array | null = null;
  #remoteManifestInternal: HostCapabilityManifest | null = null;
  #serveLoopRunning = false;
  #pending = new Map<string, PendingReverseInvoke>();
  #serveDisarms = new Set<() => void>();
  #activeServeBudgetWaits = 0;
  #toolHandlers = new Map<string, ToolHandler>();
  #sendTail: Promise<void> = Promise.resolve();

  constructor(options: ConnectResponderOptions) {
    this.#transport = options.transport;
    this.#secret = options.identity.seed;
    this.#manifest = options.manifest;
    this.#allowlist = options.allowlist;
    this.#peerKeys = options.peerKeys;
    this.#ports = options.ports;
    this.#invokeTimeoutMs = options.invokeTimeoutMs ?? DEFAULT_INVOKE_TIMEOUT_MS;
  }

  /** Read-only session state (frozen contract §6 labels). */
  get state(): ConnectResponderState {
    return this.#stateInternal;
  }

  get sessionId(): string {
    return this.#session?.session_id ?? "";
  }

  get remotePeerId(): string {
    return this.#session?.initiator_peer_id ?? "";
  }

  get remoteManifest(): HostCapabilityManifest {
    if (this.#remoteManifestInternal === null) {
      throw new Error(
        "connect session is not established — remote manifest unavailable",
      );
    }
    return this.#remoteManifestInternal;
  }

  // ── Tool serving + reverse invoke (frozen contract §6) ──────────────────

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
      if (error instanceof ResponderError) {
        return internalError(error.kind, error.message);
      }
      return internalError(
        "transport",
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  activeServeBudgetWaitsForTest(): number {
    return this.#activeServeBudgetWaits;
  }

  close(): void {
    this.#close("local shutdown");
  }

  #begin(): void {
    this.#stateInternal = "Handshaking";
    void this.#run();
  }

  #close(reason: string): void {
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
      void Promise.resolve(closeResult).catch(() => {
      });
    }
    this.#failAllPending(
      new ResponderError("session_closed", `connect session closed: ${reason}`),
    );
    for (const disarm of this.#serveDisarms) {
      disarm();
    }
    this.#serveDisarms.clear();
  }

  #failAllPending(error: ResponderError): void {
    for (const entry of this.#pending.values()) {
      clearTimeout(entry.timer);
      entry.reject(error);
    }
    this.#pending.clear();
  }

  async #handshake(): Promise<void> {
    const helloDoc: unknown = decodeJsonMessage(await this.#transport.recv());
    if (!isConnectHello(helloDoc)) {
      throw new Error("expected ConnectHello from client");
    }
    if (!isAllowlisted(this.#allowlist, helloDoc.peer_id)) {
      throw new Error(`peer ${helloDoc.peer_id} not on allowlist`);
    }
    const clientPubkey = this.#peerKeys[helloDoc.peer_id];
    if (clientPubkey === undefined) {
      throw new Error(
        `no preconfigured public key for allowlisted peer ${helloDoc.peer_id}`,
      );
    }
    const clientPeerId = derivePeerIdFromEd25519Pubkey(clientPubkey);
    await verifyHelloEd25519(clientPubkey, clientPeerId, helloDoc);
    if (!this.#nonceStore.checkAndRecord(helloDoc.peer_id, helloDoc.nonce)) {
      throw new Error("nonce replay");
    }

    const responderPeerId = derivePeerIdFromEd25519Pubkey(
      getPublicKeyEd25519(this.#secret),
    );
    this.#clientPubkey = clientPubkey;
    this.#remoteManifestInternal = helloDoc.host;

    const session = new Session({
      session_id: `${SESSION_ID_PREFIX}${clientPeerId}`,
      initiator_peer_id: clientPeerId,
      responder_peer_id: responderPeerId,
      negotiated_capabilities: negotiatedCapabilities(
        this.#manifest.capabilities,
        helloDoc.host.capabilities,
      ),
    });
    this.#session = session;

    await this.#sendEnvelope(
      await signHelloEd25519(
        this.#secret,
        generateNonce(),
        this.#manifest,
        helloDoc.nonce,
      ),
    );
    const snapshot = await authenticateSession(this.#secret, {
      session_id: session.session_id,
      initiator_peer_id: clientPeerId,
      responder_peer_id: responderPeerId,
      opened_at: new Date().toISOString(),
      negotiated_capabilities:
        session.negotiated_capabilities.length > 0
          ? (session.negotiated_capabilities as [string, ...string[]])
          : (["spoke-baseline"] as [string, ...string[]]),
      initial_sequence: 0,
    });
    await this.#sendEnvelope(snapshot);
  }

  async #run(): Promise<void> {
    try {
      await this.#handshake();
      this.#stateInternal = "Established";
      this.#runServeLoop();
    } catch (error) {
      this.#close(
        `handshake failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }

  #runServeLoop(): void {
    if (this.#serveLoopRunning) {
      return;
    }
    this.#serveLoopRunning = true;
    void this.#serveLoop().finally(() => {
      this.#serveLoopRunning = false;
    });
  }

  async #serveLoop(): Promise<void> {
    while (this.#stateInternal === "Established") {
      let doc: unknown;
      try {
        const bytes = await this.#transport.recv();
        try {
          doc = decodeJsonMessage(bytes);
        } catch (error) {
          this.#close(
            `unparseable inbound message: ${error instanceof Error ? error.message : String(error)}`,
          );
          return;
        }
      } catch {
        this.#close("transport loss");
        return;
      }

      if (isConnectInvokeRequest(doc)) {
        try {
          await this.#serveInvoke(doc);
        } catch (error) {
          this.#close(
            `invoke serving failed: ${error instanceof Error ? error.message : String(error)}`,
          );
          return;
        }
        continue;
      }

      if (isConnectInvokeResponse(doc)) {
        try {
          await this.#demuxResponse(doc);
        } catch {
        }
        continue;
      }
    }
  }

  async #serveInvoke(doc: ConnectInvokeRequest): Promise<void> {
    const gate = await this.#runGate(doc);
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
    void this.#dispatchInvoke(doc);
  }

  async #runGate(doc: ConnectInvokeRequest): Promise<ServeGateResult> {
    const session = this.#session;
    const clientPubkey = this.#clientPubkey;
    if (session === null || clientPubkey === null) {
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
      await verifyInvokeRequestAuth(clientPubkey, doc, session.session_id);
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

  async #dispatchInvoke(doc: ConnectInvokeRequest): Promise<void> {
    const session = this.#session;
    if (session === null) {
      return;
    }
    try {
      if (!this.#gateAllows(doc.op, session)) {
        const memoryBoundary = doc.op.startsWith("memory.");
        const coreGatedDeny =
          memoryBoundary ||
          doc.op.startsWith("history.") ||
          doc.op.startsWith("knowledge.");
        const deny = coreGatedDeny && requiredCapability(doc.op) !== undefined;
        await this.#sendReverseErrorEnvelope(doc, {
          code: deny ? "capability_missing" : "op_unsupported",
          message: `op ${doc.op} is not authorized by this session`,
          extensions: {},
          ...(memoryBoundary ? { details: { retry_class: "terminal" } } : {}),
        });
        return;
      }
      if (doc.op === "extract") {
        await this.#dispatchExtractInvoke(doc);
        return;
      }
      if (doc.op.startsWith("tools.")) {
        await this.#dispatchToolInvoke(doc);
        return;
      }
      if (doc.op.startsWith("memory.")) {
        await this.#dispatchMemoryInvoke(doc);
        return;
      }
      if (doc.op.startsWith("history.")) {
        await this.#dispatchHistoryInvoke(doc);
        return;
      }
      if (doc.op.startsWith("knowledge.")) {
        await this.#dispatchAdoptionInvoke(doc);
        return;
      }
      await this.#dispatchPortInvoke(doc);
    } catch (error) {
      try {
        await this.#sendReverseErrorEnvelope(doc, {
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: `invoke failed: ${error instanceof Error ? error.message : String(error)}`,
          extensions: {},
        });
      } catch {
      }
    }
  }

  #gateAllows(op: string, session: Session): boolean {
    return portDispatchAllowed(op, session.negotiated_capabilities);
  }

  /**
   * Payload-dependent cognition gate (consumer contract §2.1/§2.2), shared by
   * the two cognition-carrying serving branches.
   *
   * `true` means the request may be served; `false` means the deny envelope
   * was already sent and the caller must stop — no typed decode, probe or
   * provider call. The gate runs on the RAW payload and only inspects the
   * op's declared carrier, so a tool's arguments or unrelated product data
   * are never judged. The family comes from the both-sided hello
   * intersection (`l5-mind` never grants it); an unsupported requirement
   * rejects the whole request with its exact request field path.
   */
  async #cognitionGate(
    doc: ConnectInvokeRequest,
    payload: Record<string, unknown>,
    negotiated: readonly string[],
  ): Promise<boolean> {
    if (!cognitionRequirementPresent(doc.op, payload)) {
      return true;
    }
    if (!negotiated.includes(CAPABILITY_SPOKE_COGNITION)) {
      await this.#sendReverseErrorEnvelope(doc, {
        code: "capability_missing",
        message: `op ${doc.op} requires capability ${CAPABILITY_SPOKE_COGNITION} for its declared cognition requirement`,
        extensions: {},
      });
      return false;
    }
    const unsupported = unsupportedCognitionRequirement(doc.op, payload);
    if (unsupported !== null) {
      await this.#sendReverseErrorEnvelope(doc, {
        code: WIRE_INVALID_INPUT,
        message: unsupported.message,
        details: {
          reason: "unsupported_cognition_requirement",
          field: unsupported.field,
        },
        extensions: {},
      });
      return false;
    }
    return true;
  }

  async #dispatchToolInvoke(doc: ConnectInvokeRequest): Promise<void> {
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
  }

  async #withServeBudget<T>(
    work: () => Promise<T>,
    expiryEnvelope?: SpokeReject,
    budgetMs: number = this.#invokeTimeoutMs,
  ): Promise<T | SpokeReject> {
    const expiryReject =
      expiryEnvelope ??
      spokeReject(
        SpokeRejectCode.INTERNAL_ERROR,
        `serve wait exceeded the local budget of ${this.#invokeTimeoutMs}ms`,
        { kind: "timeout" },
      );
    if (budgetMs === 0) {
      return expiryReject;
    }
    this.#activeServeBudgetWaits += 1;
    try {
      const pending = Promise.resolve().then(() => work());
      let rejectExpired: ((reason: unknown) => void) | undefined;
      const expired = new Promise<never>((_resolve, reject) => {
        rejectExpired = reject;
        const timer = setTimeout(() => {
          this.#serveDisarms.delete(cancel);
          reject(SERVE_BUDGET_EXPIRED);
          rejectExpired = undefined;
        }, budgetMs);
        const disarmTimer = (): void => {
          clearTimeout(timer);
        };
        const cancel = (): void => {
          this.#serveDisarms.delete(cancel);
          disarmTimer();
          rejectExpired?.(SERVE_BUDGET_CLOSED);
          rejectExpired = undefined;
        };
        this.#serveDisarms.add(cancel);
        void pending.then(
          () => {
            this.#serveDisarms.delete(cancel);
            disarmTimer();
            rejectExpired = undefined;
          },
          () => {
            this.#serveDisarms.delete(cancel);
            disarmTimer();
            rejectExpired = undefined;
          },
        );
      });
      try {
        return await Promise.race([pending, expired]);
      } catch (error) {
        if (error === SERVE_BUDGET_CLOSED) {
          void pending.then(
            () => undefined,
            () => undefined,
          );
          return expiryReject;
        }
        if (error !== SERVE_BUDGET_EXPIRED) {
          throw error;
        }
        void pending.then(
          () => undefined,
          () => undefined,
        );
        return expiryReject;
      }
    } finally {
      this.#activeServeBudgetWaits -= 1;
    }
  }

  async #dispatchExtractInvoke(doc: ConnectInvokeRequest): Promise<void> {
    const ports = this.#ports;
    const extract = ports?.extract;
    if (ports === undefined || typeof extract !== "function") {
      await this.#sendReverseErrorEnvelope(doc, {
        code: "op_unsupported",
        message: `no extract service configured for op ${doc.op}`,
        extensions: {},
      });
      return;
    }
    const extractReject = validateExtractRequestPayload(doc.payload);
    if (extractReject !== null) {
      await this.#sendReverseErrorEnvelope(doc, toErrorEnvelope(extractReject));
      return;
    }
    const request = doc.payload as ExtractRequest;
    let result: SpokeResult<ExtractResponse>;
    try {
      result = await this.#withServeBudget(() => extract.call(ports, request));
    } catch (error) {
      result = spokeReject(
        SpokeRejectCode.INTERNAL_ERROR,
        error instanceof Error ? error.message : String(error),
      );
    }
    if (this.#stateInternal === "Closed") {
      return;
    }
    if (!result.ok) {
      await this.#sendReverseErrorEnvelope(doc, toErrorEnvelope(result));
      return;
    }
    const response = result.value;
    if (
      typeof response === "object" &&
      response !== null &&
      "error" in response
    ) {
      await this.#sendReverseErrorEnvelope(
        doc,
        toErrorEnvelope(fromErrorEnvelope(response.error)),
      );
      return;
    }
    await this.#sendOkResponse(doc, response);
  }

  async #dispatchMemoryInvoke(doc: ConnectInvokeRequest): Promise<void> {
    const session = this.#session;
    if (session === null) {
      return;
    }
    // Payload-dependent cognition gate (consumer contract §2.1/§2.2) on the
    // raw payload: before the service probe and any provider call, so a
    // denied requirement has no memory side effect.
    if (
      !(await this.#cognitionGate(
        doc,
        isJsonObject(doc.payload) ? doc.payload : {},
        session.negotiated_capabilities,
      ))
    ) {
      return;
    }
    const ports = this.#ports;
    const invoke = ports?.invoke;
    if (ports === undefined || typeof invoke !== "function") {
      await this.#sendReverseErrorEnvelope(doc, {
        code: "op_unsupported",
        message: `no memory service configured for op ${doc.op}`,
        extensions: {},
        details: { retry_class: "terminal" },
      });
      return;
    }
    const peerId = this.remotePeerId;
    const uncertainOnExpiry =
      this.#invokeTimeoutMs === 0
        ? undefined
        : spokeReject(
            SpokeRejectCode.OUTCOME_UNCERTAIN,
            `memory serve wait exceeded the local budget of ${this.#invokeTimeoutMs}ms; the outcome is unknown — poll the receipt or replay the same idempotency key`,
            { retry_class: "uncertain", kind: "timeout" },
          );
    let result: SpokeResult<unknown>;
    try {
      result = await this.#withServeBudget(
        () => invoke.call(ports, peerId, doc.op, doc.payload as Record<string, unknown>),
        uncertainOnExpiry,
      );
    } catch (error) {
      result = spokeReject(
        SpokeRejectCode.OUTCOME_UNCERTAIN,
        `memory service failed after invocation: ${
          error instanceof Error ? error.message : String(error)
        }; the outcome is unknown — poll the receipt or replay the same idempotency key`,
        { retry_class: "uncertain", kind: "panic" },
      );
    }
    if (this.#stateInternal === "Closed") {
      return;
    }
    if (!result.ok) {
      await this.#sendReverseErrorEnvelope(doc, toErrorEnvelope(result));
      return;
    }
    await this.#sendOkResponse(doc, result.value);
  }

  async #dispatchHistoryInvoke(doc: ConnectInvokeRequest): Promise<void> {
    const session = this.#session;
    if (session === null) {
      return;
    }
    if (
      !(await this.#cognitionGate(
        doc,
        isJsonObject(doc.payload) ? doc.payload : {},
        session.negotiated_capabilities,
      ))
    ) {
      return;
    }
    const history = this.#ports?.asHistory?.();
    if (history === undefined) {
      await this.#sendReverseErrorEnvelope(doc, {
        code: "op_unsupported",
        message: `no history service configured for op ${doc.op}`,
        extensions: {},
      });
      return;
    }
    const peerId = this.remotePeerId;
    const isRead = doc.op === "history.revisions.get";
    const uncertainOnExpiry =
      isRead || this.#invokeTimeoutMs === 0
        ? undefined
        : spokeReject(
            SpokeRejectCode.OUTCOME_UNCERTAIN,
            `history serve wait exceeded the local budget of ${this.#invokeTimeoutMs}ms; the outcome is unknown — replay the same idempotency key`,
            { retry_class: "uncertain", kind: "timeout" },
          );
    let result: SpokeResult<unknown>;
    try {
      result = await this.#withServeBudget(
        () => history.invoke(peerId, doc.op, doc.payload),
        uncertainOnExpiry,
      );
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      result = isRead
        ? spokeReject(
            SpokeRejectCode.INTERNAL_ERROR,
            `history service failed after invocation: ${message}`,
          )
        : spokeReject(
            SpokeRejectCode.OUTCOME_UNCERTAIN,
            `history service failed after invocation: ${message}; the outcome is unknown — replay the same idempotency key`,
            { retry_class: "uncertain", kind: "panic" },
          );
    }
    if (this.#stateInternal === "Closed") {
      return;
    }
    if (!result.ok) {
      await this.#sendReverseErrorEnvelope(doc, toErrorEnvelope(result));
      return;
    }
    await this.#sendOkResponse(doc, result.value);
  }

  async #dispatchAdoptionInvoke(doc: ConnectInvokeRequest): Promise<void> {
    const session = this.#session;
    if (session === null) {
      return;
    }
    const adoption = this.#ports?.asAdoption?.();
    if (adoption === undefined) {
      await this.#sendReverseErrorEnvelope(doc, {
        code: "op_unsupported",
        message: `no adoption service configured for op ${doc.op}`,
        extensions: {},
      });
      return;
    }
    const peerId = this.remotePeerId;
    const uncertainOnExpiry =
      this.#invokeTimeoutMs === 0
        ? undefined
        : spokeReject(
            SpokeRejectCode.OUTCOME_UNCERTAIN,
            `adoption serve wait exceeded the local budget of ${this.#invokeTimeoutMs}ms; the outcome is unknown — replay the same idempotency key`,
            { retry_class: "uncertain", kind: "timeout" },
          );
    let result: SpokeResult<unknown>;
    try {
      result = await this.#withServeBudget(
        () => adoption.invoke(peerId, doc.op, doc.payload),
        uncertainOnExpiry,
      );
    } catch (error) {
      result = spokeReject(
        SpokeRejectCode.OUTCOME_UNCERTAIN,
        `adoption service failed after invocation: ${
          error instanceof Error ? error.message : String(error)
        }; the outcome is unknown — replay the same idempotency key`,
        { retry_class: "uncertain", kind: "panic" },
      );
    }
    if (this.#stateInternal === "Closed") {
      return;
    }
    if (!result.ok) {
      // qc3 F-032 — the responder never trusts an arbitrary injected
      // `RemoteAdoptionService`: a Reject receipt's revision-typed detail
      // gets the same `[0, 2^53]` guard as the served branches, so an
      // out-of-domain `details.adopted_revision` answers a provider-fault
      // INTERNAL_ERROR instead of travelling on the wire.
      await this.#sendReverseErrorEnvelope(
        doc,
        toErrorEnvelope(validateAdoptRejectReceipt(result)),
      );
      return;
    }
    await this.#sendOkResponse(doc, result.value);
  }

  async #dispatchPortInvoke(doc: ConnectInvokeRequest): Promise<void> {
    const session = this.#session;
    if (session === null) {
      return;
    }
    const payload = doc.payload as Record<string, unknown>;
    const negotiated = session.negotiated_capabilities;
    const scopeReject = validateScopeOpPayload(doc.op, payload);
    if (scopeReject !== null) {
      await this.#sendReverseErrorEnvelope(doc, toErrorEnvelope(scopeReject));
      return;
    }
    if (
      optionalScopeOp(doc.op) &&
      isJsonObject(payload) &&
      "scope" in payload &&
      !negotiated.includes(CAPABILITY_SPOKE_KNOWLEDGE_SCOPED)
    ) {
      await this.#sendReverseErrorEnvelope(doc, {
        code: "op_unsupported",
        message: `op ${doc.op} requires capability ${CAPABILITY_SPOKE_KNOWLEDGE_SCOPED} for its declared scope`,
        extensions: {},
      });
      return;
    }
    if (
      scopeOpRequiresOwnershipCapability(doc.op, payload) &&
      !negotiated.includes(CAPABILITY_KE_OWNERSHIP)
    ) {
      await this.#sendReverseErrorEnvelope(doc, {
        code: "op_unsupported",
        message: `op ${doc.op} requires capability ${CAPABILITY_KE_OWNERSHIP} for its declared viewpoint`,
        extensions: {},
      });
      return;
    }
    // Payload-dependent cognition gate (consumer contract §2.1/§2.2) on the
    // raw payload: after the original shape/ownership denials and before the
    // provider is probed or called, so a denied requirement has no side
    // effect and an unsupported one keeps its exact request field path.
    if (
      !(await this.#cognitionGate(
        doc,
        isJsonObject(payload) ? payload : {},
        negotiated,
      ))
    ) {
      return;
    }
    const declaresScope = isJsonObject(payload) && "scope" in payload;
    const knowledgePayloadReject = KNOWLEDGE_OPS[doc.op]
      ? validateKnowledgeOpPayload(doc.op, payload)
      : null;
    if (declaresScope && knowledgePayloadReject !== null) {
      await this.#sendReverseErrorEnvelope(doc, toErrorEnvelope(knowledgePayloadReject));
      return;
    }
    const ports = this.#ports;
    if (ports === undefined) {
      await this.#sendReverseErrorEnvelope(doc, {
        code: "op_unsupported",
        message: `no ports face configured for port op ${doc.op}`,
        extensions: {},
      });
      return;
    }
    if (knowledgePayloadReject !== null) {
      await this.#sendReverseErrorEnvelope(doc, toErrorEnvelope(knowledgePayloadReject));
      return;
    }
    if (KNOWLEDGE_OPS[doc.op]) {
      const face = ports.knowledge;
      if (face === undefined) {
        await this.#sendReverseErrorEnvelope(doc, {
          code: "op_unsupported",
          message: `no knowledge face configured for port op ${doc.op}`,
          extensions: {},
        });
        return;
      }
      const peerId = this.remotePeerId;
      if (peerId === "") {
        await this.#sendReverseErrorEnvelope(doc, {
          code: SpokeRejectCode.NOT_AUTHORIZED,
          message: "knowledge scope not authorized",
          extensions: {},
        });
        return;
      }
      const requestedScope = isJsonObject(payload.scope)
        ? // members and viewpoint already validated by `validateScopeOpPayload`
          (payload.scope as unknown as Scope)
        : undefined;
      const declaredScopeId = requestedScope?.scope_id;
      const deadlineMs = Date.now() + this.#invokeTimeoutMs;
      const admitted = face.admission(peerId, declaredScopeId);
      if (!admitted.ok) {
        await this.#sendReverseErrorEnvelope(doc, {
          code: SpokeRejectCode.NOT_AUTHORIZED,
          message: "knowledge scope not authorized",
          extensions: {},
        });
        return;
      }
      let servedScope: Scope | undefined;
      if (requestedScope !== undefined) {
        if (!scopeMatchesAdmission(requestedScope, admitted.value)) {
          await this.#sendReverseErrorEnvelope(doc, {
            code: SpokeRejectCode.NOT_AUTHORIZED,
            message: "knowledge scope not authorized",
            extensions: {},
          });
          return;
        }
        const effective = effectiveScope(requestedScope, admitted.value);
        if (effective === null) {
          await this.#sendReverseErrorEnvelope(doc, {
            code: SpokeRejectCode.NOT_AUTHORIZED,
            message: "knowledge scope not authorized",
            extensions: {},
          });
          return;
        }
        servedScope = effective;
      }
      // An absent scope keeps the admitted one (the `0.18.1` get/put path).
      const storeScope = servedScope ?? admitted.value;
      const remainingMs = Math.max(0, deadlineMs - Date.now());
      const knowledgeUncertainOnExpiry =
        doc.op === "port.knowledge.put" && remainingMs > 0
          ? spokeReject(
              SpokeRejectCode.OUTCOME_UNCERTAIN,
              `knowledge serve wait exceeded the local budget of ${this.#invokeTimeoutMs}ms; the outcome is unknown — re-read the entry to observe the committed revision before replaying the same request`,
              { retry_class: "uncertain", kind: "timeout" },
            )
          : undefined;
      let knowledgeResult: SpokeResult<unknown>;
      try {
        knowledgeResult = await this.#withServeBudget(
          (): Promise<SpokeResult<unknown>> => {
            switch (doc.op) {
              case "port.knowledge.get":
                return face.store.getScopedKnowledgeEntry(
                  storeScope,
                  payload.entry_id as string,
                );
              case "port.knowledge.put":
                return (async (): Promise<SpokeResult<unknown>> => {
                  const candidate = payload.entry as KnowledgeEntry;
                  const storedResult = await face.store.getScopedKnowledgeEntry(
                    storeScope,
                    candidate.entry_id,
                  );
                  let stored: KnowledgeEntry | undefined;
                  if (storedResult.ok) {
                    stored = storedResult.value;
                  } else if (
                    storedResult.code !== SpokeRejectCode.NOT_FOUND &&
                    storedResult.code !==
                      SpokeRejectCode.KNOWLEDGE_ENTRY_NOT_FOUND
                  ) {
                    // A missing destination is a create — no anchor to
                    // preserve; every other read failure stops the write.
                    return storedResult;
                  }
                  const callerBase = (payload.expected_base_revision ??
                    null) as number | null;
                  if (callerBase === null) {
                    if (stored !== undefined) {
                      return spokeReject(
                        SpokeRejectCode.REVISION_CONFLICT,
                        `Entry already exists: ${candidate.entry_id}`,
                        { entry_id: candidate.entry_id },
                      );
                    }
                  } else {
                    if (stored === undefined) {
                      return spokeReject(
                        SpokeRejectCode.STORED_REVISION_STALE,
                        `KnowledgeEntry not found for update: ${candidate.entry_id}`,
                      );
                    }
                    const storedRevision = stored.revision ?? 0;
                    if (storedRevision !== callerBase) {
                      return spokeReject(
                        SpokeRejectCode.STORED_REVISION_STALE,
                        `Store revision ${storedRevision} does not match expected base ${callerBase}`,
                      );
                    }
                  }
                  // The OCC fence passed on this snapshot, so the anchor
                  // gate applies to an update the caller is entitled to
                  // make (spoke-knowledge-adoption contract §Anchor
                  // immutability): a stored `adoption_anchor` is preserved
                  // byte-identically or the write refuses
                  // `ADOPTION_ANCHOR_IMMUTABLE` with `details.entry_id` —
                  // before the put, never after (Rust parity:
                  // `call_knowledge_op` → `assert_adoption_anchor_preserved`).
                  const gate = assertAdoptionAnchorPreserved(candidate, stored);
                  if (!gate.ok) {
                    return gate;
                  }
                  return face.store.putScopedKnowledgeEntry(
                    storeScope,
                    candidate,
                    callerBase,
                  );
                })();
              default:
                return face.store.listScopedKnowledgeEntries(storeScope);
            }
          },
          knowledgeUncertainOnExpiry,
          remainingMs,
        );
      } catch (error) {
        if (doc.op !== "port.knowledge.put") {
          throw error;
        }
        knowledgeResult = spokeReject(
          SpokeRejectCode.OUTCOME_UNCERTAIN,
          "knowledge store failed after invocation; the outcome is unknown — re-read the entry to observe the committed revision before replaying the same request",
          { retry_class: "uncertain", kind: "panic" },
        );
      }
      if (this.#stateInternal === "Closed") {
        return;
      }
      if (knowledgeResult.ok) {
        await this.#sendOkResponse(
          doc,
          "value" in knowledgeResult ? knowledgeResult.value : {},
        );
      } else {
        await this.#sendReverseErrorEnvelope(doc, toErrorEnvelope(knowledgeResult));
      }
      return;
    }
    const baselinePorts = ports.baseline;
    if (baselinePorts === undefined) {
      await this.#sendReverseErrorEnvelope(doc, {
        code: "op_unsupported",
        message: `no ports face configured for port op ${doc.op}`,
        extensions: {},
      });
      return;
    }
    const missingMethod = probeOptionalPortMethod(baselinePorts, doc.op);
    if (missingMethod !== null) {
      await this.#sendReverseErrorEnvelope(doc, {
        code: "op_unsupported",
        message: `port op ${doc.op} requires optional port method ${missingMethod}, which the configured ports do not implement`,
        extensions: {},
      });
      return;
    }
    const result = await this.#withServeBudget(() =>
      dispatchPortOp(doc.op, doc.payload as Record<string, unknown>, baselinePorts),
    );
    if (this.#stateInternal === "Closed") {
      return;
    }
    if (result.ok) {
      await this.#sendOkResponse(doc, "value" in result ? result.value : {});
    } else {
      await this.#sendReverseErrorEnvelope(doc, toErrorEnvelope(result));
    }
  }

  #sendReverseResponse(doc: unknown): void {
    void this.#transport.send(encodeEnvelope(doc)).catch(() => {
    });
  }

  async #sendEnvelope(doc: unknown): Promise<void> {
    await this.#transport.send(encodeEnvelope(doc));
  }

  async #sendOkResponse(
    doc: ConnectInvokeRequest,
    payload: unknown,
  ): Promise<void> {
    this.#sendReverseResponse(
      await authenticateInvokeResponse(
        this.#secret,
        {
          session_id: doc.session_id,
          sequence: doc.sequence,
          request_id: doc.request_id,
          payload: (payload ?? {}) as Record<string, unknown>,
        },
        {},
        doc.op,
      ),
    );
  }

  async #sendReverseErrorEnvelope(
    doc: ConnectInvokeRequest,
    error: ErrorEnvelope,
  ): Promise<void> {
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

  async #demuxResponse(doc: ConnectInvokeResponse): Promise<void> {
    const session = this.#session;
    const clientPubkey = this.#clientPubkey;
    const entry = this.#pending.get(doc.request_id);
    if (entry === undefined) {
      return;
    }
    clearTimeout(entry.timer);
    this.#pending.delete(doc.request_id);
    try {
      if (session === null || clientPubkey === null) {
        throw new ResponderError("session_closed", "connect session closed");
      }
      checkResponseCorrelation(entry.correlation, correlationFromResponse(doc));
      await verifyInvokeResponseAuth(
        clientPubkey,
        doc,
        session.session_id,
        entry.op,
      );
      entry.resolve(doc);
    } catch (error) {
      if (error instanceof EnvelopeAuthError) {
        entry.reject(new ResponderError(error.kind, error.message));
      } else if (error instanceof CoreError && error.code === "crypto") {
        entry.reject(new ResponderError("envelope_auth_invalid", error.message));
      } else if (error instanceof ResponderError) {
        entry.reject(error);
      } else {
        entry.reject(
          new ResponderError(
            "correlation_mismatch",
            "response echo fields do not match the request",
          ),
        );
      }
    }
  }

  async #invokeOp(
    op: string,
    payload: Record<string, unknown>,
  ): Promise<ConnectInvokeResponse> {
    if (this.#stateInternal !== "Established" || this.#session === null) {
      throw new ResponderError(
        "session_closed",
        `connect session is not established (state ${this.#stateInternal})`,
      );
    }
    const session = this.#session;
    let sequence: number;
    try {
      sequence = session.allocateOutboundSequence();
    } catch (error) {
      this.#close("outbound sequence exhausted");
      throw new ResponderError(
        "sequence_exhausted",
        "outbound sequence space exhausted — reopen session",
      );
    }
    // The wire request to sign (`spoke-connect-invoke-request-jcs-v1`,
    // envelope-auth contract §2/§3): the signed object covers
    // `{session_id, sequence, request_id, op, payload}`.
    const unsigned = {
      session_id: session.session_id,
      sequence,
      request_id: globalThis.crypto.randomUUID(),
      op,
      payload,
    };
    return new Promise<ConnectInvokeResponse>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.#pending.delete(unsigned.request_id);
        reject(
          new ResponderError(
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
                this.#close(
                  "deferred invoke skipped (settled while queued) — outbound sequence never transmitted",
                );
                return;
              }
              try {
                return this.#transport.send(encodeEnvelope(signed));
              } catch (error) {
                throw new ResponderError(
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
                  sendError instanceof ResponderError
                    ? sendError
                    : new ResponderError(
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
            new ResponderError(
              "transport",
              `invoke sign failed: ${error instanceof Error ? error.message : String(error)}`,
            ),
          );
        },
      );
    });
  }

  static async connect(options: ConnectResponderOptions): Promise<ConnectResponder> {
    if (options.identity.seed.length !== 32) {
      throw new Error("identity.seed must be 32 bytes");
    }
    const responder = new ConnectResponder(options);
    responder.#begin();
    return responder;
  }
}

/**
 * Start a connect responder over `transport`: performs the signed hello
 * exchange + session snapshot in the background, then serves invokes. The
 * returned responder is in `Handshaking` until the dialer's hello arrives;
 * a handshake rejection closes the transport so the dial fails fast.
 */
export async function connectResponder(
  options: ConnectResponderOptions,
): Promise<ConnectResponder> {
  return ConnectResponder.connect(options);
}

/**
 * @internal Responder.test.ts only — count of in-flight `#withServeBudget`
 * waits. Lets close witnesses prove the budget race settled instead of only
 * clearing its timer.
 */
export function testActiveServeBudgetWaits(responder: ConnectResponder): number {
  return responder.activeServeBudgetWaitsForTest();
}
