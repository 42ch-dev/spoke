/**
 * Message-oriented `Transport` seam for `RemoteAdapter` (frozen contract §2).
 *
 * One connect envelope = one `send` / `recv` call — no multi-envelope
 * batching, matching `spoke-connect.md` §Transport framing. The RemoteAdapter
 * owns a single receive loop that calls `recv` continuously and
 * demultiplexes by `request_id`; callers of `BaselinePorts` never call
 * `recv`.
 *
 * WebSocket and other product transports are consumer-side; this module
 * ships the interface plus the in-repo loopback implementation used by
 * tests (frozen contract §2.2: "Loopback | In-repo paired queues for tests;
 * ships in spoke-connect").
 */

/** Bytes of exactly one UTF-8 JSON connect envelope (no multi-envelope batching). */
export type EnvelopeBytes = Uint8Array;

/** Message-oriented transport: one connect envelope per `send` / `recv` call. */
export interface Transport {
  /** Send one envelope. Resolves when the transport has accepted the bytes. */
  send(envelope: EnvelopeBytes): Promise<void>;
  /**
   * Receive the next inbound envelope. Rejects when the transport closes —
   * a pending `recv` must fail fast on connection loss so the adapter can
   * fail its in-flight invokes instead of waiting out their timeout.
   */
  recv(): Promise<EnvelopeBytes>;
  /** Optional: release resources. Idempotent. */
  close?(): void | Promise<void>;
}

/**
 * FIFO message channel: buffered pushes, awaitable pops. One direction of a
 * loopback connection. Closing rejects every pending and future pop.
 */
class LoopbackChannel {
  private readonly buffer: EnvelopeBytes[] = [];
  private readonly waiters: Array<{
    resolve: (bytes: EnvelopeBytes) => void;
    reject: (error: Error) => void;
  }> = [];
  private closed = false;

  /** Push one envelope; resolves the oldest waiting `pop` when one exists. */
  push(bytes: EnvelopeBytes): void {
    if (this.closed) {
      throw new Error("loopback transport is closed");
    }
    const waiter = this.waiters.shift();
    if (waiter !== undefined) {
      waiter.resolve(bytes);
      return;
    }
    this.buffer.push(bytes);
  }

  /**
   * Pop the next envelope. Resolves immediately when buffered, otherwise
   * waits for the next push. Rejects when the channel is closed (buffered
   * messages are lost on close, matching a real connection drop).
   */
  pop(): Promise<EnvelopeBytes> {
    if (this.closed) {
      return Promise.reject(new Error("loopback transport is closed"));
    }
    const buffered = this.buffer.shift();
    if (buffered !== undefined) {
      return Promise.resolve(buffered);
    }
    return new Promise<EnvelopeBytes>((resolve, reject) => {
      this.waiters.push({ resolve, reject });
    });
  }

  close(): void {
    if (this.closed) {
      return;
    }
    this.closed = true;
    for (const waiter of this.waiters.splice(0)) {
      waiter.reject(new Error("loopback transport is closed"));
    }
  }
}

/**
 * Shared bidirectional connection state. Both directions close together:
 * closing one end fails the peer's pending `recv` exactly like a real
 * connection close, so the RemoteAdapter sees transport loss.
 */
class LoopbackConnection {
  readonly clientToServer = new LoopbackChannel();
  readonly serverToClient = new LoopbackChannel();
  private closed = false;

  close(): void {
    if (this.closed) {
      return;
    }
    this.closed = true;
    this.clientToServer.close();
    this.serverToClient.close();
  }
}

/**
 * One end of an in-memory loopback connection. `send` delivers to the peer's
 * `recv`; `close` closes the whole connection (both directions).
 */
export class LoopbackTransport implements Transport {
  private readonly connection: LoopbackConnection;
  private readonly outbound: LoopbackChannel;
  private readonly inbound: LoopbackChannel;

  constructor(
    connection: LoopbackConnection,
    outbound: LoopbackChannel,
    inbound: LoopbackChannel,
  ) {
    this.connection = connection;
    this.outbound = outbound;
    this.inbound = inbound;
  }

  async send(envelope: EnvelopeBytes): Promise<void> {
    // Throws (→ rejected promise) when the connection is closed.
    this.outbound.push(envelope);
  }

  recv(): Promise<EnvelopeBytes> {
    return this.inbound.pop();
  }

  close(): void {
    this.connection.close();
  }
}

/**
 * Create a back-to-back loopback transport pair — `client` and `server`
 * ends of the same in-memory connection. Used by loopback interop tests:
 * the RemoteAdapter dials the `client` end, the test host serves the
 * `server` end.
 */
export function loopbackTransportPair(): {
  client: LoopbackTransport;
  server: LoopbackTransport;
} {
  const connection = new LoopbackConnection();
  return {
    client: new LoopbackTransport(
      connection,
      connection.clientToServer,
      connection.serverToClient,
    ),
    server: new LoopbackTransport(
      connection,
      connection.serverToClient,
      connection.clientToServer,
    ),
  };
}
