/**
 * `RemoteAdapter` — drop-in async `BaselinePorts` over a connect session
 * (frozen contract: `.mstar/iterations/v0-iter030/guides/remote-adapter-contract.md`).
 *
 * PUBLIC surface: the async `BaselinePorts` (six families) + the
 * `connectRemoteAdapter` dial entrypoint + read-only session info
 * (`sessionId`, `remotePeerId`, `remoteManifest`, `state`) + `close`.
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
  ConnectInvokeRequest,
  ConnectInvokeResponse,
  ErrorEnvelope,
  Finding,
  HostCapabilityManifest,
  KnowledgeEntry,
  Relation,
  Rule,
  Scope,
  TimelineEvent,
} from "@42ch/spoke-schemas";
import {
  fromErrorEnvelope,
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  type BaselinePorts,
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
  authenticateSession,
  EnvelopeAuthError,
  verifyInvokeResponseAuth,
  verifySessionAuth,
  type InvokeRequestSignInput,
} from "../core/envelope-auth.js";
import { signHelloEd25519, verifyHelloEd25519 } from "../core/hello.js";
import { generateNonce, NonceStore } from "../core/nonce.js";
import { negotiatedCapabilities, Session } from "../core/session.js";
import { decodeJsonMessage, encodeJsonMessage } from "../framing.js";
import { derivePeerIdFromEd25519Pubkey } from "../identity.js";
import {
  isConnectHello,
  isConnectInvokeResponse,
  isConnectSession,
} from "./guards.js";
import { isValidSuccessPayload } from "./payload.js";
import type { EnvelopeBytes, Transport } from "./transport.js";

const DEFAULT_INVOKE_TIMEOUT_MS = 5000;

/**
 * Process-wide single-use store of **accepted** server-hello
 * `(peer_id, nonce)` pairs (spec §Nonce / replay protection: "Receiver MUST
 * reject a hello whose `(peer_id, nonce)` pair was already accepted"; "an
 * in-memory set for the life of the process is sufficient for the reference
 * stack").
 *
 * The host-side gate (loopback-host.ts / Rust `gate.rs`) records the
 * client's hello on accept; the RemoteAdapter is the receiver of the
 * **server's** hello, so it enforces the same receiver rule here. A replayed
 * signed server hello — captured on an earlier dial through this process —
 * is rejected before any `ConnectSession` snapshot is accepted, so an active
 * transport attacker cannot re-enter `Established` with a stale signature.
 * The store is module-scoped (across adapter instances) for exactly this
 * reason: an adapter dials once, and the replay arrives on a later dial.
 *
 * Cross-restart replay (the store resets) is defeated separately by the
 * **dial binding**: the responder signs its hello over `peer_nonce` = the
 * initiator's nonce, and the dial asserts it — see the verify call below.
 */
let acceptedServerHellos = new NonceStore();

/**
 * Test-only: reset the process-wide accepted-server-hello store (simulates a
 * process restart, so the cross-restart replay test can target the dial
 * binding — the responder-hello `peer_nonce` assert — rather than the
 * in-memory nonce single-use gate). Mirrors the Rust adapter's
 * `reset_accepted_server_hellos_for_test`.
 *
 * @internal test-only — not part of the RemoteAdapter public API.
 */
export function __resetAcceptedServerHellosForTest(): void {
  acceptedServerHellos = new NonceStore();
}

const textEncoder = new TextEncoder();

function encodeEnvelope(doc: unknown): EnvelopeBytes {
  return textEncoder.encode(encodeJsonMessage(doc));
}

/**
 * Bounded wait: race `promise` against a deadline; rejects with a
 * descriptive error on elapse. Every dial await is bounded (no bare waits).
 */
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

/**
 * Internal invoke-failure classes. Consumers only ever observe these mapped
 * to `SpokeResult` `INTERNAL_ERROR` rejects with `details.kind` (contract
 * §8.2) — except `connectRemoteAdapter`, which throws for dial/hello
 * failures (§8.2 last row).
 */
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
} as const;

export type RemoteAdapterState =
  | "Disconnected"
  | "Handshaking"
  | "Established"
  | "Closed";

/** Raw Ed25519 keypair material for the local connect peer. */
export interface RemoteIdentity {
  /** 32-byte Ed25519 seed (raw). */
  seed: Uint8Array;
}

export interface RemoteAdapterOptions {
  /** Message-oriented transport (consumer-provided; loopback ships in-repo). */
  transport: Transport;
  /** This adapter's raw Ed25519 seed. */
  localIdentity: RemoteIdentity;
  /** Host manifest advertised in this adapter's signed hello. */
  localManifest: HostCapabilityManifest;
  /**
   * Preconfigured remote Ed25519 public key. How the key is obtained is
   * transport-adapter-owned (spec §Auth model); the remote's peer_id is
   * derived from it and must be on `allowlist` (fail-closed).
   */
  remotePubkey: Uint8Array;
  /** Trusted remote peer ids — must contain the remote's derived peer_id (fail-closed). */
  allowlist: readonly string[];
  /** Bounded-wait deadline for the handshake and each invoke, ms (default 5000). */
  invokeTimeoutMs?: number;
  /** Optional capability-token proof attached as `auth` on outbound invokes (§3.2/§3.3). */
  capabilityToken?: CapabilityTokenProof;
}

interface PendingInvoke {
  correlation: Correlation;
  resolve: (response: ConnectInvokeResponse) => void;
  reject: (error: RemoteError) => void;
  timer: ReturnType<typeof setTimeout>;
}

/**
 * Single-peer async `BaselinePorts` proxy over an established connect
 * session. Construct via `connectRemoteAdapter` — the constructor alone
 * yields an un-established adapter that fails closed on every port call.
 */
export class RemoteAdapter implements BaselinePorts {
  readonly #transport: Transport;
  readonly #invokeTimeoutMs: number;
  /** This adapter's raw 32-byte Ed25519 seed (hello identity — signs outbound envelopes). */
  readonly #secret: Uint8Array;
  /** The remote peer's 32-byte hello Ed25519 public key (verifies inbound envelopes). */
  readonly #remotePubkey: Uint8Array;
  #capabilityToken?: CapabilityTokenProof;

  // Verification-gating state. All of it is ECMAScript `#`-private (NOT TS
  // `private`): the invoke gate reads these slots, and no JS consumer can
  // write a `#`-slot from outside the class — a forged `Established` /
  // forged `session` / forged remote-manifest cache is unreachable even
  // through an `any` cast or a subclass.
  #stateInternal: RemoteAdapterState = "Disconnected";
  #session: Session | null = null;
  #remoteManifestInternal: HostCapabilityManifest | null = null;
  #receiveLoopRunning = false;
  #pending = new Map<string, PendingInvoke>();
  /**
   * Outbound send serialization tail. Sequences are allocated synchronously
   * in call order, but the Ed25519 sign is async (WebCrypto) — without a
   * chain, signs complete out of order and requests hit the wire
   * out of sequence, which the peer's strict inbound gate rejects. Every
   * invoke chains its send behind the previous invoke's send.
   */
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

  /** Read-only session state (contract §4 labels). */
  get state(): RemoteAdapterState {
    return this.#stateInternal;
  }

  /** The remote-assigned session id (empty before establishment). */
  get sessionId(): string {
    return this.#session?.session_id ?? "";
  }

  /** The verified remote peer id (empty before establishment). */
  get remotePeerId(): string {
    return this.#session?.responder_peer_id ?? "";
  }

  /**
   * The remote peer's `HostCapabilityManifest`, from the authenticated hello
   * `host` (contract §7). Throws when accessed before establishment
   * (programmer misuse — a dialed adapter always has one).
   */
  get remoteManifest(): HostCapabilityManifest {
    if (this.#remoteManifestInternal === null) {
      throw new Error("connect session is not established — remote manifest unavailable");
    }
    return this.#remoteManifestInternal;
  }

  // ── BaselinePorts (async) ───────────────────────────────────────────────

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
   * HostManifestPort: "self" on a RemoteAdapter is the **remote** peer
   * (contract §7). Returns the authenticated hello `host` from the session
   * cache — cache-only, no network round-trip.
   */
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

  /**
   * Release the session and transport. Idempotent; pending invokes fail with
   * `INTERNAL_ERROR` `details.kind = "session_closed"`.
   */
  close(): void {
    this.#closeSession("local shutdown");
  }

  // ── Session lifecycle (hard-private — only `connectRemoteAdapter` dials) ─
  //
  // ECMAScript `#`-private methods AND state fields: invisible to consumers
  // in the shipped `dist/remote/index.d.ts` AND unreachable at runtime (not
  // even through an `any` cast or a subclass), so no consumer can forge
  // `Established` state past the hello/allowlist/nonce verification. The
  // dial entrypoint lives on the class (`connectRemoteAdapter` static below)
  // where these stay callable.

  /** Dial-only. Adapter is in `Handshaking` while dialing. */
  #beginHandshake(): void {
    this.#stateInternal = "Handshaking";
  }

  /** Dial-only: send one handshake envelope. */
  async #sendEnvelope(doc: unknown): Promise<void> {
    await this.#transport.send(encodeEnvelope(doc));
  }

  /** Dial-only: receive + decode the next handshake envelope. */
  async #recvEnvelope(): Promise<unknown> {
    return decodeJsonMessage(await this.#transport.recv());
  }

  /** Dial-only: bind the authenticated session and start the receive loop. */
  #establish(session: Session, remoteManifest: HostCapabilityManifest): void {
    this.#session = session;
    this.#remoteManifestInternal = remoteManifest;
    this.#stateInternal = "Established";
    this.#runReceiveLoop();
  }

  /**
   * All failure paths that make the session unusable. Transitions to
   * `Closed`, settles every pending waiter, and releases the transport
   * (fire-and-forget).
   */
  #closeSession(reason: string): void {
    if (this.#stateInternal === "Closed") {
      return;
    }
    this.#stateInternal = "Closed";
    // Fire-and-forget teardown (contract §2.1: close is optional and
    // idempotent; `close()` is void): neither a synchronous throw nor a
    // rejecting async close may surface — the adapter has already
    // transitioned to `Closed` and every pending invoke is failed below,
    // so there is no caller that could receive the failure (§8.2 defines
    // no close-failure row). A sync throw is caught here; `Promise.resolve`
    // (not `instanceof Promise`) drains cross-realm / non-native thenables.
    let closeResult: unknown;
    try {
      closeResult = this.#transport.close?.();
    } catch {
      closeResult = undefined;
    }
    if (closeResult !== undefined) {
      void Promise.resolve(closeResult).catch(() => {
        // Transport close failure is intentionally swallowed: the session
        // is unusable either way and the adapter already settled waiters.
      });
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

  // ── Receive loop (adapter-owned; port methods never call recv) ──────────

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
        // Transport loss: fail every pending waiter and transition to
        // `Closed` (contract §6 "Transport close mid-flight" / §8.2).
        this.#closeSession(
          `transport loss: ${error instanceof Error ? error.message : String(error)}`,
        );
        return;
      }

      if (isConnectInvokeResponse(doc)) {
        // Demux by request_id; unknown/duplicate responses are dropped
        // (protocol v1 defines no retry).
        const entry = this.#pending.get(doc.request_id);
        if (entry === undefined) {
          continue;
        }
        clearTimeout(entry.timer);
        this.#pending.delete(doc.request_id);
        try {
          // Correlation echo check first (non-mutating wire-position
          // validation), then envelope-auth verify (contract §7: the
          // response must carry the peer's authentic signature over the
          // exact wire branch). A forged/tampered response fails closed —
          // only this waiter, never a session-state mutation.
          checkResponseCorrelation(entry.correlation, correlationFromResponse(doc));
          await verifyInvokeResponseAuth(this.#remotePubkey, doc, session.session_id);
          entry.resolve(doc);
        } catch (error) {
          if (error instanceof EnvelopeAuthError) {
            entry.reject(new RemoteError(error.kind, error.message));
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

      // Post-handshake stray envelope (hello / session / unknown shape):
      // ignored. Unexpected invoke requests are host-role — out of the
      // single-peer client scope (contract §4).
    }
  }

  // ── Invoke path ─────────────────────────────────────────────────────────

  /**
   * Send one op invoke and resolve with its correlated response envelope.
   * Rejects with `RemoteError` on timeout / transport failure / session
   * close / correlation mismatch / sequence exhaustion.
   */
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
      // Outbound sequence exhaustion: the session is unusable (no wrap);
      // close it and fail this invoke with `sequence_exhausted` (§6/§8.2).
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
        resolve,
        reject,
        timer,
      });
      // Outbound wire-order serialization: the tail entry is created HERE,
      // synchronously, so the wire order matches the allocation order even
      // though the Ed25519 sign completes asynchronously (an out-of-order
      // send would be rejected by the peer's strict inbound sequence gate).
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
              // otherwise produce a duplicate dispatch on the host. A
              // timeout drop is worse than a duplicate dispatch though: the
              // allocated outbound sequence was never transmitted, so the
              // peer's inbound gate is stuck at that sequence and every
              // later invoke fails `inbound_sequence_mismatch` — the wire
              // state is unreconcilable. Close the session (fails remaining
              // pending with `session_closed`, mirroring the
              // `sequence_exhausted` precedent at §6/§8.2) instead of
              // leaving a silently poisoned session.
              if (!this.#pending.has(unsigned.request_id)) {
                this.#closeSession(
                  "deferred invoke skipped (settled while queued) — outbound sequence never transmitted",
                );
                return;
              }
              try {
                return this.#transport.send(encodeEnvelope(signed));
              } catch (error) {
                // Synchronous encode failure (e.g. non-JSON-serializable
                // payload): same cleanup, same mapping.
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
                // Async send failure (transport closed between setup and
                // send): settle this invoke now — no dead entry waits out
                // the timeout.
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
   * Test-only: position the outbound counter of the established session so
   * the loopback suite can exercise sequence exhaustion without 2⁵³ real
   * invokes (mirrors the Rust in-module unit test, which reaches
   * `OutboundSequence::set_next` under `#[cfg(test)]`; TS has no cfg(test),
   * so the hook is a guarded public method instead).
   *
   * It never grants state forgery: it throws unless the adapter is already
   * `Established` and only advances an established session's counter.
   *
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
      // 1. Send our signed hello (nonce generated internally — single-use).
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

      // 2. Await + verify the server's signed hello (signature + identity).
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
      // Receiver-side nonce single-use (spec §Nonce / replay protection):
      // record the accepted server hello and reject a replayed one. An
      // active transport attacker cannot reuse a previously-signed hello
      // from the allowlisted peer to re-enter `Established` on a later dial
      // — the replay is rejected here, before any `ConnectSession` snapshot
      // (which Greptile's scenario would otherwise fabricate) is accepted.
      if (!acceptedServerHellos.checkAndRecord(remotePeerId, helloDoc.nonce)) {
        throw new Error(
          `server hello replay rejected: (peer_id ${remotePeerId}, nonce ${helloDoc.nonce}) was already accepted`,
        );
      }
      const remoteManifest: HostCapabilityManifest = helloDoc.host;

      // 3. Await + validate the A-assigned session snapshot. Envelope-auth
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
        throw new Error(
          "session snapshot initial_sequence must be 0 for protocol_version 1",
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
      // Handshake rejection: release the transport so the peer sees a clean
      // disconnect (mirrors the connect-client handshake-rejection pattern).
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
