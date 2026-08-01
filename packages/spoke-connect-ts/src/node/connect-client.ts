/**
 * Minimal SPOKE connect client (AD-P0-6), Node-only.
 *
 * Dial a WebSocket, perform the signed hello exchange + session snapshot,
 * then invoke ops with correlation over the framing layer (AD-P0-3):
 * - send a signed `ConnectHello` (nonce ≥16, protocol_version 1);
 * - verify the server's signed hello against a preconfigured public key
 *   (obtaining the key is transport-adapter-owned, spec §Auth model) and
 *   require the server's peer_id on the allowlist (fail-closed);
 * - accept the `ConnectSession` snapshot carrying the A-assigned session id
 *   and the authenticated peer binding (initial_sequence must be 0);
 * - `invoke` allocates the next outbound sequence (first = 0), attaches a
 *   fresh `request_id`, and correlates the response's echo fields.
 *
 * Lives under `src/node/` because `ws` is Node-only (AD-P0-5); a browser
 * build swaps in the native WebSocket. Bounded waits only — every await
 * races a `timeoutMs` deadline (default 5000), no bare sleeps.
 */

import type {
  ConnectHello,
  ConnectInvokeRequest,
  ConnectInvokeResponse,
  ConnectSession,
  HostCapabilityManifest,
} from "@42ch/spoke-schemas";
import { WebSocket } from "ws";

import { getPublicKeyEd25519 } from "../crypto.js";
import { derivePeerIdFromEd25519Pubkey } from "../identity.js";
import { isAllowlisted } from "../core/allowlist.js";
import {
  checkResponseCorrelation,
  correlationFromRequest,
  correlationFromResponse,
} from "../core/correlate.js";
import type { Correlation } from "../core/correlate.js";
import { signHelloEd25519, verifyHelloEd25519 } from "../core/hello.js";
import { generateNonce } from "../core/nonce.js";
import { negotiatedCapabilities, Session } from "../core/session.js";
import { onJsonMessage, sendJsonMessage } from "./ws.js";

/** Raw Ed25519 keypair material for a connect peer. */
export interface ConnectIdentity {
  /** 32-byte Ed25519 seed (raw). */
  seed: Uint8Array;
}

export interface ConnectClientOptions {
  /** WebSocket URL of the remote host, e.g. `ws://127.0.0.1:8080`. */
  url: string;
  /** This client's raw Ed25519 seed. */
  identity: ConnectIdentity;
  /** Host manifest advertised in this client's signed hello. */
  manifest: HostCapabilityManifest;
  /**
   * Preconfigured remote (server) Ed25519 public key. How the public key is
   * obtained is transport-adapter-owned (spec §Auth model, Path A without
   * Noise); the server's peer_id is derived from it.
   */
  remotePubkey: Uint8Array;
  /** Trusted remote peer ids — must contain the server's derived peer_id (fail-closed). */
  allowlist: readonly string[];
  /** Bounded-wait deadline for the handshake and each invoke, ms (default 5000). */
  timeoutMs?: number;
}

export interface ConnectClient {
  /** A-assigned session id (from the `ConnectSession` snapshot). */
  readonly sessionId: string;
  /** The verified server peer id. */
  readonly remotePeerId: string;
  /** The verified server host manifest (from its signed hello). */
  readonly remoteManifest: HostCapabilityManifest;

  /**
   * Invoke an op: allocates the next outbound sequence (first = 0), sends
   * `ConnectInvokeRequest`, and resolves with the response after the echo
   * correlation check passes. Rejects on `timeoutMs` elapse, correlation
   * mismatch, or socket failure.
   */
  invoke(op: string, payload: Record<string, unknown>): Promise<ConnectInvokeResponse>;

  /** Close the underlying WebSocket; pending invokes reject. */
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

// Wire-shape discrimination (spec §Transport framing: envelope types are
// distinguishable by JSON shape). Exported for adapters and tests.

/** `ConnectHello` guard: `{protocol_version, peer_id, nonce, host, signature, …}`. */
export function isConnectHello(doc: unknown): doc is ConnectHello {
  return (
    typeof doc === "object" &&
    doc !== null &&
    "protocol_version" in doc &&
    "peer_id" in doc &&
    "nonce" in doc &&
    "host" in doc &&
    "signature" in doc
  );
}

/** `ConnectSession` snapshot guard (wire shape; full field validation happens in the caller). */
export function isConnectSession(doc: unknown): doc is ConnectSession {
  return (
    typeof doc === "object" &&
    doc !== null &&
    "session_id" in doc &&
    "initiator_peer_id" in doc &&
    "responder_peer_id" in doc &&
    "initial_sequence" in doc
  );
}

/** `ConnectInvokeRequest` guard. */
export function isConnectInvokeRequest(doc: unknown): doc is ConnectInvokeRequest {
  return (
    typeof doc === "object" &&
    doc !== null &&
    "session_id" in doc &&
    "sequence" in doc &&
    "request_id" in doc &&
    "op" in doc &&
    "payload" in doc
  );
}

/** `ConnectInvokeResponse` guard (success `payload` branch or error branch). */
export function isConnectInvokeResponse(doc: unknown): doc is ConnectInvokeResponse {
  return (
    typeof doc === "object" &&
    doc !== null &&
    "session_id" in doc &&
    "sequence" in doc &&
    "request_id" in doc &&
    ("payload" in doc || "error" in doc)
  );
}

interface PendingInvoke {
  correlation: Correlation;
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
  await withTimeout(
    new Promise<void>((resolve, reject) => {
      socket.once("open", () => resolve());
      socket.once("error", (error) => reject(error));
    }),
    timeoutMs,
    `dial ${url}`,
  );

  // Handshake messages are consumed in order via a waiter queue; once the
  // session is established, invoke responses route by `request_id`.
  const inbox: unknown[] = [];
  const waiters: ((doc: unknown) => void)[] = [];
  const pending = new Map<string, PendingInvoke>();

  function nextMessage(): Promise<unknown> {
    const buffered = inbox.shift();
    if (buffered !== undefined) {
      return Promise.resolve(buffered);
    }
    return new Promise((resolve) => waiters.push(resolve));
  }

  function failAll(error: Error): void {
    for (const entry of pending.values()) {
      clearTimeout(entry.timer);
      entry.reject(error);
    }
    pending.clear();
  }

  onJsonMessage(socket, (doc) => {
    const waiter = waiters.shift();
    if (waiter) {
      waiter(doc);
      return;
    }
    if (isConnectInvokeResponse(doc)) {
      const entry = pending.get(doc.request_id);
      if (!entry) {
        return; // unknown/duplicate response — no retry semantics in protocol v1
      }
      clearTimeout(entry.timer);
      pending.delete(doc.request_id);
      try {
        checkResponseCorrelation(entry.correlation, correlationFromResponse(doc));
        entry.resolve(doc);
      } catch (error) {
        entry.reject(error instanceof Error ? error : new Error(String(error)));
      }
      return;
    }
    // Handshake-phase envelope (hello / session snapshot) arriving ahead of
    // its waiter — buffered so `nextMessage` can consume it in order. This
    // matters when the peer sends several frames in one TCP segment: ws
    // emits both 'message' events in the same macrotask, before the awaiting
    // continuation re-registers a waiter.
    inbox.push(doc);
  });

  // Send our signed hello.
  sendJsonMessage(
    socket,
    await signHelloEd25519(identity.seed, generateNonce(), manifest),
  );

  // Await the server's signed hello — the next WS message (AD-P0-3).
  const helloDoc = await withTimeout(nextMessage(), timeoutMs, "server hello");
  if (!isConnectHello(helloDoc)) {
    throw new Error("expected ConnectHello from server");
  }
  await verifyHelloEd25519(remotePubkey, remotePeerId, helloDoc);
  const remoteManifest = helloDoc.host;

  // Await the session snapshot carrying the A-assigned session id + binding.
  const sessionDoc = await withTimeout(nextMessage(), timeoutMs, "session snapshot");
  if (!isConnectSession(sessionDoc)) {
    throw new Error("expected ConnectSession snapshot after server hello");
  }
  if (
    sessionDoc.initiator_peer_id !== localPeerId ||
    sessionDoc.responder_peer_id !== remotePeerId
  ) {
    throw new Error("session snapshot peer ids do not match the authenticated hellos");
  }
  if (sessionDoc.session_id.length === 0) {
    throw new Error("session snapshot session_id must not be empty");
  }
  if (sessionDoc.initial_sequence !== 0) {
    throw new Error("session snapshot initial_sequence must be 0 for protocol_version 1");
  }

  const session = new Session({
    session_id: sessionDoc.session_id,
    initiator_peer_id: localPeerId,
    responder_peer_id: remotePeerId,
    negotiated_capabilities: negotiatedCapabilities(
      manifest.capabilities,
      remoteManifest.capabilities,
    ),
  });

  socket.on("error", () => failAll(new Error("websocket error before invoke completion")));
  socket.on("close", () => failAll(new Error("websocket closed before invoke completion")));

  return {
    sessionId: session.session_id,
    remotePeerId,
    remoteManifest,

    invoke(op, payload): Promise<ConnectInvokeResponse> {
      const request: ConnectInvokeRequest = {
        session_id: session.session_id,
        sequence: session.allocateOutboundSequence(),
        request_id: globalThis.crypto.randomUUID(),
        op,
        payload,
        extensions: {},
      };
      return new Promise<ConnectInvokeResponse>((resolve, reject) => {
        const timer = setTimeout(() => {
          pending.delete(request.request_id);
          reject(
            new Error(`invoke ${op} (${request.request_id}) timed out after ${timeoutMs}ms`),
          );
        }, timeoutMs);
        pending.set(request.request_id, {
          correlation: correlationFromRequest(request),
          resolve,
          reject,
          timer,
        });
        sendJsonMessage(socket, request);
      });
    },

    close(): void {
      failAll(new Error("client closed"));
      socket.close();
    },
  };
}
