/**
 * Loopback interop test — TS `RemoteAdapter` (client) ↔ a TS connect host
 * serving an async `ToyWorldAdapter` (server) over the in-repo
 * `loopbackTransportPair` (frozen contract §10 verification checklist).
 *
 * Asserts, per the plan:
 * (a) ENCAPSULATION — the consumer surface is ONLY the async `BaselinePorts`
 *     (+ `connectRemoteAdapter`): this file imports no session-core
 *     verification helpers, and the adapter exposes none of them at runtime.
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
import { MAX_SEQUENCE } from "../../src/core/sequence.js";
import { schemaConformantManifest } from "../../src/golden.js";
import { derivePeerIdFromEd25519Pubkey } from "../../src/identity.js";
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
    transport: pair.client,
    localIdentity: { seed: seedClient },
    localManifest: options.clientManifest ?? clientManifest(),
    remotePubkey: pubkeyHost,
    allowlist: options.allowlist ?? [peerIdHost],
    invokeTimeoutMs: options.invokeTimeoutMs,
    capabilityToken,
  });
  return { client, host, pair, peerIdHost, peerIdClient };
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
      const adapter = new RemoteAdapter(pair.client, 100);
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
      ).rejects.toThrow(/replay/);
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
      let delayMs = 100;
      const { client, host } = await dial(hostAdapter, {
        invokeTimeoutMs: 20,
        hostDelay: () => delayMs,
      });
      try {
        const result = await client.getKnowledgeEntry("kb_tw_mira");
        expect(result).toEqual({
          ok: false,
          code: SpokeRejectCode.INTERNAL_ERROR,
          message: expect.stringContaining("timed out after 20ms"),
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
    "fails pending invokes with INTERNAL_ERROR kind session_closed when the transport closes mid-flight",
    async () => {
      const hostAdapter = ToyWorldAdapter.withCommittedFixtures();
      const { client, host } = await dial(hostAdapter, {
        hostDelay: () => 100,
      });
      try {
        // The request is registered + sent synchronously, then the host
        // drops the connection while the response is still delayed.
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
});
