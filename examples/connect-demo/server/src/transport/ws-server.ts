/**
 * WebSocket server transport + programmatic demo server surface (plan T3).
 *
 * The server side of the D3 transport seam: a message-oriented `Transport`
 * adapter over one `ws` connection (one connect envelope per WS message —
 * `spoke-connect.md` §Transport framing: WS already frames messages, so no
 * length-prefix is needed), and `serveConnectDemo`, which boots the mock
 * inference host on a real WebSocketServer and attaches a `ConnectHost`
 * responder per connection.
 */

import type { AddressInfo } from "node:net";

import { WebSocket, WebSocketServer } from "ws";

import { NonceStore } from "@42ch/spoke-connect";

import type { EnvelopeBytes, Transport } from "@42ch/spoke-connect/remote";

import { DEMO_SERVER_MANIFEST, MockAdapter } from "../adapter/mock-adapter.js";
import { ConnectHost } from "../host/connect-host.js";
import {
  DEMO_CLIENT_PEER_ID,
  DEMO_CLIENT_PUBKEY,
  DEMO_SERVER_PEER_ID,
  DEMO_SERVER_SEED,
} from "../identities.js";

const DEFAULT_PORT = 8787;

/** Programmatic handle returned by {@link serveConnectDemo}. */
export interface ServeConnectDemoHandle {
  /** The dialable WebSocket URL (127.0.0.1, actual port — 0 = ephemeral). */
  url: string;
  /** The host's derived Ed25519 peer_id (what the client sees as remote). */
  peerId: string;
  /** Stop accepting connections and close every live socket. Idempotent. */
  close(): void;
}

/**
 * Message-oriented `Transport` over one server-side `ws` connection.
 *
 * - `send`: one envelope per WS message; rejects when the connection is
 *   gone (fail-fast, so the responder's fire-and-forget sends never hang).
 * - `recv`: resolves with the next inbound envelope; rejects on close so a
 *   pending `recv` fails fast like a real connection drop.
 * - `close`: idempotent; rejects pending `recv`s and closes the socket.
 */
class WsServerTransport implements Transport {
  readonly #socket: WebSocket;
  #closed = false;
  readonly #buffer: EnvelopeBytes[] = [];
  readonly #waiters: Array<{
    resolve: (bytes: EnvelopeBytes) => void;
    reject: (error: Error) => void;
  }> = [];

  constructor(socket: WebSocket) {
    this.#socket = socket;
    socket.on("message", (data) => this.#push(toEnvelopeBytes(data)));
    // Both events latch closed and reject pending recvs — close/error always
    // follow a drop. Latching matters even with no waiter pending: a later
    // recv() (the host verifies responses between recv() calls) must reject
    // too, matching LoopbackTransport close semantics.
    socket.on("close", () => {
      if (this.#closed) {
        return;
      }
      this.#closed = true;
      this.#failPending(new Error("ws connection closed"));
    });
    socket.on("error", () => {
      if (this.#closed) {
        return;
      }
      this.#closed = true;
      this.#failPending(new Error("ws connection error"));
    });
  }

  send(envelope: EnvelopeBytes): Promise<void> {
    if (this.#closed || this.#socket.readyState !== WebSocket.OPEN) {
      return Promise.reject(new Error("WsServerTransport is closed"));
    }
    return new Promise<void>((resolve, reject) => {
      this.#socket.send(envelope, (error) => {
        if (error) {
          reject(error);
          return;
        }
        resolve();
      });
    });
  }

  recv(): Promise<EnvelopeBytes> {
    if (this.#closed) {
      return Promise.reject(new Error("WsServerTransport is closed"));
    }
    const buffered = this.#buffer.shift();
    if (buffered !== undefined) {
      return Promise.resolve(buffered);
    }
    return new Promise<EnvelopeBytes>((resolve, reject) => {
      this.#waiters.push({ resolve, reject });
    });
  }

  close(): void {
    if (this.#closed) {
      return;
    }
    this.#closed = true;
    this.#failPending(new Error("WsServerTransport is closed"));
    this.#socket.close();
  }

  #push(bytes: EnvelopeBytes): void {
    const waiter = this.#waiters.shift();
    if (waiter !== undefined) {
      waiter.resolve(bytes);
      return;
    }
    this.#buffer.push(bytes);
  }

  #failPending(error: Error): void {
    for (const waiter of this.#waiters.splice(0)) {
      waiter.reject(error);
    }
  }
}

/** View a `ws` message payload as envelope bytes (fresh per message). */
function toEnvelopeBytes(data: unknown): EnvelopeBytes {
  if (Buffer.isBuffer(data)) {
    return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  }
  return new Uint8Array(data as ArrayBuffer);
}

/**
 * Settle when the WebSocketServer starts listening, or reject when binding
 * fails (e.g. EADDRINUSE). Both one-shot listeners are removed on settle: a
 * successful listen must not leave an `error` listener behind (it would
 * swallow later server errors), and a failed bind must not leave a dangling
 * `listening` listener.
 */
function waitForListening(wss: WebSocketServer): Promise<void> {
  return new Promise<void>((resolve, reject) => {
    const cleanup = (): void => {
      wss.off("listening", onListening);
      wss.off("error", onError);
    };
    const onListening = (): void => {
      cleanup();
      resolve();
    };
    const onError = (error: Error): void => {
      cleanup();
      reject(error);
    };
    wss.once("listening", onListening);
    wss.once("error", onError);
  });
}

/**
 * Boot the connect demo host on a real WebSocketServer. `port: 0` binds an
 * ephemeral port (used by the e2e); each incoming connection gets a fresh
 * `ConnectHost` serving a fresh `MockAdapter` (one host per connection).
 * Rejects when the port cannot be bound (e.g. already in use) instead of
 * hanging on `listening` with an unhandled `error`.
 */
export async function serveConnectDemo(
  options: { port?: number } = {},
): Promise<ServeConnectDemoHandle> {
  const port = options.port ?? DEFAULT_PORT;
  const wss = new WebSocketServer({ host: "127.0.0.1", port });
  try {
    await waitForListening(wss);
  } catch (error) {
    wss.close();
    throw new Error(
      `failed to bind connect demo host on port ${port}: ${(error as Error).message}`,
    );
  }
  const address = wss.address() as AddressInfo;
  const url = `ws://127.0.0.1:${address.port}`;
  const connections = new Set<WsServerTransport>();
  // Hello nonce single-use per peer across connections (spec §Nonce): one
  // store for the whole server, so a captured hello cannot re-establish.
  const nonceStore = new NonceStore();

  wss.on("connection", (socket) => {
    const transport = new WsServerTransport(socket);
    connections.add(transport);
    socket.on("close", () => {
      connections.delete(transport);
    });
    const host = new ConnectHost({
      seed: DEMO_SERVER_SEED,
      manifest: DEMO_SERVER_MANIFEST,
      allowlist: [DEMO_CLIENT_PEER_ID],
      peerKeys: {
        [DEMO_CLIENT_PEER_ID]: DEMO_CLIENT_PUBKEY,
      },
      adapter: new MockAdapter(),
      nonceStore,
    });
    host.attach(transport);
  });

  return {
    url,
    peerId: DEMO_SERVER_PEER_ID,
    close(): void {
      // Close every live connection first so the server close is immediate.
      for (const transport of connections) {
        transport.close();
      }
      connections.clear();
      wss.close();
    },
  };
}
