/**
 * Unit tests for the multi-peer capability router (`multi-peer-router.ts`):
 * registry semantics (§7.4), the locked §3 selection algorithm (match,
 * no-match, namespace / authority hard filters, role soft partition,
 * deterministic UTF-8 `peer_id` tie-break), the §5 no-capable-peer reject,
 * the §7.2 failure policy (selected-peer-down returns the underlying reject
 * as-is — no automatic alternate-retry), and the §6 HostManifest
 * aggregation (composed view + per-peer array in lexicographic order).
 *
 * The router surface is exercised through `@42ch/spoke-connect/remote` (the
 * same exports consumers import). The pure `selectPeerForOp` (§3) is
 * imported from the module src for the filter branches the fixed six-family
 * surface cannot reach with a scope-bearing payload (namespace / authority
 * derivation, and the role preference on non-`port.*` ops).
 */

import { describe, expect, it } from "vitest";

import type {
  Finding,
  HostCapabilityManifest,
  KnowledgeEntry,
  Relation,
  Rule,
  Scope,
  TimelineEvent,
} from "@42ch/spoke-schemas";
import {
  SpokeRejectCode,
  spokeOk,
  type SpokeReject,
  type SpokeResult,
} from "@42ch/spoke-operations";
import {
  connectMultiPeerRouter,
  type RemoteAdapterState,
  type RoutedRemoteAdapter,
} from "@42ch/spoke-connect/remote";

import {
  selectPeerForOp,
  type SelectablePeer,
} from "../../src/remote/multi-peer-router.js";

/** Delegate-call dummy values — routing tests never inspect the payloads. */
const FAKE_ENTRY = { entry_id: "e1" } as KnowledgeEntry;
const FAKE_RELATION = { relation_id: "r1" } as Relation;

/** Schema-shaped manifest builder; defaults to a baseline data-store peer. */
function manifest(
  overrides: Partial<HostCapabilityManifest> & { host_id: string },
): HostCapabilityManifest {
  return {
    schema_version: 1,
    roles: ["data-store"],
    capabilities: ["spoke-baseline"],
    namespaces: ["toy_world"],
    extensions: {},
    ...overrides,
  };
}

/**
 * Test double for the per-peer adapter surface (`RoutedRemoteAdapter`): a
 * plain-property `state` / `remotePeerId` / `remoteManifest` plus the ten
 * async `BaselinePorts` methods that record the delegated method name and
 * return `downReject` when a mid-op failure is being simulated (§7.2).
 */
class FakePeer implements RoutedRemoteAdapter {
  readonly remotePeerId: string;
  readonly remoteManifest: HostCapabilityManifest;
  state: RemoteAdapterState;
  /** Delegated BaselinePorts method names, in call order. */
  readonly calls: string[] = [];
  /** When set, every delegate method returns this reject (peer-down sim). */
  downReject: SpokeReject | null = null;

  constructor(
    peerId: string,
    manifestValue: HostCapabilityManifest,
    state: RemoteAdapterState = "Established",
  ) {
    this.remotePeerId = peerId;
    this.remoteManifest = manifestValue;
    this.state = state;
  }

  #recordAndFail<T>(method: string): SpokeResult<T> | undefined {
    this.calls.push(method);
    return this.downReject === null ? undefined : this.downReject;
  }

  async getKnowledgeEntry(_entryId: string): Promise<SpokeResult<KnowledgeEntry>> {
    return this.#recordAndFail("getKnowledgeEntry") ?? spokeOk(FAKE_ENTRY);
  }

  async putKnowledgeEntry(
    _entry: KnowledgeEntry,
    _expectedBaseRevision: number | null,
  ): Promise<SpokeResult<KnowledgeEntry>> {
    return this.#recordAndFail("putKnowledgeEntry") ?? spokeOk(FAKE_ENTRY);
  }

  async getRelation(_relationId: string): Promise<SpokeResult<Relation>> {
    return this.#recordAndFail("getRelation") ?? spokeOk(FAKE_RELATION);
  }

  async putRelation(
    _relation: Relation,
    _expectedBaseRevision: number | null,
  ): Promise<SpokeResult<Relation>> {
    return this.#recordAndFail("putRelation") ?? spokeOk(FAKE_RELATION);
  }

  async listKnowledgeEntries(
    _scope: Scope,
  ): Promise<SpokeResult<KnowledgeEntry[]>> {
    return this.#recordAndFail("listKnowledgeEntries") ?? spokeOk([FAKE_ENTRY]);
  }

  async listTimelineEvents(_scope: Scope): Promise<SpokeResult<TimelineEvent[]>> {
    return this.#recordAndFail("listTimelineEvents") ?? spokeOk([]);
  }

  async putFindings(_findings: Finding[]): Promise<SpokeResult<Finding[]>> {
    return this.#recordAndFail("putFindings") ?? spokeOk([]);
  }

  async listRules(_ruleRefs: string[]): Promise<SpokeResult<Rule[]>> {
    return this.#recordAndFail("listRules") ?? spokeOk([]);
  }

  async getHostCapabilityManifest(): Promise<SpokeResult<HostCapabilityManifest>> {
    return this.#recordAndFail("getHostCapabilityManifest") ??
      spokeOk(this.remoteManifest);
  }

  async listPeerHostCapabilityManifests(): Promise<
    SpokeResult<HostCapabilityManifest[]>
  > {
    return this.#recordAndFail("listPeerHostCapabilityManifests") ?? spokeOk([]);
  }
}

describe("MultiPeerRouter registry (§7.4)", () => {
  it("registerPeer stores an Established adapter, returns its peer id, and is idempotent", () => {
    const router = connectMultiPeerRouter();
    const peer = new FakePeer("peer-a", manifest({ host_id: "host-a" }));

    expect(router.registerPeer(peer)).toBe("peer-a");
    expect(router.listPeers()).toEqual(["peer-a"]);
    // Idempotent on peer_id: re-registering the same adapter returns the same id.
    expect(router.registerPeer(peer)).toBe("peer-a");
    expect(router.listPeers()).toEqual(["peer-a"]);
    // A second adapter with the same peer_id replaces the stored one.
    const replacement = new FakePeer("peer-a", manifest({ host_id: "host-a" }));
    expect(router.registerPeer(replacement)).toBe("peer-a");
    expect(router.listPeers()).toEqual(["peer-a"]);
  });

  it("registerPeer throws when the adapter has no established session", () => {
    const router = connectMultiPeerRouter();
    const handshaking = new FakePeer(
      "",
      manifest({ host_id: "host-a" }),
      "Handshaking",
    );
    expect(() => router.registerPeer(handshaking)).toThrow(
      /established connect session/,
    );
  });

  it("unregisterPeer is a no-op for unknown peer ids and removes registered ones", () => {
    const router = connectMultiPeerRouter();
    router.registerPeer(new FakePeer("peer-a", manifest({ host_id: "host-a" })));

    expect(() => router.unregisterPeer("never-registered")).not.toThrow();
    expect(router.listPeers()).toEqual(["peer-a"]);

    router.unregisterPeer("peer-a");
    expect(router.listPeers()).toEqual([]);
  });
});

describe("MultiPeerRouter concurrent interleaving (W-001)", () => {
  it("survives interleaved register/unregister with in-flight port calls — all calls complete, registry stays consistent", async () => {
    // W-001: registry mutations interleaved with in-flight port calls. In
    // the single-threaded TS event loop the interleaving happens at await
    // boundaries: wave-1 delegates are pending while registration churn
    // runs, then wave-2 calls select against the mutated registry. Proves
    // no corruption (registry ends exactly consistent) and that every call
    // completes — a rejected promise would fail the `Promise.all`.
    const router = connectMultiPeerRouter();
    const registered = new Set<string>();
    const seed = new FakePeer("peer-seed", manifest({ host_id: "h-seed" }));
    router.registerPeer(seed);
    registered.add("peer-seed");

    const inflight: Promise<SpokeResult<KnowledgeEntry[]>>[] = [];
    // Wave 1: fire port calls (selection runs synchronously, delegates await).
    for (let i = 0; i < 16; i++) {
      inflight.push(router.listKnowledgeEntries({ scope_id: "s1" }));
    }
    // Registry churn while wave 1 awaits delegates.
    const extra = new FakePeer("peer-late", manifest({ host_id: "h-late" }));
    router.registerPeer(extra);
    registered.add("peer-late");
    router.unregisterPeer("peer-seed");
    registered.delete("peer-seed");
    // Wave 2: more port calls against the mutated registry.
    for (let i = 0; i < 16; i++) {
      inflight.push(router.listKnowledgeEntries({ scope_id: "s1" }));
    }

    const results = await Promise.all(inflight);
    expect(results).toHaveLength(32);
    // Wave-1 calls selected peer-seed before the unregister; wave-2 calls
    // selected peer-late after the churn — every delegate succeeds.
    for (const result of results) {
      expect(result.ok).toBe(true);
    }
    // Registry stayed consistent: exactly the registered ids remain.
    expect([...router.listPeers()].sort()).toEqual([...registered].sort());
  });
});

describe("MultiPeerRouter selection (§3)", () => {
  it("selects the single peer with the required capability", async () => {
    const router = connectMultiPeerRouter();
    const baseline = new FakePeer(
      "peer-baseline",
      manifest({ host_id: "h-baseline" }),
    );
    const computable = new FakePeer(
      "peer-computable",
      manifest({ host_id: "h-computable", capabilities: ["l2-computable"] }),
    );
    router.registerPeer(computable);
    router.registerPeer(baseline);

    const result = await router.listKnowledgeEntries({ scope_id: "s1" });

    expect(result.ok).toBe(true);
    expect(baseline.calls).toEqual(["listKnowledgeEntries"]);
    expect(computable.calls).toEqual([]);
  });

  it("rejects with the locked no_capable_peer reject when no peer has the required capability (§5)", async () => {
    const router = connectMultiPeerRouter();
    const computable = new FakePeer(
      "peer-computable",
      manifest({ host_id: "h-computable", capabilities: ["l2-computable"] }),
    );
    router.registerPeer(computable);

    const result = await router.listKnowledgeEntries({ scope_id: "s1" });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
      expect(result.details?.wire_code).toBe("no_capable_peer");
      expect(result.details?.kind).toBe("no_capable_peer");
    }
    // Terminal: no peer delegate ran (no wrong-peer fallback).
    expect(computable.calls).toEqual([]);
  });

  it("breaks ties deterministically on the lowest peer_id (lexicographic UTF-8 byte order, §4)", async () => {
    const router = connectMultiPeerRouter();
    const peerB = new FakePeer("peer-bbb", manifest({ host_id: "h-b" }));
    const peerA = new FakePeer("peer-aaa", manifest({ host_id: "h-a" }));
    router.registerPeer(peerB);
    router.registerPeer(peerA);

    const result = await router.getKnowledgeEntry("e1");

    expect(result.ok).toBe(true);
    expect(peerA.calls).toEqual(["getKnowledgeEntry"]);
    expect(peerB.calls).toEqual([]);
  });

  it("excludes Closed / Handshaking peers from the candidate set (§7.4)", async () => {
    const router = connectMultiPeerRouter();
    const closed = new FakePeer(
      "peer-closed",
      manifest({ host_id: "h-closed" }),
      "Closed",
    );
    const handshaking = new FakePeer(
      "peer-handshaking",
      manifest({ host_id: "h-handshaking" }),
      "Handshaking",
    );
    const established = new FakePeer(
      "peer-established",
      manifest({ host_id: "h-established" }),
    );
    router.registerPeer(closed);
    router.registerPeer(handshaking);
    router.registerPeer(established);

    const result = await router.listKnowledgeEntries({ scope_id: "s1" });

    expect(result.ok).toBe(true);
    expect(established.calls).toEqual(["listKnowledgeEntries"]);
    expect(closed.calls).toEqual([]);
    expect(handshaking.calls).toEqual([]);
  });

  it("derives the request namespace from the Scope carried by scope-query port calls (§2)", async () => {
    // The wire `Scope` has no namespace field today; the derivation seam is
    // the opaque op payload (contract §2: "derived from Scope when the op
    // payload carries one"). The cast exercises that documented path.
    const router = connectMultiPeerRouter();
    const peerAlpha = new FakePeer(
      "peer-alpha",
      manifest({ host_id: "h-alpha", namespaces: ["alpha"] }),
    );
    const peerBeta = new FakePeer(
      "peer-beta",
      manifest({ host_id: "h-beta", namespaces: ["beta"] }),
    );
    router.registerPeer(peerAlpha);
    router.registerPeer(peerBeta);

    const result = await router.listKnowledgeEntries({
      scope_id: "s1",
      namespace: "beta",
    } as Scope);

    expect(result.ok).toBe(true);
    expect(peerBeta.calls).toEqual(["listKnowledgeEntries"]);
    expect(peerAlpha.calls).toEqual([]);
  });
});

describe("MultiPeerRouter dynamic peer-down (§7.4)", () => {
  it("reroutes to the surviving peer when the selected peer leaves Established between calls — no proactive eviction", async () => {
    // W-002: two Established peers; the first call routes to the tie-break
    // winner (peer-a); the winner's session drops to Closed WITHOUT
    // unregister; the next call excludes it from the candidate set and
    // routes to peer-b — reactive exclusion, no proactive eviction (the
    // registry still lists peer-a).
    const router = connectMultiPeerRouter();
    const peerA = new FakePeer("peer-a", manifest({ host_id: "h-a" }));
    const peerB = new FakePeer("peer-b", manifest({ host_id: "h-b" }));
    router.registerPeer(peerB);
    router.registerPeer(peerA);

    const first = await router.listKnowledgeEntries({ scope_id: "s1" });
    expect(first.ok).toBe(true);
    expect(peerA.calls).toEqual(["listKnowledgeEntries"]);
    expect(peerB.calls).toEqual([]);

    // The selected peer's session closes mid-session (no unregister).
    peerA.state = "Closed";
    expect(router.listPeers()).toEqual(["peer-b", "peer-a"]); // still registered

    const second = await router.listKnowledgeEntries({ scope_id: "s1" });
    expect(second.ok).toBe(true);
    expect(peerB.calls).toEqual(["listKnowledgeEntries"]);
    expect(peerA.calls).toEqual(["listKnowledgeEntries"]);
  });

  it("rejects with no_capable_peer when every Established peer leaves Established between calls", async () => {
    // W-002 terminal: when ALL Established peers drop out between calls, the
    // next selection rejects with the locked §5 reject — no delegate runs,
    // no wrong-peer fallback.
    const router = connectMultiPeerRouter();
    const peerA = new FakePeer("peer-a", manifest({ host_id: "h-a" }));
    const peerB = new FakePeer("peer-b", manifest({ host_id: "h-b" }));
    router.registerPeer(peerB);
    router.registerPeer(peerA);

    // First call succeeds on the tie-break winner.
    const first = await router.listKnowledgeEntries({ scope_id: "s1" });
    expect(first.ok).toBe(true);
    expect(peerA.calls).toEqual(["listKnowledgeEntries"]);
    expect(peerB.calls).toEqual([]);

    peerA.state = "Closed";
    peerB.state = "Closed";

    const second = await router.listKnowledgeEntries({ scope_id: "s1" });
    expect(second.ok).toBe(false);
    if (!second.ok) {
      expect(second.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
      expect(second.details?.kind).toBe("no_capable_peer");
    }
    // No delegate ran on the second call.
    expect(peerA.calls).toEqual(["listKnowledgeEntries"]);
    expect(peerB.calls).toEqual([]);
  });
});

describe("selectPeerForOp namespace filter (§2/§3)", () => {
  const peerAlpha: SelectablePeer = {
    peerId: "peer-alpha",
    manifest: manifest({ host_id: "h-alpha", namespaces: ["alpha"] }),
  };
  const peerBeta: SelectablePeer = {
    peerId: "peer-beta",
    manifest: manifest({ host_id: "h-beta", namespaces: ["beta"] }),
  };

  it("excludes peers that do not advertise the request namespace (exact match)", () => {
    const selection = selectPeerForOp(
      [peerAlpha, peerBeta],
      "port.scope.list_knowledge_entries",
      { scope: { scope_id: "s1", namespace: "beta" } },
    );
    expect(selection.ok).toBe(true);
    if (selection.ok) {
      expect(selection.value.peerId).toBe("peer-beta");
    }
  });

  it("skips the namespace filter when the request carries no namespace", () => {
    const selection = selectPeerForOp(
      [peerAlpha, peerBeta],
      "port.scope.list_knowledge_entries",
      { scope: { scope_id: "s1" } },
    );
    expect(selection.ok).toBe(true);
    if (selection.ok) {
      expect(selection.value.peerId).toBe("peer-alpha"); // tie-break
    }
  });

  it("treats a literal '*' namespace as the literal string — no wildcard (§2)", () => {
    const wildcardPeer: SelectablePeer = {
      peerId: "peer-star",
      manifest: manifest({ host_id: "h-star", namespaces: ["*"] }),
    };
    const selection = selectPeerForOp(
      [wildcardPeer],
      "port.scope.list_knowledge_entries",
      { scope: { scope_id: "s1", namespace: "alpha" } },
    );
    expect(selection.ok).toBe(false);
    if (!selection.ok) {
      expect(selection.details?.kind).toBe("no_capable_peer");
    }
  });
});

describe("selectPeerForOp unknown-op rejection (QC2 S-1)", () => {
  it("rejects ops outside the locked capability table instead of skipping the gate (§3 step 1)", () => {
    const selection = selectPeerForOp(
      [{ peerId: "peer-a", manifest: manifest({ host_id: "h-a" }) }],
      "product.op.unknown",
      {},
    );
    expect(selection.ok).toBe(false);
    if (!selection.ok) {
      expect(selection.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
      expect(selection.details?.kind).toBe("no_capable_peer");
      expect(selection.details?.op).toBe("product.op.unknown");
    }
  });
});

describe("selectPeerForOp role preference (§3 step 5)", () => {
  const plain: SelectablePeer = {
    peerId: "peer-a",
    manifest: manifest({ host_id: "h-a" }),
  };
  const checker: SelectablePeer = {
    peerId: "peer-z",
    manifest: manifest({ host_id: "h-z", roles: ["data-store", "checker"] }),
  };

  it("prefers the peer whose roles include the op's preferred role when both are capable", () => {
    const selection = selectPeerForOp([plain, checker], "check", {});
    expect(selection.ok).toBe(true);
    if (selection.ok) {
      expect(selection.value.peerId).toBe("peer-z"); // role partition beats tie-break
    }
  });

  it("falls back to the role-unmatched partition when no candidate has the preferred role (soft)", () => {
    const selection = selectPeerForOp([plain, checker], "assemble", {});
    // Neither peer has "assembler" → tie-break decides.
    expect(selection.ok).toBe(true);
    if (selection.ok) {
      expect(selection.value.peerId).toBe("peer-a");
    }
  });

  it("applies no role preference to port.* baseline ops", () => {
    const selection = selectPeerForOp([plain, checker], "port.knowledge.put", {});
    expect(selection.ok).toBe(true);
    if (selection.ok) {
      expect(selection.value.peerId).toBe("peer-a"); // tie-break wins; checker role ignored
    }
  });
});

describe("selectPeerForOp authority filter (§3 step 4)", () => {
  const matching: SelectablePeer = {
    peerId: "peer-match",
    manifest: manifest({
      host_id: "h-match",
      authority: { scope_key: "scope-K" },
    }),
  };
  const mismatched: SelectablePeer = {
    peerId: "peer-mismatch",
    manifest: manifest({
      host_id: "h-mismatch",
      authority: { scope_key: "scope-Z" },
    }),
  };

  it("excludes peers whose declared authority scope key mismatches the request", () => {
    const selection = selectPeerForOp(
      [mismatched, matching],
      "port.scope.list_knowledge_entries",
      { scope_key: "scope-K" },
    );
    expect(selection.ok).toBe(true);
    if (selection.ok) {
      expect(selection.value.peerId).toBe("peer-match");
    }
  });

  it("skips the authority filter when only one side declares a scope key", () => {
    const undeclared: SelectablePeer = {
      peerId: "peer-a",
      manifest: manifest({ host_id: "h-a" }),
    };
    const selection = selectPeerForOp(
      [mismatched, undeclared],
      "port.scope.list_knowledge_entries",
      { scope_key: "scope-K" },
    );
    // peer-a declares nothing → filter skipped for it; peer-mismatch excluded.
    expect(selection.ok).toBe(true);
    if (selection.ok) {
      expect(selection.value.peerId).toBe("peer-a");
    }
  });

  it("skips the authority filter when the request carries no scope key", () => {
    const selection = selectPeerForOp(
      [mismatched, matching],
      "port.scope.list_knowledge_entries",
      { scope: { scope_id: "s1" } },
    );
    expect(selection.ok).toBe(true);
    if (selection.ok) {
      expect(selection.value.peerId).toBe("peer-match"); // tie-break
    }
  });
});

describe("selectPeerForOp tie-break byte order (§4)", () => {
  it("orders peer_ids by UTF-8 byte order, not UTF-16 code-unit order", () => {
    // U+E000 encodes as UTF-8 EE 80 80; U+10000 encodes as F0 90 80 80.
    // UTF-16 code units: 0xE000 > 0xD800 (surrogate), so code-unit order
    // would pick U+10000 first; UTF-8 byte order picks U+E000 first.
    const bmp: SelectablePeer = {
      peerId: "\uE000",
      manifest: manifest({ host_id: "h-bmp" }),
    };
    const astral: SelectablePeer = {
      peerId: "\u{10000}",
      manifest: manifest({ host_id: "h-astral" }),
    };
    const selection = selectPeerForOp([astral, bmp], "port.knowledge.get", {});
    expect(selection.ok).toBe(true);
    if (selection.ok) {
      expect(selection.value.peerId).toBe("\uE000");
    }
  });
});

describe("failure policy (§7.2)", () => {
  it("returns the selected peer's underlying reject as-is — no automatic alternate-retry", async () => {
    const router = connectMultiPeerRouter();
    const peerA = new FakePeer("peer-a", manifest({ host_id: "h-a" }));
    const peerB = new FakePeer("peer-b", manifest({ host_id: "h-b" }));
    router.registerPeer(peerB);
    router.registerPeer(peerA);
    // peer-a is selected (tie-break) and its session dies mid-op.
    peerA.downReject = {
      ok: false,
      code: SpokeRejectCode.INTERNAL_ERROR,
      message: "transport loss",
      details: { kind: "transport" },
    };

    const result = await router.listKnowledgeEntries({ scope_id: "s1" });

    expect(result).toEqual(peerA.downReject);
    // No retry on the alternate capable peer.
    expect(peerB.calls).toEqual([]);
  });

  it("does not remap an envelope-auth failure kind to no_capable_peer (handoff §2)", async () => {
    const router = connectMultiPeerRouter();
    const peerA = new FakePeer("peer-a", manifest({ host_id: "h-a" }));
    router.registerPeer(peerA);
    peerA.downReject = {
      ok: false,
      code: SpokeRejectCode.INTERNAL_ERROR,
      message: "unauthenticated envelope",
      details: { kind: "envelope_auth_missing" },
    };

    const result = await router.getKnowledgeEntry("e1");

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INTERNAL_ERROR);
      expect(result.details?.kind).toBe("envelope_auth_missing");
      expect(result.details?.wire_code).toBeUndefined();
    }
  });
});

describe("HostManifest aggregation (§6)", () => {
  it("composes the union of connected peers' capabilities/roles/namespaces with the router's own host_id", async () => {
    const router = connectMultiPeerRouter({ hostId: "router-own" });
    router.registerPeer(
      new FakePeer(
        "peer-a",
        manifest({
          host_id: "h-a",
          capabilities: ["spoke-baseline"],
          roles: ["data-store"],
          namespaces: ["alpha"],
        }),
      ),
    );
    router.registerPeer(
      new FakePeer(
        "peer-b",
        manifest({
          host_id: "h-b",
          capabilities: ["spoke-baseline", "l2-computable"],
          roles: ["data-store", "checker"],
          namespaces: ["alpha", "beta"],
        }),
      ),
    );

    const result = await router.getHostCapabilityManifest();

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.host_id).toBe("router-own");
      expect([...result.value.capabilities].sort()).toEqual([
        "l2-computable",
        "spoke-baseline",
      ]);
      expect([...result.value.roles].sort()).toEqual(["checker", "data-store"]);
      expect([...result.value.namespaces].sort()).toEqual(["alpha", "beta"]);
      // §6: authority.scope_key omitted; extensions.router.peers sorted.
      expect(result.value.authority).toBeUndefined();
      expect(result.value.extensions.router).toEqual({
        peers: ["peer-a", "peer-b"],
      });
    }
  });

  it("treats an empty hostId as unset, defaulting to multi-peer-router (Rust parity)", async () => {
    const router = connectMultiPeerRouter({ hostId: "" });

    const result = await router.getHostCapabilityManifest();

    expect(result.ok).toBe(true);
    if (result.ok) {
      // §8 constructor options: empty string defaults like null/undefined.
      expect(result.value.host_id).toBe("multi-peer-router");
    }
  });

  it("lists per-peer manifests sorted by peer_id (lexicographic UTF-8 byte order)", async () => {
    const router = connectMultiPeerRouter();
    router.registerPeer(
      new FakePeer("peer-b", manifest({ host_id: "h-b", namespaces: ["beta"] })),
    );
    router.registerPeer(
      new FakePeer("peer-a", manifest({ host_id: "h-a", namespaces: ["alpha"] })),
    );

    const result = await router.listPeerHostCapabilityManifests();

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.map((entry) => entry.host_id)).toEqual(["h-a", "h-b"]);
    }
  });

  it("returns [] and empty unions for a router with zero registered peers", async () => {
    const router = connectMultiPeerRouter({ hostId: "router-alone" });

    const peersResult = await router.listPeerHostCapabilityManifests();
    expect(peersResult.ok).toBe(true);
    if (peersResult.ok) {
      expect(peersResult.value).toEqual([]);
    }

    const composedResult = await router.getHostCapabilityManifest();
    expect(composedResult.ok).toBe(true);
    if (composedResult.ok) {
      expect(composedResult.value.host_id).toBe("router-alone");
      expect(composedResult.value.capabilities).toEqual([]);
      expect(composedResult.value.roles).toEqual([]);
      expect(composedResult.value.namespaces).toEqual([]);
      expect(composedResult.value.extensions.router).toEqual({ peers: [] });
    }
  });

  it("excludes non-Established registered peers from the composed view", async () => {
    const router = connectMultiPeerRouter();
    router.registerPeer(
      new FakePeer(
        "peer-closed",
        manifest({ host_id: "h-closed" }),
        "Closed",
      ),
    );
    router.registerPeer(
      new FakePeer(
        "peer-live",
        manifest({
          host_id: "h-live",
          capabilities: ["spoke-baseline", "l2-computable"],
        }),
      ),
    );

    const result = await router.listPeerHostCapabilityManifests();

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.map((entry) => entry.host_id)).toEqual(["h-live"]);
    }
  });
});
