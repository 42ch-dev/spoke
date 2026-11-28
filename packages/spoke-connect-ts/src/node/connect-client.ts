/**
 * Minimal SPOKE connect client (AD-P0-6), Node-only.
 *
 * Dial a WebSocket, perform the signed hello exchange + session snapshot,
 * then invoke ops with correlation over the framing layer (AD-P0-3):
 * - send a signed `ConnectHello` (nonce ≥16; hello stays at the core
 *   protocol version while post-hello envelopes carry v2 signatures);
 * - verify the server's signed hello against a preconfigured public key
 *   (obtaining the key is transport-adapter-owned, spec §Auth model) and
 *   require the server's peer_id on the allowlist (fail-closed);
 * - accept the `ConnectSession` snapshot carrying the A-assigned session id
 *   and the authenticated peer binding (initial_sequence must be 0);
 * - `invoke` allocates the next outbound sequence (first = 0), attaches a
 *   fresh `request_id`, and correlates the response's echo fields.
 */

import type {
  ConnectInvokeResponse,
  HostCapabilityManifest,
} from "@42ch/spoke-schemas";
import { WebSocket } from "ws";

import { getPublicKeyEd25519 } from "../crypto.js";
import { derivePeerIdFromEd25519Pubkey } from "../identity.js";
import { isAllowlisted } from "../core/allowlist.js";
import {
  checkResponseCorrelation,
  correlationFromResponse,
} from "../core/correlate.js";
import type { Correlation } from "../core/correlate.js";
import {
  authenticateInvokeRequest,
  verifyInvokeResponseAuth,
  verifySessionAuth,
  type InvokeRequestSignInput,
} from "../core/envelope-auth.js";
import { signHelloEd25519, verifyHelloEd25519 } from "../core/hello.js";
import { generateNonce } from "../core/nonce.js";
import { negotiatedCapabilities, Session } from "../core/session.js";
import { onJsonMessage, sendJsonMessage } from "./ws.js";

/** Raw Ed25519 keypair material for a connect peer. */
export interface ConnectIdentity {
  seed: Uint8Array;
}

export interface ConnectClientOptions {
  url: string;
  identity: ConnectIdentity;
  manifest: HostCapabilityManifest;
  remotePubkey: Uint8Array;
  allowlist: readonly string[];
  timeoutMs?: number;
}

export interface ConnectClient {
  readonly sessionId: string;
  readonly remotePeerId: string;
  readonly remoteManifest: HostCapabilityManifest;

  invoke(op: string, payload: Record<string, unknown>): Promise<ConnectInvokeResponse>;

  close(): void;
}

const DEFAULT_TIMEOUT_MS = 5000;

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

import {
  isConnectHello,
  isConnectInvokeRequest,
  isConnectInvokeResponse,
  isConnectSession,
} from "../remote/guards.js";
export {
  isConnectHello,
  isConnectSession,
  isConnectInvokeRequest,
  isConnectInvokeResponse,
} from "../remote/guards.js";

interface PendingInvoke {
  correlation: Correlation;
  op: string;
  resolve: (response: ConnectInvokeResponse) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

/**
 * Connect to `options.url`, perform the hello exchange + session snapshot,
 * and return an established client.
 */
export async function connectClient(
  options: ConnectClientOptions,
): Promise<ConnectClient> {
  const { url, identity, manifest, remotePubkey, allowlist } = options;
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  if (identity.seed.length !== 32) {
    throw new Error("identity.seed must be 32 bytes");
  }
  if (remotePubkey.length !== 32) {
    throw new Error("remotePubkey must be 32 bytes");
  }

  const remotePeerId = derivePeerIdFromEd25519Pubkey(remotePubkey);
  if (!isAllowlisted(allowlist, remotePeerId)) {
    throw new Error(`remote peer ${remotePeerId} is not on the allowlist (fail-closed)`);
  }
  const localPeerId = derivePeerIdFromEd25519Pubkey(
    getPublicKeyEd25519(identity.seed),
  );

  const socket = new WebSocket(url);
  try {
    await withTimeout(
      new Promise<void>((resolve, reject) => {
        socket.once("open", () => resolve());
        socket.once("error", (error) => reject(error));
      }),
      timeoutMs,
      `dial ${url}`,
    );
  } catch (error) {
    socket.close();
    throw error;
  }

  const inbox: unknown[] = [];
  const waiters: {
    resolve: (doc: unknown) => void;
    reject: (error: Error) => void;
  }[] = [];
  const pending = new Map<string, PendingInvoke>();
  let sessionEstablished = false;
  let sendTail: Promise<void> = Promise.resolve();

  function nextMessage(): Promise<unknown> {
    const buffered = inbox.shift();
    if (buffered !== undefined) {
      return Promise.resolve(buffered);
    }
    return new Promise((resolve, reject) => waiters.push({ resolve, reject }));
  }

  function failAll(error: Error): void {
    for (const entry of pending.values()) {
      clearTimeout(entry.timer);
      entry.reject(error);
    }
    pending.clear();
    for (const waiter of waiters.splice(0)) {
      waiter.reject(error);
    }
  }

  socket.on("error", () => failAll(new Error("websocket error before invoke completion")));
  socket.on("close", () => failAll(new Error("websocket closed before invoke completion")));

  onJsonMessage(
    socket,
    async (doc) => {
      const waiter = waiters.shift();
      if (waiter) {
        waiter.resolve(doc);
        return;
      }
      if (isConnectInvokeResponse(doc)) {
        const entry = pending.get(doc.request_id);
        if (!entry) {
          return;
        }
        clearTimeout(entry.timer);
        pending.delete(doc.request_id);
        try {
          // Correlation echo check first (non-mutating wire-position
          // validation), then envelope-auth verify (contract §7: the
          // response must carry the peer's authentic signature over the
          // exact wire branch). A forged/tampered response fails closed —
          // only this waiter, never a session-state mutation.
          checkResponseCorrelation(entry.correlation, correlationFromResponse(doc));
          await verifyInvokeResponseAuth(
            remotePubkey,
            doc,
            session.session_id,
            entry.op,
          );
          entry.resolve(doc);
        } catch (error) {
          entry.reject(error instanceof Error ? error : new Error(String(error)));
        }
        return;
      }
      if (sessionEstablished) {
        return;
      }
      inbox.push(doc);
    },
    (error) => {
      failAll(new Error(`malformed JSON frame: ${error.message}`));
      socket.close();
    },
  );

  let remoteManifest: HostCapabilityManifest;
  let session: Session;
  try {
    const initiatorNonce = generateNonce();
    sendJsonMessage(
      socket,
      await signHelloEd25519(identity.seed, initiatorNonce, manifest),
    );

    const helloDoc = await withTimeout(nextMessage(), timeoutMs, "server hello");
    if (!isConnectHello(helloDoc)) {
      throw new Error("expected ConnectHello from server");
    }
    await verifyHelloEd25519(remotePubkey, remotePeerId, helloDoc, initiatorNonce);
    remoteManifest = helloDoc.host;

    const sessionDoc = await withTimeout(nextMessage(), timeoutMs, "session snapshot");
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

    session = new Session({
      session_id: sessionDoc.session_id,
      initiator_peer_id: localPeerId,
      responder_peer_id: remotePeerId,
      negotiated_capabilities: negotiatedCapabilities(
        manifest.capabilities,
        remoteManifest.capabilities,
      ),
    });
  } catch (error) {
    failAll(error instanceof Error ? error : new Error(String(error)));
    socket.close();
    throw error;
  }

  sessionEstablished = true;

  return {
    sessionId: session.session_id,
    remotePeerId,
    remoteManifest,

    async invoke(op: string, payload: Record<string, unknown>): Promise<ConnectInvokeResponse> {
      let sequence: number;
      try {
        sequence = session.allocateOutboundSequence();
      } catch (error) {
        return Promise.reject(
          error instanceof Error ? error : new Error(String(error)),
        );
      }
      const prevSend = sendTail;
      let releaseSend: () => void = () => {};
      sendTail = new Promise<void>((resolveTail) => {
        releaseSend = resolveTail;
      });
      // The wire request to sign (`spoke-connect-invoke-request-jcs-v1`,
      // envelope-auth contract §2/§3). `request_id` is allocated here,
      // synchronously, so the pending waiter can be registered before the
      // async sign below.
      const unsigned: InvokeRequestSignInput = {
        session_id: session.session_id,
        sequence,
        request_id: globalThis.crypto.randomUUID(),
        op,
        payload,
      };
      return new Promise<ConnectInvokeResponse>((resolve, reject) => {
        // The waiter is registered SYNCHRONOUSLY, before the async sign:
        // from the caller's perspective the invoke is in flight immediately,
        // so a socket close during signing fails it with the failAll
        // message (contract §8.2 mid-flight) — not a send-after-close ws
        // error. Signing failures (non-JSON-serializable payload) are local
        // client bugs and reject immediately with the sign error.
        const timer = setTimeout(() => {
          pending.delete(unsigned.request_id);
          reject(
            new Error(`invoke ${op} (${unsigned.request_id}) timed out after ${timeoutMs}ms`),
          );
        }, timeoutMs);
        pending.set(unsigned.request_id, {
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
        void authenticateInvokeRequest(identity.seed, unsigned).then(
          (request) => {
            void prevSend
              .then(() => {
                if (!pending.has(unsigned.request_id)) {
                  failAll(
                    new Error(
                      "connect session closed: deferred invoke skipped (settled while queued) — outbound sequence never transmitted",
                    ),
                  );
                  socket.close();
                  return;
                }
                return sendJsonMessage(socket, request);
              })
              .then(
                () => {
                  releaseSend();
                },
                (error) => {
                  releaseSend();
                  clearTimeout(timer);
                  pending.delete(unsigned.request_id);
                  reject(error instanceof Error ? error : new Error(String(error)));
                },
              );
          },
          (error) => {
            releaseSend();
            clearTimeout(timer);
            pending.delete(unsigned.request_id);
            reject(error instanceof Error ? error : new Error(String(error)));
          },
        );
      });
    },

    close(): void {
      failAll(new Error("client closed"));
      socket.close();
    },
  };
}
