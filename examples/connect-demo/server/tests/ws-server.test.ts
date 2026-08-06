/**
 * serveConnectDemo bind-handling regression tests (QC fix W-A).
 *
 * `serveConnectDemo` must settle on bind failure (e.g. EADDRINUSE) instead
 * of awaiting `listening` forever while the unhandled `error` event crashes
 * the process — and a successful bind must not leave the race's `error`
 * listener attached (it would swallow later server errors).
 */

import { describe, expect, it } from "vitest";
import { WebSocket } from "ws";

import { serveConnectDemo } from "../src/transport/ws-server.js";

describe("serveConnectDemo bind handling", () => {
  it("rejects when the port is already bound (no hang, no unhandled error)", async () => {
    const first = await serveConnectDemo({ port: 0 });
    try {
      // Fixed, actually-bound port: a second server on the same port hits
      // EADDRINUSE. The rejection must name the port.
      const port = Number(new URL(first.url).port);
      await expect(serveConnectDemo({ port })).rejects.toThrow(
        new RegExp(`${port}`),
      );
    } finally {
      first.close();
    }
  });

  it("keeps serving after the bind race settles (no leftover race listeners)", async () => {
    const server = await serveConnectDemo({ port: 0 });
    try {
      // A dial must still connect: proves the listening/error race cleanup
      // did not leave the server in a broken state.
      await new Promise<void>((resolve, reject) => {
        const socket = new WebSocket(server.url);
        socket.once("open", () => {
          socket.close();
          resolve();
        });
        socket.once("error", reject);
      });
    } finally {
      server.close();
    }
  });
});
