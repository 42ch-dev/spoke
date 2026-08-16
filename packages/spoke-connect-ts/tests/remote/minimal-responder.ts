/**
 * Minimal test responder — the dialed (responder) side of a loopback pair
 * used to exercise the RemoteAdapter's tool-serving mode (Task 1 of
 * tools-connect-reverse-invoke).
 *
 * It completes the signed-hello handshake (allowlist → signature verify →
 * nonce single-use → dial-bound responder hello → A-assigned session
 * snapshot) exactly like `loopback-host.ts`, then:
 *
 * 1. SERVES forward invokes from the dialer through the frozen serving
 *    pipeline (`tool-contracts.md` §4): classify request-first → stray →
 *    sequence peek → envelope-auth verify → advance → dispatch gate →
 *    registered tool handler → signed response. Deny answers use the frozen
 *    deny-code matrix (`op_unsupported` gate/no-handler, `auth_failed`,
 *    `invalid_sequence`).
 * 2. ISSUES reverse invokes toward the dialer (`issueInvoke`): outbound
 *    counter, request signing, waiter demux by `request_id`, correlation +
 *    envelope-auth verify on the response, and the D7 error mapping — the
 *    mirror of what Task 2's `connectResponder` productizes.
 *
 * Test-only (lives under tests/). The serving pipeline intentionally
 * mirrors `Session.dispatchAllowed`-level gating (not the demo host's
 * product-map composition) so `tools.*` self-describing ops pass the gate
 * exactly as the normative spec requires.
 */

import type {
  ConnectInvokeRequest,
  ConnectInvokeResponse,
  ErrorEnvelope,
  HostCapabilityManifest,
} from "@42ch/spoke-schemas";
import {
  fromErrorEnvelope,
  parseToolCapabilityId,
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  toErrorEnvelope,
  type SpokeReject,
  type SpokeResult,
} from "@42ch/spoke-operations";

import { getPublicKeyEd25519 } from "../../src/crypto.js";
import { isAllowlisted } from "../../src/core/allowlist.js";
import {
  checkResponseCorrelation,
  correlationFromResponse,
  type Correlation,
} from "../../src/core/correlate.js";
import {
  authenticateInvokeRequest,
  authenticateInvokeResponse,
  authenticateSession,
  EnvelopeAuthError,
  verifyInvokeRequestAuth,
  verifyInvokeResponseAuth,
} from "../../src/core/envelope-auth.js";
import { CoreError } from "../../src/core/error.js";
import { signHelloEd25519, verifyHelloEd25519 } from "../../src/core/hello.js";
import { generateNonce, NonceStore } from "../../src/core/nonce.js";
import { negotiatedCapabilities, Session } from "../../src/core/session.js";
import { decodeJsonMessage, encodeJsonMessage } from "../../src/framing.js";
import { derivePeerIdFromEd25519Pubkey } from "../../src/identity.js";
import {
  isConnectHello,
  isConnectInvokeRequest,
  isConnectInvokeResponse,
} from "../../src/remote/guards.js";
import type { EnvelopeBytes, Transport } from "../../src/remote/transport.js";

const SESSION_ID = "test-session-reverse-0001";

/** Dispatch-deny wire codes (D7): deny → `CAPABILITY_PORT_MISSING` + wire_code. */
const DISPATCH_DENY_CODES = new Set(["op_unsupported", "capability_missing"]);

/** Map an error-branch envelope to a `SpokeResult` reject (D7 mapping, mirrors remote-adapter.ts). */
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
  return fromErrorEnvelope(error);
}

const textEncoder = new TextEncoder();

function encodeEnvelope(doc: unknown): EnvelopeBytes {
  return textEncoder.encode(encodeJsonMessage(doc));
}

/**
 * Normalize a raw response override (test fixture) into an
 * `InvokeResponseSignInput`: strip `signature`/`extensions` and select the
 * wire branch (`payload` XOR `error`) so the signed object mirrors the wire
 * exactly (envelope-auth contract §3 — the two branches are never merged).
 */
function overrideSignInput(
  override: unknown,
): Parameters<typeof authenticateInvokeResponse>[1] {
  const wire = override as {
    session_id?: string;
    sequence?: number;
    request_id?: string;
    payload?: unknown;
    error?: unknown;
  };
  const base = {
    session_id: wire.session_id as string,
    sequence: wire.sequence as number,
    request_id: wire.request_id as string,
  };
  if (wire.payload !== undefined) {
    return {
      ...base,
      payload: wire.payload as Record<string, unknown>,
    };
  }
  if (wire.error === undefined) {
    throw new Error("response override must carry exactly one of payload or error");
  }
  return { ...base, error: wire.error as ErrorEnvelope };
}

/** Test-side tool handler registry entry (mirror of the frozen §1 handler shape). */
export type TestToolHandler = (
  args: Record<string, unknown>,
) => Promise<SpokeResult<unknown>>;

export interface MinimalResponderOptions {
  /** The server end of a loopback transport pair (the end the client dials). */
  transport: Transport;
  /** Responder Ed25519 seed. */
  seed: Uint8Array;
  /** The client's Ed25519 public key (key distribution is test-owned). */
  clientPubkey: Uint8Array;
  /** Trusted client peer ids (fail-closed). */
  allowlist: readonly string[];
  /** The responder's advertised manifest (both hellos must list tool capabilities for negotiation). */
  manifest: HostCapabilityManifest;
  /** Bounded wait for the response to an issued reverse invoke, ms (default 2000). */
  invokeTimeoutMs?: number;
  /**
   * Test-only: when non-undefined for a served request, the returned
   * envelope is sent verbatim instead of the real response (malformed-
   * response fixtures for the dialer's success-payload gate).
   */
  responseOverride?: (request: ConnectInvokeRequest) => unknown;
}

export interface MinimalResponderStats {
  /** Signed client hellos that passed allowlist + signature + nonce gates. */
  hellosVerified: number;
  /** Tool handlers that ran (gate passed + handler registered + invoked). */
  handlersRun: number;
  /** Reverse invokes issued toward the dialer. */
  reverseInvokesIssued: number;
  /** Responses received and correlation+auth-verified. */
  responsesVerified: number;
  /** Forward invokes rejected by the inbound sequence gate. */
  sequenceRejections: number;
  /** Forward invokes rejected by the envelope-auth verify gate. */
  authRejections: number;
  /** Forward invokes rejected by the dispatch gate. */
  dispatchDenials: number;
}

export interface MinimalResponder {
  readonly transport: Transport;
  readonly sessionId: string;
  readonly stats: MinimalResponderStats;
  /** Register a served tool handler (last-wins overwrite). */
  registerToolHandler(capabilityId: string, handler: TestToolHandler): void;
  /**
   * Issue a reverse tool invoke toward the dialer and await the verified
   * response, mapped to `SpokeResult`. `args` is the tool arguments object
   * — the harness wraps it as the frozen request payload
   * `{ "arguments": <opaque JSON> }` (`tool-contracts.md` §4), mirroring
   * the `connectResponder.invokeTool` face. `sequence` overrides the
   * auto-allocated outbound counter (test control for sequence-gap /
   * tamper fixtures that must re-issue a specific sequence).
   */
  issueInvoke(
    op: string,
    args: Record<string, unknown>,
    sequence?: number,
  ): Promise<SpokeResult<unknown>>;
  /** Close the connection (fails the client's pending recv / invokes). */
  close(): void;
}

/**
 * Start a minimal responder: performs the signed-hello handshake over
 * `transport` in the background (the client's dial is the synchronization
 * point), then serves forward invokes and correlates responses to its own
 * reverse invokes.
 */
export async function startMinimalResponder(
  options: MinimalResponderOptions,
): Promise<MinimalResponder> {
  const { transport: transportEnd, seed, clientPubkey, allowlist, manifest } =
    options;
  const invokeTimeoutMs = options.invokeTimeoutMs ?? 2000;
  const clientPeerId = derivePeerIdFromEd25519Pubkey(clientPubkey);
  const responderPeerId = derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed));
  const nonceStore = new NonceStore();
  const stats: MinimalResponderStats = {
    hellosVerified: 0,
    handlersRun: 0,
    reverseInvokesIssued: 0,
    responsesVerified: 0,
    sequenceRejections: 0,
    authRejections: 0,
    dispatchDenials: 0,
  };

  let session: Session | null = null;
  let closed = false;
  const toolHandlers = new Map<string, TestToolHandler>();
  const pendingReverse = new Map<
    string,
    {
      correlation: Correlation;
      resolve: (result: SpokeResult<unknown>) => void;
      timer: ReturnType<typeof setTimeout>;
    }
  >();

  function sendEnvelope(doc: unknown): void {
    void transportEnd.send(encodeEnvelope(doc)).catch(() => {
      // Peer gone — responses are fire-and-forget at the serving boundary.
    });
  }

  async function sendErrorEnvelope(
    doc: ConnectInvokeRequest,
    error: ErrorEnvelope,
  ): Promise<void> {
    sendEnvelope(
      await authenticateInvokeResponse(seed, {
        session_id: doc.session_id,
        sequence: doc.sequence,
        request_id: doc.request_id,
        error,
      }),
    );
  }

  async function sendOkResponse(
    doc: ConnectInvokeRequest,
    payload: unknown,
  ): Promise<void> {
    sendEnvelope(
      await authenticateInvokeResponse(seed, {
        session_id: doc.session_id,
        sequence: doc.sequence,
        request_id: doc.request_id,
        payload: (payload ?? {}) as Record<string, unknown>,
      }),
    );
  }

  // ── Serving pipeline (frozen order: classify → stray → peek → verify →
  // ── advance → gate → handler → signed response) ─────────────────────────

  /**
   * Gate phase — sequence peek (non-mutating) → envelope-auth verify →
   * advance. Auth-before-advance: a failed verify leaves the inbound
   * counter unchanged. Returns `null` for stray requests (ignored), a
   * rejection spec for gate failures, or `{ok: true}` when dispatch may
   * run. The caller awaits this inline so peek → verify → advance are
   * serialized per session.
   */
  async function runGate(
    doc: ConnectInvokeRequest,
  ): Promise<
    | { ok: true }
    | { ok: false; code: string; message: string; details?: Record<string, unknown> }
    | null
  > {
    const current = session;
    if (current === null) {
      return null; // stray — no established session
    }
    // Stray check: a session_id bound to a different live session is
    // ignored; a session_id bound to no established session stays on this
    // path and is rejected `auth_failed` at verify (session-binding
    // assert) — single-session peer, so verify owns the binding check.
    try {
      current.peekInboundSequence(doc.sequence);
    } catch {
      stats.sequenceRejections += 1;
      return {
        ok: false,
        code: "invalid_sequence",
        message: `inbound sequence ${doc.sequence} is not the next expected`,
      };
    }
    try {
      await verifyInvokeRequestAuth(clientPubkey, doc, current.session_id);
    } catch (error) {
      if (error instanceof EnvelopeAuthError) {
        stats.authRejections += 1;
        return {
          ok: false,
          code: "auth_failed",
          message: error.message,
          details: { kind: error.kind },
        };
      }
      throw error; // wrong-length key is responder misconfiguration — fail loudly
    }
    current.acceptInboundSequence(doc.sequence);
    return { ok: true };
  }

  /** Dispatch phase — runs after the serialized gate; may interleave. */
  async function handleInvoke(doc: ConnectInvokeRequest): Promise<void> {
    const current = session;
    if (current === null) {
      return; // stray — belt-and-braces; the gate checked
    }

    // Dispatch gate — `Session.dispatchAllowed`-level logic (frozen §3):
    // `tools.*` ops require the op string itself, evaluated against
    // `negotiated_capabilities`.
    if (!current.dispatchAllowed(doc.op)) {
      stats.dispatchDenials += 1;
      await sendErrorEnvelope(doc, {
        code: "op_unsupported",
        message: `op ${doc.op} is not authorized by this session`,
        extensions: {},
      });
      return;
    }

    // Test-only response override (malformed-response fixtures).
    const override = options.responseOverride?.(doc);
    if (override !== undefined) {
      sendEnvelope(
        await authenticateInvokeResponse(
          seed,
          overrideSignInput(override),
        ),
      );
      return;
    }

    // Handler or deny (fail-closed serving, frozen §4).
    const handler = toolHandlers.get(doc.op);
    if (handler === undefined) {
      stats.dispatchDenials += 1;
      await sendErrorEnvelope(doc, {
        code: "op_unsupported",
        message: `no handler registered for ${doc.op}`,
        extensions: {},
      });
      return;
    }
    let result: SpokeResult<unknown>;
    try {
      result = await handler(
        (doc.payload as { arguments?: Record<string, unknown> }).arguments ?? {},
      );
    } catch (error) {
      // Handler threw → error branch via toErrorEnvelope (INTERNAL_ERROR);
      // never crash the serve loop.
      result = spokeReject(
        SpokeRejectCode.INTERNAL_ERROR,
        error instanceof Error ? error.message : String(error),
      );
    }
    if (result.ok) {
      stats.handlersRun += 1;
      await sendOkResponse(doc, { result: "value" in result ? result.value : undefined });
    } else {
      stats.handlersRun += 1;
      await sendErrorEnvelope(doc, toErrorEnvelope(result));
    }
  }

  // ── Handshake (allowlist → hello verify → nonce → dial-bound hello →
  // ── session snapshot) ────────────────────────────────────────────────────

  async function handshake(): Promise<void> {
    const helloDoc: unknown = decodeJsonMessage(await transportEnd.recv());
    if (!isConnectHello(helloDoc)) {
      throw new Error("expected ConnectHello from client");
    }
    if (!isAllowlisted(allowlist, helloDoc.peer_id)) {
      throw new Error(`peer ${helloDoc.peer_id} not on allowlist`);
    }
    await verifyHelloEd25519(clientPubkey, clientPeerId, helloDoc);
    if (!nonceStore.checkAndRecord(helloDoc.peer_id, helloDoc.nonce)) {
      throw new Error("nonce replay");
    }
    stats.hellosVerified += 1;

    session = new Session({
      session_id: SESSION_ID,
      initiator_peer_id: clientPeerId,
      responder_peer_id: responderPeerId,
      negotiated_capabilities: negotiatedCapabilities(
        manifest.capabilities,
        helloDoc.host.capabilities,
      ),
    });

    sendEnvelope(
      await signHelloEd25519(seed, generateNonce(), manifest, helloDoc.nonce),
    );
    const snapshot = await authenticateSession(
      seed,
      {
        session_id: SESSION_ID,
        initiator_peer_id: clientPeerId,
        responder_peer_id: responderPeerId,
        opened_at: new Date().toISOString(),
        negotiated_capabilities:
          session.negotiated_capabilities.length > 0
            ? (session.negotiated_capabilities as [string, ...string[]])
            : (["spoke-baseline"] as [string, ...string[]]),
        initial_sequence: 0,
      },
      {},
    );
    sendEnvelope(snapshot);
  }

  // ── Serve loop (request-first classification) ────────────────────────────

  async function serve(): Promise<void> {
    while (!closed) {
      let doc: unknown;
      try {
        doc = decodeJsonMessage(await transportEnd.recv());
      } catch {
        return; // transport closed — stop serving
      }
      // Classify request shape FIRST (an op-bearing doc is never a
      // response, even though it carries the same echo fields + payload).
      if (isConnectInvokeRequest(doc)) {
        // Gate serialization: await peek → verify → advance inline; the
        // loop reads the next envelope only after this gate completes.
        // Dispatch fires without blocking the loop.
        let gate:
          | { ok: true }
          | { ok: false; code: string; message: string; details?: Record<string, unknown> }
          | null;
        try {
          gate = await runGate(doc);
        } catch {
          // Misconfiguration — fail the request closed, never crash the loop.
          await sendErrorEnvelope(doc, {
            code: "auth_failed",
            message: "responder gate failure",
            details: { kind: "envelope_auth_invalid" },
            extensions: {},
          }).catch(() => {});
          continue;
        }
        if (gate === null) {
          continue; // stray — ignored
        }
        if (!gate.ok) {
          await sendErrorEnvelope(doc, {
            code: gate.code,
            message: gate.message,
            ...(gate.details !== undefined ? { details: gate.details } : {}),
            extensions: {},
          });
          continue;
        }
        void handleInvoke(doc).catch(() => {
          // Handler failures are contained in handleInvoke; anything that
          // escapes is a harness bug — never crash the loop.
        });
        continue;
      }
      // Response branch — demux by request_id.
      if (isConnectInvokeResponse(doc)) {
        const entry = pendingReverse.get(doc.request_id);
        if (entry === undefined) {
          continue; // unknown/duplicate response — dropped
        }
        clearTimeout(entry.timer);
        pendingReverse.delete(doc.request_id);
        try {
          checkResponseCorrelation(entry.correlation, correlationFromResponse(doc));
          await verifyInvokeResponseAuth(clientPubkey, doc, session?.session_id ?? "");
          stats.responsesVerified += 1;
          if ("error" in doc) {
            entry.resolve(mapErrorEnvelope(doc.error));
          } else if (
            typeof doc.payload === "object" &&
            doc.payload !== null &&
            "result" in doc.payload
          ) {
            entry.resolve(spokeOk(doc.payload.result));
          } else {
            entry.resolve(
              spokeReject(
                SpokeRejectCode.INTERNAL_ERROR,
                "response payload does not carry a result key",
                { kind: "transport" },
              ),
            );
          }
        } catch (error) {
          if (error instanceof EnvelopeAuthError) {
            entry.resolve(
              spokeReject(SpokeRejectCode.INTERNAL_ERROR, error.message, {
                kind: error.kind,
              }),
            );
          } else if (error instanceof CoreError && error.code === "crypto") {
            entry.resolve(
              spokeReject(
                SpokeRejectCode.INTERNAL_ERROR,
                error.message,
                { kind: "envelope_auth_invalid" },
              ),
            );
          } else {
            entry.resolve(
              spokeReject(
                SpokeRejectCode.INTERNAL_ERROR,
                "response echo fields do not match the request",
                { kind: "correlation_mismatch" },
              ),
            );
          }
        }
        continue;
      }
      // Stray envelope (hello / session / unknown shape) — ignored.
    }
  }

  // Run the handshake + serve loop in the background (the client's dial is
  // the synchronization point). A hello-gate failure closes the transport,
  // which fails the client's dial fast instead of waiting out its timeout.
  void (async () => {
    try {
      await handshake();
      await serve();
    } catch {
      transportEnd.close?.();
    }
  })();

  return {
    transport: transportEnd,
    sessionId: SESSION_ID,
    stats,
    registerToolHandler(capabilityId, handler) {
      // Grammar gate parity with the production surface (D13): a non-`tools.`
      // capability id is a test-harness programming error — throw instead of
      // registering a handler that could never be dispatched.
      const parsed = parseToolCapabilityId(capabilityId);
      if (!parsed.ok) {
        throw new Error(parsed.message);
      }
      toolHandlers.set(capabilityId, handler);
    },
    async issueInvoke(op, args, sequence) {
      const current = session;
      if (current === null) {
        return spokeReject(
          SpokeRejectCode.INTERNAL_ERROR,
          "connect session is not established",
          { kind: "session_closed" },
        );
      }
      stats.reverseInvokesIssued += 1;
      let seq: number;
      if (sequence === undefined) {
        try {
          seq = current.allocateOutboundSequence();
        } catch {
          return spokeReject(
            SpokeRejectCode.INTERNAL_ERROR,
            "outbound sequence space exhausted",
            { kind: "sequence_exhausted" },
          );
        }
      } else {
        seq = sequence;
      }
      const requestId = globalThis.crypto.randomUUID();
      const signed = await authenticateInvokeRequest(seed, {
        session_id: current.session_id,
        sequence: seq,
        request_id: requestId,
        op,
        // Tool invoke payload shape: `{ "arguments": <opaque JSON> }` (§4).
        payload: { arguments: args },
      });
      // Waiter registered before send: the reverse invoke is in flight
      // immediately from the caller's perspective.
      return new Promise<SpokeResult<unknown>>((resolve) => {
        const timer = setTimeout(() => {
          pendingReverse.delete(requestId);
          resolve(
            spokeReject(
              SpokeRejectCode.INTERNAL_ERROR,
              `reverse invoke ${op} (${requestId}) timed out after ${invokeTimeoutMs}ms`,
              { kind: "timeout" },
            ),
          );
        }, invokeTimeoutMs);
        pendingReverse.set(requestId, {
          correlation: {
            session_id: current.session_id,
            sequence: seq,
            request_id: requestId,
          },
          resolve,
          timer,
        });
        void transportEnd.send(encodeEnvelope(signed)).catch(() => {
          clearTimeout(timer);
          pendingReverse.delete(requestId);
          resolve(
            spokeReject(
              SpokeRejectCode.INTERNAL_ERROR,
              "reverse invoke send failed",
              { kind: "transport" },
            ),
          );
        });
      });
    },
    close() {
      closed = true;
      transportEnd.close?.();
    },
  };
}
