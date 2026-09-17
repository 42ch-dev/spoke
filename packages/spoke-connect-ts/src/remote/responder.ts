/**
 * `connectResponder` — productized connect responder (frozen contract
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
  parseToolCapabilityId,
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  toErrorEnvelope,
  type BaselinePorts,
  type ComputablePort,
  type ForkTimelineQueryPort,
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
import type { RemoteIdentity, ToolHandler } from "./remote-adapter.js";
import type { EnvelopeBytes, Transport } from "./transport.js";

const DEFAULT_INVOKE_TIMEOUT_MS = 5000;

/** Session id prefix — deterministic per dialing peer (opaque, not schema-enforced). */
const SESSION_ID_PREFIX = "connect-responder-session-";

const textEncoder = new TextEncoder();

function encodeEnvelope(doc: unknown): EnvelopeBytes {
  return textEncoder.encode(encodeJsonMessage(doc));
}

/**
 * Internal invoke-failure classes. Consumers only ever observe these mapped
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
 * Sentinel rejection marking local serve-budget expiry, so the timed-await
 * helper can tell the budget from a provider failure.
 */
const SERVE_BUDGET_EXPIRED = Symbol("serve-budget-expired");

/**
 * Dispatch-deny wire codes (D7): the peer answered that the op or its
 * required capability is not available → `CAPABILITY_PORT_MISSING`.
 */
const DISPATCH_DENY_CODES = new Set(["op_unsupported", "capability_missing"]);

/** Map an error-branch envelope to a `SpokeResult` reject (D7 mapping). */
function mapErrorEnvelope(error: ErrorEnvelope): SpokeReject {
  if (DISPATCH_DENY_CODES.has(error.code)) {
    return spokeReject(SpokeRejectCode.CAPABILITY_PORT_MISSING, error.message, {
      ...(error.details ?? {}),
      wire_code: error.code,
    });
  }
  // Envelope-auth rejection: `auth_failed` is not a SpokeRejectCode, so
  // `fromErrorEnvelope` would map it to INVALID_INPUT; the locked mapping
  // is `INTERNAL_ERROR` with the envelope-auth `details.kind` verbatim.
  if (error.code === "auth_failed") {
    return spokeReject(SpokeRejectCode.INTERNAL_ERROR, error.message, {
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
 */
const PORT_OP_CAPABILITY_REQUIREMENTS: Record<string, string> = {
  "port.knowledge.get": "spoke-baseline",
  "port.knowledge.put": "spoke-baseline",
  "port.relation.get": "spoke-baseline",
  "port.relation.put": "spoke-baseline",
  "port.scope.list_knowledge_entries": "spoke-baseline",
  "port.scope.list_timeline_events": "spoke-baseline",
  "port.finding.put": "spoke-baseline",
  "port.rule.list": "spoke-baseline",
  "port.host.list_peer_manifests": "spoke-baseline",
  // Optional families (D4 catalogue): project / compute require the
  // `l2-computable` family capability; the fork timeline query requires
  // `l5-fork` — all evaluated against `negotiated_capabilities` (the
  // both-hello intersection) by `#gateAllows`.
  "port.computable.project": "l2-computable",
  "port.computable.compute": "l2-computable",
  "port.fork.list_timeline_events": "l5-fork",
};

/**
 * Optional-family op → method on the injected `ports` object (D4
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

function isJsonObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
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

/** Structural decode/validate for a declared Scope (mirrors Rust `Scope::deserialize`). */
function validateDeclaredScope(scope: Record<string, unknown>): string | null {
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

const OPTIONAL_PORT_METHODS: Readonly<Record<string, string>> = {
  "port.computable.project": "project",
  "port.computable.compute": "compute",
  "port.fork.list_timeline_events": "listForkTimelineEvents",
};

/**
 * Structural probe (the `requirePortMethod` pattern): return the missing
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
    return null; // baseline op — always present on BaselinePorts
  }
  return typeof (ports as unknown as Record<string, unknown>)[method] ===
    "function"
    ? null
    : method;
}

/**
 * Map a `port.*` op + payload to the injected adapter method per the D4
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
    case "port.knowledge.get":
      return ports.getKnowledgeEntry(payload.entry_id as string);
    case "port.knowledge.put":
      return ports.putKnowledgeEntry(
        payload.entry as KnowledgeEntry,
        payload.expected_base_revision as number | null,
      );
    case "port.relation.get":
      return ports.getRelation(payload.relation_id as string);
    case "port.relation.put":
      return ports.putRelation(
        payload.relation as Relation,
        payload.expected_base_revision as number | null,
      );
    case "port.scope.list_knowledge_entries":
      return ports.listKnowledgeEntries(payload.scope as Scope);
    case "port.scope.list_timeline_events":
      return ports.listTimelineEvents(payload.scope as Scope);
    case "port.finding.put":
      return ports.putFindings(payload.findings as Finding[]);
    case "port.rule.list":
      return ports.listRules(payload.rule_refs as string[]);
    case "port.host.list_peer_manifests":
      return ports.listPeerHostCapabilityManifests();
    // Optional families (D4 catalogue). The serving method
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
      // Unreachable when the dispatch gate denies unknown ops first; kept
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
  /** Message-oriented transport (consumer-provided; loopback ships in-repo). */
  transport: Transport;
  /** This responder's raw Ed25519 seed (hello + envelope-auth identity). */
  identity: RemoteIdentity;
  /** Host manifest advertised in the responder's signed hello. */
  manifest: HostCapabilityManifest;
  /** Trusted dialer peer ids (fail-closed allowlist). */
  allowlist: readonly string[];
  /**
   * Preconfigured dialer Ed25519 public keys by peer_id. Key distribution
   * is transport-adapter-owned per the spec; the responder knows its
   * trusted identities statically. A dialing peer on `allowlist` without a
   * preconfigured key fails the handshake (fail-closed).
   */
  peerKeys: Readonly<Record<string, Uint8Array>>;
  /**
   * Local async `BaselinePorts` served on the remote side via the D4
   * catalogue. Absent `ports` still answers `port.*` invokes with the
   * dispatch-deny branch (documented behavior).
   */
  ports?: BaselinePorts & Partial<RemoteExtractService>;
  /**
   * Bounded-wait deadline for each reverse-invoke waiter AND the local
   * serve-wait budget bounding every served `port.*` provider call and the
   * whole `extract` service call, ms (default 5000). Zero means immediate
   * serve timeout with zero provider calls. The value is local: it does not
   * inherit the dialer's budget and does not promise the reply beats the
   * dialer's timeout.
   */
  invokeTimeoutMs?: number;
}

interface PendingReverseInvoke {
  correlation: Correlation;
  resolve: (response: ConnectInvokeResponse) => void;
  reject: (error: ResponderError) => void;
  timer: ReturnType<typeof setTimeout>;
}

/** Gate-phase outcome for an inbound invoke (see `#runGate`). */
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
  readonly #ports: (BaselinePorts & Partial<RemoteExtractService>) | undefined;
  readonly #invokeTimeoutMs: number;
  readonly #nonceStore = new NonceStore();

  // Verification-gating state. All of it is ECMAScript `#`-private: no JS
  // consumer can write a `#`-slot from outside the class — a forged
  // `Established` / forged `session` / forged remote-manifest cache is
  // unreachable even through an `any` cast or a subclass.
  #stateInternal: ConnectResponderState = "Disconnected";
  #session: Session | null = null;
  /** The verified dialer's hello public key (set at handshake). */
  #clientPubkey: Uint8Array | null = null;
  #remoteManifestInternal: HostCapabilityManifest | null = null;
  #serveLoopRunning = false;
  #pending = new Map<string, PendingReverseInvoke>();
  /**
   * Armed serve-budget disarms. Tracked so `#close` cancels every serve wait
   * it still owns: no serve wait's timer outlives the session it was armed
   * for. An entry removes itself when it fires or when its provider settles.
   */
  #serveDisarms = new Set<() => void>();
  /**
   * Tool-handler registry for `tools.*` invokes (frozen contract §6):
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
   * Outbound send serialization tail. Sequences are allocated synchronously
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

  /** The assigned session id (empty before establishment). */
  get sessionId(): string {
    return this.#session?.session_id ?? "";
  }

  /** The verified dialing peer id (empty before establishment). */
  get remotePeerId(): string {
    return this.#session?.initiator_peer_id ?? "";
  }

  /**
   * The dialing peer's `HostCapabilityManifest`, from the authenticated
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
   * Register a handler for a `tools.<ns>.<tool_id>` capability served on
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
   * Reverse tool-invoke face (frozen contract §6): issue a
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
    // Fail fast on a non-tool capability id (the op string IS the
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
      // Tool success-payload gate (frozen §4): success is
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

  /** Release the session and transport. Idempotent. */
  close(): void {
    this.#close("local shutdown");
  }

  // ── Session lifecycle (hard-private — only `connectResponder` starts) ───

  /** Factory-only: start the handshake + serve loop in the background. */
  #begin(): void {
    this.#stateInternal = "Handshaking";
    void this.#run();
  }

  /**
   * All failure paths that make the session unusable. Transitions to
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
        // Transport close failure is intentionally swallowed: the session
        // is unusable either way and the responder already settled waiters.
      });
    }
    this.#failAllPending(
      new ResponderError("session_closed", `connect session closed: ${reason}`),
    );
    // Serve waits owned by this session: cancel their armed budgets so no
    // serve timer outlives the close. A pending provider completion that
    // arrives afterwards is discarded by the closed-state check in the serve
    // dispatch (no late success, no late second error).
    for (const disarm of this.#serveDisarms) {
      disarm();
    }
  }

  #failAllPending(error: ResponderError): void {
    for (const entry of this.#pending.values()) {
      clearTimeout(entry.timer);
      entry.reject(error);
    }
    this.#pending.clear();
  }

  // ── Handshake (allowlist-first → hello verify → nonce → dial-bound
  // ── responder hello → signed session snapshot) ──────────────────────────

  async #handshake(): Promise<void> {
    const helloDoc: unknown = decodeJsonMessage(await this.#transport.recv());
    if (!isConnectHello(helloDoc)) {
      throw new Error("expected ConnectHello from client");
    }
    // Allowlist fail-closed check FIRST: an untrusted peer is rejected
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

    // Answer with our signed hello (responder role — dial binding): the
    // hello signs the 5-field object incl. `peer_nonce` = the initiator's
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
    // The session snapshot is signed with the responder identity
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

  // ── Serve loop (request-first classification) ───────────────────────────

  async #run(): Promise<void> {
    try {
      await this.#handshake();
      this.#stateInternal = "Established";
      this.#runServeLoop();
    } catch (error) {
      // Handshake failure (or responder misconfiguration): fail the peer's
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
          // Unparseable inbound: closes the connection per the carried-over
          // demo semantics — a bare return would leave the socket open
          // while the responder stops recv'ing, and the dialer's
          // established session would hang on its next invoke.
          this.#close(
            `unparseable inbound message: ${error instanceof Error ? error.message : String(error)}`,
          );
          return;
        }
      } catch {
        // Transport closed — the session is unusable; fail pending waiters.
        this.#close("transport loss");
        return;
      }

      // Classify request shape FIRST (normative `spoke-connect.md`
      // §Request / response classification): an inbound envelope carrying
      // `op` is a `ConnectInvokeRequest` — never a response, even though a
      // reverse request carries the same correlation echo fields + payload
      // as the response success branch.
      if (isConnectInvokeRequest(doc)) {
        // Gate serialization: await peek → verify → advance inline; the
        // loop reads the next envelope only after this gate completes.
        // Dispatch fires without blocking the loop.
        try {
          await this.#serveInvoke(doc);
        } catch (error) {
          // Unexpected serving failure (local key misconfiguration /
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
          // Demux failures are contained per-waiter; nothing here should
          // escape, but never crash the loop.
        }
        continue;
      }

      // Post-handshake stray envelope (hello / session / unknown shape):
      // ignored.
    }
  }

  /**
   * Serve one inbound invoke per the canonical order (frozen §4):
   * classify (caller) → stray → sequence peek → envelope-auth verify →
   * advance → gate → handler → signed response. The caller (serve loop)
   * awaits this method inline so peek → verify → advance are serialized;
   * dispatch fires without blocking the loop.
   */
  async #serveInvoke(doc: ConnectInvokeRequest): Promise<void> {
    const gate = await this.#runGate(doc);
    if (gate === null) {
      return; // stray — ignored
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
   * Gate phase — sequence peek (non-mutating) → envelope-auth verify →
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
      return null; // stray — no established session
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
      throw error; // wrong-length key is responder misconfiguration — fail loudly
    }
    // Advance the inbound counter only after envelope-auth verify passed.
    session.acceptInboundSequence(doc.sequence);
    return { ok: true };
  }

  /**
   * Dispatch phase — runs after the serialized gate; may interleave with
   * other invokes. Fully contained: a throwing handler answers the error
   * branch and never crashes the loop.
   */
  async #dispatchInvoke(doc: ConnectInvokeRequest): Promise<void> {
    const session = this.#session;
    if (session === null) {
      return; // stray — belt-and-braces; the gate checked
    }
    try {
      // Dispatch gate — `Session.dispatchAllowed`-level logic (frozen §3):
      // `tools.*` ops require the op string itself; baseline `port.*` ops
      // require `spoke-baseline`; optional families route via the same
      // product map to `l2-computable` / `l5-fork`. All evaluate against
      // `negotiated_capabilities` (never a raw requirements-map composition,
      // which would deny the self-describing tools family).
      if (!this.#gateAllows(doc.op, session)) {
        await this.#sendReverseErrorEnvelope(doc, {
          code: "op_unsupported",
          message: `op ${doc.op} is not authorized by this session`,
          extensions: {},
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
      await this.#dispatchPortInvoke(doc);
    } catch (error) {
      // Unexpected serving failure (e.g. non-JSON-serializable handler
      // result): answer the error branch so the invoker never hangs, and
      // never crash the loop.
      try {
        await this.#sendReverseErrorEnvelope(doc, {
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: `invoke failed: ${error instanceof Error ? error.message : String(error)}`,
          extensions: {},
        });
      } catch {
        // Peer gone — responses are fire-and-forget.
      }
    }
  }

  /** Dispatch gate: core table (incl. `tools.*`) then the port product map. */
  #gateAllows(op: string, session: Session): boolean {
    const negotiated = session.negotiated_capabilities;
    if (session.dispatchAllowed(op)) {
      return true;
    }
    const required = PORT_OP_CAPABILITY_REQUIREMENTS[op];
    if (required === undefined || !negotiated.includes(required)) {
      return false;
    }
    return true;
  }

  /** Serve a `tools.*` invoke through the registered handler (or deny). */
  async #dispatchToolInvoke(doc: ConnectInvokeRequest): Promise<void> {
    // Handler or deny — fail-closed serving (frozen deny matrix): a gate
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
    // The request payload carries the tool arguments as
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
      // Handler threw → error branch via toErrorEnvelope (INTERNAL_ERROR
      // for a crash); never crashes the loop (frozen §6).
      result = spokeReject(
        SpokeRejectCode.INTERNAL_ERROR,
        error instanceof Error ? error.message : String(error),
      );
    }
    if (result.ok) {
      // Success branch: `payload = { "result": <opaque JSON> }` (frozen §4).
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

  // ── serve-wait bound (B1/B2) ─────────────────────────────────────────────

  /**
   * Await one serve-side provider call under the responder's local
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
   */
  async #withServeBudget<T>(work: () => Promise<T>): Promise<T | SpokeReject> {
    const budgetMs = this.#invokeTimeoutMs;
    const expiryReject = spokeReject(
      SpokeRejectCode.INTERNAL_ERROR,
      `serve wait exceeded the local budget of ${budgetMs}ms`,
      { kind: "timeout" },
    );
    if (budgetMs === 0) {
      return expiryReject;
    }
    // Asynchronously invoked thunk: a synchronous provider throw is captured
    // as a rejection instead of escaping the timer setup.
    const pending = Promise.resolve().then(() => work());
    // Executor form: the project's TS lib target predates
    // `Promise.withResolvers` (ES2024).
    const expired = new Promise<never>((_resolve, reject) => {
      const timer = setTimeout(() => {
        this.#serveDisarms.delete(disarm);
        reject(SERVE_BUDGET_EXPIRED);
      }, budgetMs);
      // Timer cleanup on the provider's own settle (success or rejection) and
      // on `#close`, which runs every tracked disarm. Expiry needs no
      // `clearTimeout` — a fired timer is already gone.
      const disarm = (): void => {
        this.#serveDisarms.delete(disarm);
        clearTimeout(timer);
      };
      this.#serveDisarms.add(disarm);
      void pending.then(disarm, disarm);
    });
    try {
      return await Promise.race([pending, expired]);
    } catch (error) {
      if (error !== SERVE_BUDGET_EXPIRED) {
        throw error;
      }
      // Observation-only cancellation: consume the late settlement so it
      // cannot surface as an unhandled rejection or a late second response.
      void pending.then(
        () => undefined,
        () => undefined,
      );
      return expiryReject;
    }
  }

  /** Serve the `extract` core op through the optional extract service (F3). */
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
      // B1/B2 serve bound: the complete extraction service (loader +
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
    // The bounded wait is the only thing the close can interrupt: once the
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

  /** Serve a `port.*` invoke through the D4 catalogue (or dispatch-deny). */
  async #dispatchPortInvoke(doc: ConnectInvokeRequest): Promise<void> {
    const session = this.#session;
    if (session === null) {
      return;
    }
    const payload = doc.payload as Record<string, unknown>;
    const scopeReject = validateScopeOpPayload(doc.op, payload);
    if (scopeReject !== null) {
      await this.#sendReverseErrorEnvelope(doc, toErrorEnvelope(scopeReject));
      return;
    }
    const negotiated = session.negotiated_capabilities;
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
    const ports = this.#ports;
    if (ports === undefined) {
      // Absent `ports` (documented): the capability gate passes but there
      // is no BaselinePorts to serve — answer the dispatch-deny branch.
      await this.#sendReverseErrorEnvelope(doc, {
        code: "op_unsupported",
        message: `no ports face configured for port op ${doc.op}`,
        extensions: {},
      });
      return;
    }
    // Optional-family probe (gate → probe → serve/deny serving order): the
    // capability gate passed, so a missing optional method means the host
    // declared a family it does not provide (misconfiguration) — answer
    // the same dispatch-deny branch as absent ports.
    const missingMethod = probeOptionalPortMethod(ports, doc.op);
    if (missingMethod !== null) {
      await this.#sendReverseErrorEnvelope(doc, {
        code: "op_unsupported",
        message: `port op ${doc.op} requires optional port method ${missingMethod}, which the configured ports do not implement`,
        extensions: {},
      });
      return;
    }
    // B1/B2 serve bound: the local budget starts immediately before the
    // provider is dispatched and covers the whole wait. A zero budget answers
    // the signed timeout without ever invoking the provider (zero provider
    // calls); expiry stops awaiting it — a bounded wait, never forced
    // termination or rollback of host work.
    const result = await this.#withServeBudget(() =>
      dispatchPortOp(doc.op, doc.payload as Record<string, unknown>, ports),
    );
    // The bounded wait is the only thing the close can interrupt: once the
    // session is Closed, neither a late provider success nor a late (or
    // second) error may be answered.
    if (this.#stateInternal === "Closed") {
      return;
    }
    if (result.ok) {
      // Success payload carries the raw success value `T` (D4), NOT the
      // `{ result }` tool shape — the dialer's `#invokeMapped` validates it.
      await this.#sendOkResponse(doc, "value" in result ? result.value : {});
    } else {
      await this.#sendReverseErrorEnvelope(doc, toErrorEnvelope(result));
    }
  }

  // ── response helpers ─────────────────────────────────────────────────────

  /** Send one signed response envelope, fire-and-forget. */
  #sendReverseResponse(doc: unknown): void {
    void this.#transport.send(encodeEnvelope(doc)).catch(() => {
      // Peer gone — responses are fire-and-forget at the serving boundary.
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

  // ── Reverse-invoke response demux (request_id → waiter) ─────────────────

  /**
   * Demux a response envelope to its pending reverse-invoke waiter.
   * Correlation echo check first (non-mutating wire-position validation),
   * then envelope-auth verify against the dialer's hello public key. A
   * forged/tampered response fails closed — only this waiter, never a
   * session-state mutation.
   */
  async #demuxResponse(doc: ConnectInvokeResponse): Promise<void> {
    const session = this.#session;
    const clientPubkey = this.#clientPubkey;
    const entry = this.#pending.get(doc.request_id);
    if (entry === undefined) {
      return; // unknown/duplicate response — dropped
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
        // Crypto verify failure (wrong-length / invalid key bytes — local
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

  // ── Reverse invoke path (outbound counter → sign → send tail → waiter) ──

  /**
   * Send one reverse invoke and resolve with its correlated response
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
      // Outbound sequence exhaustion: the session is unusable (no wrap);
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
    // The waiter is registered SYNCHRONOUSLY, before the async sign: from
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
      // Outbound wire-order serialization: the tail entry is created HERE,
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
              // The invoke may have settled while this send waited behind
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
                // Synchronous encode failure (e.g. non-JSON-serializable
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
                // Async send failure (transport closed between setup and
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
   * Start a responder over `transport`: the signed-hello handshake runs in
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
