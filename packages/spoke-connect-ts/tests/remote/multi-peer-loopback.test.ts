/**
 * Dual-peer loopback proof for the multi-peer capability router — the §9
 * end-to-end integration row behind the Task-2 unit tests. Real loopback
 * hosts (signed-hello handshake over `loopbackTransportPair`) back per-peer
 * `RemoteAdapter` instances registered on one `MultiPeerRouter`; the
 * consumer calls `orchestrateUpsert(router, req)` /
 * `orchestrateCheck(router, req, checker)` with NO per-op `peer_id` — the
 * router auto-routes (§8: "consumer continues orchestrate*(router, req)
 * with no per-op peer_id").
 *
 * Proves, over real wires (contract §9 parity row):
 * 1. DISJOINT CAPABILITIES — a `spoke-baseline` peer and an
 *    `l2-computable`-only peer; both orchestrated baseline flows land on
 *    the peer whose manifest carries the required capability, and the other
 *    peer receives ZERO invokes (hard capability filter, no wrong-peer
 *    fallback).
 * 2. NO-MATCH — when no registered peer advertises the op's required
 *    capability, `orchestrateUpsert` rejects with the stable terminal
 *    `no_capable_peer` reject (`CAPABILITY_PORT_MISSING` +
 *    `details.wire_code === "no_capable_peer"`), deterministically, and no
 *    invoke reaches any peer.
 * 3. TIE-BREAK — with three peers that all match both ops, every port call
 *    of both families routes to the lexicographically-lowest `peer_id`
 *    (UTF-8 byte order, §4), and repeat calls re-select the same peer.
 *
 * Per-peer observability: each loopback host's `stats.invokesDispatched`
 * counts only the invokes its own transport served, and each host's
 * `ToyWorldAdapter.store` shows where writes physically landed. The host
 * manifest is per-peer: `startLoopbackHost({ hostManifest })` advertises
 * each host's own capabilities in its signed hello, which the client
 * caches as `remoteManifest` — the exact per-peer input the router's §3
 * selection reads.
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
} from "@42ch/spoke-operations";
import { ToyWorldAdapter } from "@42ch/spoke-fixture-toy-world";
import {
  connectMultiPeerRouter,
  connectRemoteAdapter,
  loopbackTransportPair,
  type RemoteAdapter,
} from "@42ch/spoke-connect/remote";

import { getPublicKeyEd25519 } from "../../src/crypto.js";
import { schemaConformantManifest } from "../../src/golden.js";
import { derivePeerIdFromEd25519Pubkey } from "../../src/identity.js";
import {
  startLoopbackHost,
  type LoopbackHost,
} from "./loopback-host.js";

/** Fixture seed: base+i, all values within byte range for base ≤ 0xe0. */
function seed(base: number): Uint8Array {
  return Uint8Array.from({ length: 32 }, (_, i) => base + i);
}

/**
 * Schema-valid per-peer host manifest. The capability split is the routing
 * discriminator under test (contract §2: `upsert` / `check` / `port.*`
 * baseline ops require `spoke-baseline`).
 */
function peerManifest(
  hostId: string,
  capabilities: readonly string[],
  namespaces: readonly string[],
  roles: readonly string[],
): HostCapabilityManifest {
  return {
    schema_version: 1,
    host_id: hostId,
    capabilities: [...capabilities] as [string, ...string[]],
    namespaces: [...namespaces] as [string, ...string[]],
    roles: [...roles] as [string, ...string[]],
    extensions: {},
  };
}

/** Minimal schema-valid provisional KnowledgeEntry for the upsert path. */
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

/** One dialed loopback peer: client adapter + host + host-side store. */
interface LoopbackPeer {
  client: RemoteAdapter;
  host: LoopbackHost;
  adapter: ToyWorldAdapter;
  peerId: string;
}

/**
 * Dial one peer: a loopback host advertising `hostManifest` in its signed
 * hello, and a `RemoteAdapter` client that caches it as `remoteManifest`
 * (the router's §2 selection input). The client manifest offers both
 * baseline capabilities so each host's negotiated set covers the ops its
 * own manifest advertises.
 */
async function dialPeer(
  hostSeed: Uint8Array,
  clientSeed: Uint8Array,
  hostManifest: HostCapabilityManifest,
): Promise<LoopbackPeer> {
  const pubkeyHost = getPublicKeyEd25519(hostSeed);
  const pubkeyClient = getPublicKeyEd25519(clientSeed);
  const peerIdHost = derivePeerIdFromEd25519Pubkey(pubkeyHost);
  const peerIdClient = derivePeerIdFromEd25519Pubkey(pubkeyClient);
  const pair = loopbackTransportPair();
  const adapter = new ToyWorldAdapter();
  const host = await startLoopbackHost({
    transport: pair.server,
    seed: hostSeed,
    clientPubkey: pubkeyClient,
    allowlist: [peerIdClient],
    adapter,
    hostManifest,
  });
  const client = await connectRemoteAdapter({
    transport: pair.client,
    localIdentity: { seed: clientSeed },
    localManifest: {
      ...schemaConformantManifest(),
      host_id: "test-client",
      capabilities: ["spoke-baseline", "l2-computable"],
    },
    remotePubkey: pubkeyHost,
    allowlist: [peerIdHost],
  });
  return { client, host, adapter, peerId: peerIdHost };
}

/** Close a peer pair (client + host) deterministically. */
function closePeer(peer: LoopbackPeer): void {
  peer.client.close();
  peer.host.close();
}

const textEncoder = new TextEncoder();

/**
 * Lexicographic UTF-8 byte order on strings — the same comparison the
 * router applies for its §4 deterministic tie-break and §6 peer_id
 * ordering. (Peer ids are base58btc ASCII, so byte order and code-unit
 * order coincide; the explicit byte compare keeps the mirror faithful to
 * the contract.)
 */
function compareUtf8Strings(a: string, b: string): number {
  const aBytes = textEncoder.encode(a);
  const bBytes = textEncoder.encode(b);
  const shared = Math.min(aBytes.length, bBytes.length);
  for (let i = 0; i < shared; i++) {
    if (aBytes[i] !== bBytes[i]) {
      return aBytes[i] < bBytes[i] ? -1 : 1;
    }
  }
  return aBytes.length - bBytes.length;
}

/** Lowest `peer_id` in lexicographic UTF-8 byte order (contract §4). */
function lowestPeerId(ids: readonly string[]): string {
  return [...ids].sort(compareUtf8Strings)[0];
}

describe("MultiPeerRouter dual-peer loopback proof (§9)", () => {
  it(
    "routes orchestrateUpsert and orchestrateCheck to the peer with the required capability (disjoint capabilities; the consumer never names a peer)",
    async () => {
      // Disjoint capabilities: peer A serves baseline ops, peer B only
      // computable ops. Both orchestrated families require spoke-baseline
      // (contract §2), so every port call must land on A and none on B.
      const peerA = await dialPeer(
        seed(0xa0),
        seed(0x10),
        peerManifest("peer-a", ["spoke-baseline"], ["toy_world"], ["data-store"]),
      );
      const peerB = await dialPeer(
        seed(0xb0),
        seed(0x20),
        peerManifest("peer-b", ["l2-computable"], ["toy_world"], ["computable"]),
      );
      try {
        const router = connectMultiPeerRouter({ hostId: "test-router" });
        expect(router.registerPeer(peerA.client)).toBe(peerA.peerId);
        expect(router.registerPeer(peerB.client)).toBe(peerB.peerId);

        // DoD #6 — the upsert callsite names no peer: the router is the only
        // routing input, and the request carries no peer selector.
        const upsertRequest: UpsertRequest = {
          knowledge_entries: [
            freshEntry("kb_router_baseline_1", "Router Baseline One"),
          ],
        };
        expect(
          (upsertRequest as unknown as Record<string, unknown>).peer_id,
        ).toBeUndefined();

        const upsert = await orchestrateUpsert(router, upsertRequest);
        expect(upsert.ok).toBe(true);
        if (upsert.ok && "knowledge_entries" in upsert.value) {
          expect(upsert.value.knowledge_entries[0]?.entry_id).toBe(
            "kb_router_baseline_1",
          );
        }
        // Capability hard filter: A served both port calls (get + put);
        // B received ZERO invokes.
        expect(peerA.host.stats.invokesDispatched).toBe(2);
        expect(peerB.host.stats.invokesDispatched).toBe(0);
        // The write physically landed in peer A's host-side store.
        expect(peerA.adapter.store.entries.has("kb_router_baseline_1")).toBe(true);
        expect(peerB.adapter.store.entries.has("kb_router_baseline_1")).toBe(false);

        // DoD #6 — same for the check callsite.
        const checkRequest: CheckRequest = {
          scope: { scope_id: "toy-scope-001" },
        };
        expect(
          (checkRequest as unknown as Record<string, unknown>).peer_id,
        ).toBeUndefined();

        const checker = () => ({ ok: true as const, value: [] as never[] });
        const check = await orchestrateCheck(router, checkRequest, checker);
        expect(check.ok).toBe(true);
        // check = listKnowledgeEntries + listTimelineEvents + putFindings;
        // all three landed on A (2 + 3 = 5 invokes total), none on B.
        expect(peerA.host.stats.invokesDispatched).toBe(5);
        expect(peerB.host.stats.invokesDispatched).toBe(0);
      } finally {
        closePeer(peerA);
        closePeer(peerB);
      }
    },
    15000,
  );

  it(
    "rejects with no_capable_peer when no registered peer advertises the required capability (terminal, deterministic, no wrong-peer fallback)",
    async () => {
      // Neither peer can serve baseline ops — both advertise l2-computable
      // only, so the first upsert port call (port.knowledge.get →
      // spoke-baseline) has no capable peer.
      const peerX = await dialPeer(
        seed(0xc0),
        seed(0x30),
        peerManifest("peer-x", ["l2-computable"], ["toy_world"], ["computable"]),
      );
      const peerY = await dialPeer(
        seed(0xd0),
        seed(0x40),
        peerManifest("peer-y", ["l2-computable"], ["toy_world"], ["computable"]),
      );
      try {
        const router = connectMultiPeerRouter({ hostId: "test-router" });
        router.registerPeer(peerX.client);
        router.registerPeer(peerY.client);

        const upsertRequest: UpsertRequest = {
          knowledge_entries: [
            freshEntry("kb_router_none_1", "Router No Peer"),
          ],
        };
        // The reject is stable (§5): repeated calls produce the identical
        // terminal reject, and no invoke ever reaches a host.
        const first = await orchestrateUpsert(router, upsertRequest);
        const second = await orchestrateUpsert(router, upsertRequest);

        for (const result of [first, second]) {
          expect(result.ok).toBe(false);
          if (!result.ok) {
            expect(result.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
            expect(result.details?.wire_code).toBe("no_capable_peer");
            expect(result.details?.kind).toBe("no_capable_peer");
          }
        }
        expect(second).toEqual(first);
        expect(peerX.host.stats.invokesDispatched).toBe(0);
        expect(peerY.host.stats.invokesDispatched).toBe(0);
      } finally {
        closePeer(peerX);
        closePeer(peerY);
      }
    },
    15000,
  );

  it(
    "deterministic tie-break: the lowest peer_id serves both orchestrated families when three peers match",
    async () => {
      // Three peers, all baseline-capable — every one matches the upsert AND
      // check requirements (DoD: "a third peer that matches both"). Selection
      // must collapse to the lexicographically-lowest peer_id (§4).
      const peers = [
        await dialPeer(
          seed(0xa1),
          seed(0x11),
          peerManifest("peer-a", ["spoke-baseline"], ["toy_world"], ["data-store"]),
        ),
        await dialPeer(
          seed(0xb1),
          seed(0x21),
          peerManifest("peer-b", ["spoke-baseline"], ["toy_world"], ["data-store"]),
        ),
        await dialPeer(
          seed(0xc1),
          seed(0x31),
          peerManifest("peer-c", ["spoke-baseline"], ["toy_world"], ["data-store"]),
        ),
      ];
      try {
        const router = connectMultiPeerRouter({ hostId: "test-router" });
        for (const peer of peers) {
          router.registerPeer(peer.client);
        }

        const expectedWinner = lowestPeerId(peers.map((peer) => peer.peerId));
        expect(peers.some((peer) => peer.peerId === expectedWinner)).toBe(true);

        const upsertRequest: UpsertRequest = {
          knowledge_entries: [
            freshEntry("kb_router_tie_1", "Router Tie One"),
          ],
        };
        const upsert = await orchestrateUpsert(router, upsertRequest);
        expect(upsert.ok).toBe(true);

        const checkRequest: CheckRequest = {
          scope: { scope_id: "toy-scope-001" },
        };
        const checker = () => ({ ok: true as const, value: [] as never[] });
        const check = await orchestrateCheck(router, checkRequest, checker);
        expect(check.ok).toBe(true);

        // upsert = 2 invokes (get + put), check = 3 (two scope queries +
        // putFindings) — all on the same lowest peer_id, none elsewhere.
        for (const peer of peers) {
          expect(peer.host.stats.invokesDispatched).toBe(
            peer.peerId === expectedWinner ? 5 : 0,
          );
        }

        // Stability (§4): a repeat upsert (fresh entry) re-selects the same
        // peer — 2 more invokes on the winner, still zero elsewhere.
        const repeatRequest: UpsertRequest = {
          knowledge_entries: [
            freshEntry("kb_router_tie_2", "Router Tie Two"),
          ],
        };
        const repeat = await orchestrateUpsert(router, repeatRequest);
        expect(repeat.ok).toBe(true);
        for (const peer of peers) {
          expect(peer.host.stats.invokesDispatched).toBe(
            peer.peerId === expectedWinner ? 7 : 0,
          );
        }
      } finally {
        for (const peer of peers) {
          closePeer(peer);
        }
      }
    },
    15000,
  );

  it(
    "aggregates a composed view and a per-peer manifest list over real loopback peers (§6)",
    async () => {
      // Overlapping capabilities with distinct roles/namespaces: the
      // composed view unions + dedups the real signed-hello manifests,
      // carries the ROUTER's own host_id (never a peer's), omits
      // authority, and lists contributing peer ids sorted; the per-peer
      // array returns each peer's OWN cached hello manifest, sorted by
      // peer_id — per-peer data, never the union.
      const peerA = await dialPeer(
        seed(0xe1),
        seed(0x41),
        peerManifest(
          "host-a",
          ["spoke-baseline", "l2-computable"],
          ["alpha", "beta"],
          ["data-store", "checker"],
        ),
      );
      const peerB = await dialPeer(
        seed(0xf2),
        seed(0x52),
        peerManifest(
          "host-b",
          ["spoke-baseline", "l2-computable"],
          ["beta", "gamma"],
          ["data-store", "assembler"],
        ),
      );
      try {
        const router = connectMultiPeerRouter({ hostId: "test-router" });
        router.registerPeer(peerA.client);
        router.registerPeer(peerB.client);

        const composed = await router.getHostCapabilityManifest();
        expect(composed.ok).toBe(true);
        if (composed.ok) {
          // §6: the router's own identity, not a peer's host_id.
          expect(composed.value.host_id).toBe("test-router");
          // Set-union, deduplicated across the two peers.
          expect([...composed.value.capabilities].sort()).toEqual([
            "l2-computable",
            "spoke-baseline",
          ]);
          expect([...composed.value.roles].sort()).toEqual([
            "assembler",
            "checker",
            "data-store",
          ]);
          expect([...composed.value.namespaces].sort()).toEqual([
            "alpha",
            "beta",
            "gamma",
          ]);
          // §6: the composed view synthesizes no authority scope.
          expect(composed.value.authority).toBeUndefined();
          // Contributing peer ids, lexicographic UTF-8 byte order.
          expect(composed.value.extensions.router).toEqual({
            peers: [peerA.peerId, peerB.peerId].sort(compareUtf8Strings),
          });
        }

        const perPeer = await router.listPeerHostCapabilityManifests();
        expect(perPeer.ok).toBe(true);
        if (perPeer.ok) {
          // Each entry is one peer's cached hello manifest (per-peer, not
          // the union), ordered by peer_id.
          expect(perPeer.value.map((entry) => entry.host_id).sort()).toEqual([
            "host-a",
            "host-b",
          ]);
          const hostA = perPeer.value.find((entry) => entry.host_id === "host-a");
          const hostB = perPeer.value.find((entry) => entry.host_id === "host-b");
          expect(hostA?.roles).toEqual(["data-store", "checker"]);
          expect(hostA?.namespaces).toEqual(["alpha", "beta"]);
          expect(hostB?.roles).toEqual(["data-store", "assembler"]);
          expect(hostB?.namespaces).toEqual(["beta", "gamma"]);
        }
      } finally {
        closePeer(peerA);
        closePeer(peerB);
      }
    },
    15000,
  );
});
