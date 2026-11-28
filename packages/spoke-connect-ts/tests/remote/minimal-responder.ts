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
  transport: Transport;
  seed: Uint8Array;
  clientPubkey: Uint8Array;
  allowlist: readonly string[];
  manifest: HostCapabilityManifest;
  invokeTimeoutMs?: number;
  responseOverride?: (request: ConnectInvokeRequest) => unknown;
}

export interface MinimalResponderStats {
  hellosVerified: number;
  handlersRun: number;
  reverseInvokesIssued: number;
  responsesVerified: number;
  sequenceRejections: number;
  authRejections: number;
  dispatchDenials: number;
}

export interface MinimalResponder {
  readonly transport: Transport;
  readonly sessionId: string;
  readonly stats: MinimalResponderStats;
  registerToolHandler(capabilityId: string, handler: TestToolHandler): void;
  issueInvoke(
    op: string,
    args: Record<string, unknown>,
    sequence?: number,
  ): Promise<SpokeResult<unknown>>;
  close(): void;
}

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

  async function runGate(
    doc: ConnectInvokeRequest,
  ): Promise<
    | { ok: true }
    | { ok: false; code: string; message: string; details?: Record<string, unknown> }
    | null
  > {
    const current = session;
    if (current === null) {
      return null;
    }
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
      throw error;
    }
    current.acceptInboundSequence(doc.sequence);
    return { ok: true };
  }

  async function handleInvoke(doc: ConnectInvokeRequest): Promise<void> {
    const current = session;
    if (current === null) {
      return;
    }

    if (!current.dispatchAllowed(doc.op)) {
      stats.dispatchDenials += 1;
      await sendErrorEnvelope(doc, {
        code: "op_unsupported",
        message: `op ${doc.op} is not authorized by this session`,
        extensions: {},
      });
      return;
    }

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

  async function serve(): Promise<void> {
    while (!closed) {
      let doc: unknown;
      try {
        doc = decodeJsonMessage(await transportEnd.recv());
      } catch {
        return;
      }
      if (isConnectInvokeRequest(doc)) {
        let gate:
          | { ok: true }
          | { ok: false; code: string; message: string; details?: Record<string, unknown> }
          | null;
        try {
          gate = await runGate(doc);
        } catch {
          await sendErrorEnvelope(doc, {
            code: "auth_failed",
            message: "responder gate failure",
            details: { kind: "envelope_auth_invalid" },
            extensions: {},
          }).catch(() => {});
          continue;
        }
        if (gate === null) {
          continue;
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
        });
        continue;
      }
      if (isConnectInvokeResponse(doc)) {
        const entry = pendingReverse.get(doc.request_id);
        if (entry === undefined) {
          continue;
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
    }
  }

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
        payload: { arguments: args },
      });
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
