/**
 * ConnectHost — spec-faithful connect responder for the demo server.
 *
 * Mirrors the responder phases of the library's test loopback host
 * (`packages/spoke-connect-ts/tests/remote/loopback-host.ts`), reusing the
 * library's PUBLIC session-core primitives:
 *
 *   handshake(): allowlist fail-closed check FIRST → hello verify (peer_id
 *   binding via the preconfigured client pubkey) → nonce replay record →
 *   responder hello carrying the dial-binding `peer_nonce` → signed
 *   `ConnectSession` snapshot (`spoke-connect-session-jcs-v1`).
 *
 *   serve(): per inbound invoke, a serialized gate (`runGate`) runs
 *   peekInboundSequence (non-mutating) → verifyInvokeRequestAuth
 *   (protocol_version 2 envelope-auth) → acceptInboundSequence (the inbound
 *   counter advances ONLY after verify passes), releasing before the
 *   concurrent dispatch phase (`handleInvoke`: product
 *   `op_capability_requirements` dispatch gate → `dispatchPortOp` →
 *   signed response). A failed envelope-auth verify produces no handler
 *   side effect and no session-state mutation, so a forged envelope cannot
 *   desync the session (spec §Envelope authentication — Verify rules).
 *
 * The capability-token step-up gate is NOT enabled (matching the reference
 * hosts): `noise-peerid` allowlist + signature is the whole auth model.
 */

import type {
  ConnectInvokeRequest,
  ErrorEnvelope,
  HostCapabilityManifest,
} from "@42ch/spoke-schemas";
import {
  toErrorEnvelope,
  type BaselinePorts,
} from "@42ch/spoke-operations";
import {
  decodeJsonMessage,
  derivePeerIdFromEd25519Pubkey,
  encodeJsonMessage,
  generateNonce,
  getPublicKeyEd25519,
  isAllowlisted,
  negotiatedCapabilities,
  NonceStore,
  requiredCapability,
  Session,
  signHelloEd25519,
  verifyHelloEd25519,
} from "@42ch/spoke-connect";
import {
  isConnectHello,
  isConnectInvokeRequest,
  type Transport,
} from "@42ch/spoke-connect/remote";

import {
  createEnvelopeAuth,
  EnvelopeAuthError,
  type EnvelopeAuth,
} from "./envelope-auth.js";
import {
  dispatchPortOp,
  PORT_OP_CAPABILITY_REQUIREMENTS,
} from "./port-dispatch.js";

/** Session id prefix — deterministic per dialing peer (opaque, not schema-enforced). */
const SESSION_ID_PREFIX = "demo-connect-session-";

const textEncoder = new TextEncoder();

export interface ConnectHostOptions {
  /** Host Ed25519 seed (hello + envelope-auth identity). */
  seed: Uint8Array;
  /** Host manifest advertised in the signed hello (the client's `remoteManifest`). */
  manifest: HostCapabilityManifest;
  /** Trusted client peer ids (fail-closed allowlist). */
  allowlist: readonly string[];
  /**
   * Preconfigured client Ed25519 public keys by peer_id. Key distribution
   * is transport-adapter-owned per the spec; the demo server knows its
   * demo identities statically.
   */
  peerKeys: Readonly<Record<string, Uint8Array>>;
  /** Local async BaselinePorts served on the remote side. */
  adapter: BaselinePorts;
  /**
   * Product `op_capability_requirements` map (spec §Op dispatch gate). Ops
   * not listed fall back to the core table; unknown ops are denied.
   * Defaults to every `port.*` op → `spoke-baseline`.
   */
  opCapabilityRequirements?: Record<string, string>;
  /**
   * Shared hello-nonce store (spec §Nonce — single-use per peer). Pass one
   * store per server so a captured hello cannot re-establish on a second
   * connection; defaults to a fresh per-instance store.
   */
  nonceStore?: NonceStore;
}

export interface ConnectHostStats {
  /** Signed client hellos that passed allowlist + signature + nonce gates. */
  hellosVerified: number;
  /** Invokes that passed the gates and were dispatched to the local adapter. */
  invokesDispatched: number;
  /** Invokes rejected by the inbound sequence gate. */
  sequenceRejections: number;
  /** Invokes rejected by the envelope-auth verify gate (fail-closed). */
  authRejections: number;
  /** Invokes rejected by the dispatch gate. */
  dispatchDenials: number;
}

/** Result of the serialized gate phase (peek → verify → advance). */
type GateResult =
  | { ok: true }
  | {
      ok: false;
      code: string;
      message: string;
      details?: Record<string, unknown>;
    }
  | null;

/**
 * Connect responder: attach a message-oriented `Transport`, perform the
 * signed-hello handshake, then serve invokes into the local adapter. One
 * host instance serves one connection.
 */
export class ConnectHost {
  readonly stats: ConnectHostStats = {
    hellosVerified: 0,
    invokesDispatched: 0,
    sequenceRejections: 0,
    authRejections: 0,
    dispatchDenials: 0,
  };

  readonly #auth: EnvelopeAuth;
  readonly #seed: Uint8Array;
  readonly #manifest: HostCapabilityManifest;
  readonly #allowlist: readonly string[];
  readonly #peerKeys: Readonly<Record<string, Uint8Array>>;
  readonly #adapter: BaselinePorts;
  readonly #requirements: Record<string, string>;
  readonly #nonceStore: NonceStore;

  #session: Session | null = null;
  /** The verified client's hello public key (set at handshake). */
  #clientPubkey: Uint8Array | null = null;
  #transport: Transport | null = null;
  #closed = false;
  /**
   * Gate-phase serialization tail: `runGate` (peek → verify → advance) must
   * not interleave with a concurrent invoke — the async verify would let a
   * second request peek against the pre-advance counter and be mis-rejected
   * as `inbound_sequence_mismatch`. Invokes queue on this tail; after the
   * gate releases, the dispatch phase runs concurrently.
   */
  #gateTail: Promise<void> = Promise.resolve();

  constructor(options: ConnectHostOptions) {
    this.#auth = createEnvelopeAuth({ seed: options.seed });
    this.#seed = options.seed;
    this.#manifest = options.manifest;
    this.#allowlist = options.allowlist;
    this.#peerKeys = options.peerKeys;
    this.#adapter = options.adapter;
    this.#requirements = {
      ...PORT_OP_CAPABILITY_REQUIREMENTS,
      ...(options.opCapabilityRequirements ?? {}),
    };
    this.#nonceStore = options.nonceStore ?? new NonceStore();
  }

  /** The established session id ("" before the handshake completes). */
  get sessionId(): string {
    return this.#session?.session_id ?? "";
  }

  /**
   * Run the responder loop (handshake then serve) over `transport` in the
   * background. The client's dial is the synchronization point; a
   * hello-gate failure closes the transport so the client's dial fails fast
   * instead of waiting out its timeout.
   */
  attach(transport: Transport): void {
    if (this.#transport !== null) {
      throw new Error("ConnectHost is already attached to a transport");
    }
    this.#transport = transport;
    void (async () => {
      try {
        await this.#handshake(transport);
        await this.#serve(transport);
      } catch {
        // Handshake failure (or host misconfiguration): fail the peer's
        // pending recv like a connection drop.
        transport.close?.();
      }
    })();
  }

  /** Close the connection (fails the peer's pending recv / invokes). Idempotent. */
  close(): void {
    if (this.#closed) {
      return;
    }
    this.#closed = true;
    this.#transport?.close?.();
  }

  // ── handshake ────────────────────────────────────────────────────────────

  async #handshake(transport: Transport): Promise<void> {
    const helloDoc: unknown = decodeJsonMessage(await transport.recv());
    if (!isConnectHello(helloDoc)) {
      throw new Error("expected ConnectHello from client");
    }
    // Allowlist fail-closed check FIRST (mirrors loopback-host.handshake()):
    // an untrusted peer is rejected before any signature work.
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
    this.stats.hellosVerified += 1;

    const hostPeerId = derivePeerIdFromEd25519Pubkey(
      getPublicKeyEd25519(this.#seed),
    );
    this.#clientPubkey = clientPubkey;

    this.#session = new Session({
      session_id: `${SESSION_ID_PREFIX}${clientPeerId}`,
      initiator_peer_id: clientPeerId,
      responder_peer_id: hostPeerId,
      negotiated_capabilities: negotiatedCapabilities(
        this.#manifest.capabilities,
        helloDoc.host.capabilities,
      ),
    });

    // Answer with our signed hello (responder role — dial binding): the
    // hello signs the 5-field object incl. `peer_nonce` = the initiator's
    // nonce, so a captured responder hello cannot be replayed into a fresh
    // dial. Then the signed session snapshot.
    this.#sendEnvelope(
      transport,
      await signHelloEd25519(
        this.#seed,
        generateNonce(),
        this.#manifest,
        helloDoc.nonce,
      ),
    );
    // The session snapshot is signed with the host identity
    // (`spoke-connect-session-jcs-v1`) — the client's dial verifies it
    // against this host's hello public key. The wire snapshot requires ≥1
    // negotiated capability; the client derives its own set from the hellos,
    // so the fallback only covers a degenerate empty-intersection dial.
    const session = this.#session;
    const snapshot = await this.#auth.authenticateSession({
      session_id: session.session_id,
      initiator_peer_id: clientPeerId,
      responder_peer_id: hostPeerId,
      opened_at: new Date().toISOString(),
      negotiated_capabilities:
        session.negotiated_capabilities.length > 0
          ? (session.negotiated_capabilities as [string, ...string[]])
          : (["spoke-baseline"] as [string, ...string[]]),
      initial_sequence: 0,
    });
    this.#sendEnvelope(transport, snapshot);
  }

  // ── serve loop ───────────────────────────────────────────────────────────

  async #serve(transport: Transport): Promise<void> {
    while (!this.#closed) {
      let doc: unknown;
      try {
        const bytes = await transport.recv();
        try {
          doc = decodeJsonMessage(bytes);
        } catch (error) {
          // Unparseable inbound: closes the connection per current
          // semantics, but logged distinctly from a transport close so an
          // operator can tell a dead peer from a protocol violation.
          console.error(
            `[connect-demo] unparseable inbound message; closing connection: ${(error as Error).message}`,
          );
          return;
        }
      } catch {
        return; // transport closed — normal teardown, silent
      }
      if (!isConnectInvokeRequest(doc)) {
        console.error(
          "[connect-demo] ignoring non-invoke envelope (not a connect invoke request)",
        );
        continue; // stray envelope — ignored
      }
      this.#queueInvoke(transport, doc);
    }
  }

  /**
   * Gate phase — inbound sequence peek (non-mutating) → envelope-auth
   * verify → advance. Fail-closed per spec §Envelope authentication — Verify
   * rules: a forged/tampered/stripped signature produces no handler side
   * effect and no session-state mutation. Returns `null` for stray requests
   * (ignored), a rejection spec for gate failures, or `{ok: true}` when the
   * invoke may dispatch.
   */
  async #runGate(doc: ConnectInvokeRequest): Promise<GateResult> {
    const current = this.#session;
    const clientPubkey = this.#clientPubkey;
    if (current === null || clientPubkey === null) {
      return null; // stray request — ignored
    }
    if (doc.session_id !== current.session_id) {
      return null; // stray request — ignored
    }
    try {
      current.peekInboundSequence(doc.sequence);
    } catch {
      this.stats.sequenceRejections += 1;
      return {
        ok: false,
        code: "inbound_sequence_mismatch",
        message: `inbound sequence ${doc.sequence} is not the next expected`,
      };
    }
    try {
      await this.#auth.verifyInvokeRequestAuth(
        clientPubkey,
        doc,
        current.session_id,
      );
    } catch (error) {
      if (error instanceof EnvelopeAuthError) {
        this.stats.authRejections += 1;
        return {
          ok: false,
          code: "auth_failed",
          message: error.message,
          details: { kind: error.kind },
        };
      }
      throw error; // wrong-length key is host misconfiguration — fail loudly
    }
    // Advance the inbound counter only after envelope-auth verify passed.
    current.acceptInboundSequence(doc.sequence);
    return { ok: true };
  }

  /** Dispatch phase — runs after the serialized gate; may interleave. */
  async #handleInvoke(transport: Transport, doc: ConnectInvokeRequest): Promise<void> {
    const current = this.#session;
    if (current === null || doc.session_id !== current.session_id) {
      return; // stray request — ignored (belt-and-braces; the gate checked)
    }

    // Dispatch gate — product map first, then the core table; unknown ops
    // and missing capabilities answer `op_unsupported`.
    const required = this.#requirements[doc.op] ?? requiredCapability(doc.op);
    if (
      required === undefined ||
      !current.negotiated_capabilities.includes(required)
    ) {
      this.stats.dispatchDenials += 1;
      await this.#sendErrorEnvelope(
        transport,
        doc,
        {
          code: "op_unsupported",
          message: `op ${doc.op} is not authorized by this session`,
          extensions: {},
        },
      );
      return;
    }

    // Dispatch to the local adapter; map the outcome to a signed response.
    const result = await dispatchPortOp(
      doc.op,
      doc.payload as Record<string, unknown>,
      this.#adapter,
    );
    this.stats.invokesDispatched += 1;
    if (result.ok) {
      await this.#sendOkResponse(
        transport,
        doc,
        "value" in result ? result.value : {},
      );
    } else {
      await this.#sendErrorEnvelope(transport, doc, toErrorEnvelope(result));
    }
  }

  // ── gate serialization ───────────────────────────────────────────────────

  #queueInvoke(transport: Transport, doc: ConnectInvokeRequest): void {
    const prev = this.#gateTail;
    let release: () => void = () => {};
    this.#gateTail = new Promise<void>((resolve) => {
      release = resolve;
    });
    void (async () => {
      await prev;
      try {
        const gate = await this.#runGate(doc);
        release();
        if (gate === null) {
          return; // stray request — ignored
        }
        if (!gate.ok) {
          await this.#sendErrorEnvelope(transport, doc, {
            code: gate.code,
            message: gate.message,
            ...(gate.details !== undefined ? { details: gate.details } : {}),
            extensions: {},
          });
          return;
        }
        await this.#handleInvoke(transport, doc);
      } catch (error) {
        release();
        throw error;
      }
    })().catch((error) => {
      // Host-side handler failure must not crash the serve loop, but it
      // must not vanish either — one concise line for the demo operator.
      console.error(
        `[connect-demo] invoke handler failed (${doc.op}): ${(error as Error).message}`,
      );
    });
  }

  // ── response helpers ─────────────────────────────────────────────────────

  #sendEnvelope(transport: Transport, doc: unknown): void {
    void transport.send(textEncoder.encode(encodeJsonMessage(doc))).catch(() => {
      // Peer gone — responses are fire-and-forget at the host boundary.
    });
  }

  async #sendOkResponse(
    transport: Transport,
    doc: ConnectInvokeRequest,
    payload: unknown,
  ): Promise<void> {
    this.#sendEnvelope(
      transport,
      await this.#auth.authenticateInvokeResponse({
        session_id: doc.session_id,
        sequence: doc.sequence,
        request_id: doc.request_id,
        payload: (payload ?? {}) as Record<string, unknown>,
      }),
    );
  }

  async #sendErrorEnvelope(
    transport: Transport,
    doc: ConnectInvokeRequest,
    error: ErrorEnvelope,
  ): Promise<void> {
    this.#sendEnvelope(
      transport,
      await this.#auth.authenticateInvokeResponse({
        session_id: doc.session_id,
        sequence: doc.sequence,
        request_id: doc.request_id,
        error,
      }),
    );
  }
}
