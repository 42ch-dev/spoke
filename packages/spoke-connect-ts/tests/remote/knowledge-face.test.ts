/**
 * note: Knowledge capability split (see
 * .agents/adr/implemented/feature/2026-11-19-knowledge-capability-split.md):
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

describe("capabilitySatisfies", () => {
  it("is plain membership: no capability satisfies another", () => {
    const baseline = [CAPABILITY_SPOKE_BASELINE];
    const knowledge = [CAPABILITY_SPOKE_KNOWLEDGE];
    const both = [CAPABILITY_SPOKE_KNOWLEDGE, CAPABILITY_SPOKE_BASELINE];

    expect(capabilitySatisfies(knowledge, CAPABILITY_SPOKE_KNOWLEDGE)).toBe(true);
    expect(capabilitySatisfies(both, CAPABILITY_SPOKE_KNOWLEDGE)).toBe(true);
    expect(capabilitySatisfies(knowledge, CAPABILITY_SPOKE_BASELINE)).toBe(false);
    expect(capabilitySatisfies(baseline, CAPABILITY_SPOKE_KNOWLEDGE)).toBe(false);
    expect(capabilitySatisfies([], CAPABILITY_SPOKE_KNOWLEDGE)).toBe(false);
  });
});

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

function scopedStoreFace(
  admission: (peerId: string) => SpokeResult<Scope>,
  storeCalls: string[] = [],
  storeGate?: Promise<void>,
  storeRejectOn?: "put" | "get",
  storeGateOn?: "put" | "get",
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
        if (storeRejectOn === "get") {
          throw new Error("store get exploded");
        }
        if (storeGate !== undefined && (storeGateOn ?? "put") === "get") {
          await storeGate;
        }
        const found = bucket(scope.scope_id).get(entryId);
        return found === undefined
          ? spokeReject("KNOWLEDGE_ENTRY_NOT_FOUND", "no such entry")
          : spokeOk(found);
      },
      async putScopedKnowledgeEntry(scope, entry, expectedBaseRevision) {
        storeCalls.push("put");
        if (storeRejectOn === "put") {
          throw new Error("store put exploded");
        }
        if (storeGate !== undefined && (storeGateOn ?? "put") === "put") {
          await storeGate;
        }
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
        // why: a pending gate keeps the store call outstanding, so only the
        // serve deadline can answer.
        if (storeGate !== undefined) {
          await storeGate;
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
  storeGate?: Promise<void>;
  storeRejectOn?: "put" | "get";
  invokeTimeoutMs?: number;
  storeGateOn?: "put" | "get";
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
      scopedStoreFace(
        options.admission,
        options.storeCalls,
        options.storeGate,
        options.storeRejectOn,
        options.storeGateOn,
      ),
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

      // note: core op denial is covered by the raw-wire probe below.
    } finally {
      close();
    }
  });
  it("a decimal SourceSpan is schema-valid: the put succeeds (F-019)", async () => {
    // note: `SourceSpan.start`/`end` are plain numbers on the wire
    // (Rust `f64`), not integers — the put decode must admit decimal
    // span members.
    const { client, close } = await dialKnowledgeOnlyHost({
      admission: () => spokeOk(SCOPE_A),
      clientSeed: seed(0x10),
      clientCapabilities: ["spoke-knowledge"],
    });
    try {
      const spanned = {
        ...entry("kb_span"),
        source_anchor: {
          schema_version: 1,
          source_id: "src_1",
          span: { start: 1.5, end: 4.25 },
          extensions: {},
        },
      } as KnowledgeEntry;
      const put = await client.putKnowledgeEntry(spanned, null);
      expect(put.ok).toBe(true);
      const got = await client.getKnowledgeEntry("kb_span");
      expect(got.ok).toBe(true);
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
    // note: one shared scoped store behind two host instances; each caller is
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

    // why: caller A carries ke-ownership so the viewpoint-matching probe below
    // reaches the scope-equality gate rather than the ownership conjunct.
    const callerA = await dial(seed(0x10), ["spoke-knowledge", "ke-ownership"]);
    const callerB = await dial(seed(0x11));
    try {
      const putA = await callerA.client.putKnowledgeEntry(entry("kb_shared"), null);
      expect(putA.ok).toBe(true);

      const gotByB = await callerB.client.getKnowledgeEntry("kb_shared");
      expect(gotByB.ok).toBe(false);
      const listedByB = await callerB.client.listKnowledgeEntries(SCOPE_B);
      expect(listedByB.ok).toBe(true);
      if (listedByB.ok) {
        expect(listedByB.value).toEqual([]);
      }

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

      const callsBeforeMismatch = storeCalls.length;
      const mismatch = await callerA.client.listKnowledgeEntries(SCOPE_B);
      expect(mismatch.ok).toBe(false);
      if (!mismatch.ok) {
        expect(mismatch.code).toBe("NOT_AUTHORIZED");
      }
      expect(storeCalls.length).toBe(callsBeforeMismatch);

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

      const bump = await client.putKnowledgeEntry(
        { ...entry("kb_stale"), revision: 2, body: { summary: "updated" } },
        1,
      );
      expect(bump.ok).toBe(true);

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
    // why: executor form — `Promise.withResolvers` needs the ES2024 lib. The
    // store call stays outstanding, so only the serve deadline can answer.
    const storeGate = new Promise<void>(() => {
    });
    // why: only `Date` is faked — admission advances the responder's clock past
    // the budget instead of sleeping, so no wall-clock bound is asserted and
    // the outcome does not depend on machine timing.
    vi.useFakeTimers({ toFake: ["Date"] });
    const { client, close } = await dialKnowledgeOnlyHost({
      // why: admission is synchronous producer work, so it spends the budget in
      // place (the Rust path parks it on the blocking pool).
      admission: () => {
        vi.setSystemTime(Date.now() + 500);
        return spokeOk(SCOPE_A);
      },
      clientSeed: seed(0x13),
      clientCapabilities: ["spoke-knowledge"],
      storeCalls,
      storeGate,
      invokeTimeoutMs: 100,
    });
    try {
      const listed = await client.listKnowledgeEntries(SCOPE_A);
      expect(listed.ok).toBe(false);
      if (!listed.ok) {
        expect(listed.code).toBe("INTERNAL_ERROR");
        expect(listed.details?.kind).toBe("timeout");
      }
      // why: the whole budget went to admission, so the store gets none of it
      // and is never called — a second full budget would have called it.
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

  it("answers OUTCOME_UNCERTAIN when the put store call outlives the budget", async () => {
    const storeCalls: string[] = [];
    // why: executor form — `Promise.withResolvers` needs the ES2024 lib. The
    // gate never settles, so the put is invoked (recorded in `storeCalls`) and
    // the serve deadline answers while it may have applied — contract §14.3
    // uncertain, not a clean timeout.
    const storeGate = new Promise<void>(() => {});
    const { client, close } = await dialKnowledgeOnlyHost({
      admission: () => spokeOk(SCOPE_A),
      clientSeed: seed(0x16),
      clientCapabilities: ["spoke-knowledge"],
      storeCalls,
      storeGate,
      invokeTimeoutMs: 60,
    });
    try {
      const put = await client.putKnowledgeEntry(entry("kb_unc"), null);
      expect(put.ok).toBe(false);
      if (!put.ok) {
        expect(put.code).toBe("OUTCOME_UNCERTAIN");
        expect(put.details?.kind).toBe("timeout");
        expect(put.details?.retry_class).toBe("uncertain");
        expect(put.message).toContain("outcome is unknown");
      }
      expect(storeCalls).toEqual(["get", "put"]);
    } finally {
      close();
    }
  });

  it("answers OUTCOME_UNCERTAIN when the put store rejects after invocation", async () => {
    const storeCalls: string[] = [];
    const { client, close } = await dialKnowledgeOnlyHost({
      admission: () => spokeOk(SCOPE_A),
      clientSeed: seed(0x18),
      clientCapabilities: ["spoke-knowledge"],
      storeCalls,
      storeRejectOn: "put",
    });
    try {
      const put = await client.putKnowledgeEntry(entry("kb_rej"), null);
      expect(put.ok).toBe(false);
      if (!put.ok) {
        expect(put.code).toBe("OUTCOME_UNCERTAIN");
        expect(put.details?.kind).toBe("panic");
        expect(put.details?.retry_class).toBe("uncertain");
        expect(put.message).not.toContain("exploded");
        expect(put.message).toContain("outcome is unknown");
      }
      expect(storeCalls).toEqual(["get", "put"]);
    } finally {
      close();
    }
  });

  it("keeps the internal error when the get store rejects after invocation", async () => {
    const storeCalls: string[] = [];
    const { client, close } = await dialKnowledgeOnlyHost({
      admission: () => spokeOk(SCOPE_A),
      clientSeed: seed(0x19),
      clientCapabilities: ["spoke-knowledge"],
      storeCalls,
      storeRejectOn: "get",
    });
    try {
      const got = await client.getKnowledgeEntry("kb_rej_get");
      expect(got.ok).toBe(false);
      if (!got.ok) {
        expect(got.code).toBe("INTERNAL_ERROR");
        expect(got.details?.retry_class).toBeUndefined();
      }
      expect(storeCalls).toEqual(["get"]);
    } finally {
      close();
    }
  });

  it("keeps the plain serve timeout when the get store call outlives the budget", async () => {
    const storeCalls: string[] = [];
    // why: same never-settling gate — a read has no store effect to be
    // uncertain about, so the expiry keeps the plain serve timeout.
    const storeGate = new Promise<void>(() => {});
    const { client, close } = await dialKnowledgeOnlyHost({
      admission: () => spokeOk(SCOPE_A),
      clientSeed: seed(0x17),
      clientCapabilities: ["spoke-knowledge"],
      storeCalls,
      storeGate,
      storeGateOn: "get",
      invokeTimeoutMs: 60,
    });
    try {
      const got = await client.getKnowledgeEntry("kb_plain");
      expect(got.ok).toBe(false);
      if (!got.ok) {
        expect(got.code).toBe("INTERNAL_ERROR");
        expect(got.details?.kind).toBe("timeout");
        expect(got.details?.retry_class).toBeUndefined();
      }
      expect(storeCalls).toEqual(["get"]);
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

      const admittedOnly = await client.listKnowledgeEntries({
        scope_id: "scope_a",
      });
      expect(admittedOnly.ok).toBe(true);
      if (admittedOnly.ok) {
        expect(admittedOnly.value.map((listed) => listed.entry_id)).toEqual([
          "kb_adm1",
        ]);
      }

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

      const disjoint = await client.listKnowledgeEntries({
        scope_id: "scope_a",
        entry_ids: ["kb_adm2"],
      });
      expect(disjoint.ok).toBe(true);
      if (disjoint.ok) {
        expect(disjoint.value).toEqual([]);
      }

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

describe("knowledge dispatch pre-validation and refusals", () => {
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

      // note: a malformed `entry` object (a required KnowledgeEntry member
      // missing) is refused like Rust's typed decode, before admission and the
      // store.
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

      // note: OCC revisions mirror Rust's `Option<u64>` — negative and
      // fractional values are invalid, not merely non-numeric ones.
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

      // note: `entry.schema_version` is a typed `SchemaVersion` (integer >= 1)
      // in Rust; a missing or zero version fails decode.
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

      // note: Rust decodes the revision as `Option<u64>` — an integer beyond
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
      expect(storeCalls).toEqual(["get", "put"]);

      // note: Rust's `Option<u64>` decodes a missing member as None — a put
      // without `expected_base_revision` is a create, and the store sees `null`
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
      expect(storeCalls).toEqual(["get", "put", "get", "put"]);
    } finally {
      close();
    }
  });

  it("a malformed nested entry member answers INVALID_INPUT before the OCC read — zero store calls (QC2-15)", async () => {
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
      // note: `body.tags: 5` fails the full wire-shape decode (Rust typed
      // decode parity) — the refusal is INVALID_INPUT, never a
      // stored-state OCC answer, even on an occupied slot.
      const malformed = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        0,
        "put-malformed-tags",
        "port.knowledge.put",
        {
          entry: { ...entry("kb_mal"), body: { tags: 5 } },
          expected_base_revision: null,
        },
      );
      expect(malformed.error?.code).toBe("INVALID_INPUT");
      expect(storeCalls).toHaveLength(0);

      // An undeclared entry member is the same decode refusal (serde
      // `deny_unknown_fields` parity).
      const extra = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        1,
        "put-extra-member",
        "port.knowledge.put",
        {
          entry: { ...entry("kb_ext"), product_field: 1 },
          expected_base_revision: null,
        },
      );
      expect(extra.error?.code).toBe("INVALID_INPUT");

      // A malformed `adoption_anchor` fails decode before the OCC fence
      // — the anchor gate never sees it.
      const badAnchor = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        2,
        "put-bad-anchor",
        "port.knowledge.put",
        {
          entry: { ...entry("kb_anchor"), adoption_anchor: { bogus: true } },
          expected_base_revision: null,
        },
      );
      expect(badAnchor.error?.code).toBe("INVALID_INPUT");
      expect(storeCalls).toHaveLength(0);
      // An explicit `adoption_anchor: null` is the same INVALID_INPUT —
      // a null anchor is malformed input, never an absent one (F-016:
      // Rust's `Option` decode must not collapse it either).
      const nullAnchor = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        3,
        "put-null-anchor",
        "port.knowledge.put",
        {
          entry: { ...entry("kb_null_anchor"), adoption_anchor: null },
          expected_base_revision: null,
        },
      );
      expect(nullAnchor.error?.code).toBe("INVALID_INPUT");
      expect(storeCalls).toHaveLength(0);

      // A top-level `destination_entry_id: null` is malformed input
      // under the scoped-put precedence rule — refused regardless of
      // stored state (contract D1 / F-016).
      const nullSelector = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        4,
        "put-null-selector",
        "port.knowledge.put",
        {
          entry: entry("kb_null_sel"),
          expected_base_revision: null,
          destination_entry_id: null,
        },
      );
      expect(nullSelector.error?.code).toBe("INVALID_INPUT");
      expect(storeCalls).toHaveLength(0);

      // An anchor at the inclusive `2^53` endpoint is schema-valid
      // provenance (C5 / F-013): the deep validator must pass it through
      // to the OCC/store path, not refuse it upfront.
      const endpointAnchor = await rawInvoke(
        clientEnd,
        seed(0x10),
        sessionId,
        5,
        "put-endpoint-anchor",
        "port.knowledge.put",
        {
          entry: {
            ...entry("kb_endpoint"),
            adoption_anchor: {
              schema_version: 1,
              source_scope_id: "spc_src",
              source_entry_id: "ke_src",
              source_revision: 9007199254740992,
              extensions: {},
            },
          },
          expected_base_revision: null,
        },
      );
      expect(endpointAnchor.error).toBeUndefined();
      expect(storeCalls).toEqual(["get", "put"]);
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
      // note: the list scope decodes as a typed Scope (Rust parity) — a scalar
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
      // note: the dialer maps this dispatch-deny row to CAPABILITY_PORT_MISSING
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
