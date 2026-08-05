/**
 * Loopback interop test — TS `RemoteAdapter` (client) ↔ a TS connect host
 * serving an async `ToyWorldAdapter` (server) over the in-repo
 * `loopbackTransportPair` (frozen contract §10 verification checklist).
 *
 * Asserts, per the plan:
 * (a) ENCAPSULATION — the consumer surface is ONLY the async `BaselinePorts`
 *     (+ `connectRemoteAdapter`): this file imports no session-core
 *     verification helpers through the adapter surface, and the adapter
 *     exposes none of them at runtime. (Test fixtures MAY import session-core
 *     helpers directly to craft hostile wire envelopes — see the fail-closed
 *     dial-binding tests.)
 * (b) DROP-IN — `orchestrateUpsert(remoteAdapter, req)` /
 *     `orchestrateCheck(...)` return the same `SpokeResult` as the local
 *     `ToyWorldAdapter` for identical requests (upsert + conflict-reject +
 *     check paths).
 * (c) VERIFICATION RAN — the connect handshake actually happened (host
 *     allowlist + signature + nonce gates; remote hello host cached).
 *
 * Plus the §10 concurrency/error rows: concurrent invokes demuxed by
 * `request_id` with out-of-order responses, invoke timeout, transport close
 * mid-flight, dispatch deny mapping, and fail-closed allowlist dials.
 */

import { describe, expect, it } from "vitest";

import type {
  CheckRequest,
  ConnectInvokeRequest,
  HostCapabilityManifest,
  KnowledgeEntry,
  UpsertRequest,
} from "@42ch/spoke-schemas";
import {
  orchestrateCheck,
  orchestrateUpsert,
  SpokeRejectCode,
  type BaselinePorts,
} from "@42ch/spoke-operations";
import { ToyWorldAdapter } from "@42ch/spoke-fixture-toy-world";
import {
  connectRemoteAdapter,
  loopbackTransportPair,
  RemoteAdapter,
  type EnvelopeBytes,
  type Transport,
} from "@42ch/spoke-connect/remote";

import { getPublicKeyEd25519 } from "../../src/crypto.js";
import { issueCapabilityToken } from "../../src/core/capability-token.js";
// Fixture crafting for the fail-closed dial tests: a genuinely-signed
// 4-field (pre-dial-binding) responder hello, served over a scripted
// transport (test-only import; the adapter surface stays clean).
import { signHelloEd25519 } from "../../src/core/hello.js";
import { schemaConformantManifest } from "../../src/golden.js";
import { MAX_SEQUENCE } from "../../src/core/sequence.js";
import { decodeJsonMessage, encodeJsonMessage } from "../../src/framing.js";
import { derivePeerIdFromEd25519Pubkey } from "../../src/identity.js";
// Test-only reset hook for the process-wide accepted-server-hello store
// (simulates a restart in the cross-restart dial-binding replay test).
import { __resetAcceptedServerHellosForTest } from "../../src/remote/remote-adapter.js";
import { startLoopbackHost, type LoopbackHost } from "./loopback-host.js";

/** Fixture seed: base+i, all values within byte range for base ≤ 0xe0. */
function seed(base: number): Uint8Array {
  return Uint8Array.from({ length: 32 }, (_, i) => base + i);
}

/** Client manifest — distinct host_id so the remote-host cache is provable. */
function clientManifest(): HostCapabilityManifest {
  return { ...schemaConformantManifest(), host_id: "test-client" };
}

/** Minimal schema-valid provisional KnowledgeEntry for the upsert parity. */
function freshEntry(entryId: string, canonicalName: string): KnowledgeEntry {
  return {
    schema_version: 1,
    entry_id: entryId,
    entry_type: "character",
    canonical_name: canonicalName,
    status: "provisional",
    body: { summary: `Upserted over the loopback: ${entryId}` },
    extensions: {},
  };
}

/**
 * Dial a client against a fresh loopback host serving `hostAdapter`.
 * Returns the pair so tests can close either side deterministically.
 */
async function dial(
  hostAdapter: ToyWorldAdapter,
  options: {
    allowlist?: readonly string[];
    clientManifest?: HostCapabilityManifest;
    invokeTimeoutMs?: number;
    hostDelay?: (request: { sequence: number }) => number;
    hostAllowlist?: readonly string[];
    /** Replace the host's response envelope for a request (malformed fixtures). */
    hostResponseOverride?: (request: ConnectInvokeRequest) => unknown;
    /** Issue a real capability token and attach it as `auth` on invokes. */
    attachToken?: boolean;
    /**
     * Wrap the client's transport end (wire-level injector fixtures: tamper
     * / strip signatures on outbound requests or inbound responses).
     */
    clientTransport?: (clientEnd: Transport) => Transport;
  } = {},
): Promise<{
  client: RemoteAdapter;
  host: LoopbackHost;
  pair: ReturnType<typeof loopbackTransportPair>;
  peerIdHost: string;
  peerIdClient: string;
}> {
  const seedHost = seed(0xa0);
  const seedClient = seed(0x10);
  const pubkeyHost = getPublicKeyEd25519(seedHost);
  const pubkeyClient = getPublicKeyEd25519(seedClient);
  const peerIdHost = derivePeerIdFromEd25519Pubkey(pubkeyHost);
  const peerIdClient = derivePeerIdFromEd25519Pubkey(pubkeyClient);

  const pair = loopbackTransportPair();
  const host = await startLoopbackHost({
    transport: pair.server,
    seed: seedHost,
    clientPubkey: pubkeyClient,
    allowlist: options.hostAllowlist ?? [peerIdClient],
    adapter: hostAdapter,
    delay: options.hostDelay,
    responseOverride: options.hostResponseOverride,
  });
  const capabilityToken =
    options.attachToken === true
      ? await issueCapabilityToken(seedClient, {
          // Client-issued proof for itself; the host (aud) may validate per
          // its product trusted-issuers config — the attach path is what
          // this fixture exercises.
          iss: peerIdClient,
          sub: peerIdClient,
          aud: peerIdHost,
          capabilities: ["spoke-baseline"],
          exp: Math.floor(Date.now() / 1000) + 3600,
        })
      : undefined;
  const client = await connectRemoteAdapter({
    transport: options.clientTransport?.(pair.client) ?? pair.client,
    localIdentity: { seed: seedClient },
    localManifest: options.clientManifest ?? clientManifest(),
    remotePubkey: pubkeyHost,
    allowlist: options.allowlist ?? [peerIdHost],
    invokeTimeoutMs: options.invokeTimeoutMs,
    capabilityToken,
  });
  return { client, host, pair, peerIdHost, peerIdClient };
}

/**
 * Decode wire bytes to a JSON document (wire-level injector fixtures).
 */
function decodeWire(bytes: EnvelopeBytes): Record<string, unknown> {
  return decodeJsonMessage(bytes) as Record<string, unknown>;
}

/** Re-encode a mutated document as wire bytes (wire-level injector fixtures). */
function encodeWire(doc: unknown): EnvelopeBytes {
  return new TextEncoder().encode(encodeJsonMessage(doc));
}

/**
 * Transport wrapper: mutates outbound invoke requests on the wire (the view
 * an active transport attacker has of the client's signed envelopes).
 * `mutate` receives the decoded request and must return the mutated doc.
 */
function tamperOutboundRequests(
  clientEnd: Transport,
  mutate: (doc: Record<string, unknown>) => Record<string, unknown>,
): Transport {
  return {
    send: async (envelope) => {
      const doc = decodeWire(envelope);
      if ("op" in doc) {
        // Outbound invoke request — apply the wire-level mutation. The
        // handshake hello passes through unchanged so the host's hello
        // gates still succeed.
        await clientEnd.send(encodeWire(mutate(doc)));
        return;
      }
      await clientEnd.send(envelope);
    },
    recv: () => clientEnd.recv(),
    close: () => clientEnd.close?.(),
  };
}

/**
 * Transport wrapper: mutates inbound invoke responses on the wire. `mutate`
 * receives the decoded response and must return the mutated doc. The hello
 * / session snapshot pass through unchanged so the dial still establishes.
 */
function tamperInboundResponses(
  clientEnd: Transport,
  mutate: (doc: Record<string, unknown>) => Record<string, unknown>,
): Transport {
  return {
    send: (envelope) => clientEnd.send(envelope),
    recv: async () => {
      const bytes = await clientEnd.recv();
      const doc = decodeWire(bytes);
      if ("request_id" in doc && ("payload" in doc || "error" in doc)) {
        // Inbound invoke response — apply the wire-level mutation.
        return encodeWire(mutate(doc));
      }
      return bytes;
    },
    close: () => clientEnd.close?.(),
  };
}

describe("RemoteAdapter loopback interop", () => {
  it(
    "encapsulates connect verification and is a drop-in async BaselinePorts for orchestrateUpsert",
    async () => {
      const hostAdapter = new ToyWorldAdapter();
      const localAdapter = new ToyWorldAdapter(); // drop-in parity target
      const { client, host, peerIdHost } = await dial(hostAdapter);
      try {
        // (a) Encapsulation: the consumer surface is ONLY the async
        //     BaselinePorts — the compile-time assignment proves the type
        //     surface, and no session-core verification helper exists at
        //     runtime. This test file itself imports no hello/nonce/sequence
        //     helpers.
        const ports: BaselinePorts = client;
        expect(ports).toBe(client);
        const runtime = client as unknown as Record<string, unknown>;
        for (const hidden of [
          // Session-core verification helpers must not exist at runtime.
          "allocateOutboundSequence",
          "acceptInboundSequence",
          "generateNonce",
          "signHelloEd25519",
          "verifyHelloEd25519",
          "checkResponseCorrelation",
          "isAllowlisted",
          "correlationFromRequest",
          // Envelope-auth helpers are module-internal imports (contract §9):
          // absent from the instance at runtime, exactly like the
          // session-core helpers above.
          "authenticateSession",
          "verifySessionAuth",
          "authenticateInvokeRequest",
          "verifyInvokeRequestAuth",
          "authenticateInvokeResponse",
          "verifyInvokeResponseAuth",
          // Session-lifecycle methods are `#`-private: absent from the shipped
          // .d.ts AND unreachable at runtime (no forging `Established` state
          // past hello/allowlist verification, not even via an `any` cast).
          "beginHandshake",
          "sendEnvelope",
          "recvEnvelope",
          "establish",
          "closeSession",
          // Verification-gating STATE is `#`-private too (Greptile #2): the
          // invoke gate reads `#stateInternal` / `#session`, and a JS
          // consumer cannot overwrite `#`-slots with inert own properties.
          "stateInternal",
          "session",
          "remoteManifestInternal",
          "receiveLoopRunning",
          "pending",
          "transport",
          "invokeTimeoutMs",
          "capabilityToken",
          // The remaining internal machinery is `#`-private as well.
          "failAllPending",
          "runReceiveLoop",
          "receiveLoop",
          "invokeOp",
          "invokeMapped",
        ]) {
          expect(runtime[hidden], `verification helper ${hidden} must not leak`).toBeUndefined();
        }

        // (c) Verification ran: both hellos authenticated, remote hello host
        //     cached (the REMOTE "test-host", not the client's "test-client").
        expect(host.stats.hellosVerified).toBe(1);
        expect(client.sessionId).toBe(host.sessionId);
        expect(client.remotePeerId).toBe(peerIdHost);
        expect(client.state).toBe("Established");
        expect(client.remoteManifest.host_id).toBe("test-host");

        // (b) Drop-in parity — upsert path: identical requests produce
        //     identical SpokeResults on the local adapter and the remote one.
        const candidate = freshEntry("kb_tw_remote_cartographer", "Remote Cartographer");
        const request: UpsertRequest = { knowledge_entries: [candidate] };
        const localUpsert = await orchestrateUpsert(localAdapter, request);
        const remoteUpsert = await orchestrateUpsert(client, request);
        expect(remoteUpsert).toEqual(localUpsert);
        expect(remoteUpsert).toEqual({
          ok: true,
          value: { knowledge_entries: [candidate] },
        });
        // The remote write actually landed in the host-side store.
        expect(hostAdapter.store.entries.has("kb_tw_remote_cartographer")).toBe(true);

        // Drop-in parity — reject path: a conflicting second upsert rejects
        // identically on both sides (error branch → SpokeResult reject).
        const localConflict = await orchestrateUpsert(localAdapter, request);
        const remoteConflict = await orchestrateUpsert(client, request);
        expect(remoteConflict.ok).toBe(false);
        expect(remoteConflict).toEqual(localConflict);

        // Drop-in parity — check path (listKnowledgeEntries + listTimelineEvents +
        // listRules + putFindings over the wire).
        const checkRequest: CheckRequest = {
          scope: { scope_id: "toy-scope-001" },
        };
        const checker = () => ({ ok: true as const, value: [] as never[] });
        const localCheck = await orchestrateCheck(localAdapter, checkRequest, checker);
        const remoteCheck = await orchestrateCheck(client, checkRequest, checker);
        expect(remoteCheck).toEqual(localCheck);
        expect(remoteCheck.ok).toBe(true);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "rejects a forged Established state — verification-gating fields are ECMAScript #private (Greptile #2)",
    async () => {
      // A JS consumer who constructs the adapter directly (never dialed)
      // cannot overwrite the state the invoke gate reads: TS `private`
      // compiles to writable own properties, ECMAScript `#private` slots do
      // not — every write below creates an inert own property (or a
      // strict-mode TypeError on the constructor-created accessor-less
      // slot) that the gate never reads.
      const pair = loopbackTransportPair();
      // Direct construction is dial-only; the key material is required but
      // irrelevant here — the adapter never dials, so it never signs.
      const adapter = new RemoteAdapter(
        pair.client,
        seed(0x10),
        getPublicKeyEd25519(seed(0xa0)),
        100,
      );
      const raw = adapter as unknown as Record<string, unknown>;
      raw.stateInternal = "Established";
      raw.session = {
        session_id: "forged-session",
        initiator_peer_id: "forged-initiator",
        responder_peer_id: "forged-responder",
        negotiated_capabilities: ["spoke-baseline"],
      };
      raw.remoteManifestInternal = {
        ...schemaConformantManifest(),
        host_id: "forged-host",
      };
      raw.receiveLoopRunning = true;
      raw.pending = new Map();

      // The invoke gate reads the real `#`-slots: the forged writes changed
      // nothing, so a port call still fails closed (never `Established`).
      expect(adapter.state).toBe("Disconnected");
      const result = await adapter.getKnowledgeEntry("kb_tw_mira");
      expect(result).toEqual({
        ok: false,
        code: SpokeRejectCode.INTERNAL_ERROR,
        message: expect.stringContaining("connect session is not established"),
        details: { kind: "session_closed" },
      });
    },
    15000,
  );

  it(
    "does not leak an unhandled rejection when Transport.close() rejects (Greptile #3)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const unhandled: unknown[] = [];
      const onUnhandled = (reason: unknown): void => {
        unhandled.push(reason);
      };
      process.on("unhandledRejection", onUnhandled);
      let host: LoopbackHost | undefined;
      try {
        const pair = loopbackTransportPair();
        // Delegate everything except close(), which rejects: the adapter
        // must swallow the failure (fire-and-forget teardown, contract
        // §2.1/§8.2) without surfacing an unhandled rejection, and still
        // transition to `Closed`.
        const rejectingClose: Transport = {
          send: (envelope) => pair.client.send(envelope),
          recv: () => pair.client.recv(),
          close: () => Promise.reject(new Error("close boom")),
        };
        host = await startLoopbackHost({
          transport: pair.server,
          seed: seed(0xa0),
          clientPubkey: getPublicKeyEd25519(seed(0x10)),
          allowlist: [
            derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0x10))),
          ],
          adapter: hostAdapter,
        });
        const client = await connectRemoteAdapter({
          transport: rejectingClose,
          localIdentity: { seed: seed(0x10) },
          localManifest: clientManifest(),
          remotePubkey: getPublicKeyEd25519(seed(0xa0)),
          allowlist: [
            derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0xa0))),
          ],
        });
        expect(client.state).toBe("Established");

        client.close();
        // Let a discarded close() rejection surface if the fix is absent
        // (Node reports unhandled rejections on the next macrotask).
        await new Promise((resolve) => setTimeout(resolve, 25));
        expect(unhandled, "close() rejection must be handled").toEqual([]);
        expect(client.state).toBe("Closed");
      } finally {
        process.off("unhandledRejection", onUnhandled);
        host?.close();
      }
    },
    15000,
  );

  it(
    "does not leak an unhandled rejection when Transport.close() returns a rejecting non-native thenable (Greptile #4)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const unhandled: unknown[] = [];
      const onUnhandled = (reason: unknown): void => {
        unhandled.push(reason);
      };
      process.on("unhandledRejection", onUnhandled);
      let host: LoopbackHost | undefined;
      try {
        const pair = loopbackTransportPair();
        // close() returns a PLAIN-OBJECT thenable — `instanceof Promise` is
        // FALSE (a cross-realm / library-interop object is not a native
        // Promise), yet it rejects through `.then(onRejected)`. The old
        // `instanceof Promise` guard skipped the handler entirely, so the
        // inner rejection surfaced as an unhandled rejection.
        const rejectingThenableClose = (): unknown => {
          const inner = Promise.reject(new Error("close boom"));
          return {
            then(
              onFulfilled?: (value: never) => unknown,
              onRejected?: (reason: unknown) => unknown,
            ) {
              inner.then(onFulfilled, onRejected);
            },
          };
        };
        const thenableClose: Transport = {
          send: (envelope) => pair.client.send(envelope),
          recv: () => pair.client.recv(),
          // `Transport.close` is typed `void | Promise<void>`; the adapter
          // reads the return as `unknown`, so the thenable needs a narrow
          // cast at the seam.
          close: rejectingThenableClose as () => void,
        };
        host = await startLoopbackHost({
          transport: pair.server,
          seed: seed(0xa0),
          clientPubkey: getPublicKeyEd25519(seed(0x10)),
          allowlist: [
            derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0x10))),
          ],
          adapter: hostAdapter,
        });
        const client = await connectRemoteAdapter({
          transport: thenableClose,
          localIdentity: { seed: seed(0x10) },
          localManifest: clientManifest(),
          remotePubkey: getPublicKeyEd25519(seed(0xa0)),
          allowlist: [
            derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0xa0))),
          ],
        });
        expect(client.state).toBe("Established");

        client.close();
        // Let a discarded thenable rejection surface if the fix is absent
        // (Node reports unhandled rejections on the next macrotask).
        await new Promise((resolve) => setTimeout(resolve, 25));
        expect(
          unhandled,
          "thenable close() rejection must be handled",
        ).toEqual([]);
        expect(client.state).toBe("Closed");
      } finally {
        process.off("unhandledRejection", onUnhandled);
        host?.close();
      }
    },
    15000,
  );

  it(
    "does not let a synchronous Transport.close() throw escape (Greptile #5)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      let host: LoopbackHost | undefined;
      try {
        const pair = loopbackTransportPair();
        // Delegate everything except close(), which THROWS synchronously:
        // the adapter must catch the throw at the call site (the state is
        // already `Closed` and waiters are failed below), so the public
        // `close()` returns normally and the session ends cleanly.
        const throwingClose: Transport = {
          send: (envelope) => pair.client.send(envelope),
          recv: () => pair.client.recv(),
          close: () => {
            throw new Error("close boom");
          },
        };
        host = await startLoopbackHost({
          transport: pair.server,
          seed: seed(0xa0),
          clientPubkey: getPublicKeyEd25519(seed(0x10)),
          allowlist: [
            derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0x10))),
          ],
          adapter: hostAdapter,
        });
        const client = await connectRemoteAdapter({
          transport: throwingClose,
          localIdentity: { seed: seed(0x10) },
          localManifest: clientManifest(),
          remotePubkey: getPublicKeyEd25519(seed(0xa0)),
          allowlist: [
            derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0xa0))),
          ],
        });
        expect(client.state).toBe("Established");

        // The synchronous throw must NOT escape the public close().
        expect(() => client.close()).not.toThrow();
        expect(client.state).toBe("Closed");
      } finally {
        host?.close();
      }
    },
    15000,
  );

  it(
    "rejects a replayed server hello — accepted (peer_id, nonce) pairs are single-use (Greptile #1)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const seedHost = seed(0xa0);
      const seedClient = seed(0x10);
      const pubkeyHost = getPublicKeyEd25519(seedHost);
      const pubkeyClient = getPublicKeyEd25519(seedClient);
      const peerIdHost = derivePeerIdFromEd25519Pubkey(pubkeyHost);
      const peerIdClient = derivePeerIdFromEd25519Pubkey(pubkeyClient);

      // Dial 1 through a recording transport — the view an active transport
      // attacker has after one legitimate dial (server hello + session
      // snapshot captured at the wire).
      const pair1 = loopbackTransportPair();
      const captured: EnvelopeBytes[] = [];
      const recording: Transport = {
        send: (envelope) => pair1.client.send(envelope),
        recv: async () => {
          const bytes = await pair1.client.recv();
          captured.push(bytes);
          return bytes;
        },
        close: () => pair1.client.close(),
      };
      const host = await startLoopbackHost({
        transport: pair1.server,
        seed: seedHost,
        clientPubkey: pubkeyClient,
        allowlist: [peerIdClient],
        adapter: hostAdapter,
      });
      const client = await connectRemoteAdapter({
        transport: recording,
        localIdentity: { seed: seedClient },
        localManifest: clientManifest(),
        remotePubkey: pubkeyHost,
        allowlist: [peerIdHost],
      });
      expect(client.state).toBe("Established");
      client.close();
      host.close();
      expect(captured.length).toBeGreaterThanOrEqual(2);
      const [replayHello, replaySession] = captured;

      // Dial 2: replay the captured envelopes through a scripted transport
      // with NO real host on the other end. Without receiver-side nonce
      // single-use this dial would succeed — the signature is genuinely the
      // allowlisted peer's — and the attacker could fabricate a session and
      // answer invokes; the fix rejects the replay before any
      // `ConnectSession` snapshot is accepted.
      let replayIndex = 0;
      const replayTransport: Transport = {
        send: async () => {},
        recv: async () => {
          if (replayIndex === 0) {
            replayIndex += 1;
            return replayHello;
          }
          if (replayIndex === 1) {
            replayIndex += 1;
            return replaySession;
          }
          throw new Error("scripted replay transport closed");
        },
        close: () => {},
      };
      await expect(
        connectRemoteAdapter({
          transport: replayTransport,
          localIdentity: { seed: seedClient },
          localManifest: clientManifest(),
          remotePubkey: pubkeyHost,
          allowlist: [peerIdHost],
        }),
      ).rejects.toThrow(/replay|dial binding/);
    },
    15000,
  );

  it(
    "rejects a replayed server hello across a simulated restart — responder peer_nonce no longer matches (Greptile P1 dial binding)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const seedHost = seed(0xa0);
      const seedClient = seed(0x10);
      const pubkeyHost = getPublicKeyEd25519(seedHost);
      const pubkeyClient = getPublicKeyEd25519(seedClient);
      const peerIdHost = derivePeerIdFromEd25519Pubkey(pubkeyHost);
      const peerIdClient = derivePeerIdFromEd25519Pubkey(pubkeyClient);

      // Dial 1 through a recording transport — capture the responder hello
      // (signed over `peer_nonce` = dial 1's initiator nonce) + session
      // snapshot at the wire (the view an active transport attacker has).
      const pair1 = loopbackTransportPair();
      const captured: EnvelopeBytes[] = [];
      const recording: Transport = {
        send: (envelope) => pair1.client.send(envelope),
        recv: async () => {
          const bytes = await pair1.client.recv();
          captured.push(bytes);
          return bytes;
        },
        close: () => pair1.client.close(),
      };
      const host = await startLoopbackHost({
        transport: pair1.server,
        seed: seedHost,
        clientPubkey: pubkeyClient,
        allowlist: [peerIdClient],
        adapter: hostAdapter,
      });
      const client = await connectRemoteAdapter({
        transport: recording,
        localIdentity: { seed: seedClient },
        localManifest: clientManifest(),
        remotePubkey: pubkeyHost,
        allowlist: [peerIdHost],
      });
      expect(client.state).toBe("Established");
      client.close();
      host.close();
      expect(captured.length).toBeGreaterThanOrEqual(2);
      const [replayHello] = captured;

      // Simulate a restart: the process-wide accepted-server-hello store
      // resets, so the receiver-side nonce single-use gate can no longer
      // recognize the captured pair. The dial binding is the remaining
      // defense: dial 2 generates a FRESH initiator nonce, and the captured
      // responder hello was signed over dial 1's nonce — the assert rejects
      // the replay before any `ConnectSession` snapshot is accepted.
      __resetAcceptedServerHellosForTest();

      let replayIndex = 0;
      const replayTransport: Transport = {
        send: async () => {},
        recv: async () => {
          if (replayIndex === 0) {
            replayIndex += 1;
            return replayHello;
          }
          throw new Error("scripted replay transport closed");
        },
        close: () => {},
      };
      await expect(
        connectRemoteAdapter({
          transport: replayTransport,
          localIdentity: { seed: seedClient },
          localManifest: clientManifest(),
          remotePubkey: pubkeyHost,
          allowlist: [peerIdHost],
        }),
      ).rejects.toThrow(/dial binding/);
    },
    15000,
  );

  it(
    "fails the dial fail-closed when the responder hello omits peer_nonce (mixed-version downgrade)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const seedHost = seed(0xa0);
      const seedClient = seed(0x10);
      const pubkeyHost = getPublicKeyEd25519(seedHost);
      const pubkeyClient = getPublicKeyEd25519(seedClient);
      const peerIdHost = derivePeerIdFromEd25519Pubkey(pubkeyHost);
      const peerIdClient = derivePeerIdFromEd25519Pubkey(pubkeyClient);

      // An OLD responder (pre-dial-binding) signs the 4-field initiator
      // object — no `peer_nonce` on the wire, with a GENUINELY valid
      // signature from the allowlisted host key (passes identity checks).
      // The NEW initiator dial expects a responder (it supplies its own
      // nonce), so the missing `peer_nonce` must fail the dial closed — not
      // silently fall back to the 4-field verify (fail-open downgrade).
      const legacyHello = await signHelloEd25519(
        seedHost,
        "legacy-host-nonce-1234567",
        schemaConformantManifest(),
      );
      const legacyBytes = new TextEncoder().encode(encodeJsonMessage(legacyHello));

      const legacyTransport: Transport = {
        send: async () => {},
        recv: async () => legacyBytes,
        close: () => {},
      };
      await expect(
        connectRemoteAdapter({
          transport: legacyTransport,
          localIdentity: { seed: seedClient },
          localManifest: clientManifest(),
          remotePubkey: pubkeyHost,
          allowlist: [peerIdHost],
        }),
      ).rejects.toThrow(/dial binding/);
    },
    15000,
  );

  it(
    "demuxes concurrent invokes by request_id with out-of-order responses",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      // Deterministic out-of-order fixture: sequence-0 responses are delayed
      // 30ms, so the sequence-1 response arrives first.
      const { client, host } = await dial(hostAdapter, {
        hostDelay: (req) => (req.sequence === 0 ? 30 : 0),
      });
      try {
        const [first, second] = await Promise.all([
          client.getKnowledgeEntry("kb_tw_mira"), // sequence 0 — delayed
          client.getKnowledgeEntry("kb_tw_harbor"), // sequence 1 — fast
        ]);
        expect(first.ok && first.value.entry_id).toBe("kb_tw_mira");
        expect(second.ok && second.value.entry_id).toBe("kb_tw_harbor");
        // The delayed response landed second: demux delivered to the right
        // waiter despite arrival order.
        expect(host.stats.responseOrder).toEqual([1, 0]);
        expect(host.stats.invokesDispatched).toBe(2);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "maps an invoke timeout to INTERNAL_ERROR kind timeout without closing the session",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      // The invoke timeout also gates the dial handshake, so keep it well
      // above the handshake's worst-case latency under parallel-suite load
      // (a 20ms budget here flakes the dial with "server hello timed out").
      let delayMs = 200;
      const { client, host } = await dial(hostAdapter, {
        invokeTimeoutMs: 100,
        hostDelay: () => delayMs,
      });
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("timed out after 100ms"),
          details: { kind: "timeout" },
        });
        // Timeout fails only the waiter — the session stays usable.
        expect(client.state).toBe("Established");
        delayMs = 0;
        const retry = await client.getKnowledgeEntry("kb_tw_mira");
        expect(retry.ok).toBe(true);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "closes the session when a timed-out queued invoke's send is skipped (never transmitted, no silent poisoning)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      // Wire-level injector: the FIRST outbound invoke request's send is
      // delayed on the wire, and every transmission (plus every inbound
      // response) is recorded by sequence. The second invoke's send is
      // serialized behind the first (send tail) and times out while
      // waiting — before its send ever starts. The F-1 guard must skip
      // that late send: the host must never see the timed-out request, so
      // a retry cannot duplicate a handler dispatch.
      //
      // F-3 (this test's extension): skipping the send without tearing the
      // session down would leave the allocated outbound sequence
      // untransmitted — the host's inbound gate is stuck at it, so every
      // later invoke fails `inbound_sequence_mismatch` (silent poisoning).
      // The skip must instead close the session: the adapter state becomes
      // `Closed` and the next invoke fails `session_closed`.
      //
      // Real timers are deliberate here (integration-test exception): the
      // race under test IS a real-clock race between the adapter's invoke
      // timeout timer and a slow transport send, with both peers running
      // on real timers — fake timers cannot drive the loopback transport
      // or WebCrypto signing. The 200ms wire delay sits far outside the
      // 100ms invoke timeout (which also gates the dial handshake, so it
      // must stay well above the handshake's worst-case latency under
      // parallel-suite load), so the ordering is deterministic under load.
      const sentSequences: number[] = [];
      const receivedSequences: number[] = [];
      let firstSendDelayed = false;
      const { client, host } = await dial(hostAdapter, {
        invokeTimeoutMs: 100,
        clientTransport: (clientEnd) => ({
          send: async (envelope) => {
            const doc = decodeWire(envelope);
            if ("op" in doc) {
              if (!firstSendDelayed) {
                firstSendDelayed = true;
                await new Promise((resolve) => setTimeout(resolve, 200));
              }
              sentSequences.push(doc.sequence as number);
            }
            await clientEnd.send(envelope);
          },
          recv: async () => {
            const bytes = await clientEnd.recv();
            const doc = decodeWire(bytes);
            if ("request_id" in doc && ("payload" in doc || "error" in doc)) {
              receivedSequences.push(doc.sequence as number);
            }
            return bytes;
          },
          close: () => clientEnd.close?.(),
        }),
      });
      try {
        const [first, second] = await Promise.all([
          client.getKnowledgeEntry("kb_tw_mira"), // sequence 0 — send delayed
          client.getKnowledgeEntry("kb_tw_harbor"), // sequence 1 — times out queued behind it
        ]);
        expect(first).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("timed out after 100ms"),
          details: { kind: "timeout" },
        });
        expect(second).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("timed out after 100ms"),
          details: { kind: "timeout" },
        });
        // Both invokes observed the timeout. The skip (F-3) closes the
        // session as soon as the queued send is dropped — wait for the
        // adapter's state to become `Closed` (the transport is torn down,
        // so no further transmission is possible), then assert the second
        // (timed-out-while-queued) invoke was never transmitted.
        //
        // The host processes the first invoke on a detached chain that
        // outlives the client's close (gate verify → dispatch → response
        // attempt), so also wait for its `responseOrder` to be recorded —
        // the host-side completion point. Without this second condition the
        // dispatch-count assertion below would race the host's async gate.
        const closeDeadline = Date.now() + 5000;
        while (
          client.state !== "Closed" ||
          host.stats.responseOrder.length === 0
        ) {
          if (Date.now() >= closeDeadline) {
            throw new Error(
              "adapter never closed / host never finished processing after the skipped send",
            );
          }
          await new Promise((resolve) => setTimeout(resolve, 10));
        }
        expect(sentSequences).toEqual([0]);
        // The first invoke's response never made it back: the session
        // closed (transport torn down) before the host's delayed response
        // arrived — no silent continuation on a poisoned wire.
        expect(receivedSequences).toEqual([]);
        expect(client.state).toBe("Closed");
        expect(host.stats.invokesDispatched).toBe(1);
        // The session is closed, not poisoned: a follow-up invoke fails
        // with `session_closed` instead of hanging or being mis-rejected
        // by the host's stuck inbound gate.
        const after = await client.getKnowledgeEntry("kb_tw_mira");
        expect(after).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("connect session is not established"),
          details: { kind: "session_closed" },
        });
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "fails pending invokes with INTERNAL_ERROR kind session_closed when the transport closes mid-flight",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const { client, host } = await dial(hostAdapter, {
        hostDelay: () => 100,
      });
      try {
        // The pending waiter is registered synchronously (the request is in
        // flight from the caller's perspective immediately; only the
        // Ed25519 sign is deferred), so the host drops the connection
        // while the response is still delayed.
        const pending = client.getKnowledgeEntry("kb_tw_mira");
        host.close();
        const result = await pending;
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("connect session closed"),
          details: { kind: "session_closed" },
        });
        expect(client.state).toBe("Closed");
        // Subsequent port calls also fail closed with session_closed.
        const after = await client.getKnowledgeEntry("kb_tw_mira");
        expect(after.ok).toBe(false);
        expect(after).toMatchObject({ code: SpokeRejectCode.INTERNAL_ERROR });
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "maps host dispatch denials to CAPABILITY_PORT_MISSING",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      // A client manifest without spoke-baseline ⇒ the negotiated set is
      // empty ⇒ the host's dispatch gate denies every port.* op.
      const noBaseline: HostCapabilityManifest = {
        ...schemaConformantManifest(),
        host_id: "test-client",
        capabilities: ["l2-computable"],
      };
      const { client, host } = await dial(hostAdapter, {
        clientManifest: noBaseline,
      });
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.CAPABILITY_PORT_MISSING,
          message: expect.stringContaining("not authorized"),
          details: { wire_code: "op_unsupported" },
        });
        expect(host.stats.dispatchDenials).toBe(1);
        expect(host.stats.invokesDispatched).toBe(0);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "HostManifestPort: get returns the remote hello host from cache; listPeer proxies",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const { client, host } = await dial(hostAdapter);
      try {
        // getHostCapabilityManifest = remote hello host cache — NO invoke.
        const self = await client.getHostCapabilityManifest();
        expect(self.ok && self.value.host_id).toBe("test-host");
        expect(host.stats.invokesDispatched).toBe(0);

        // listPeerHostCapabilityManifests = remote proxy (product-seeded peers).
        const peers = await client.listPeerHostCapabilityManifests();
        expect(peers.ok && peers.value.map((m) => m.host_id)).toEqual([
          "host_tw_peer",
        ]);
        expect(host.stats.invokesDispatched).toBe(1);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "attaches a configured capability token as auth on outbound invokes",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const { client, host } = await dial(hostAdapter, { attachToken: true });
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result.ok).toBe(true);
        // The host observed the auth field on the wire (attach path ran).
        expect(host.stats.authSeen).toBe(true);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "fails dial fail-closed when the remote peer is not on the allowlist",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const pair = loopbackTransportPair();
      const foreignPeer = derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0x70)));
      const host = await startLoopbackHost({
        transport: pair.server,
        seed: seed(0xa0),
        clientPubkey: getPublicKeyEd25519(seed(0x10)),
        allowlist: [derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0x10)))],
        adapter: hostAdapter,
      });
      try {
        await expect(
          connectRemoteAdapter({
            transport: pair.client,
            localIdentity: { seed: seed(0x10) },
            localManifest: clientManifest(),
            remotePubkey: getPublicKeyEd25519(seed(0xa0)),
            allowlist: [foreignPeer], // wrong peer — fail-closed before any hello
          }),
        ).rejects.toThrow(/not on the allowlist/);
      } finally {
        host.close();
      }
    },
    15000,
  );

  it(
    "fails dial when the host rejects the client hello (host-side allowlist)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const pubkeyClient = getPublicKeyEd25519(seed(0x10));
      const clientPeerId = derivePeerIdFromEd25519Pubkey(pubkeyClient);
      const otherPeerId = derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0x20)));
      const pair = loopbackTransportPair();
      const host = await startLoopbackHost({
        transport: pair.server,
        seed: seed(0xa0),
        clientPubkey: pubkeyClient,
        allowlist: [otherPeerId], // the real client is NOT allowed
        adapter: hostAdapter,
      }).catch(() => null); // handshake rejection may surface here or at dial
      try {
        await expect(
          connectRemoteAdapter({
            transport: pair.client,
            localIdentity: { seed: seed(0x10) },
            localManifest: clientManifest(),
            remotePubkey: getPublicKeyEd25519(seed(0xa0)),
            allowlist: [derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0xa0)))],
            invokeTimeoutMs: 2000,
          }),
        ).rejects.toThrow();
      } finally {
        host?.close();
      }
    },
    15000,
  );

  it(
    "rejects a malformed success payload with INTERNAL_ERROR kind transport (Rust §8.2 parity)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      // The host answers with a success envelope whose payload is not a
      // KnowledgeEntry shape (missing all required fields) — the Rust
      // adapter's `serde_json::from_value::<T>` rejects this; the TS side
      // must not surface `spokeOk(garbage)`.
      const { client, host } = await dial(hostAdapter, {
        hostResponseOverride: (request) => ({
          session_id: request.session_id,
          sequence: request.sequence,
          request_id: request.request_id,
          payload: { garbage: true },
          extensions: {},
        }),
      });
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("payload decode failed"),
          details: { kind: "transport" },
        });
        // A malformed payload fails only this waiter — the session stays
        // usable (parity with the Rust invoke_mapped path).
        expect(client.state).toBe("Established");
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "maps a correlation mismatch to INTERNAL_ERROR kind correlation_mismatch without closing the session",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      let mangled = true;
      const { client, host } = await dial(hostAdapter, {
        // Same request_id (so the demux still finds the pending waiter) but
        // a wrong sequence echo — a correlation failure (§6 echo rules).
        hostResponseOverride: (request) => {
          if (!mangled) {
            return undefined;
          }
          mangled = false;
          return {
            session_id: request.session_id,
            sequence: request.sequence + 1,
            request_id: request.request_id,
            payload: {},
            extensions: {},
          };
        },
      });
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: "response echo fields do not match the request",
          details: { kind: "correlation_mismatch" },
        });
        // Mismatch fails only the waiter — the session stays usable.
        expect(client.state).toBe("Established");
        const retry = await client.getKnowledgeEntry("kb_tw_mira");
        expect(retry.ok).toBe(true);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "maps outbound sequence exhaustion to INTERNAL_ERROR kind sequence_exhausted and closes the session",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const { client, host } = await dial(hostAdapter);
      try {
        // Position the adapter's outbound counter past the JSON-safe wire
        // maximum so the next allocate fails deterministically — 2⁵³ real
        // invokes are not an option. The hook is guarded (throws unless
        // `Established`) and only advances an established session's counter;
        // it is the TS twin of the Rust in-module unit test reaching
        // `OutboundSequence::set_next` under `#[cfg(test)]`.
        client.setOutboundNextForTest(MAX_SEQUENCE + 1);

        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("sequence"),
          details: { kind: "sequence_exhausted" },
        });
        // Exhaustion makes the session unusable (no wrap-around) — Closed.
        expect(client.state).toBe("Closed");
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "round-trips an authenticated invoke (host verifies client request signatures; client verifies host response signatures)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const { client, host } = await dial(hostAdapter);
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result.ok).toBe(true);
        // The client's signed request passed the host's envelope-auth gate
        // (zero rejections) and the host's signed response passed the
        // client's verify — the wire carried genuine
        // `spoke-connect-invoke-request-jcs-v1` / `-response-jcs-v1`
        // signatures in both directions.
        expect(host.stats.authRejections).toBe(0);
        expect(host.stats.invokesDispatched).toBe(1);
        expect(client.state).toBe("Established");
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "rejects a wire-tampered invoke response with envelope_auth_invalid (fail-closed, session stays usable)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      // One-shot wire-level injector: only the FIRST response is tampered,
      // so the follow-up invoke proves the session stayed usable.
      let tampered = false;
      const { client, host } = await dial(hostAdapter, {
        // Wire-level injector: the host's signed success response has its
        // payload mutated after the signature was computed.
        clientTransport: (clientEnd) =>
          tamperInboundResponses(clientEnd, (doc) => {
            if (!tampered && "payload" in doc) {
              tampered = true;
              (doc.payload as Record<string, unknown>).tampered = true;
            }
            return doc;
          }),
      });
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("signature does not verify"),
          details: { kind: "envelope_auth_invalid" },
        });
        // A forged signature fails only this waiter — no session-state
        // mutation, the session stays usable (parity with the
        // correlation-mismatch path).
        expect(client.state).toBe("Established");
        const retry = await client.getKnowledgeEntry("kb_tw_mira");
        expect(retry.ok).toBe(true);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "rejects an invoke response with a stripped signature (envelope_auth_missing)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const { client, host } = await dial(hostAdapter, {
        // Wire-level injector: delete the response signature field.
        clientTransport: (clientEnd) =>
          tamperInboundResponses(clientEnd, (doc) => {
            delete doc.signature;
            return doc;
          }),
      });
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("signature"),
          details: { kind: "envelope_auth_missing" },
        });
        // Mixed-version fail-closed: an unauthenticated envelope is never
        // accepted because a transport supplied peer identity — the session
        // stays usable but the strip is rejected every time.
        expect(client.state).toBe("Established");
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "host rejects a wire-tampered invoke request with auth_failed envelope_auth_invalid",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const { client, host } = await dial(hostAdapter, {
        // Wire-level injector: mutate the client's outbound request payload
        // after signing — the host's envelope-auth verify must reject it
        // BEFORE dispatch, with no handler side effect.
        clientTransport: (clientEnd) =>
          tamperOutboundRequests(clientEnd, (doc) => {
            (doc.payload as Record<string, unknown>).tampered = true;
            return doc;
          }),
      });
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        // The host answered `auth_failed` (wire code) with the locked
        // `details.kind`; the client maps it to INTERNAL_ERROR verbatim.
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("signature does not verify"),
          details: { kind: "envelope_auth_invalid" },
        });
        expect(host.stats.authRejections).toBe(1);
        expect(host.stats.invokesDispatched).toBe(0);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "host rejects an invoke request with a stripped signature (auth_failed envelope_auth_missing)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const { client, host } = await dial(hostAdapter, {
        // Wire-level injector: strip the request signature — the host must
        // treat the unauthenticated envelope as missing its authenticator
        // (mixed-version fail-closed, contract §8).
        clientTransport: (clientEnd) =>
          tamperOutboundRequests(clientEnd, (doc) => {
            delete doc.signature;
            return doc;
          }),
      });
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("signature"),
          details: { kind: "envelope_auth_missing" },
        });
        expect(host.stats.authRejections).toBe(1);
        expect(host.stats.invokesDispatched).toBe(0);
      } finally {
        client.close();
        host.close();
      }
    },
    15000,
  );

  it(
    "fails the dial when the session snapshot signature is stripped (verify before establish)",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const pair = loopbackTransportPair();
      const host = await startLoopbackHost({
        transport: pair.server,
        seed: seed(0xa0),
        clientPubkey: getPublicKeyEd25519(seed(0x10)),
        allowlist: [
          derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0x10))),
        ],
        adapter: hostAdapter,
      });
      try {
        // Wire-level injector on the client end: strip the signature from
        // the host's session snapshot; the hello passes through unchanged.
        const stripping: Transport = {
          send: (envelope) => pair.client.send(envelope),
          recv: async () => {
            const bytes = await pair.client.recv();
            const doc = decodeWire(bytes);
            if ("initiator_peer_id" in doc) {
              delete doc.signature;
              return encodeWire(doc);
            }
            return bytes;
          },
          close: () => pair.client.close(),
        };
        await expect(
          connectRemoteAdapter({
            transport: stripping,
            localIdentity: { seed: seed(0x10) },
            localManifest: clientManifest(),
            remotePubkey: getPublicKeyEd25519(seed(0xa0)),
            allowlist: [
              derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0xa0))),
            ],
          }),
        ).rejects.toThrow(/missing a signature/);
      } finally {
        host.close();
      }
    },
    15000,
  );
});
