/**
 * WsTransport remote-close regression test (Phase 5 Bugbot fix #2).
 *
 * `recv()` must reject promptly when the PEER closes the connection even if
 * no `recv()` waiter was pending at close time. The RemoteAdapter's receive
 * loop awaits response verification between `recv()` calls, so a remote
 * close landing in that window must latch the transport closed — otherwise
 * the next `recv()` parks forever and in-flight invokes hang until timeout.
 *
 * The close handshake is fully observed before `recv()` is called: the
 * server-side `close` event only fires after the client echoed the close
 * frame, and a short settle lets the client transport process it. That keeps
 * this test on the no-waiter latch path, not the already-working
 * pending-waiter path.
 */

import type { AddressInfo } from "node:net";

import { describe, expect, it } from "vitest";
import { WebSocket, WebSocketServer } from "ws";

import { WsTransport } from "../src/transport/ws-transport.js";

describe("WsTransport remote close", () => {
  it("recv() rejects promptly after a remote close with no pending waiter", async () => {
    const wss = new WebSocketServer({ host: "127.0.0.1", port: 0 });
    try {
      // Real integration test over a live loopback socket: the delays below
      // exist only to let the OS/ws stack deliver and process the close
      // frame — fake timers cannot drive real network events, so
      // deterministic time control cannot work here.
      const listening = new Promise<void>((resolve, reject) => {
        wss.once("listening", resolve);
        wss.once("error", reject);
      });
      await listening;
      const address = wss.address() as AddressInfo;
      const url = `ws://127.0.0.1:${address.port}`;

      // Construct BEFORE awaiting the server-side connection (the client
      // only dials once the transport exists).
      const transport = new WsTransport(url);
      const serverSocket = await new Promise<WebSocket>((resolve) => {
        wss.once("connection", resolve);
      });
      // Deterministic "client is open" signal: send() awaits the transport's
      // open handshake before putting the (empty) frame on the wire.
      await transport.send(new Uint8Array());

      // Remote close: the SERVER hangs up. The server's `close` event only
      // fires once the client echoed the close frame; the short settle lets
      // the client transport run its close handler — no waiter is pending.
      await new Promise<void>((resolve) => {
        serverSocket.once("close", () => setTimeout(resolve, 50));
        serverSocket.close();
      });

      // No waiter is pending now: pre-fix this hangs (the latch is missing);
      // post-fix recv() rejects because the remote close latched #closed.
      let timer: NodeJS.Timeout | undefined;
      const hung = new Promise<never>((_, reject) => {
        timer = setTimeout(
          () =>
            reject(
              new Error("recv() hung after remote close (no waiter, no latch)"),
            ),
          500,
        );
      });
      try {
        await expect(Promise.race([transport.recv(), hung])).rejects.toThrow(
          /closed/,
        );
      } finally {
        clearTimeout(timer);
      }

      transport.close();
    } finally {
      wss.close();
    }
  });
});
