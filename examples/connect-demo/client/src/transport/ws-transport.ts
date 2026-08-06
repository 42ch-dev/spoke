/**
 * WsTransport — a message-oriented connect `Transport` over the `ws`
 * WebSocket package (the demo-local consumer implementation of the D3
 * transport seam; roadmap "Per-language WebSocket transports" productizes
 * this pattern).
 *
 * One connect envelope per WS message (message-oriented framing — WS
 * already frames messages, so no length-prefix is needed; `spoke-connect.md`
 * §Transport framing). `recv` rejects when the connection closes so the
 * RemoteAdapter fails its in-flight invokes fast instead of waiting out
 * their timeouts; `close` is idempotent.
 */

import { WebSocket } from "ws";

import type { EnvelopeBytes, Transport } from "@42ch/spoke-connect/remote";

/** A pending `recv` waiter. */
type RecvWaiter = {
  resolve: (bytes: EnvelopeBytes) => void;
  reject: (error: Error) => void;
};

/** View a `ws` message payload as envelope bytes (fresh per message). */
function toEnvelopeBytes(data: unknown): EnvelopeBytes {
  if (Buffer.isBuffer(data)) {
    return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  }
  return new Uint8Array(data as ArrayBuffer);
}

export class WsTransport implements Transport {
  readonly #socket: WebSocket;
  /** Resolves once the socket is open; rejects if the connect fails. */
  readonly #open: Promise<void>;
  #closed = false;
  readonly #buffer: EnvelopeBytes[] = [];
  readonly #waiters: RecvWaiter[] = [];

  constructor(url: string) {
    this.#socket = new WebSocket(url);
    this.#open = new Promise<void>((resolve, reject) => {
      this.#socket.once("open", () => resolve());
      this.#socket.once("error", (error) => {
        reject(
          error instanceof Error
            ? error
            : new Error(`ws connect to ${url} failed`),
        );
      });
    });
    this.#socket.on("message", (data) => this.#push(toEnvelopeBytes(data)));
    // Both events fail pending recvs — a drop always surfaces as close/error.
    const fail = (): void => this.#failPending(new Error("ws connection closed"));
    this.#socket.on("close", fail);
    this.#socket.on("error", fail);
  }

  async send(envelope: EnvelopeBytes): Promise<void> {
    await this.#open;
    if (this.#closed || this.#socket.readyState !== WebSocket.OPEN) {
      throw new Error("WsTransport is closed");
    }
    await new Promise<void>((resolve, reject) => {
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
      return Promise.reject(new Error("WsTransport is closed"));
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
    this.#failPending(new Error("WsTransport is closed"));
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
