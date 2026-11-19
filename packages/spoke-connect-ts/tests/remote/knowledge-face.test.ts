/**
 * Knowledge capability split (design §4/§5 acceptance):
 *
 * 1. `capabilitySatisfies` — plain membership: `spoke-baseline` never
 *    satisfies `spoke-knowledge` (the knowledge face serves those ops).
 * 2. Knowledge-only serve ports (`knowledgeOnlyServePorts`) serve the three
 *    knowledge ops without a baseline face; `assemble` stays denied.
 * 3. Per-peer admission: two callers admitted to different scopes cannot
 *    read/list/update each other's same-id entries; an unbound caller and a
 *    `scope_id`/`viewpoint` mismatch refuse `NOT_AUTHORIZED` before any
 *    store call.
 * 4. The router's hard filter selects knowledge-only peers for the three
 *    knowledge ops while a knowledge-only peer never qualifies for core
 *    baseline ops.
 */

import { describe, expect, it, vi } from "vitest";

import { spokeOk, spokeReject, type SpokeResult } from "@42ch/spoke-operations";

import {
  capabilitySatisfies,
  CAPABILITY_SPOKE_BASELINE,
  CAPABILITY_SPOKE_KNOWLEDGE,
} from "../../src/core/dispatch.js";
import {
  connectMultiPeerRouter,
  connectRemoteAdapter,
  loopbackTransportPair,
} from "../../src/remote/index.js";
import {
  connectResponder,
  ConnectResponder,
  knowledgeOnlyServePorts,
  type RemoteAdapter,
  type RemoteKnowledgeFace,
  type RemoteServePorts,
} from "../../src/remote/index.js";
import type { Transport } from "../../src/remote/transport.js";
import {
  selectPeerForOp,
  type SelectablePeer,
} from "../../src/remote/multi-peer-router.js";
import { getPublicKeyEd25519 } from "../../src/crypto.js";
import { derivePeerIdFromEd25519Pubkey } from "../../src/identity.js";
import { authenticateInvokeRequest } from "../../src/core/envelope-auth.js";
import { generateNonce } from "../../src/core/nonce.js";
import { signHelloEd25519, verifyHelloEd25519 } from "../../src/core/hello.js";
import { decodeJsonMessage, encodeJsonMessage } from "../../src/framing.js";
import type { HostCapabilityManifest, KnowledgeEntry, Scope } from "@42ch/spoke-schemas";

const textEncoder = new TextEncoder();

function seed(base: number): Uint8Array {
  return Uint8Array.from({ length: 32 }, (_, i) => base + i);
}

function manifest(hostId: string, capabilities: string[]): HostCapabilityManifest {
  return {
    schema_version: 1,
    host_id: hostId,
    roles: ["data-store"],
    capabilities: [capabilities[0]!, ...capabilities.slice(1)],
    namespaces: ["world_a"],
    extensions: {},
  };
}

// ── capabilitySatisfies (design §1) ───────────────────────────────────────

describe("capabilitySatisfies", () => {
  it("is plain membership: no capability satisfies another", () => {
    const baseline = [CAPABILITY_SPOKE_BASELINE];
    const knowledge = [CAPABILITY_SPOKE_KNOWLEDGE];
    const both = [CAPABILITY_SPOKE_KNOWLEDGE, CAPABILITY_SPOKE_BASELINE];

    expect(capabilitySatisfies(knowledge, CAPABILITY_SPOKE_KNOWLEDGE)).toBe(true);
    expect(capabilitySatisfies(both, CAPABILITY_SPOKE_KNOWLEDGE)).toBe(true);
    expect(capabilitySatisfies(knowledge, CAPABILITY_SPOKE_BASELINE)).toBe(false);
    // `spoke-knowledge` must be negotiated explicitly.
    expect(capabilitySatisfies(baseline, CAPABILITY_SPOKE_KNOWLEDGE)).toBe(false);
    expect(capabilitySatisfies([], CAPABILITY_SPOKE_KNOWLEDGE)).toBe(false);
  });
});

// ── Router hard filter (design §5) ────────────────────────────────────────

describe("selectPeerForOp knowledge rows", () => {
  const knowledgeOnlyPeer: SelectablePeer = {
    peerId: "peer-knowledge",
    manifest: manifest("peer-knowledge", ["spoke-knowledge"]),
  };
  const baselinePeer: SelectablePeer = {
    peerId: "peer-baseline",
    manifest: manifest("peer-baseline", ["spoke-baseline"]),
  };

  it("selects a knowledge-only peer for the three knowledge ops", () => {
    for (const op of [
      "port.knowledge.get",
      "port.knowledge.put",
      "port.scope.list_knowledge_entries",
    ]) {
      const selected = selectPeerForOp([knowledgeOnlyPeer], op, {});
      expect(selected.ok, op).toBe(true);
    }
  });

  it("never selects a baseline-only peer for a knowledge op", () => {
    for (const op of [
      "port.knowledge.get",
      "port.knowledge.put",
      "port.scope.list_knowledge_entries",
    ]) {
      const selected = selectPeerForOp([baselinePeer], op, {});
      expect(selected.ok, op).toBe(false);
    }
  });

  it("never selects a knowledge-only peer for a core baseline op", () => {
    const selected = selectPeerForOp([knowledgeOnlyPeer], "assemble", {});
    expect(selected.ok).toBe(false);
    if (!selected.ok) {
      expect(selected.code).toBe("CAPABILITY_PORT_MISSING");
    }
  });
});

// ── Knowledge-only responder serving (design §4) ──────────────────────────

/** In-memory scoped store: entries partitioned by admitted scope_id. */
function scopedStoreFace(
  admission: (peerId: string) => SpokeResult<Scope>,
  storeCalls: string[] = [],
  listGate?: Promise<void>,
): RemoteKnowledgeFace {
  const entries = new Map<string, Map<string, KnowledgeEntry>>();
  const bucket = (scopeId: string): Map<string, KnowledgeEntry> => {
    let existing = entries.get(scopeId);
    if (existing === undefined) {
      existing = new Map();
      entries.set(scopeId, existing);
    }
    return existing;
  };
  return {
    store: {
      async getScopedKnowledgeEntry(scope, entryId) {
        storeCalls.push("get");
        const found = bucket(scope.scope_id).get(entryId);
        return found === undefined
          ? spokeReject("KNOWLEDGE_ENTRY_NOT_FOUND", "no such entry")
          : spokeOk(found);
      },
      async putScopedKnowledgeEntry(scope, entry, expectedBaseRevision) {
        storeCalls.push("put");
        const store = bucket(scope.scope_id);
        const stored = store.get(entry.entry_id);
        if (expectedBaseRevision === null) {
          if (stored !== undefined) {
            return spokeReject("KNOWLEDGE_ENTRY_ALREADY_EXISTS", "exists");
          }
        } else if (
          stored === undefined ||
          stored.revision !== expectedBaseRevision
        ) {
          return spokeReject("STORED_REVISION_STALE", "revision mismatch");
        }
        store.set(entry.entry_id, { ...entry });
        return spokeOk({ ...entry });
      },
      async listScopedKnowledgeEntries(scope) {
        storeCalls.push("list");
        // A pending gate keeps the store call outstanding, so only the serve
        // deadline can answer.
        if (listGate !== undefined) {
          await listGate;
        }
        const all = [...bucket(scope.scope_id).values()].filter(
          (candidate) =>
            scope.entry_types === undefined ||
            scope.entry_types.includes(candidate.entry_type),
        );
        return spokeOk(
          scope.entry_ids === undefined
            ? all
            : all.filter((candidate) =>
                scope.entry_ids!.includes(candidate.entry_id),
              ),
        );
      },
    },
    admission,
  };
}

interface DialedKnowledgeHost {
  host: { close(): void; state: string };
  client: RemoteAdapter;
  close: () => void;
}

async function dialKnowledgeOnlyHost(options: {
  admission: (peerId: string) => SpokeResult<Scope>;
  clientSeed: Uint8Array;
  clientCapabilities: string[];
  storeCalls?: string[];
  listGate?: Promise<void>;
  invokeTimeoutMs?: number;
}): Promise<DialedKnowledgeHost> {
  const hostSeed = seed(0xa0);
  const hostPubkey = getPublicKeyEd25519(hostSeed);
  const hostPeerId = derivePeerIdFromEd25519Pubkey(hostPubkey);
  const clientPubkey = getPublicKeyEd25519(options.clientSeed);
  const clientPeerId = derivePeerIdFromEd25519Pubkey(clientPubkey);
  const pair = loopbackTransportPair();
  const host = await connectResponder({
    transport: pair.server,
    identity: { seed: hostSeed },
    manifest: manifest("knowledge-host", ["spoke-knowledge"]),
    allowlist: [clientPeerId],
    peerKeys: { [clientPeerId]: clientPubkey },
    ports: knowledgeOnlyServePorts(
      scopedStoreFace(options.admission, options.storeCalls, options.listGate),
    ),
    invokeTimeoutMs: options.invokeTimeoutMs,
  });
  const client = await connectRemoteAdapter({
    transport: pair.client,
    localIdentity: { seed: options.clientSeed },
    localManifest: manifest("client", options.clientCapabilities),
    remotePubkey: hostPubkey,
    allowlist: [hostPeerId],
  });
  return { host, client, close: (): void => { client.close(); host.close(); } };
}

const SCOPE_A: Scope = { scope_id: "scope_a" };
const SCOPE_B: Scope = { scope_id: "scope_b" };

function entry(id: string): KnowledgeEntry {
  return {
    schema_version: 1,
    entry_id: id,
    entry_type: "note",
    canonical_name: id,
    status: "confirmed",
    revision: 1,
    body: { summary: id },
    extensions: {},
  };
}

describe("knowledge-only responder", () => {
  it("serves the three knowledge ops and denies assemble without a baseline face", async () => {
    const { client, close } = await dialKnowledgeOnlyHost({
      admission: () => spokeOk(SCOPE_A),
      clientSeed: seed(0x10),
      clientCapabilities: ["spoke-knowledge"],
    });
    try {
      const put = await client.putKnowledgeEntry(entry("kb_a1"), null);
      expect(put.ok).toBe(true);

      const got = await client.getKnowledgeEntry("kb_a1");
      expect(got.ok).toBe(true);

      const listed = await client.listKnowledgeEntries(SCOPE_A);
      expect(listed.ok).toBe(true);
      if (listed.ok) {
        expect(listed.value.map((e) => e.entry_id)).toEqual(["kb_a1"]);
      }

      // Core op denial is covered by the raw-wire probe below.
    } finally {
      close();
    }
  });

  it("denies assemble on the knowledge-only dispatch gate (raw wire)", async () => {
    const face = scopedStoreFace(() => spokeOk(SCOPE_A));
    const hostSeed = seed(0xa0);
    const hostPubkey = getPublicKeyEd25519(hostSeed);
    const hostPeerId = derivePeerIdFromEd25519Pubkey(hostPubkey);
    const clientSeed = seed(0x10);
    const clientPubkey = getPublicKeyEd25519(clientSeed);
    const clientPeerId = derivePeerIdFromEd25519Pubkey(clientPubkey);
    const pair = loopbackTransportPair();
    const host = await connectResponder({
      transport: pair.server,
      identity: { seed: hostSeed },
      manifest: manifest("knowledge-host", ["spoke-knowledge"]),
      allowlist: [clientPeerId],
      peerKeys: { [clientPeerId]: clientPubkey },
      ports: knowledgeOnlyServePorts(face),
    });
    const clientEnd = pair.client;
    try {
      const nonce = generateNonce();
      await clientEnd.send(
        textEncoder.encode(
          encodeJsonMessage(
            await signHelloEd25519(clientSeed, nonce, manifest("client", ["spoke-knowledge"])),
          ),
        ),
      );
      const responderHello = decodeJsonMessage(await clientEnd.recv()) as Parameters<
        typeof verifyHelloEd25519
      >[2];
      await verifyHelloEd25519(hostPubkey, hostPeerId, responderHello, nonce);
      const snapshot = decodeJsonMessage(await clientEnd.recv()) as {
        session_id: string;
      };
      const request = await authenticateInvokeRequest(clientSeed, {
        session_id: snapshot.session_id,
        sequence: 0,
        request_id: "assemble-deny",
        op: "assemble",
        payload: { scope: { scope_id: "scope_a" } },
      });
      await clientEnd.send(textEncoder.encode(encodeJsonMessage(request)));
      const response = decodeJsonMessage(await clientEnd.recv()) as {
        error?: { code?: string };
      };
      expect(response.error?.code).toBe("op_unsupported");
    } finally {
      clientEnd.close();
      host.close();
    }
  });

  it("isolates same-id entries across two admitted scopes", async () => {
    // One shared scoped store behind two host instances; each caller is
    // admitted to its own scope by peer id.
    const admitted = new Map<string, Scope>([
      [derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0x10))), SCOPE_A],
      [derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0x11))), SCOPE_B],
    ]);
    const unboundSeed = seed(0x12);
    const unboundPeerId = derivePeerIdFromEd25519Pubkey(
      getPublicKeyEd25519(unboundSeed),
    );
    const storeCalls: string[] = [];
    const face = scopedStoreFace((peerId) => {
      const scope = admitted.get(peerId);
      return scope === undefined
        ? spokeReject("NOT_AUTHORIZED", "knowledge scope not authorized")
        : spokeOk(scope);
    }, storeCalls);

    const dial = async (
      clientSeed: Uint8Array,
      capabilities: string[] = ["spoke-knowledge"],
    ): Promise<DialedKnowledgeHost> => {
      const hostSeed = seed(0xa0);
      const hostPubkey = getPublicKeyEd25519(hostSeed);
      const hostPeerId = derivePeerIdFromEd25519Pubkey(hostPubkey);
      const clientPubkey = getPublicKeyEd25519(clientSeed);
      const clientPeerId = derivePeerIdFromEd25519Pubkey(clientPubkey);
      const pair = loopbackTransportPair();
      const host = await connectResponder({
        transport: pair.server,
        identity: { seed: hostSeed },
        manifest: manifest("knowledge-host", ["spoke-knowledge", "ke-ownership"]),
        allowlist: [clientPeerId, unboundPeerId],
        peerKeys: {
          [clientPeerId]: clientPubkey,
          [unboundPeerId]: getPublicKeyEd25519(unboundSeed),
        },
        ports: knowledgeOnlyServePorts(face),
      });
      const client = await connectRemoteAdapter({
        transport: pair.client,
        localIdentity: { seed: clientSeed },
        localManifest: manifest("client", capabilities),
        remotePubkey: hostPubkey,
        allowlist: [hostPeerId],
      });
      return {
        host,
        client,
        close: (): void => {
          client.close();
          host.close();
        },
      };
    };

    // Caller A carries ke-ownership so the viewpoint-matching probe below
    // reaches the scope-equality gate rather than the ownership conjunct.
    const callerA = await dial(seed(0x10), ["spoke-knowledge", "ke-ownership"]);
    const callerB = await dial(seed(0x11));
    try {
      // Caller A writes entry "kb_shared" in scope A.
      const putA = await callerA.client.putKnowledgeEntry(entry("kb_shared"), null);
      expect(putA.ok).toBe(true);

      // Caller B (scope B) sees neither the entry nor the scope A listing.
      const gotByB = await callerB.client.getKnowledgeEntry("kb_shared");
      expect(gotByB.ok).toBe(false);
      const listedByB = await callerB.client.listKnowledgeEntries(SCOPE_B);
      expect(listedByB.ok).toBe(true);
      if (listedByB.ok) {
        expect(listedByB.value).toEqual([]);
      }

      // Caller B's same-id write lands in scope B, never overwriting A's.
      const putByB = await callerB.client.putKnowledgeEntry(
        entry("kb_shared"),
        null,
      );
      expect(putByB.ok).toBe(true);
      const listedByA = await callerA.client.listKnowledgeEntries(SCOPE_A);
      expect(listedByA.ok).toBe(true);
      if (listedByA.ok) {
        expect(listedByA.value).toHaveLength(1);
        expect(listedByA.value[0]!.body).toEqual({ summary: "kb_shared" });
      }

      // An unbound caller is refused NOT_AUTHORIZED before any store call.
      const unbound = await dial(unboundSeed);
      try {
        const callsBefore = storeCalls.length;
        const refused = await unbound.client.listKnowledgeEntries(SCOPE_A);
        expect(refused.ok).toBe(false);
        if (!refused.ok) {
          expect(refused.code).toBe("NOT_AUTHORIZED");
        }
        expect(storeCalls.length).toBe(callsBefore);
      } finally {
        unbound.close();
      }

      // A scope_id mismatch refuses NOT_AUTHORIZED: caller A admitted to
      // scope A cannot list scope B even though the store call would have
      // been scoped.
      const callsBeforeMismatch = storeCalls.length;
      const mismatch = await callerA.client.listKnowledgeEntries(SCOPE_B);
      expect(mismatch.ok).toBe(false);
      if (!mismatch.ok) {
        expect(mismatch.code).toBe("NOT_AUTHORIZED");
      }
      expect(storeCalls.length).toBe(callsBeforeMismatch);

      // A viewpoint mismatch (admitted viewpoint the request omits) refuses.
      admitted.set(
        derivePeerIdFromEd25519Pubkey(getPublicKeyEd25519(seed(0x10))),
        { scope_id: "scope_a", viewpoint: "holder-a" },
      );
      const callsBeforeViewpoint = storeCalls.length;
      const omitted = await callerA.client.listKnowledgeEntries(SCOPE_A);
      expect(omitted.ok).toBe(false);
      if (!omitted.ok) {
        expect(omitted.code).toBe("NOT_AUTHORIZED");
      }
      expect(storeCalls.length).toBe(callsBeforeViewpoint);
      // The matching viewpoint serves.
      const withViewpoint = await callerA.client.listKnowledgeEntries({
        scope_id: "scope_a",
        viewpoint: "holder-a",
      });
      expect(withViewpoint.ok).toBe(true);
    } finally {
      callerA.close();
      callerB.close();
    }
  });

  it("rejects a stale-revision put end to end without replacing the entry", async () => {
    const { client, close } = await dialKnowledgeOnlyHost({
      admission: () => spokeOk(SCOPE_A),
      clientSeed: seed(0x10),
      clientCapabilities: ["spoke-knowledge"],
    });
    try {
      const created = await client.putKnowledgeEntry(entry("kb_stale"), null);
      expect(created.ok).toBe(true);

      // Advance the stored entry to revision 2.
      const bump = await client.putKnowledgeEntry(
        { ...entry("kb_stale"), revision: 2, body: { summary: "updated" } },
        1,
      );
      expect(bump.ok).toBe(true);

      // A put with a non-null expected revision that mismatches the stored
      // one is refused and the entry is unchanged.
      const stale = await client.putKnowledgeEntry(
        { ...entry("kb_stale"), body: { summary: "overwritten" } },
        1,
      );
      expect(stale.ok).toBe(false);
      if (!stale.ok) {
        expect(["STORED_REVISION_STALE", "REVISION_CONFLICT"]).toContain(
          stale.code,
        );
      }
      const got = await client.getKnowledgeEntry("kb_stale");
      expect(got.ok).toBe(true);
      if (got.ok) {
        expect(got.value.body).toEqual({ summary: "updated" });
        expect(got.value.revision).toBe(2);
      }

      // The matching expected revision serves.
      const fresh = await client.putKnowledgeEntry(
        { ...entry("kb_stale"), revision: 3, body: { summary: "v3" } },
        2,
      );
      expect(fresh.ok).toBe(true);
    } finally {
      close();
    }
  });

  it("shares one serve deadline between admission and the list store call", async () => {
    const storeCalls: string[] = [];
    // Executor form: `Promise.withResolvers` needs the ES2024 lib. The store
    // call stays outstanding, so only the serve deadline can answer.
    const listGate = new Promise<void>(() => {
      /* never settles */
    });
    // Only `Date` is faked: admission advances the responder's clock past the
    // budget instead of sleeping, so no wall-clock bound is asserted and the
    // outcome does not depend on machine timing.
    vi.useFakeTimers({ toFake: ["Date"] });
    const { client, close } = await dialKnowledgeOnlyHost({
      // Admission is synchronous producer work, so it spends the budget in
      // place (the Rust path parks it on the blocking pool).
      admission: () => {
        vi.setSystemTime(Date.now() + 500);
        return spokeOk(SCOPE_A);
      },
      clientSeed: seed(0x13),
      clientCapabilities: ["spoke-knowledge"],
      storeCalls,
      listGate,
      invokeTimeoutMs: 100,
    });
    try {
      const listed = await client.listKnowledgeEntries(SCOPE_A);
      expect(listed.ok).toBe(false);
      if (!listed.ok) {
        expect(listed.code).toBe("INTERNAL_ERROR");
        expect(listed.details?.kind).toBe("timeout");
      }
      // The whole budget went to admission, so the store gets none of it and
      // is never called — a second full budget would have called it.
      expect(storeCalls).toEqual([]);
    } finally {
      close();
      vi.useRealTimers();
    }
  });

  it("serves the list once when admission leaves the budget", async () => {
    const storeCalls: string[] = [];
    const { client, close } = await dialKnowledgeOnlyHost({
      admission: () => spokeOk(SCOPE_A),
      clientSeed: seed(0x13),
      clientCapabilities: ["spoke-knowledge"],
      storeCalls,
      invokeTimeoutMs: 100,
    });
    try {
      const listed = await client.listKnowledgeEntries(SCOPE_A);
      expect(listed.ok).toBe(true);
      expect(storeCalls).toEqual(["list"]);
    } finally {
      close();
    }
  });

  it("never widens a list scope beyond the admitted authority", async () => {
    const admitted: Scope = {
      scope_id: "scope_a",
      entry_ids: ["kb_adm1"],
      entry_types: ["note"],
      source_id: "source_a",
      extensions: { product: { limit: "admitted" } },
    };
    const storeCalls: string[] = [];
    const { client, close } = await dialKnowledgeOnlyHost({
      admission: () => spokeOk(admitted),
      clientSeed: seed(0x14),
      clientCapabilities: ["spoke-knowledge"],
      storeCalls,
    });
    try {
      expect((await client.putKnowledgeEntry(entry("kb_adm1"), null)).ok).toBe(
        true,
      );
      expect((await client.putKnowledgeEntry(entry("kb_adm2"), null)).ok).toBe(
        true,
      );

      // The request omits the admitted narrowing: only the admitted id returns.
      const admittedOnly = await client.listKnowledgeEntries({
        scope_id: "scope_a",
      });
      expect(admittedOnly.ok).toBe(true);
      if (admittedOnly.ok) {
        expect(admittedOnly.value.map((listed) => listed.entry_id)).toEqual([
          "kb_adm1",
        ]);
      }

      // A wider request id set still intersects the admitted one.
      const narrowed = await client.listKnowledgeEntries({
        scope_id: "scope_a",
        entry_ids: ["kb_adm1", "kb_adm2"],
      });
      expect(narrowed.ok).toBe(true);
      if (narrowed.ok) {
        expect(narrowed.value.map((listed) => listed.entry_id)).toEqual([
          "kb_adm1",
        ]);
      }

      // A disjoint admitted id is an empty intersection: no results, no
      // refusal.
      const disjoint = await client.listKnowledgeEntries({
        scope_id: "scope_a",
        entry_ids: ["kb_adm2"],
      });
      expect(disjoint.ok).toBe(true);
      if (disjoint.ok) {
        expect(disjoint.value).toEqual([]);
      }

      // A scalar the admission does not grant is not a narrowing: refuse
      // before the store.
      const callsBefore = storeCalls.length;
      const refused = await client.listKnowledgeEntries({
        scope_id: "scope_a",
        source_id: "source_b",
      });
      expect(refused.ok).toBe(false);
      if (!refused.ok) {
        expect(refused.code).toBe("NOT_AUTHORIZED");
      }
      expect(storeCalls).toHaveLength(callsBefore);
    } finally {
      close();
    }
  });

  it("serves a requested entry_types subset and refuses values outside it", async () => {
    const admitted: Scope = {
      scope_id: "scope_a",
      entry_types: ["note", "decision"],
    };
    const storeCalls: string[] = [];
    const { client, close } = await dialKnowledgeOnlyHost({
      admission: () => spokeOk(admitted),
      clientSeed: seed(0x15),
      clientCapabilities: ["spoke-knowledge"],
      storeCalls,
    });
    try {
      expect((await client.putKnowledgeEntry(entry("kb_note"), null)).ok).toBe(
        true,
      );
      expect(
        (
          await client.putKnowledgeEntry(
            { ...entry("kb_decision"), entry_type: "decision" },
            null,
          )
        ).ok,
      ).toBe(true);

      // A subset of the admitted entry_types narrows: only notes are served.
      const narrowed = await client.listKnowledgeEntries({
        scope_id: "scope_a",
        entry_types: ["note"],
      });
      expect(narrowed.ok).toBe(true);
      if (narrowed.ok) {
        expect(narrowed.value.map((listed) => listed.entry_id)).toEqual([
          "kb_note",
        ]);
      }

      // A value outside the admitted set is a widening: refuse before the
      // store.
      const callsBefore = storeCalls.length;
      const widened = await client.listKnowledgeEntries({
        scope_id: "scope_a",
        entry_types: ["note", "other"],
      });
      expect(widened.ok).toBe(false);
      if (!widened.ok) {
        expect(widened.code).toBe("NOT_AUTHORIZED");
      }
      expect(storeCalls).toHaveLength(callsBefore);
    } finally {
      close();
    }
  });
});

// ── Payload pre-validation + refusal branches (QC B2/B3/B4) ───────────────

describe("knowledge dispatch pre-validation and refusals", () => {
  /** Raw-wire knowledge-only host with admission/store call counters. */
  async function dialRawKnowledgeHost(ports: unknown): Promise<{
    responder: ConnectResponder;
    clientEnd: Transport;
    sessionId: string;
    admissionCalls: string[];
    storeCalls: string[];
    close: () => void;
  }> {
    const hostSeed = seed(0xa0);
    const hostPubkey = getPublicKeyEd25519(hostSeed);
    const hostPeerId = derivePeerIdFromEd25519Pubkey(hostPubkey);
    const clientSeed = seed(0x10);
    const clientPubkey = getPublicKeyEd25519(clientSeed);
    const clientPeerId = derivePeerIdFromEd25519Pubkey(clientPubkey);
    const pair = loopbackTransportPair();
    const admissionCalls: string[] = [];
    const storeCalls: string[] = [];
    const responder = await connectResponder({
      transport: pair.server,
      identity: { seed: hostSeed },
      manifest: manifest("knowledge-host", ["spoke-knowledge"]),
      allowlist: [clientPeerId],
      peerKeys: { [clientPeerId]: clientPubkey },
      ports: ports as RemoteServePorts | undefined,
    });
    const clientEnd = pair.client;
    const nonce = generateNonce();
    await clientEnd.send(
      textEncoder.encode(
        encodeJsonMessage(
          await signHelloEd25519(
            clientSeed,
            nonce,
            manifest("client", ["spoke-knowledge"]),
          ),
        ),
      ),
    );
    const responderHello = decodeJsonMessage(await clientEnd.recv()) as
      Parameters<typeof verifyHelloEd25519>[2];
    await verifyHelloEd25519(hostPubkey, hostPeerId, responderHello, nonce);
    const snapshot = decodeJsonMessage(await clientEnd.recv()) as {
      session_id: string;
    };
    return {
      responder,
      clientEnd,
      sessionId: snapshot.session_id,
      admissionCalls,
      storeCalls,
      close: (): void => {
        clientEnd.close();
        responder.close();
      },
    };
  }

  async function rawInvoke(
    clientEnd: Transport,
    clientSeed: Uint8Array,
    sessionId: string,
    sequence: number,
    requestId: string,
    op: string,
    payload: Record<string, unknown>,
  ): Promise<{ error?: { code?: string; details?: Record<string, unknown> } }> {
    const request = await authenticateInvokeRequest(clientSeed, {
      session_id: sessionId,
      sequence,
      request_id: requestId,
      op,
      payload,
    });
    await clientEnd.send(textEncoder.encode(encodeJsonMessage(request)));
    return decodeJsonMessage(await clientEnd.recv()) as {
      error?: { code?: string; details?: Record<string, unknown> };
    };
  }

  it("answers INVALID_INPUT with zero admission and store calls on malformed get payloads (B2)", async () => {
    const admissionCalls: string[] = [];
    const face = scopedStoreFace((peerId) => {
      admissionCalls.push(peerId);
      return spokeOk(SCOPE_A);
    });
    const { clientEnd, sessionId, close } = await dialRawKnowledgeHost(
      knowledgeOnlyServePorts(face),
    );
    try {
      const missing = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        0,
        "get-missing",
        "port.knowledge.get",
        {},
      );
      expect(missing.error?.code).toBe("INVALID_INPUT");

      const nonString = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        1,
        "get-nonstring",
        "port.knowledge.get",
        { entry_id: 42 },
      );
      expect(nonString.error?.code).toBe("INVALID_INPUT");

      const empty = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        2,
        "get-empty",
        "port.knowledge.get",
        { entry_id: "" },
      );
      expect(empty.error?.code).toBe("INVALID_INPUT");

      expect(admissionCalls).toHaveLength(0);
    } finally {
      close();
    }
  });

  it("answers INVALID_INPUT with zero admission and store calls on malformed put payloads (B2)", async () => {
    const admissionCalls: string[] = [];
    const storeCalls: string[] = [];
    const face = scopedStoreFace((peerId) => {
      admissionCalls.push(peerId);
      return spokeOk(SCOPE_A);
    }, storeCalls);
    const { clientEnd, sessionId, close } = await dialRawKnowledgeHost(
      knowledgeOnlyServePorts(face),
    );
    try {
      const badEntry = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        0,
        "put-bad-entry",
        "port.knowledge.put",
        { entry: "not-an-object" },
      );
      expect(badEntry.error?.code).toBe("INVALID_INPUT");

      const badRevision = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        1,
        "put-bad-revision",
        "port.knowledge.put",
        {
          entry: entry("kb_put"),
          expected_base_revision: "1",
        },
      );
      expect(badRevision.error?.code).toBe("INVALID_INPUT");

      // A malformed `entry` object (a required KnowledgeEntry member missing)
      // is refused like Rust's typed decode, before admission and the store.
      const missingName = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        2,
        "put-missing-canonical-name",
        "port.knowledge.put",
        {
          entry: { ...entry("kb_put"), canonical_name: undefined },
          expected_base_revision: null,
        },
      );
      expect(missingName.error?.code).toBe("INVALID_INPUT");

      // OCC revisions mirror Rust's `Option<u64>`: negative and fractional
      // values are invalid, not merely non-numeric ones.
      const negativeRevision = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        3,
        "put-negative-revision",
        "port.knowledge.put",
        { entry: entry("kb_put"), expected_base_revision: -1 },
      );
      expect(negativeRevision.error?.code).toBe("INVALID_INPUT");

      const fractionalRevision = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        4,
        "put-fractional-revision",
        "port.knowledge.put",
        { entry: entry("kb_put"), expected_base_revision: 1.5 },
      );
      expect(fractionalRevision.error?.code).toBe("INVALID_INPUT");

      // `entry.schema_version` is a typed `SchemaVersion` (integer >= 1) in
      // Rust; a missing or zero version fails decode.
      const missingSchemaVersion = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        5,
        "put-missing-schema-version",
        "port.knowledge.put",
        {
          entry: { ...entry("kb_put"), schema_version: undefined },
          expected_base_revision: null,
        },
      );
      expect(missingSchemaVersion.error?.code).toBe("INVALID_INPUT");

      const zeroSchemaVersion = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        6,
        "put-zero-schema-version",
        "port.knowledge.put",
        {
          entry: { ...entry("kb_put"), schema_version: 0 },
          expected_base_revision: null,
        },
      );
      expect(zeroSchemaVersion.error?.code).toBe("INVALID_INPUT");

      // Rust decodes the revision as `Option<u64>`: an integer beyond
      // `Number.MAX_SAFE_INTEGER` cannot be represented faithfully.
      const unsafeRevision = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        7,
        "put-unsafe-revision",
        "port.knowledge.put",
        { entry: entry("kb_put"), expected_base_revision: 1e20 },
      );
      expect(unsafeRevision.error?.code).toBe("INVALID_INPUT");

      expect(admissionCalls).toHaveLength(0);
      expect(storeCalls).toHaveLength(0);

      // A well-formed put passes validation and reaches admission + store.
      const valid = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        8,
        "put-valid",
        "port.knowledge.put",
        { entry: entry("kb_put"), expected_base_revision: null },
      );
      expect(valid.error).toBeUndefined();
      expect(admissionCalls).toHaveLength(1);
      expect(storeCalls).toEqual(["put"]);

      // Rust's `Option<u64>` decodes a missing member as None: a put without
      // `expected_base_revision` is a create, and the store sees `null`
      // (an `undefined` would take the stale-revision branch instead).
      const missingRevision = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        9,
        "put-missing-revision",
        "port.knowledge.put",
        { entry: entry("kb_put_no_revision") },
      );
      expect(missingRevision.error).toBeUndefined();
      expect(storeCalls).toEqual(["put", "put"]);
    } finally {
      close();
    }
  });

  it("refuses a non-object list scope before admission and the store", async () => {
    const admissionCalls: string[] = [];
    const storeCalls: string[] = [];
    const face = scopedStoreFace((peerId) => {
      admissionCalls.push(peerId);
      return spokeOk(SCOPE_A);
    }, storeCalls);
    const { clientEnd, sessionId, close } = await dialRawKnowledgeHost(
      knowledgeOnlyServePorts(face),
    );
    try {
      // The list scope decodes as a typed Scope (Rust parity): a scalar
      // never reaches the list-admission guard or the store.
      const refused = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        0,
        "list-non-object-scope",
        "port.scope.list_knowledge_entries",
        { scope: "scope_a" },
      );
      expect(refused.error?.code).toBe("INVALID_INPUT");
      expect(admissionCalls).toHaveLength(0);
      expect(storeCalls).toHaveLength(0);

      // A sound scope serves through the same host.
      const served = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        1,
        "list-object-scope",
        "port.scope.list_knowledge_entries",
        { scope: SCOPE_A },
      );
      expect(served.error).toBeUndefined();
      expect(storeCalls).toEqual(["list"]);
    } finally {
      close();
    }
  });

  it("answers the dispatch-deny wire op_unsupported for absent faces (B4)", async () => {
    const admissionCalls: string[] = [];
    const face = scopedStoreFace((peerId) => {
      admissionCalls.push(peerId);
      return spokeOk(SCOPE_A);
    });
    // No knowledge face (and no baseline face): the absent-face branch.
    const { clientEnd, sessionId, close } = await dialRawKnowledgeHost({});
    try {
      const denied = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        0,
        "absent-knowledge",
        "port.knowledge.get",
        { entry_id: "kb_any" },
      );
      // The dialer maps this dispatch-deny row to CAPABILITY_PORT_MISSING
      // (details.wire_code = "op_unsupported"); the wire never carries the
      // application-coded literal.
      expect(denied.error?.code).toBe("op_unsupported");

      const baselineDenied = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        1,
        "absent-baseline",
        "assemble",
        { scope: { scope_id: "scope_a" } },
      );
      expect(baselineDenied.error?.code).toBe("op_unsupported");
      expect(admissionCalls).toHaveLength(0);
    } finally {
      close();
    }
  });
});
