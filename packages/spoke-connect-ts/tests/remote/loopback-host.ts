/**
 * Loopback test host — a TS connect host serving a local async
 * `BaselinePorts` (e.g. `ToyWorldAdapter`) over the server end of a
 * `loopbackTransportPair` (frozen contract §5.4 "Host handler contract").
 *
 * Host responsibilities, in order per invoke:
 * 1. session_id match;
 * 2. inbound sequence gate (existing session-core `Session.acceptInboundSequence`);
 * 3. dispatch gate via the product `op_capability_requirements` map (default:
 *    the baseline `port.*` → `spoke-baseline` map) composed with the core
 *    table — unknown ops are denied;
 * 4. deserialize the payload, call the local async `BaselinePorts` method;
 * 5. `SpokeResult::Ok` → success response with the value as `payload`;
 *    `SpokeResult::Reject` → error branch via `toErrorEnvelope`.
 *
 * No new wire envelopes — `ConnectInvokeRequest` / `ConnectInvokeResponse`
 * only. This helper is test-only (lives under `tests/`); product hosts are
 * consumer-side.
 */

import type {
  ConnectInvokeRequest,
  Finding,
  KnowledgeEntry,
  Relation,
  Scope,
} from "@42ch/spoke-schemas";
import {
  SpokeRejectCode,
  spokeReject,
  toErrorEnvelope,
  type BaselinePorts,
  type SpokeResult,
} from "@42ch/spoke-operations";

import { getPublicKeyEd25519 } from "../../src/crypto.js";
import { isAllowlisted } from "../../src/core/allowlist.js";
import { requiredCapability } from "../../src/core/dispatch.js";
import { signHelloEd25519, verifyHelloEd25519 } from "../../src/core/hello.js";
import { generateNonce, NonceStore } from "../../src/core/nonce.js";
import { negotiatedCapabilities, Session } from "../../src/core/session.js";
import { decodeJsonMessage, encodeJsonMessage } from "../../src/framing.js";
import { schemaConformantManifest } from "../../src/golden.js";
import { derivePeerIdFromEd25519Pubkey } from "../../src/identity.js";
import {
  isConnectHello,
  isConnectInvokeRequest,
} from "../../src/remote/guards.js";
import type { EnvelopeBytes, Transport } from "../../src/remote/transport.js";

const SESSION_ID = "test-session-loopback-0001";

/**
 * Default product `op_capability_requirements` map for the loopback host
 * (frozen contract §5.1): every baseline `port.*` op requires
 * `spoke-baseline`.
 */
const DEFAULT_PORT_CAPABILITY_REQUIREMENTS: Record<string, string> = {
  "port.knowledge.get": "spoke-baseline",
  "port.knowledge.put": "spoke-baseline",
  "port.relation.get": "spoke-baseline",
  "port.relation.put": "spoke-baseline",
  "port.scope.list_knowledge_entries": "spoke-baseline",
  "port.scope.list_timeline_events": "spoke-baseline",
  "port.finding.put": "spoke-baseline",
  "port.rule.list": "spoke-baseline",
  "port.host.list_peer_manifests": "spoke-baseline",
};

export interface LoopbackHostOptions {
  /** The server end of a loopback transport pair (the end the client dials). */
  transport: Transport;
  /** Host Ed25519 seed. */
  seed: Uint8Array;
  /** The client's Ed25519 public key (key distribution is test-owned). */
  clientPubkey: Uint8Array;
  /** Trusted client peer ids (fail-closed). */
  allowlist: readonly string[];
  /** Local async BaselinePorts served on the remote side (e.g. ToyWorldAdapter). */
  adapter: BaselinePorts;
  /**
   * Product `op_capability_requirements` map (contract §5.1). Ops not
   * listed fall back to the core table; unknown ops are denied. Defaults to
   * the baseline `port.*` → `spoke-baseline` map.
   */
  opCapabilityRequirements?: Record<string, string>;
  /** Optional per-request response delay (ms) — out-of-order fixtures. */
  delay?: (request: ConnectInvokeRequest) => number;
  /**
   * Test-only: when non-undefined for a request, the returned envelope is
   * sent verbatim instead of the host's real response (deterministic
   * malformed-response fixtures, e.g. corrupted echo fields or a garbage
   * success payload).
   */
  responseOverride?: (request: ConnectInvokeRequest) => unknown;
}

export interface LoopbackHostStats {
  /** Signed client hellos that passed allowlist + signature + nonce gates. */
  hellosVerified: number;
  /** Invokes that passed the gates and were dispatched to the local adapter. */
  invokesDispatched: number;
  /** Invokes rejected by the inbound sequence gate. */
  sequenceRejections: number;
  /** Invokes rejected by the dispatch gate. */
  dispatchDenials: number;
  /** Request sequence numbers in response-arrival order (out-of-order fixture). */
  responseOrder: number[];
  /** Whether any invoke carried an `auth` field (capability-token attach). */
  authSeen: boolean;
}

export interface LoopbackHost {
  /** The server end of the loopback transport the client dials. */
  readonly transport: Transport;
  readonly sessionId: string;
  readonly stats: LoopbackHostStats;
  /** Close the connection (fails the client's pending recv / invokes). */
  close(): void;
}

const textEncoder = new TextEncoder();

function encodeEnvelope(doc: unknown): EnvelopeBytes {
  return textEncoder.encode(encodeJsonMessage(doc));
}

/**
 * Start a loopback host: performs the signed-hello handshake over
 * `transport` (allowlist + signature + nonce single-use gates), sends its
 * own signed hello + `ConnectSession` snapshot, then serves invokes — all
 * in the background. The client's dial is the synchronization point; a
 * hello-gate failure closes the transport so the client's dial fails fast.
 */
export async function startLoopbackHost(
  options: LoopbackHostOptions,
): Promise<LoopbackHost> {
  const { transport: transportEnd, seed, clientPubkey, allowlist, adapter } =
    options;
  const clientPeerId = derivePeerIdFromEd25519Pubkey(clientPubkey);
  const hostPeerId = derivePeerIdFromEd25519Pubkey(
    getPublicKeyEd25519(seed),
  );
  const hostManifest = schemaConformantManifest();
  const nonceStore = new NonceStore();
  const requirements = {
    ...DEFAULT_PORT_CAPABILITY_REQUIREMENTS,
    ...(options.opCapabilityRequirements ?? {}),
  };
  const delay = options.delay ?? (() => 0);
  const stats: LoopbackHostStats = {
    hellosVerified: 0,
    invokesDispatched: 0,
    sequenceRejections: 0,
    dispatchDenials: 0,
    responseOrder: [],
    authSeen: false,
  };

  let session: Session | null = null;
  let closed = false;

  function sendEnvelope(doc: unknown): void {
    void transportEnd.send(encodeEnvelope(doc)).catch(() => {
      // Peer gone — responses are fire-and-forget at the host boundary.
    });
  }

  function sendErrorResponse(doc: ConnectInvokeRequest, code: string, message: string): void {
    stats.responseOrder.push(doc.sequence);
    sendEnvelope({
      session_id: doc.session_id,
      sequence: doc.sequence,
      request_id: doc.request_id,
      error: { code, message, extensions: {} },
      extensions: {},
    });
  }

  function sendOkResponse(doc: ConnectInvokeRequest, payload: unknown): void {
    stats.responseOrder.push(doc.sequence);
    sendEnvelope({
      session_id: doc.session_id,
      sequence: doc.sequence,
      request_id: doc.request_id,
      payload: (payload ?? {}) as Record<string, unknown>,
      extensions: {},
    });
  }

  async function handleInvoke(doc: ConnectInvokeRequest): Promise<void> {
    const current = session;
    if (current === null || doc.session_id !== current.session_id) {
      return; // stray request — ignored
    }
    if (doc.auth !== undefined) {
      stats.authSeen = true;
    }

    // 1. Inbound sequence gate — replay/out-of-order throws; no handler
    //    side effect on failure.
    try {
      current.acceptInboundSequence(doc.sequence);
    } catch {
      stats.sequenceRejections += 1;
      sendErrorResponse(
        doc,
        "inbound_sequence_mismatch",
        `inbound sequence ${doc.sequence} is not the next expected`,
      );
      return;
    }

    // 2. Dispatch gate — product map first, then the core table; unknown
    //    ops and missing capabilities answer `op_unsupported`.
    const required = requirements[doc.op] ?? requiredCapability(doc.op);
    if (
      required === undefined ||
      !current.negotiated_capabilities.includes(required)
    ) {
      stats.dispatchDenials += 1;
      sendErrorResponse(
        doc,
        "op_unsupported",
        `op ${doc.op} is not authorized by this session`,
      );
      return;
    }

    // Optional deterministic delay (out-of-order response fixtures).
    const delayMs = delay(doc);
    if (delayMs > 0) {
      await new Promise((resolve) => setTimeout(resolve, delayMs));
    }
    if (closed) {
      return;
    }

    // Test-only response override: replace the envelope the host would send
    // (malformed-response fixtures). The request already passed the gates, so
    // the client has a pending waiter to exercise against.
    const override = options.responseOverride?.(doc);
    if (override !== undefined) {
      stats.responseOrder.push(doc.sequence);
      sendEnvelope(override);
      return;
    }

    // 3. Dispatch to the local adapter.
    const result = await dispatchOp(doc);
    if (result.ok) {
      stats.invokesDispatched += 1;
      sendOkResponse(doc, "value" in result ? result.value : {});
    } else {
      stats.invokesDispatched += 1;
      stats.responseOrder.push(doc.sequence);
      sendEnvelope({
        session_id: doc.session_id,
        sequence: doc.sequence,
        request_id: doc.request_id,
        error: toErrorEnvelope(result),
        extensions: {},
      });
    }
  }

  function dispatchOp(
    doc: ConnectInvokeRequest,
  ): Promise<SpokeResult<unknown>> {
    const payload = doc.payload;
    switch (doc.op) {
      case "port.knowledge.get":
        return adapter.getKnowledgeEntry(payload.entry_id as string);
      case "port.knowledge.put":
        return adapter.putKnowledgeEntry(
          payload.entry as KnowledgeEntry,
          payload.expected_base_revision as number | null,
        );
      case "port.relation.get":
        return adapter.getRelation(payload.relation_id as string);
      case "port.relation.put":
        return adapter.putRelation(
          payload.relation as Relation,
          payload.expected_base_revision as number | null,
        );
      case "port.scope.list_knowledge_entries":
        return adapter.listKnowledgeEntries(payload.scope as Scope);
      case "port.scope.list_timeline_events":
        return adapter.listTimelineEvents(payload.scope as Scope);
      case "port.finding.put":
        return adapter.putFindings(payload.findings as Finding[]);
      case "port.rule.list":
        return adapter.listRules(payload.rule_refs as string[]);
      case "port.host.list_peer_manifests":
        return adapter.listPeerHostCapabilityManifests();
      default:
        // Unreachable: the dispatch gate denies unknown ops first. Kept as
        // a safety net for host misconfiguration.
        return Promise.resolve(
          spokeReject(
            SpokeRejectCode.CAPABILITY_PORT_MISSING,
            `unimplemented port op ${doc.op}`,
            { op: doc.op },
          ),
        );
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
      responder_peer_id: hostPeerId,
      negotiated_capabilities: negotiatedCapabilities(
        hostManifest.capabilities,
        helloDoc.host.capabilities,
      ),
    });

    // Answer with our signed hello (responder role — dial binding): the
    // hello signs the 5-field object incl. `peer_nonce` = the initiator's
    // nonce, so a captured responder hello cannot be replayed into a fresh
    // dial. Then the A-assigned session snapshot.
    sendEnvelope(
      await signHelloEd25519(
        seed,
        generateNonce(),
        hostManifest,
        helloDoc.nonce,
      ),
    );
    const snapshot = {
      session_id: SESSION_ID,
      initiator_peer_id: clientPeerId,
      responder_peer_id: hostPeerId,
      opened_at: new Date().toISOString(),
      // The wire snapshot requires ≥1 negotiated capability; the client
      // derives its own negotiated set from the hellos, so the fallback
      // only covers the degenerate empty-intersection test fixture.
      negotiated_capabilities:
        session.negotiated_capabilities.length > 0
          ? (session.negotiated_capabilities as [string, ...string[]])
          : (["spoke-baseline"] as [string, ...string[]]),
      initial_sequence: 0,
      extensions: {},
    };
    sendEnvelope(snapshot);
  }

  async function serve(): Promise<void> {
    while (!closed) {
      let doc: unknown;
      try {
        doc = decodeJsonMessage(await transportEnd.recv());
      } catch {
        return; // transport closed — stop serving
      }
      if (!isConnectInvokeRequest(doc)) {
        continue; // stray envelope — ignored
      }
      void handleInvoke(doc).catch(() => {
        // Host-side handler failure must not crash the loop.
      });
    }
  }

  // Run the handshake + serve loop in the background: the client's dial is
  // the synchronization point (its hello triggers the host's reply), so
  // awaiting the handshake here would deadlock a caller that dials after
  // `startLoopbackHost` returns. A hello-gate failure closes the transport,
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
    close() {
      closed = true;
      transportEnd.close?.();
    },
  };
}
