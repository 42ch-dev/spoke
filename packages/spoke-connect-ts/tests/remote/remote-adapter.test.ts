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
  type RemoteAdapter,
} from "@42ch/spoke-connect/remote";

import { getPublicKeyEd25519 } from "../../src/crypto.js";
import { issueCapabilityToken } from "../../src/core/capability-token.js";
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
});
