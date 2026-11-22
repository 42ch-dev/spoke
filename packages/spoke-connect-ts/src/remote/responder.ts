/**
 * note: `connectResponder` — productized connect responder (frozen contract
 * `tool-contracts.md` §6, ported from the demo-server recipe
 * `examples/connect-demo/server/src/host/connect-host.ts`).
 *
 * PUBLIC surface: the `connectResponder(options)` factory + the responder
 * object's `registerToolHandler`, `invokeTool(capabilityId, arguments)`
 * (the reverse face), read-only session info (`sessionId`,
 * `remotePeerId`, `remoteManifest`, `state`), and `close`. Consumers never
 * touch envelope-auth / sequence internals.
 *
 * The responder owns the server end of a message-oriented `Transport`:
 *
 *   handshake(): allowlist fail-closed check FIRST → hello verify (peer_id
 *   binding via the preconfigured `peerKeys` pubkey) → nonce replay record
 *   → dial-bound responder hello (5-field signed object carrying the
 *   initiator's nonce as `peer_nonce`) → signed `ConnectSession` snapshot.
 *   Empty-intersection fallback preserved verbatim from the demo: the wire
 *   snapshot requires ≥1 negotiated capability, so a degenerate
 *   empty-intersection dial emits `["spoke-baseline"]` — the dialer
 *   derives its own set from the hellos, so the fallback has no
 *   authorization impact (documented carried-over behavior).
 *
 *   serve(): per inbound invoke, a serialized gate (peek → verify →
 *   advance; the async verify cannot interleave with a concurrent invoke)
 *   followed by a concurrent dispatch phase: `tools.*` ops run the
 *   registered tool handler (or deny `op_unsupported`), `port.*` ops run
 *   the D4 catalogue against the injected async `BaselinePorts` (absent
 *   `ports` still answers the dispatch-deny branch), and unknown ops are
 *   denied. A failed envelope-auth verify produces no handler side effect
 *   and no session-state mutation (auth-before-advance, spec §Verify
 *   rules). Every served `port.*` provider call and the whole `extract`
 *   service call are bounded by the responder's local serve-wait budget
 *   (`invokeTimeoutMs`, default 5000): expiry answers exactly one signed
 *   `INTERNAL_ERROR` with `details.kind = "timeout"` and leaves the session
 *   Established (bounded wait, not forced host-work termination). An
 *   unparseable inbound frame closes the connection (carried over from the
 *   demo).
 *
 *   invokeTool(): the reverse face — outbound counter, request signing,
 *   send-tail wire-order serialization, response correlation +
 *   envelope-auth verify, per-waiter timeout, and the deferred-send
 *   poison-close mirror (a waiter that settles while its send is still
 *   queued means the allocated outbound sequence never hit the wire —
 *   close the session, same semantics as the adapter's `#invokeOp`).
 */

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
  capabilitySatisfies,
  cognitionRequirementField,
  cognitionRequirementPresent,
  dispatchAllowed,
  requiredCapability,
  reservedPrefixOp,
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
import { isValidKnowledgeEntryDeep } from "./payload.js";
import type { RemoteAdoptionService, RemoteHistoryService } from "./serve_ports.js";
import type { RemoteIdentity, ToolHandler } from "./remote-adapter.js";
import type { EnvelopeBytes, Transport } from "./transport.js";


const DEFAULT_INVOKE_TIMEOUT_MS = 5000;

/** note: session id prefix — deterministic per dialing peer (opaque, not schema-enforced). */
const SESSION_ID_PREFIX = "connect-responder-session-";

const textEncoder = new TextEncoder();

function encodeEnvelope(doc: unknown): EnvelopeBytes {
  return textEncoder.encode(encodeJsonMessage(doc));
}

/**
 * note: internal invoke-failure classes — consumers only ever observe these mapped
 * to `SpokeResult` `INTERNAL_ERROR` rejects with `details.kind`.
 */
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

/**
 * note: sentinel rejection marking local serve-budget expiry, so the timed-await
 * helper can tell the budget from a provider failure.
 */
const SERVE_BUDGET_EXPIRED = Symbol("serve-budget-expired");

/**
 * note: sentinel rejection marking close-driven serve-budget cancellation, so
 * `#withServeBudget` can settle its race without answering the invoke.
 */
const SERVE_BUDGET_CLOSED = Symbol("serve-budget-closed");

/**
 * note: dispatch-deny wire codes (D7) — the peer answered that the op or its
 * required capability is not available → `CAPABILITY_PORT_MISSING`.
 */
const DISPATCH_DENY_CODES = new Set(["op_unsupported", "capability_missing"]);

function mapErrorEnvelope(error: ErrorEnvelope): SpokeReject {
  if (DISPATCH_DENY_CODES.has(error.code)) {
    return spokeReject(SpokeRejectCode.CAPABILITY_PORT_MISSING, error.message, {
      ...(error.details ?? {}),
      wire_code: error.code,
    });
  }
  // why: `auth_failed` is not a SpokeRejectCode, so
  // `fromErrorEnvelope` would map it to INVALID_INPUT; the locked mapping
  // is `INTERNAL_ERROR` with the envelope-auth `details.kind` verbatim.
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
  // note: optional families (D4 catalogue) — project / compute require the
  // `l2-computable` family capability; the fork timeline query requires
  // `l5-fork` — all evaluated against `negotiated_capabilities` (the
  // both-hello intersection) by `#gateAllows`.
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
  // note: QC2-16 / F-018 — a `knowledge.*` name with no core-table row is
  // a reserved unknown: the core decision is terminal and answers
  // `op_unsupported` BEFORE the product map is consulted, so a
  // conflicting product-map row can never advance it (Rust parity: the
  // composed gate's reserved-prefix short-circuit).
  if (reservedPrefixOp(op)) {
    return false;
  }
  if (dispatchAllowed(op, negotiated)) {
    return true;
  }
  const required = productMap[op];
  return required !== undefined && capabilitySatisfies(negotiated, required);
}

/**
 * note: optional-family op → method on the injected `ports` object (D4
 * catalogue). Serving order is gate → probe → serve/deny: after the
 * capability gate passes, the responder structurally probes the injected
 * ports object for these methods (the `requirePortMethod` pattern from
 * `spoke-operations/src/adapter/orchestrate.ts`, re-implemented locally —
 * module-private there). Baseline ops are not probed: the `BaselinePorts`
 * type guarantees their methods.
 */

/** Capability required when a Scope-bearing remote op carries viewpoint. */
export const CAPABILITY_KE_OWNERSHIP = "ke-ownership";

const SCOPE_OWNERSHIP_OPS = new Set([
  "port.scope.list_knowledge_entries",
  "port.scope.list_timeline_events",
  "port.fork.list_timeline_events",
]);

/**
 * Whether `op` + `payload` require `ke-ownership` in the negotiated set (F2).
 * Malformed Scope payloads return false here — `validateScopeOpPayload` rejects
 * them with `INVALID_INPUT` before provider calls.
 */
export function scopeOpRequiresOwnershipCapability(
  op: string,
  payload: Record<string, unknown>,
): boolean {
  if (!SCOPE_OWNERSHIP_OPS.has(op)) {
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
  // why: C5 is the single grammar and names the failing member relative to the
  // validated value; this boundary only prefixes the carrier location.
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

/** note: structural decode/validate for a declared Scope (mirrors Rust `Scope::deserialize`); exported for the history serving face (`serve_ports.ts`). */
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

/** Scope validation for the three remote catalogue ops (F2 malformed rows). */
export function validateScopeOpPayload(
  op: string,
  payload: Record<string, unknown>,
): SpokeReject | null {
  if (!SCOPE_OWNERSHIP_OPS.has(op)) {
    return null;
  }
  if (!("scope" in payload)) {
    return scopePayloadReject(op, "missing scope");
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
  return null;
}

/**
 * note: required `KnowledgeEntry` members of a `port.knowledge.put` payload. Rust
 * decodes the typed struct, so a missing or mistyped member is `INVALID_INPUT`
 * before admission; these are the members that decode enforces.
 */
function validateKnowledgeEntryFields(
  entry: Record<string, unknown>,
): string | null {
  const schemaVersion = entry.schema_version;
  if (
    typeof schemaVersion !== "number" ||
    !Number.isInteger(schemaVersion) ||
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
  // note: QC2-15 / F-016 — the fast field gate above only sees the top
  // level; the FULL wire-shape check must also run here, before any
  // stored-entry read, OCC fence or anchor comparison (contract D1: (a)
  // malformed → (b) OCC → (c) anchor). Rust's typed entry decoder
  // (`deny_unknown_fields`) refuses a malformed nested member at decode —
  // e.g. `body.tags: 5` is `INVALID_INPUT`, never a stored-state refusal.
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
    // note: the scoped-put wire-wide precedence (contract D1 / F-016) —
    // an explicit `null` `destination_entry_id` is malformed input,
    // answered INVALID_INPUT before any stored-state read, OCC fence or
    // anchor comparison (Rust parity: the raw-payload null check in
    // `decode_knowledge_call`).
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
      !(
        typeof expected === "number" &&
        Number.isSafeInteger(expected) &&
        expected >= 0
      )
    ) {
      return scopePayloadReject(
        op,
        "expected_base_revision must be a non-negative safe integer or null",
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
    !Number.isInteger(schemaVersion) ||
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
 * `admission(peerId)` returns the peer's admitted `Scope` or rejects —
 * the responder then serves only inside that admitted scope; the wire
 * request never carries a scope for the get/put ops and the list op's
 * scope must match the admission exactly.
 */
export interface RemoteKnowledgeFace {
  store: ScopedKnowledgePort;
  admission: (peerId: string) => SpokeResult<Scope>;
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
  /**
   * note: optional `spoke-history` service face (revision replay +
   * explicit pruning with dedup), when provided — attach a provider
   * `HistoryPort` via `HistoryPortService` (`serve_ports.ts`). Absent
   * means the host declared `spoke-history` without providing the
   * service — a `None` probe answers the documented `op_unsupported`
   * dispatch-deny branch (Rust parity: `RemoteServePorts::as_history`).
   */
  asHistory?: () => RemoteHistoryService | undefined;
  /**
   * note: optional `spoke-knowledge-adoption` service face
   * (`knowledge.adopt` with connect idempotency dedup), when provided —
   * attach a provider `AdoptionPort` via `AdoptionPortService`
   * (`serve_ports.ts`). Absent means the host declared
   * `spoke-knowledge-adoption` without providing the service — an
   * `undefined` probe answers the documented `op_unsupported`
   * dispatch-deny branch (Rust parity: `RemoteServePorts::as_adoption`).
   */
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

/** note: the three remote knowledge ops — served only through the knowledge face. */
const KNOWLEDGE_OPS: Readonly<Record<string, true>> = {
  "port.knowledge.get": true,
  "port.knowledge.put": true,
  "port.scope.list_knowledge_entries": true,
};

/**
 * invariant: scope equality — same `scope_id` AND the same `viewpoint`
 * (both absent or equal strings). Any mismatch — including an admitted
 * viewpoint the request omits — refuses.
 */
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

/**
 * note: adopt one admitted constraint — the admitted value when the request omits or
 * repeats it, the request's value when admission omits it, `WIDENING` when the
 * request names a different value (a widening, never a narrowing).
 */
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

/**
 * note: adopt an admitted set-valued constraint — a requested set that is a subset of
 * the admitted one narrows the query and is used as given; a requested set
 * naming any value outside the admitted one is a widening (`WIDENING`).
 */
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

/** note: admitted ∩ requested ids — an empty intersection stays an empty filter. */
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

/**
 * note: the scope passed to the store for a list — the admitted `scope_id` /
 * `viewpoint` are authoritative (equal to the request — the caller checked)
 * and every other admitted constraint bounds the request. `null` refuses: the
 * request named a scalar value the admission does not grant, and honouring it
 * would widen the query. Id lists intersect; admitted `extensions` namespaces
 * win on conflict; the request's cognition selection is untouched.
 */
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

/**
 * note: structural probe (the `requirePortMethod` pattern) — return the missing
 * optional-port method name when the injected `ports` object lacks it, or
 * null when present / the op is not an optional-family op. Called AFTER
 * the capability gate.
 */
function probeOptionalPortMethod(
  ports: BaselinePorts,
  op: string,
): string | null {
  const method = OPTIONAL_PORT_METHODS[op];
  if (method === undefined) {
    return null; // note: baseline op — always present on BaselinePorts
  }
  return typeof (ports as unknown as Record<string, unknown>)[method] ===
    "function"
    ? null
    : method;
}

/**
 * note: map a `port.*` op + payload to the injected adapter method per the D4
 * catalogue. The dispatch gate (capability check) and the optional-method
 * probe (gate → probe → serve/deny) have already run when this is called;
 * unknown ops reject `CAPABILITY_PORT_MISSING` as a safety net for host
 * misconfiguration (the gate denies them first).
 */
function dispatchPortOp(
  op: string,
  payload: Record<string, unknown>,
  ports: BaselinePorts,
): Promise<SpokeResult<unknown>> {
  switch (op) {
    case "port.relation.get":
      return ports.getRelation(payload.relation_id as string);
    case "port.relation.put":
      return ports.putRelation(
        payload.relation as Relation,
        payload.expected_base_revision as number | null,
      );
    case "port.scope.list_timeline_events":
      return ports.listTimelineEvents(payload.scope as Scope);
    case "port.finding.put":
      return ports.putFindings(payload.findings as Finding[]);
    case "port.rule.list":
      return ports.listRules(payload.rule_refs as string[]);
    case "port.host.list_peer_manifests":
      return ports.listPeerHostCapabilityManifests();
    // note: optional families (D4 catalogue). The serving method
    // (`#dispatchPortInvoke`) structurally probed the injected ports
    // object BEFORE dispatch (gate → probe → serve/deny), so these cases
    // are reachable only when the method exists on the provider.
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
      // note: unreachable when the dispatch gate denies unknown ops first; kept
      // as a safety net for host misconfiguration (mirrors the demo).
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
  /** note: message-oriented transport (consumer-provided; loopback ships in-repo). */
  transport: Transport;
  /** note: this responder's raw Ed25519 seed (hello + envelope-auth identity). */
  identity: RemoteIdentity;
  /** note: host manifest advertised in the responder's signed hello. */
  manifest: HostCapabilityManifest;
  /** note: trusted dialer peer ids (fail-closed allowlist). */
  allowlist: readonly string[];
  /**
   * note: preconfigured dialer Ed25519 public keys by peer_id — key distribution
   * is transport-adapter-owned per the spec; the responder knows its
   * trusted identities statically. A dialing peer on `allowlist` without a
   * preconfigured key fails the handshake (fail-closed).
   */
  peerKeys: Readonly<Record<string, Uint8Array>>;
  /**
   * note: local serving faces (D4 catalogue + knowledge face) — absent `ports`
   * still answers `port.*` invokes with the dispatch-deny branch
   * (documented behavior). Baseline ops require `ports.baseline`; the
   * three knowledge ops require `ports.knowledge` — a knowledge-only host
   * injects only that face via `knowledgeOnlyServePorts`.
   */
  ports?: RemoteServePorts & Partial<RemoteExtractService> & Partial<RemoteMemoryService>;
  /**
   * note: bounded-wait deadline for each reverse-invoke waiter AND the local
   * serve-wait budget bounding every served `port.*` provider call and the
   * whole `extract` service call, ms (default 5000). One value, two uses —
   * a host setting zero for immediate serve-timeout semantics also collapses
   * the reverse-invoke waiter deadline: zero answers the serve timeout with
   * zero provider calls AND expires every reverse-invoke waiter
   * immediately, so it is not a serve-only switch. The value is local: it
   * does not inherit the dialer's budget and does not promise the reply
   * beats the dialer's timeout.
   */
  invokeTimeoutMs?: number;
}

interface PendingReverseInvoke {
  correlation: Correlation;
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

  // invariant: verification-gating state — all of it is ECMAScript `#`-private: no JS
  // consumer can write a `#`-slot from outside the class — a forged
  // `Established` / forged `session` / forged remote-manifest cache is
  // unreachable even through an `any` cast or a subclass.
  #stateInternal: ConnectResponderState = "Disconnected";
  #session: Session | null = null;
  /** note: the verified dialer's hello public key (set at handshake). */
  #clientPubkey: Uint8Array | null = null;
  #remoteManifestInternal: HostCapabilityManifest | null = null;
  #serveLoopRunning = false;
  #pending = new Map<string, PendingReverseInvoke>();
  /**
   * note: armed serve-budget disarms — tracked so `#close` cancels every serve wait
   * it still owns: no serve wait's timer outlives the session it was armed
   * for. An entry removes itself when it fires or when its provider settles.
   */
  #serveDisarms = new Set<() => void>();
  /** note: in-flight `#withServeBudget` waits (test observable via export below). */
  #activeServeBudgetWaits = 0;
  /**
   * note: tool-handler registry for `tools.*` invokes (frozen contract §6) —
   * `registerToolHandler` fills it; the serving path looks it up by exact
   * capability id. The local manifest's `tools[]` (carried through hello)
   * is the discovery source — this registry MUST NOT mutate the manifest;
   * a registry/manifest mismatch is surfaced at invoke time: a
   * manifest-declared tool with no registered handler is denied
   * fail-closed (op_unsupported → CAPABILITY_PORT_MISSING);
   * `validateManifestTools` checks manifest-internal consistency only.
   */
  #toolHandlers = new Map<string, ToolHandler>();
  /**
   * invariant: outbound send serialization tail — sequences are allocated synchronously
   * in call order, but the Ed25519 sign is async (WebCrypto) — without a
   * chain, signs complete out of order and requests hit the wire
   * out of sequence, which the peer's strict inbound gate rejects. Every
   * reverse invoke chains its send behind the previous invoke's send.
   */
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

  /** note: the assigned session id (empty before establishment). */
  get sessionId(): string {
    return this.#session?.session_id ?? "";
  }

  /** note: the verified dialing peer id (empty before establishment). */
  get remotePeerId(): string {
    return this.#session?.initiator_peer_id ?? "";
  }

  /**
   * note: the dialing peer's `HostCapabilityManifest`, from the authenticated
   * hello `host` (discovery after auth). Throws when accessed before
   * establishment (programmer misuse — a handshaken responder always has
   * one).
   */
  get remoteManifest(): HostCapabilityManifest {
    if (this.#remoteManifestInternal === null) {
      throw new Error(
        "connect session is not established — remote manifest unavailable",
      );
    }
    return this.#remoteManifestInternal;
  }

  // ── Tool serving + reverse invoke (frozen contract §6) ──────────────────

  /**
   * note: register a handler for a `tools.<ns>.<tool_id>` capability served on
   * this responder. Grammar-asserted: a non-`tools.` id throws (programmer
   * misuse, like the generated-type ergonomics of `toolCapabilityId`).
   * Duplicate registration for the same id OVERWRITES the previous handler
   * (last-wins, documented). The registry does NOT mutate the local
   * manifest — descriptor truth for discovery stays in the manifest's
   * `tools[]` (sent through hello).
   */
  registerToolHandler(capabilityId: string, handler: ToolHandler): void {
    const parsed = parseToolCapabilityId(capabilityId);
    if (!parsed.ok) {
      throw new Error(parsed.message);
    }
    this.#toolHandlers.set(capabilityId, handler);
  }

  /**
   * note: reverse tool-invoke face (frozen contract §6) — issue a
   * `ConnectInvokeRequest` with `op = capabilityId` toward the dialer and
   * resolve with the tool's `result` (extracted from the success
   * `payload = { result: <opaque JSON> }`). Deny answers map via the
   * existing D7 row (`op_unsupported` / `capability_missing` →
   * `CAPABILITY_PORT_MISSING` with `details.wire_code` preserved).
   */
  async invokeTool(
    capabilityId: string,
    args: Record<string, unknown>,
  ): Promise<SpokeResult<unknown>> {
    // why: fail fast on a non-tool capability id — the op string IS the
    // capability string; a non-`tools.` id is a programming error).
    const parsed = parseToolCapabilityId(capabilityId);
    if (!parsed.ok) {
      return parsed;
    }
    try {
      const response = await this.#invokeOp(capabilityId, { arguments: args });
      if ("error" in response) {
        return mapErrorEnvelope(response.error);
      }
      // note: tool success-payload gate (frozen §4) — success is
      // `payload = { "result": <opaque JSON> }`; a success payload without
      // a `result` key rejects with `INTERNAL_ERROR` `details.kind =
      // "transport"` (mirrors the adapter's `#invokeMapped` shape gate).
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

  /**
   * note: @internal Responder.test.ts only — in-flight `#withServeBudget` waits.
   */
  activeServeBudgetWaitsForTest(): number {
    return this.#activeServeBudgetWaits;
  }

  /** note: release the session and transport. Idempotent. */
  close(): void {
    this.#close("local shutdown");
  }

  // note: session lifecycle (hard-private — only `connectResponder` starts)

  /** note: factory-only — start the handshake + serve loop in the background. */
  #begin(): void {
    this.#stateInternal = "Handshaking";
    void this.#run();
  }

  /**
   * note: all failure paths that make the session unusable — transitions to
   * `Closed`, settles every pending waiter, and releases the transport
   * (fire-and-forget).
   */
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
        // why: transport close failure is intentionally swallowed — the session
        // is unusable either way and the responder already settled waiters.
      });
    }
    this.#failAllPending(
      new ResponderError("session_closed", `connect session closed: ${reason}`),
    );
    // why: serve waits owned by this session — cancel their armed budgets and
    // settle the `Promise.race` so fire-and-forget dispatches do not hang
    // when the provider never settles. A pending provider completion that
    // arrives afterwards is discarded by the closed-state check in the serve
    // dispatch (no late success, no late second error).
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

  // note: handshake (allowlist-first → hello verify → nonce → dial-bound
  // responder hello → signed session snapshot)

  async #handshake(): Promise<void> {
    const helloDoc: unknown = decodeJsonMessage(await this.#transport.recv());
    if (!isConnectHello(helloDoc)) {
      throw new Error("expected ConnectHello from client");
    }
    // safety: allowlist fail-closed check FIRST — an untrusted peer is rejected
    // before any signature work.
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

    // why: the responder hello signs the 5-field object incl. `peer_nonce` = the initiator's
    // nonce, so a captured responder hello cannot be replayed into a fresh
    // dial. Then the signed session snapshot.
    await this.#sendEnvelope(
      await signHelloEd25519(
        this.#secret,
        generateNonce(),
        this.#manifest,
        helloDoc.nonce,
      ),
    );
    // note: the session snapshot is signed with the responder identity
    // (`spoke-connect-session-jcs-v1`) — the dialer verifies it against
    // this responder's hello public key. The wire snapshot requires ≥1
    // negotiated capability; the dialer derives its own set from the
    // hellos, so the fallback only covers a degenerate empty-intersection
    // dial (carried-over demo behavior, no authorization impact).
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

  // note: serve loop (request-first classification)

  async #run(): Promise<void> {
    try {
      await this.#handshake();
      this.#stateInternal = "Established";
      this.#runServeLoop();
    } catch (error) {
      // why: handshake failure (or responder misconfiguration) fails the peer's
      // pending recv like a connection drop.
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
          // why: unparseable inbound closes the connection per the carried-over
          // demo semantics — a bare return would leave the socket open
          // while the responder stops recv'ing, and the dialer's
          // established session would hang on its next invoke.
          this.#close(
            `unparseable inbound message: ${error instanceof Error ? error.message : String(error)}`,
          );
          return;
        }
      } catch {
        // why: transport closed — the session is unusable; fail pending waiters.
        this.#close("transport loss");
        return;
      }

      // invariant: classify request shape FIRST (normative `spoke-connect.md`
      // §Request / response classification): an inbound envelope carrying
      // `op` is a `ConnectInvokeRequest` — never a response, even though a
      // reverse request carries the same correlation echo fields + payload
      // as the response success branch.
      if (isConnectInvokeRequest(doc)) {
        // invariant: gate serialization — await peek → verify → advance inline; the
        // loop reads the next envelope only after this gate completes.
        // Dispatch fires without blocking the loop.
        try {
          await this.#serveInvoke(doc);
        } catch (error) {
          // why: unexpected serving failure (local key misconfiguration /
          // internal bug): fail closed like transport loss — a live
          // session must not silently drop invokes.
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
          // why: demux failures are contained per-waiter; nothing here should
          // escape, but never crash the loop.
        }
        continue;
      }

      // note: post-handshake stray envelope (hello / session / unknown shape) is
      // ignored.
    }
  }

  /**
   * invariant: serve one inbound invoke per the canonical order (frozen §4) —
   * classify (caller) → stray → sequence peek → envelope-auth verify →
   * advance → gate → handler → signed response. The caller (serve loop)
   * awaits this method inline so peek → verify → advance are serialized;
   * dispatch fires without blocking the loop.
   */
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

  /**
   * invariant: gate phase — sequence peek (non-mutating) → envelope-auth verify →
   * advance. Fail-closed (auth-before-advance, spec §Verify rules): a
   * forged/tampered/stripped signature answers `auth_failed` and leaves the
   * inbound counter unchanged. Returns `null` for stray requests (ignored),
   * a rejection spec for gate failures, or `{ok: true}` when dispatch may
   * run.
   */
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
      // why: a wrong-length key is responder misconfiguration — fail loudly.
      throw error;
    }
    // safety: advance the inbound counter only after envelope-auth verify passed.
    session.acceptInboundSequence(doc.sequence);
    return { ok: true };
  }

  /**
   * note: dispatch phase — runs after the serialized gate; may interleave with
   * other invokes. Fully contained: a throwing handler answers the error
   * branch and never crashes the loop.
   */
  async #dispatchInvoke(doc: ConnectInvokeRequest): Promise<void> {
    const session = this.#session;
    if (session === null) {
      return;
    }
    try {
      // note: dispatch gate — `Session.dispatchAllowed`-level logic (frozen §3) —
      // `tools.*` ops require the op string itself; baseline `port.*` ops
      // require `spoke-baseline`; optional families route via the same
      // product map to `l2-computable` / `l5-fork`. All evaluate against
      // `negotiated_capabilities` (never a raw requirements-map composition,
      // which would deny the self-describing tools family).
      if (!this.#gateAllows(doc.op, session)) {
        // note: a known `memory.*` or `history.*` op is a capability gap
        // (protocol §4.1 / spoke-history contract) — the gate evaluates
        // negotiated capabilities only, so an op with a dispatch row that
        // was not negotiated answers `capability_missing`. An op with no
        // row at all stays `op_unsupported`.
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
          // note: §14.2 — every memory-boundary denial (known or unknown op) is
          // classified `terminal`, so the code choice never drops the metadata.
          // History denials carry no extra details.
          ...(memoryBoundary ? { details: { retry_class: "terminal" } } : {}),
        });
        return;
      }
      // note: the payload-dependent cognition gate lives inside the three
      // cognition-carrying branches (`#dispatchPortInvoke` /
      // `#dispatchMemoryInvoke` / `#dispatchHistoryInvoke`), after their own
      // op/ownership denials and
      // before any typed decode, provider probe or handler call: a tool's
      // arguments or an unrelated product payload never reach it.
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
      // note: `history.*` ops are served by the optional injected
      // `RemoteHistoryService` face (spoke-history contract) — the same
      // gate → probe → serve/deny order as the optional `port.*` families.
      if (doc.op.startsWith("history.")) {
        await this.#dispatchHistoryInvoke(doc);
        return;
      }
      // note: `knowledge.adopt` is served by the optional injected
      // `RemoteAdoptionService` face (spoke-knowledge-adoption contract) —
      // the same gate → probe → serve/deny order as the `history.*`
      // family; the `knowledge.*` reserved-product prefix owns no other
      // row, so other `knowledge.*` ops keep falling through to the
      // `port.*` dispatch-deny path.
      if (doc.op.startsWith("knowledge.")) {
        await this.#dispatchAdoptionInvoke(doc);
        return;
      }
      await this.#dispatchPortInvoke(doc);
    } catch (error) {
      // why: unexpected serving failure (e.g. non-JSON-serializable handler
      // result): answer the error branch so the invoker never hangs, and
      // never crash the loop.
      try {
        await this.#sendReverseErrorEnvelope(doc, {
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: `invoke failed: ${error instanceof Error ? error.message : String(error)}`,
          extensions: {},
        });
      } catch {
        // note: peer gone — responses are fire-and-forget.
      }
    }
  }

  /** note: dispatch gate — the named combined gate (core table incl. `tools.*`, then the port product map). */
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

  /** note: serve a `tools.*` invoke through the registered handler (or deny). */
  async #dispatchToolInvoke(doc: ConnectInvokeRequest): Promise<void> {
    // invariant: handler or deny — fail-closed serving (frozen deny matrix) — a gate
    // pass with no registered handler answers `op_unsupported`.
    const handler = this.#toolHandlers.get(doc.op);
    if (handler === undefined) {
      await this.#sendReverseErrorEnvelope(doc, {
        code: "op_unsupported",
        message: `no handler registered for ${doc.op}`,
        extensions: {},
      });
      return;
    }
    // note: the request payload carries the tool arguments as
    // `{ "arguments": <opaque JSON> }` (frozen §4). A non-object
    // arguments field is a malformed provider request — serve `{}`
    // (the structural argument gate is caller-side). The unchecked cast is
    // deliberate: `payload` is any-JSON on the wire, so a null / malformed
    // value keeps its pre-existing property-access outcome (serving error path).
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
      // why: a handler throw answers the error branch via toErrorEnvelope (INTERNAL_ERROR
      // for a crash); never crashes the loop (frozen §6).
      result = spokeReject(
        SpokeRejectCode.INTERNAL_ERROR,
        error instanceof Error ? error.message : String(error),
      );
    }
    if (result.ok) {
      // note: success branch — `payload = { "result": <opaque JSON> }` (frozen §4).
      this.#sendReverseResponse(
        await authenticateInvokeResponse(this.#secret, {
          session_id: doc.session_id,
          sequence: doc.sequence,
          request_id: doc.request_id,
          payload: {
            result: "value" in result ? result.value : undefined,
          },
        }),
      );
    } else {
      await this.#sendReverseErrorEnvelope(doc, toErrorEnvelope(result));
    }
  }

  // note: serve-wait bound (B1/B2)

  /**
   * note: await one serve-side provider call under the responder's local
   * bounded-wait budget: the configured `invokeTimeoutMs` (default 5000),
   * which is also the reverse-invoke waiter deadline. The budget is local —
   * it does not inherit the dialer's budget and does not promise the reply
   * beats the dialer's timeout.
   *
   * Expiry answers one signed `INTERNAL_ERROR` with the existing
   * `details.kind = "timeout"` vocabulary (the same projection the
   * reverse-invoke waiter timeout uses) and touches nothing else: no counter
   * rollback, no capability mutation, no forced close.
   *
   * A zero budget answers that timeout without invoking the provider at all.
   * Otherwise the provider thunk is invoked asynchronously; on expiry the
   * responder stops awaiting it and consumes the late settlement (no unhandled
   * rejection and no late second response). Provider invocation, synchronous
   * throws and asynchronous rejections flow back to the caller unchanged (the
   * existing containment path). No timed branch starts a second provider call.
   *
   * The armed budget is tracked so `#close` disarms it (no serve timer
   * outlives the session), and a serve dispatch that resumes after a close
   * answers nothing.
   *
   * `budgetMs` defaults to the configured budget; a multi-stage serve passes
   * what remains of its ONE deadline so stages cannot each get a full budget.
   */
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
      // why: asynchronously invoked thunk — a synchronous provider throw is captured
      // as a rejection instead of escaping the timer setup.
      const pending = Promise.resolve().then(() => work());
      // why: executor form — the project's TS lib target predates
      // `Promise.withResolvers` (ES2024).
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
          // why: close settled the race — consume the late provider settlement so
          // it cannot surface as an unhandled rejection or a late response.
          void pending.then(
            () => undefined,
            () => undefined,
          );
          return expiryReject;
        }
        if (error !== SERVE_BUDGET_EXPIRED) {
          throw error;
        }
        // why: observation-only cancellation — consume the late settlement so it
        // cannot surface as an unhandled rejection or a late second response.
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

  /** note: serve the `extract` core op through the optional extract service (F3). */
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
      // note: B1/B2 serve bound — the complete extraction service (loader +
      // extractor) shares ONE local budget — the wait starts at provider
      // invocation, not per stage. A zero budget answers the signed timeout
      // without starting the service; expiry stops awaiting it.
      result = await this.#withServeBudget(() => extract.call(ports, request));
    } catch (error) {
      result = spokeReject(
        SpokeRejectCode.INTERNAL_ERROR,
        error instanceof Error ? error.message : String(error),
      );
    }
    // note: the bounded wait is the only thing the close can interrupt — once the
    // session is Closed, neither a late service success nor a late (or
    // second) error may be answered.
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

  /**
   * note: serve a `memory.*` invoke through the optional memory service seam
   * (protocol §13). The capability gate already passed: a known memory op that
   * was not negotiated never reaches here (`capability_missing`). An absent
   * face is the documented dispatch-deny branch.
   */
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
        // note: §14.2 — a memory-boundary denial is classified `terminal`.
        details: { retry_class: "terminal" },
      });
      return;
    }
    // invariant: the admitted principal derives from the authenticated session peer
    // (MEM-R003), never from the payload.
    const peerId = this.remotePeerId;
    // note: a zero budget rejects before the service is invoked (baseline
    // serve-timeout rejection stands). A non-zero budget that expires AFTER
    // invocation means the mutation may have committed: that is
    // `OUTCOME_UNCERTAIN`/`uncertain` (§14.2/§14.3), not a clean internal error.
    // The provider promise is left running — nothing here claims cancellation
    // or rollback of provider work.
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
      // why: the provider threw or rejected AFTER admission, so the transport has
      // no evidence that nothing committed (§14.3): answer uncertain, never a
      // clean internal error. The Rust path answers the same when the
      // detached invoke panics.
      result = spokeReject(
        SpokeRejectCode.OUTCOME_UNCERTAIN,
        `memory service failed after invocation: ${
          error instanceof Error ? error.message : String(error)
        }; the outcome is unknown — poll the receipt or replay the same idempotency key`,
        { retry_class: "uncertain", kind: "panic" },
      );
    }
    // note: a close during the bounded wait answers nothing afterwards.
    if (this.#stateInternal === "Closed") {
      return;
    }
    if (!result.ok) {
      await this.#sendReverseErrorEnvelope(doc, toErrorEnvelope(result));
      return;
    }
    await this.#sendOkResponse(doc, result.value);
  }

  /**
   * note: serve a `history.*` invoke through the optional injected
   * `RemoteHistoryService` face (spoke-history contract §Connect
   * operations). The static capability gate already passed in
   * `#dispatchInvoke` — a known history op that was not negotiated never
   * reaches here (`capability_missing`). The serving order mirrors the
   * `memory.*` branch: cognition gate → `asHistory` probe → bounded
   * service invoke → response envelope. The service owns the closed
   * request decode, peer admission, effective-scope resolution and prune
   * dedup; an absent face is the documented dispatch-deny branch (the
   * host declared `spoke-history` without providing the service).
   *
   * The serve bound matches the `memory.*` ops: a zero budget answers the
   * plain timeout without invoking the service at all, while expiry AFTER
   * invocation means the admitted mutation may have committed — that is
   * `OUTCOME_UNCERTAIN`/`uncertain` (§14.2/§14.3), recoverable through the
   * recorded-outcome dedup, never a clean internal error or a silent
   * re-execution. The service promise keeps running past the deadline so
   * a settling prune still records its outcome.
   */
  async #dispatchHistoryInvoke(doc: ConnectInvokeRequest): Promise<void> {
    const session = this.#session;
    if (session === null) {
      return;
    }
    // note: payload-dependent cognition gate on the raw payload: the history
    // request `scope` carries the `Scope.cognition` selection slot, so a
    // declared requirement needs `spoke-cognition` negotiated before the
    // probe and any provider call.
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
    // invariant: the admitted principal/scope derive from the authenticated
    // session peer, never from the payload (MEM-R003; contract
    // §Authorization).
    const peerId = this.remotePeerId;
    // note: a zero budget rejects before the service is invoked (plain
    // timeout, no dedup key reserved). A non-zero budget expiring AFTER
    // invocation answers the uncertain outcome identically to the memory
    // ops — the invoked service is left running; nothing here claims
    // cancellation or rollback of provider work.
    const uncertainOnExpiry =
      this.#invokeTimeoutMs === 0
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
      // why: the service threw or rejected AFTER invocation, so the transport
      // has no evidence that nothing committed (§14.3): answer uncertain —
      // the same containment as a provider panic on the memory path —
      // never a clean internal error.
      result = spokeReject(
        SpokeRejectCode.OUTCOME_UNCERTAIN,
        `history service failed after invocation: ${
          error instanceof Error ? error.message : String(error)
        }; the outcome is unknown — replay the same idempotency key`,
        { retry_class: "uncertain", kind: "panic" },
      );
    }
    // note: a close during the bounded wait answers nothing afterwards.
    if (this.#stateInternal === "Closed") {
      return;
    }
    if (!result.ok) {
      await this.#sendReverseErrorEnvelope(doc, toErrorEnvelope(result));
      return;
    }
    // note: the service returns the op's response object verbatim —
    // `history.revisions.get` wraps the snapshot under `snapshot`, prune
    // serializes `HistoryPruneResult` (`{ deleted, floor_revision }`).
    await this.#sendOkResponse(doc, result.value);
  }

  /**
   * note: serve a `knowledge.*` invoke (`knowledge.adopt`) through the
   * optional injected `RemoteAdoptionService` face (spoke-knowledge-
   * adoption contract §Connect operation). The static compound gate
   * (`spoke-knowledge-adoption` AND `spoke-knowledge`) already passed in
   * `#dispatchInvoke` — an unnegotiated `knowledge.adopt` never reaches
   * here (`capability_missing`). The serving order mirrors the
   * `history.*` branch: `asAdoption` probe → bounded service invoke →
   * response envelope. No cognition gate: the request carries scope ids,
   * not a declared `Scope`, so `cognitionRequirementPresent` has no
   * carrier (contract §Connect operation — `ke-ownership` stays scoped
   * to Scope-bearing ops). The service owns the closed request decode,
   * peer admission, and connect idempotency dedup; an absent face is the
   * documented dispatch-deny branch.
   *
   * The serve bound matches the `history.*` ops: a zero budget answers
   * the plain timeout without invoking the service at all, while expiry
   * AFTER invocation means the admitted mutation may have committed —
   * `OUTCOME_UNCERTAIN`/`uncertain`, recoverable through the
   * recorded-outcome dedup, never a clean internal error or a silent
   * re-execution.
   */
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
    // invariant: the admitted principal/scope derive from the authenticated
    // session peer, never from the payload (MEM-R003 parity; contract
    // §Admission and owner-private absence).
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
      // why: the service threw or rejected AFTER invocation, so the transport
      // has no evidence that nothing committed — answer uncertain (same
      // containment as the memory/history paths), never a clean internal
      // error.
      result = spokeReject(
        SpokeRejectCode.OUTCOME_UNCERTAIN,
        `adoption service failed after invocation: ${
          error instanceof Error ? error.message : String(error)
        }; the outcome is unknown — replay the same idempotency key`,
        { retry_class: "uncertain", kind: "panic" },
      );
    }
    // note: a close during the bounded wait answers nothing afterwards.
    if (this.#stateInternal === "Closed") {
      return;
    }
    if (!result.ok) {
      await this.#sendReverseErrorEnvelope(doc, toErrorEnvelope(result));
      return;
    }
    // note: the service returns the `knowledge.adopt` success branch
    // verbatim — `{ outcome, entry }`.
    await this.#sendOkResponse(doc, result.value);
  }

  /** note: serve a `port.*` invoke through the D4 catalogue (or dispatch-deny). */
  async #dispatchPortInvoke(doc: ConnectInvokeRequest): Promise<void> {
    const session = this.#session;
    if (session === null) {
      return;
    }
    const payload = doc.payload as Record<string, unknown>;
    const negotiated = session.negotiated_capabilities;
    // note: declared Scope shape keeps its original first-denial position, before
    // the F2 ownership conjunct. It validates the baseline Scope wire keys
    // only — the cognition subtree is judged by the §2.2 diagnosis below — so
    // a malformed baseline Scope still answers its existing input failure
    // first even when ownership is also missing.
    const scopeReject = validateScopeOpPayload(doc.op, payload);
    if (scopeReject !== null) {
      await this.#sendReverseErrorEnvelope(doc, toErrorEnvelope(scopeReject));
      return;
    }
    // note: F2 ownership conjunct next — the original first-denial wins over a
    // delivered cognition requirement, and the router filters the same way
    // before it considers the family.
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
    const ports = this.#ports;
    if (ports === undefined) {
      // note: absent `ports` (documented) — the capability gate passes but there
      // is no face to serve — answer the dispatch-deny branch.
      await this.#sendReverseErrorEnvelope(doc, {
        code: "op_unsupported",
        message: `no ports face configured for port op ${doc.op}`,
        extensions: {},
      });
      return;
    }
    // note: knowledge ops (three-row catalogue) — served only through the
    // knowledge face. Absent face → CAPABILITY_PORT_MISSING (gate → probe
    // → serve/deny); the admitted scope comes from the face's admission
    // callback keyed by the authenticated peer id, never from the wire.
    if (KNOWLEDGE_OPS[doc.op]) {
      // note: B2 — mirror Rust decode_knowledge_call; a malformed get/put payload
      // answers INVALID_INPUT before the face probe, admission, and any
      // store call. The list op's Scope shape validated earlier.
      const payloadReject = validateKnowledgeOpPayload(doc.op, payload);
      if (payloadReject !== null) {
        await this.#sendReverseErrorEnvelope(doc, toErrorEnvelope(payloadReject));
        return;
      }
      const face = ports.knowledge;
      if (face === undefined) {
        // note: B4 — an absent knowledge face is the dispatch-deny branch; the
        // same wire row as absent ports/baseline; the dialer maps it to
        // CAPABILITY_PORT_MISSING (wire_code preserved), never an
        // application-coded literal on the wire.
        await this.#sendReverseErrorEnvelope(doc, {
          code: "op_unsupported",
          message: `no knowledge face configured for port op ${doc.op}`,
          extensions: {},
        });
        return;
      }
      // note: B3 — an empty authenticated peer id (session not established) refuses
      // NOT_AUTHORIZED before admission, matching Rust's `remote_peer_id()`
      // None arm. One refusal shape, no existence detail.
      const peerId = this.remotePeerId;
      if (peerId === "") {
        await this.#sendReverseErrorEnvelope(doc, {
          code: SpokeRejectCode.NOT_AUTHORIZED,
          message: "knowledge scope not authorized",
          extensions: {},
        });
        return;
      }
      // note: one deadline per knowledge op — admission spends part of the local
      // budget, so the store call only gets what remains of it.
      const deadlineMs = Date.now() + this.#invokeTimeoutMs;
      const admitted = face.admission(peerId);
      if (!admitted.ok) {
        // note: no existence detail — the refusal names only the scope verdict.
        await this.#sendReverseErrorEnvelope(doc, {
          code: SpokeRejectCode.NOT_AUTHORIZED,
          message: "knowledge scope not authorized",
          extensions: {},
        });
        return;
      }
      const requestedScope = payload.scope as Scope | undefined;
      if (
        doc.op === "port.scope.list_knowledge_entries" &&
        (requestedScope === undefined ||
          !isJsonObject(requestedScope) ||
          !scopeMatchesAdmission(requestedScope, admitted.value))
      ) {
        // note: list requires an exact scope match; get/put carry no wire scope.
        await this.#sendReverseErrorEnvelope(doc, {
          code: SpokeRejectCode.NOT_AUTHORIZED,
          message: "knowledge scope not authorized",
          extensions: {},
        });
        return;
      }
      // note: the admitted scope owns scope_id/viewpoint and bounds every other
      // constraint; a request naming a scalar the admission does not grant
      // refuses here, before the store.
      let listedScope: Scope = admitted.value;
      if (doc.op === "port.scope.list_knowledge_entries") {
        const effective = effectiveScope(requestedScope as Scope, admitted.value);
        if (effective === null) {
          await this.#sendReverseErrorEnvelope(doc, {
            code: SpokeRejectCode.NOT_AUTHORIZED,
            message: "knowledge scope not authorized",
            extensions: {},
          });
          return;
        }
        listedScope = effective;
      }
      // why: only a put mutates — an expiry after the store was invoked may have
      // applied it, so that expiry reports an unknown outcome (matching
      // Rust's `knowledge_uncertain_reject`). Reads have no store effect to
      // be uncertain about and keep the plain timeout; a budget spent by
      // admission answers the plain timeout without reaching the store.
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
                  admitted.value,
                  payload.entry_id as string,
                );
              case "port.knowledge.put":
                // note: the scoped put path owns the same anchor-immutability
                // obligation as the upsert/promote orchestrations
                // (spoke-knowledge-adoption contract §Anchor immutability): a
                // stored `adoption_anchor` is preserved byte-identically or
                // the write refuses `ADOPTION_ANCHOR_IMMUTABLE` with
                // `details.entry_id` — before the put, never after (Rust
                // parity: `call_knowledge_op` →
                // `assert_adoption_anchor_preserved`).
                return (async (): Promise<SpokeResult<unknown>> => {
                  const candidate = payload.entry as KnowledgeEntry;
                  const storedResult = await face.store.getScopedKnowledgeEntry(
                    admitted.value,
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
                  // note: C1 refusal order (QC2-14 / F-016) — the
                  // caller's OCC fence is judged on THIS stored snapshot
                  // BEFORE the anchor check: a create-only put into an
                  // occupied slot keeps the existing create-only
                  // `REVISION_CONFLICT` refusal even when the candidate
                  // would also fail anchor immutability, and a base
                  // mismatch keeps `STORED_REVISION_STALE`. The caller's
                  // `expected_base_revision` IS the CAS fence: a non-null
                  // base requires that exact stored revision (absent
                  // entry fails the update), a null base is create-only
                  // and fails on an existing entry. `undefined` (absent
                  // member) decodes the same as `null` — never forwarded
                  // to the store.
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
                    admitted.value,
                    candidate,
                    callerBase,
                  );
                })();
              default:
                return face.store.listScopedKnowledgeEntries(listedScope);
            }
          },
          knowledgeUncertainOnExpiry,
          remainingMs,
        );
      } catch (error) {
        // why: a store that threw or rejected was already invoked, so a put may
        // have applied: answer an unknown outcome (matching Rust's
        // `knowledge_uncertain_reject` and the memory provider path) instead of
        // the generic internal error. Reads keep the internal-error mapping.
        if (doc.op !== "port.knowledge.put") {
          throw error;
        }
        // why: the store error stays local — its text may describe entry existence,
        // so the wire answer is fixed and carries only what the caller can act on.
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
      // note: absent baseline face (documented dispatch-deny branch) — knowledge
      // hosts serve no baseline op — this answer precedes any probe.
      await this.#sendReverseErrorEnvelope(doc, {
        code: "op_unsupported",
        message: `no ports face configured for port op ${doc.op}`,
        extensions: {},
      });
      return;
    }
    // note: optional-family probe (gate → probe → serve/deny serving order) — the
    // capability gate passed, so a missing optional method means the host
    // declared a family it does not provide (misconfiguration) — answer
    // the same dispatch-deny branch as absent ports.
    const missingMethod = probeOptionalPortMethod(baselinePorts, doc.op);
    if (missingMethod !== null) {
      await this.#sendReverseErrorEnvelope(doc, {
        code: "op_unsupported",
        message: `port op ${doc.op} requires optional port method ${missingMethod}, which the configured ports do not implement`,
        extensions: {},
      });
      return;
    }
    // note: B1/B2 serve bound — the local budget starts immediately before the
    // provider is dispatched and covers the whole wait. A zero budget answers
    // the signed timeout without ever invoking the provider (zero provider
    // calls); expiry stops awaiting it — a bounded wait, never forced
    // termination or rollback of host work.
    const result = await this.#withServeBudget(() =>
      dispatchPortOp(doc.op, doc.payload as Record<string, unknown>, baselinePorts),
    );
    // note: the bounded wait is the only thing the close can interrupt — once the
    // session is Closed, neither a late provider success nor a late (or
    // second) error may be answered.
    if (this.#stateInternal === "Closed") {
      return;
    }
    if (result.ok) {
      // note: success payload carries the raw success value `T` (D4), NOT the
      // `{ result }` tool shape — the dialer's `#invokeMapped` validates it.
      await this.#sendOkResponse(doc, "value" in result ? result.value : {});
    } else {
      await this.#sendReverseErrorEnvelope(doc, toErrorEnvelope(result));
    }
  }

  // note: response helpers

  /** note: send one signed response envelope, fire-and-forget. */
  #sendReverseResponse(doc: unknown): void {
    void this.#transport.send(encodeEnvelope(doc)).catch(() => {
      // note: peer gone — responses are fire-and-forget at the serving boundary.
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
      await authenticateInvokeResponse(this.#secret, {
        session_id: doc.session_id,
        sequence: doc.sequence,
        request_id: doc.request_id,
        payload: (payload ?? {}) as Record<string, unknown>,
      }),
    );
  }

  async #sendReverseErrorEnvelope(
    doc: ConnectInvokeRequest,
    error: ErrorEnvelope,
  ): Promise<void> {
    this.#sendReverseResponse(
      await authenticateInvokeResponse(this.#secret, {
        session_id: doc.session_id,
        sequence: doc.sequence,
        request_id: doc.request_id,
        error,
      }),
    );
  }

  // note: reverse-invoke response demux (request_id → waiter)

  /**
   * invariant: demux a response envelope to its pending reverse-invoke waiter —
   * correlation echo check first (non-mutating wire-position validation), then
   * envelope-auth verify against the dialer's hello public key. A
   * forged/tampered response fails closed — only this waiter, never a
   * session-state mutation.
   */
  async #demuxResponse(doc: ConnectInvokeResponse): Promise<void> {
    const session = this.#session;
    const clientPubkey = this.#clientPubkey;
    const entry = this.#pending.get(doc.request_id);
    if (entry === undefined) {
      return; // note: unknown/duplicate response — dropped
    }
    clearTimeout(entry.timer);
    this.#pending.delete(doc.request_id);
    try {
      if (session === null || clientPubkey === null) {
        throw new ResponderError("session_closed", "connect session closed");
      }
      checkResponseCorrelation(entry.correlation, correlationFromResponse(doc));
      await verifyInvokeResponseAuth(clientPubkey, doc, session.session_id);
      entry.resolve(doc);
    } catch (error) {
      if (error instanceof EnvelopeAuthError) {
        entry.reject(new ResponderError(error.kind, error.message));
      } else if (error instanceof CoreError && error.code === "crypto") {
        // why: crypto verify failure (wrong-length / invalid key bytes — local
        // key misuse, no envelope-auth kind): still a verification failure
        // of the response's authenticator → `envelope_auth_invalid`.
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

  // note: reverse invoke path (outbound counter → sign → send tail → waiter)

  /**
   * note: send one reverse invoke and resolve with its correlated response
   * envelope. Rejects with `ResponderError` on timeout / transport failure
   * / session close / correlation mismatch / sequence exhaustion.
   */
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
      // why: outbound sequence exhaustion makes the session unusable (no wrap) —
      // close it and fail this invoke with `sequence_exhausted`.
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
    // invariant: the waiter is registered SYNCHRONOUSLY, before the async sign — from
    // the caller's perspective the invoke is in flight immediately, so a
    // transport close during signing fails it with `session_closed`.
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
        resolve,
        reject,
        timer,
      });
      // invariant: outbound wire-order serialization — the tail entry is created HERE,
      // synchronously, so the wire order matches the allocation order even
      // though the Ed25519 sign completes asynchronously.
      const prevSend = this.#sendTail;
      let releaseSend: () => void = () => {};
      this.#sendTail = new Promise<void>((resolveTail) => {
        releaseSend = resolveTail;
      });
      void authenticateInvokeRequest(this.#secret, unsigned).then(
        (signed) => {
          void prevSend
            .then(() => {
              // why: the invoke may have settled while this send waited behind
              // the tail (timeout fired → entry deleted; session closed →
              // `#failAllPending` cleared the map). Do NOT transmit late:
              // the caller already observed the failure, and a retry would
              // otherwise produce a duplicate dispatch on the peer. A
              // timeout drop is worse than a duplicate dispatch though:
              // the allocated outbound sequence was never transmitted, so
              // the peer's inbound gate is stuck at that sequence and
              // every later invoke fails — the wire state is
              // unreconcilable. Close the session (fails remaining pending
              // with `session_closed`, mirroring the adapter's `#invokeOp`)
              // instead of leaving a silently poisoned session.
              if (!this.#pending.has(unsigned.request_id)) {
                this.#close(
                  "deferred invoke skipped (settled while queued) — outbound sequence never transmitted",
                );
                return;
              }
              try {
                return this.#transport.send(encodeEnvelope(signed));
              } catch (error) {
                // why: synchronous encode failure (e.g. non-JSON-serializable
                // payload): same cleanup, same mapping.
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
                // why: async send failure (transport closed between setup and
                // send): settle this invoke now — no dead entry waits out
                // the timeout.
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

  /**
   * note: start a responder over `transport` — the signed-hello handshake runs in
   * the background (the dialer's hello is the synchronization point), then
   * the serve loop. Handshake failures close the transport so the dialer's
   * dial fails fast instead of waiting out its timeout.
   *
   * Lives on the class because the session-lifecycle methods are
   * `#`-private and only reachable from inside the class body; the
   * module-level `connectResponder` below is the same entrypoint for
   * consumers.
   */
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
