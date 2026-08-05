import { describe, expect, it } from "vitest";
import { WebSocket } from "ws";
import { WebSocketServer } from "ws";
import { createServer } from "node:net";

import type { ConnectSession } from "@42ch/spoke-schemas";

import { getPublicKeyEd25519 } from "../src/crypto.js";
import { isAllowlisted } from "../src/core/allowlist.js";
import {
  authenticateInvokeResponse,
  authenticateSession,
  EnvelopeAuthError,
  verifyInvokeRequestAuth,
  type InvokeResponseSignInput,
} from "../src/core/envelope-auth.js";
import { signHelloEd25519, verifyHelloEd25519 } from "../src/core/hello.js";
import { NonceStore } from "../src/core/nonce.js";
import { negotiatedCapabilities, Session } from "../src/core/session.js";
import { schemaConformantManifest } from "../src/golden.js";
import { derivePeerIdFromEd25519Pubkey } from "../src/identity.js";
import {
  connectClient,
  isConnectHello,
  isConnectInvokeRequest,
} from "../src/node/connect-client.js";
import type { ConnectClient } from "../src/node/connect-client.js";
import { onJsonMessage, sendJsonMessage } from "../src/node/ws.js";

// Deterministic fixture values (both ≥ 16 chars for the hello nonce floor).
const NONCE_A = "test-nonce-a-000000000001";
const NONCE_B = "test-nonce-b-000000000001";
const SESSION_ID = "test-session-0000000001";

/** Fixture seed: base+i, all values within byte range for base ≤ 0xe0. */
function seed(base: number): Uint8Array {
  return Uint8Array.from({ length: 32 }, (_, i) => base + i);
}

/**
 * Minimal handshake server for robustness tests: answers the client's
 * signed hello with its own signed hello + `ConnectSession` snapshot, then
 * routes every later frame to `onMessage`. The hello verification gates are
 * intentionally skipped — these tests exercise the client transport, not
 * the server-side hello path (covered by the interop test above).
 */
async function startHandshakeServer(options: {
  seed: Uint8Array;
  peerId: string;
  onMessage: (socket: WebSocket, doc: unknown) => void;
}): Promise<{
  port: number;
  connections: WebSocket[];
  close: () => Promise<void>;
}> {
  const { seed: seedA, peerId: peerIdA, onMessage } = options;
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 });
  const port = await new Promise<number>((resolve, reject) => {
    server.once("error", reject);
    server.once("listening", () => {
      const address = server.address();
      if (address === null || typeof address === "string") {
        reject(new Error("unexpected server address"));
        return;
      }
      resolve(address.port);
    });
  });

  const connections: WebSocket[] = [];
  server.on("connection", (socket) => {
    connections.push(socket);
    let phase: "hello" | "invoke" = "hello";
    onJsonMessage(socket, (doc) => {
      const task =
        phase === "hello"
          ? (async () => {
              if (!isConnectHello(doc)) {
                throw new Error("expected ConnectHello");
              }
              sendJsonMessage(
                socket,
                // Responder role — dial binding: sign the 5-field object
                // incl. `peer_nonce` = the client's (initiator) nonce.
                await signHelloEd25519(
                  seedA,
                  NONCE_A,
                  schemaConformantManifest(),
                  doc.nonce,
                ),
              );
              // A's session snapshot, signed with A's hello identity
              // (`spoke-connect-session-jcs-v1`) — the client's dial
              // verifies it against A's hello public key.
              const snapshot: ConnectSession = await authenticateSession(
                seedA,
                {
                  session_id: SESSION_ID,
                  initiator_peer_id: doc.peer_id,
                  responder_peer_id: peerIdA,
                  opened_at: new Date().toISOString(),
                  negotiated_capabilities: ["spoke-baseline"],
                  initial_sequence: 0,
                },
                {},
              );
              sendJsonMessage(socket, snapshot);
            })()
          : (async () => onMessage(socket, doc))();
      phase = "invoke";
      void task.catch((error) => {
        socket.close(1002, error instanceof Error ? error.message : String(error));
      });
    });
  });

  return {
    port,
    connections,
    close: () => new Promise<void>((resolve) => server.close(() => resolve())),
  };
}

/**
 * Two-node local WebSocket interop (AD-P0-5 / AD-P0-3 "Two-node interop
 * contract"):
 * - Node A = in-process `ws` server on 127.0.0.1:<ephemeral>; accepts B's
 *   signed hello (allowlist + signature + nonce single-use), answers with its
 *   own signed hello, assigns the session via a `ConnectSession` snapshot,
 *   then runs sequence gate → dispatch gate → handler stub per invoke.
 * - Node B = `connectClient`; sends the signed hello, verifies A's hello and
 *   snapshot, then invokes with correlation (outbound sequence 0 → 1 → 2).
 *
 * Bounded waits only (≤ 5s race timeouts), no bare sleeps.
 */
describe("two-node local WebSocket interop", () => {
  it(
    "exchanges signed hellos, establishes a session, and round-trips invokes with correlation",
    async () => {
      const seedA = seed(0xa0);
      const seedB = seed(0x10);
      const pubkeyA = getPublicKeyEd25519(seedA);
      const pubkeyB = getPublicKeyEd25519(seedB);
      const peerIdA = derivePeerIdFromEd25519Pubkey(pubkeyA);
      const peerIdB = derivePeerIdFromEd25519Pubkey(pubkeyB);

      const server = new WebSocketServer({ host: "127.0.0.1", port: 0 });
      const port = await new Promise<number>((resolve, reject) => {
        server.once("error", reject);
        server.once("listening", () => {
          const address = server.address();
          if (address === null || typeof address === "string") {
            reject(new Error("unexpected server address"));
            return;
          }
          resolve(address.port);
        });
      });

      const nonceStore = new NonceStore();
      let serverSession: Session | null = null;
      const connections: WebSocket[] = [];

      // ── Node A: accept/verify + invoke handler stub ────────────────────
      async function handleHello(socket: WebSocket, doc: unknown): Promise<void> {
        if (!isConnectHello(doc)) {
          throw new Error("expected ConnectHello");
        }
        if (!isAllowlisted([peerIdB], doc.peer_id)) {
          throw new Error(`peer ${doc.peer_id} not on allowlist`);
        }
        await verifyHelloEd25519(pubkeyB, peerIdB, doc);
        if (!nonceStore.checkAndRecord(doc.peer_id, doc.nonce)) {
          throw new Error("nonce replay");
        }

        serverSession = new Session({
          session_id: SESSION_ID,
          initiator_peer_id: peerIdB,
          responder_peer_id: peerIdA,
          // The schema-conformant manifests on both sides advertise exactly
          // `spoke-baseline`, so the agreed subset is this single capability.
          negotiated_capabilities: negotiatedCapabilities(
            schemaConformantManifest().capabilities,
            doc.host.capabilities,
          ),
        });

        // A answers with its own signed hello (responder role — dial
        // binding: `peer_nonce` = the client's nonce), then assigns the
        // session.
        sendJsonMessage(
          socket,
          await signHelloEd25519(
            seedA,
            NONCE_A,
            schemaConformantManifest(),
            doc.nonce,
          ),
        );
        // A's session snapshot, signed with A's hello identity
        // (`spoke-connect-session-jcs-v1`) — the client's dial verifies it
        // against A's hello public key.
        const snapshot: ConnectSession = await authenticateSession(
          seedA,
          {
            session_id: SESSION_ID,
            initiator_peer_id: peerIdB,
            responder_peer_id: peerIdA,
            opened_at: new Date().toISOString(),
            negotiated_capabilities: ["spoke-baseline"],
            initial_sequence: 0,
          },
          {},
        );
        sendJsonMessage(socket, snapshot);
      }

      async function handleInvoke(socket: WebSocket, doc: unknown): Promise<void> {
        const session = serverSession;
        if (!session || !isConnectInvokeRequest(doc)) {
          throw new Error("unexpected message before session established");
        }
        if (doc.session_id !== session.session_id) {
          throw new Error("invoke session_id does not match the session");
        }

        // 1. Inbound sequence gate — non-mutating peek (auth-before-advance,
        //    envelope-auth contract §7): the counter advances only after the
        //    envelope-auth verify passes below. No handler side effect on
        //    failure.
        session.peekInboundSequence(doc.sequence);

        // 2. Envelope-auth verify — fail-closed BEFORE dispatch and before
        //    the counter advances: the request must carry B's authentic
        //    signature (`spoke-connect-invoke-request-jcs-v1`).
        try {
          await verifyInvokeRequestAuth(pubkeyB, doc, session.session_id);
        } catch (error) {
          if (error instanceof EnvelopeAuthError) {
            throw new Error(
              `invoke envelope auth failed (${error.kind}): ${error.message}`,
            );
          }
          throw error;
        }

        // 3. Advance the inbound counter only after envelope-auth verify
        //    passed.
        session.acceptInboundSequence(doc.sequence);

        // 4. Dispatch gate — unknown op or missing capability → op_unsupported
        //    error branch (AD-P0-3: map to wire code on the server path).
        if (!session.dispatchAllowed(doc.op)) {
          sendJsonMessage(
            socket,
            await authenticateInvokeResponse(seedA, {
              session_id: doc.session_id,
              sequence: doc.sequence,
              request_id: doc.request_id,
              error: {
                code: "op_unsupported",
                message: `op ${doc.op} is not authorized by this session`,
                extensions: {},
              },
            }),
          );
          return;
        }

        // 5. Handler stub — only `check` is implemented in this fixture;
        //    `payload.fail === true` selects the error branch.
        if (doc.op !== "check") {
          sendJsonMessage(
            socket,
            await authenticateInvokeResponse(seedA, {
              session_id: doc.session_id,
              sequence: doc.sequence,
              request_id: doc.request_id,
              error: {
                code: "op_unsupported",
                message: `unimplemented op ${doc.op}`,
                extensions: {},
              },
            }),
          );
          return;
        }
        const echo = {
          session_id: doc.session_id,
          sequence: doc.sequence,
          request_id: doc.request_id,
        };
        if (doc.payload?.fail === true) {
          sendJsonMessage(
            socket,
            await authenticateInvokeResponse(seedA, {
              ...echo,
              error: {
                code: "check_failed",
                message: "spike check failed",
                extensions: {},
              },
            }),
          );
        } else {
          sendJsonMessage(
            socket,
            await authenticateInvokeResponse(seedA, {
              ...echo,
              payload: { findings: [] },
            }),
          );
        }
      }

      server.on("connection", (socket) => {
        connections.push(socket);
        let phase: "hello" | "invoke" = "hello";
        onJsonMessage(socket, (doc) => {
          const task =
            phase === "hello" ? handleHello(socket, doc) : handleInvoke(socket, doc);
          phase = "invoke";
          // A hello-gate failure closes the stream (spec §Auth model:
          // handshake rejection closes the connection; no hello error envelope).
          void task.catch((error) => {
            socket.close(1002, error instanceof Error ? error.message : String(error));
          });
        });
      });

      // ── Node B: dial + sign hello + invoke (the library client path) ───
      let client: ConnectClient | null = null;
      try {
        client = await connectClient({
          url: `ws://127.0.0.1:${port}`,
          identity: { seed: seedB },
          manifest: schemaConformantManifest(),
          remotePubkey: pubkeyA,
          allowlist: [peerIdA],
          timeoutMs: 5000,
        });

        // Session established: both hellos accepted, A's snapshot validated.
        expect(client.sessionId).toBe(SESSION_ID);
        expect(client.remotePeerId).toBe(peerIdA);
        expect(client.remoteManifest.host_id).toBe("test-host");

        // Invoke 1 — outbound sequence 0 → success branch, correlation echo.
        const res1 = await client.invoke("check", { findings: [] });
        expect(res1).toMatchObject({
          session_id: SESSION_ID,
          sequence: 0,
          payload: { findings: [] },
        });

        // Invoke 2 — outbound sequence 1 → error branch, correlation echo.
        const res2 = await client.invoke("check", { fail: true });
        expect(res2).toMatchObject({
          session_id: SESSION_ID,
          sequence: 1,
          request_id: expect.any(String),
          error: { code: "check_failed", message: "spike check failed" },
        });

        // Invoke 3 — unknown op → dispatch gate deny mapped to wire
        // `op_unsupported` on the error branch (still correlated).
        const res3 = await client.invoke("custom-op", {});
        expect(res3).toMatchObject({
          session_id: SESSION_ID,
          sequence: 2,
          error: { code: "op_unsupported" },
        });
      } finally {
        client?.close();
        for (const c of connections) c.close();
        await new Promise<void>((resolve) => server.close(() => resolve()));
      }
    },
    15000,
  );

  it(
    "rejects fast when the server drops the connection mid-handshake",
    async () => {
      // Deterministic mid-handshake close: the server accepts, waits for the
      // client's hello, then closes the socket without answering. The client
      // must reject with the socket failure (fail-fast), not wait out the
      // 5 s handshake timeout, and its own socket must be released.
      const seedB = seed(0x30);
      const pubkeyA = getPublicKeyEd25519(seed(0xb0));
      const peerIdA = derivePeerIdFromEd25519Pubkey(pubkeyA);

      const server = new WebSocketServer({ host: "127.0.0.1", port: 0 });
      const port = await new Promise<number>((resolve, reject) => {
        server.once("error", reject);
        server.once("listening", () => {
          const address = server.address();
          if (address === null || typeof address === "string") {
            reject(new Error("unexpected server address"));
            return;
          }
          resolve(address.port);
        });
      });

      const connections: WebSocket[] = [];
      server.on("connection", (socket) => {
        connections.push(socket);
        socket.once("message", () => socket.close());
      });

      try {
        const started = Date.now();
        await expect(
          connectClient({
            url: `ws://127.0.0.1:${port}`,
            identity: { seed: seedB },
            manifest: schemaConformantManifest(),
            remotePubkey: pubkeyA,
            allowlist: [peerIdA],
            timeoutMs: 5000,
          }),
        ).rejects.toThrow(/websocket (closed|error)/);
        // Fail-fast: rejection arrives well before the 5 s handshake timeout.
        expect(Date.now() - started).toBeLessThan(2000);
      } finally {
        for (const c of connections) c.close();
        await new Promise<void>((resolve) => server.close(() => resolve()));
      }
    },
    15000,
  );
});

describe("connectClient transport robustness", () => {
  it(
    "rejects an invoke with the failAll message when the socket closes during signing (not a raw ws error)",
    async () => {
      const seedA = seed(0xe0);
      const pubkeyA = getPublicKeyEd25519(seedA);
      const peerIdA = derivePeerIdFromEd25519Pubkey(pubkeyA);
      const seedB = seed(0x60);

      const server = await startHandshakeServer({
        seed: seedA,
        peerId: peerIdA,
        onMessage: () => {
          // Unreachable in this test: the invoke must never reach the wire
          // (the socket is closed before its sign completes).
        },
      });

      let client: ConnectClient | null = null;
      try {
        client = await connectClient({
          url: `ws://127.0.0.1:${server.port}`,
          identity: { seed: seedB },
          manifest: schemaConformantManifest(),
          remotePubkey: pubkeyA,
          allowlist: [peerIdA],
          timeoutMs: 5000,
        });

        // The pending waiter is registered SYNCHRONOUSLY — before the async
        // Ed25519 sign — so a close immediately after `invoke` fails the
        // in-flight invoke with the failAll message ("client closed"). If
        // the entry were registered only after the sign, failAll would miss
        // it and the deferred send would hit the closed socket, rejecting
        // with a raw ws error instead.
        const pending = client.invoke("check", {});
        client.close();
        await expect(pending).rejects.toThrow(/client closed/);
      } finally {
        client?.close();
        for (const c of server.connections) c.close();
        await server.close();
      }
    },
    15000,
  );

  it(
    "closes the socket when the dial times out (no leaked connection)",
    async () => {
      // A TCP server that accepts connections but never completes the
      // WebSocket upgrade: the client's dial never opens and must time out —
      // and the client must close its socket so the server observes the
      // disconnect (the leak this guards: the connection staying
      // established with no cleanup). `resume()` keeps the stream flowing —
      // a paused socket defers `end`/`close` until its data is consumed,
      // which would hide the client's FIN.
      const server = createServer((sock) => {
        sock.resume(); // Swallow the upgrade request — no response ever arrives.
      });
      await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
      const address = server.address();
      if (address === null || typeof address === "string") {
        throw new Error("unexpected server address");
      }
      const serverSideClosed = new Promise<void>((resolve) => {
        server.on("connection", (sock) => sock.once("close", () => resolve()));
      });

      try {
        const seedB = seed(0x40);
        const remotePubkey = getPublicKeyEd25519(seed(0xc0));
        const remotePeerId = derivePeerIdFromEd25519Pubkey(remotePubkey);
        const started = Date.now();
        await expect(
          connectClient({
            url: `ws://127.0.0.1:${address.port}`,
            identity: { seed: seedB },
            manifest: schemaConformantManifest(),
            remotePubkey,
            allowlist: [remotePeerId],
            timeoutMs: 400,
          }),
        ).rejects.toThrow(/dial .* timed out/);
        // The dial really timed out (nothing rejects it sooner on this path).
        expect(Date.now() - started).toBeGreaterThanOrEqual(350);

        // The client released its socket after the dial failure: the server
        // must observe the TCP disconnect (bounded race, no bare sleep).
        await Promise.race([
          serverSideClosed,
          new Promise((_, reject) =>
            setTimeout(
              () => reject(new Error("server never observed the client socket close")),
              2000,
            ),
          ),
        ]);
      } finally {
        server.close();
      }
    },
    15000,
  );

  it(
    "fails fast on a malformed JSON frame (no crash, no hang until timeout)",
    async () => {
      const seedA = seed(0xd0);
      const pubkeyA = getPublicKeyEd25519(seedA);
      const peerIdA = derivePeerIdFromEd25519Pubkey(pubkeyA);
      const seedB = seed(0x50);

      const server = await startHandshakeServer({
        seed: seedA,
        peerId: peerIdA,
        onMessage: (socket) => {
          // Raw malformed text frame — not a JSON document at all. The
          // decode must fail at the transport boundary: the client rejects
          // fast and the host process must not crash (a JSON.parse throw
          // inside the ws listener would surface as an uncaught exception
          // and fail this suite).
          socket.send("not-json{{{");
        },
      });

      let client: ConnectClient | null = null;
      try {
        client = await connectClient({
          url: `ws://127.0.0.1:${server.port}`,
          identity: { seed: seedB },
          manifest: schemaConformantManifest(),
          remotePubkey: pubkeyA,
          allowlist: [peerIdA],
          timeoutMs: 5000,
        });

        const started = Date.now();
        await expect(client.invoke("check", {})).rejects.toThrow(
          /malformed JSON frame/,
        );
        // Fail-fast: well under the 5 s invoke timeout.
        expect(Date.now() - started).toBeLessThan(2000);
      } finally {
        client?.close();
        for (const c of server.connections) c.close();
        await server.close();
      }
    },
    15000,
  );

  it(
    "rejects an invoke immediately when the send fails (encode error; no timeout wait, no dead entry)",
    async () => {
      const seedA = seed(0xe0);
      const pubkeyA = getPublicKeyEd25519(seedA);
      const peerIdA = derivePeerIdFromEd25519Pubkey(pubkeyA);
      const seedB = seed(0x60);

      const server = await startHandshakeServer({
        seed: seedA,
        peerId: peerIdA,
        onMessage: () => {
          // Unreachable in this test: the client's invoke never reaches the
          // wire (its frame cannot be encoded).
        },
      });

      let client: ConnectClient | null = null;
      try {
        client = await connectClient({
          url: `ws://127.0.0.1:${server.port}`,
          identity: { seed: seedB },
          manifest: schemaConformantManifest(),
          remotePubkey: pubkeyA,
          allowlist: [peerIdA],
          timeoutMs: 5000,
        });

        // A payload that is not JSON-serializable makes `sendJsonMessage`
        // throw synchronously inside `invoke` (`encodeJsonMessage` fails).
        // The pending entry + timer must be cleaned up and the invoke must
        // reject with the send error immediately — not wait out the 5 s
        // timeout and not leave a dead entry for failAll to trip over.
        const started = Date.now();
        await expect(
          client.invoke("check", { findings: [1n] }),
        ).rejects.toThrow(/BigInt|JSON|serializable/);
        expect(Date.now() - started).toBeLessThan(2000);
      } finally {
        client?.close();
        for (const c of server.connections) c.close();
        await server.close();
      }
    },
    15000,
  );
  it(
    "closes the socket when a timed-out queued invoke's send is skipped (no silent sequence poisoning)",
    async () => {
      // F-3: an invoke whose send is still queued behind an earlier
      // invoke's slow sign/send times out before its send starts. The skip
      // guard must not just drop the send: the allocated outbound sequence
      // was never transmitted, so the host's inbound gate is stuck at it
      // and every later invoke fails `inbound_sequence_mismatch` — a
      // silently poisoned session. The client must tear the connection
      // down: the server observes the socket close and neither invoke ever
      // reaches the wire.
      //
      // Real timers are deliberate here (integration-test exception): the
      // race under test IS the real-clock race between the client's invoke
      // timeout timer and the WebCrypto sign of a large payload — fake
      // timers cannot drive WebCrypto. The large payload's JCS
      // canonicalization + Ed25519 sign reliably outlasts the 100ms invoke
      // timeout (~310ms measured for 800k elements), and the localhost
      // handshake completes in ~10ms, well inside the same timeout — so
      // the ordering is deterministic under load. (The test is last in
      // this file so its main-thread canonicalization does not overlap
      // other suites' tight invoke-timeout windows.)
      const seedA = seed(0xf0);
      const pubkeyA = getPublicKeyEd25519(seedA);
      const peerIdA = derivePeerIdFromEd25519Pubkey(pubkeyA);
      const seedB = seed(0x70);

      const receivedOps: string[] = [];
      const server = await startHandshakeServer({
        seed: seedA,
        peerId: peerIdA,
        onMessage: (_socket, doc) => {
          if (isConnectInvokeRequest(doc)) {
            receivedOps.push(doc.op);
          }
        },
      });

      let client: ConnectClient | null = null;
      try {
        client = await connectClient({
          url: `ws://127.0.0.1:${server.port}`,
          identity: { seed: seedB },
          manifest: schemaConformantManifest(),
          remotePubkey: pubkeyA,
          allowlist: [peerIdA],
          timeoutMs: 100,
        });

        // The skip branch closes the socket; the server observes the
        // disconnect (bounded race, no bare sleep).
        const serverSideClosed = new Promise<void>((resolve) => {
          server.connections[0].once("close", () => resolve());
        });

        // First invoke: a payload large enough that signing it (JCS
        // canonicalization + Ed25519 over the full signed object) reliably
        // outlasts the invoke timeout — its send call completes late, so
        // the second invoke times out while still queued behind it.
        const slow = client.invoke("check", {
          findings: Array.from({ length: 800_000 }, (_, i) => ({ a: i })),
        });
        const queued = client.invoke("check", {});

        const [slowResult, queuedResult] = await Promise.allSettled([
          slow,
          queued,
        ]);
        expect(slowResult.status).toBe("rejected");
        expect(queuedResult.status).toBe("rejected");

        await Promise.race([
          serverSideClosed,
          new Promise((_, reject) =>
            setTimeout(
              () => reject(new Error("server never observed the client socket close")),
              5000,
            ),
          ),
        ]);

        // Neither invoke was ever transmitted (both sends were skipped) —
        // the host's inbound gate was never advanced past a gap.
        expect(receivedOps).toEqual([]);
      } finally {
        client?.close();
        for (const c of server.connections) c.close();
        await server.close();
      }
    },
    15000,
  );

});
