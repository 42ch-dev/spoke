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
  ToolDescriptor,
} from "@42ch/spoke-schemas";
import {
  SpokeRejectCode,
  spokeOk,
  validateManifestTools,
  type SpokeReject,
  type SpokeResult,
} from "@42ch/spoke-operations";
import {
  connectMultiPeerRouter,
  connectRemoteAdapter,
  loopbackTransportPair,
  type RemoteAdapter,
  type RemoteAdapterState,
  type RoutedRemoteAdapter,
} from "@42ch/spoke-connect/remote";

import { getPublicKeyEd25519 } from "../../src/crypto.js";
import { schemaConformantManifest } from "../../src/golden.js";
import { derivePeerIdFromEd25519Pubkey } from "../../src/identity.js";
import {
  startMinimalResponder,
  type MinimalResponder,
  type TestToolHandler,
} from "./minimal-responder.js";
import {
  selectPeerForOp,
  type SelectablePeer,
} from "../../src/remote/multi-peer-router.js";

/** Delegate-call dummy values — routing tests never inspect the payloads. */
const FAKE_ENTRY = { entry_id: "e1" } as KnowledgeEntry;
const FAKE_RELATION = { relation_id: "r1" } as Relation;

// ── Tool fixtures (frozen §2: op === capability_id, namespaces owned) ─────

const ADD_DESCRIPTOR: ToolDescriptor = {
  schema_version: 1,
  capability_id: "tools.math.add",
  op: "tools.math.add",
  description: "Add two numbers",
  input: {},
  output: {},
};

const ECHO_DESCRIPTOR: ToolDescriptor = {
  schema_version: 1,
  capability_id: "tools.echo.echo",
  op: "tools.echo.echo",
  description: "Echo the arguments back",
  input: {},
  output: {},
};

const BOOM_DESCRIPTOR: ToolDescriptor = {
  schema_version: 1,
  capability_id: "tools.echo.boom",
  op: "tools.echo.boom",
  description: "Handler that throws",
  input: {},
  output: {},
};

/** Descriptor lookup by capability id (the fixture set above). */
const DESCRIPTOR_BY_ID: Readonly<Record<string, ToolDescriptor>> = {
  "tools.math.add": ADD_DESCRIPTOR,
  "tools.echo.echo": ECHO_DESCRIPTOR,
  "tools.echo.boom": BOOM_DESCRIPTOR,
};

/**
 * Tool-carrying manifest: namespaces own the tool namespaces; every tool
 * capability ∈ capabilities[] (the router's hard filter keys on
 * `capabilities`, the composed view unions `tools`).
 */
function toolManifest(hostId: string, toolIds: readonly string[]): HostCapabilityManifest {
  const descriptors = toolIds.map((capabilityId) => DESCRIPTOR_BY_ID[capabilityId]);
  const namespaces = new Set<string>(["toy_world"]);
  for (const capabilityId of toolIds) {
    namespaces.add(capabilityId.split(".")[1]);
  }
  return {
    ...schemaConformantManifest(),
    host_id: hostId,
    namespaces: [...namespaces] as [string, ...string[]],
    capabilities: ["spoke-baseline", ...toolIds] as [string, ...string[]],
    tools: descriptors,
  };
}

/**
 * Loopback pair for the dual-peer proof: a locally-dialed `RemoteAdapter`
 * (initiator) against a tool-serving minimal responder double (the peer).
 * The RESPONDER's manifest (the peer's cached hello manifest) carries only
 * `tools` — the router's hard filter sees exactly what each responder
 * serves. The client (dialer) advertises the full fixture tool set so the
 * negotiated intersection includes every tool under test.
 */
async function dialToolPeer(options: {
  /** Responder Ed25519 seed base (distinct per peer → distinct peer ids). */
  responderSeedBase: number;
  /** Tools the responder advertises + serves. */
  tools: string[];
}): Promise<{ client: RemoteAdapter; responder: MinimalResponder; peerId: string }> {
  const seedResponder = seed(options.responderSeedBase);
  const seedClient = seed(0x10);
  const pubkeyResponder = getPublicKeyEd25519(seedResponder);
  const pubkeyClient = getPublicKeyEd25519(seedClient);
  const peerIdResponder = derivePeerIdFromEd25519Pubkey(pubkeyResponder);
  const peerIdClient = derivePeerIdFromEd25519Pubkey(pubkeyClient);

  const responderManifest = toolManifest("test-responder", options.tools);
  const clientManifest = toolManifest("test-client", [
    "tools.math.add",
    "tools.echo.echo",
    "tools.echo.boom",
  ]);
  // Fixture hygiene: both manifests satisfy the manifest-tools rules.
  for (const manifest of [responderManifest, clientManifest]) {
    const validated = validateManifestTools(manifest);
    expect(validated.ok, `manifest tools must validate: ${manifest.host_id}`).toBe(true);
  }

  const pair = loopbackTransportPair();
  const responder = await startMinimalResponder({
    transport: pair.server,
    seed: seedResponder,
    clientPubkey: pubkeyClient,
    allowlist: [peerIdClient],
    manifest: responderManifest,
  });
  const client = await connectRemoteAdapter({
    transport: pair.client,
    localIdentity: { seed: seedClient },
    localManifest: clientManifest,
    remotePubkey: pubkeyResponder,
    allowlist: [peerIdResponder],
  });
  return { client, responder, peerId: peerIdResponder };
}

/** Fixture seed: base+i, all values within byte range for base ≤ 0xe0. */
function seed(base: number): Uint8Array {
  return Uint8Array.from({ length: 32 }, (_, i) => base + i);
}

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

  /** Forward tool-invoke face (§6) — records the delegation, returns the peer id. */
  async invokeTool(
    _capabilityId: string,
    _args: Record<string, unknown>,
  ): Promise<SpokeResult<unknown>> {
    return this.#recordAndFail("invokeTool") ?? spokeOk({ served_by: this.remotePeerId });
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
    const router = connectMultiPeerRouter({ hostId: "router-only-live" });
    router.registerPeer(
      new FakePeer(
        "peer-closed",
        manifest({
          host_id: "h-closed",
          capabilities: ["archive-scan"],
          roles: ["ghost"],
          namespaces: ["zeta"],
        }),
        "Closed",
      ),
    );
    router.registerPeer(
      new FakePeer(
        "peer-live",
        manifest({
          host_id: "h-live",
          capabilities: ["spoke-baseline", "l2-computable"],
          roles: ["data-store", "checker"],
          namespaces: ["alpha", "beta"],
        }),
      ),
    );

    // Per-peer array: only the Established peer's cached manifest.
    const perPeer = await router.listPeerHostCapabilityManifests();
    expect(perPeer.ok).toBe(true);
    if (perPeer.ok) {
      expect(perPeer.value.map((entry) => entry.host_id)).toEqual(["h-live"]);
    }

    // Composed view (§6 "connected peers"): the Closed peer's unique unions
    // are absent — only the Established peer contributes, and
    // extensions.router.peers lists only it.
    const composed = await router.getHostCapabilityManifest();
    expect(composed.ok).toBe(true);
    if (composed.ok) {
      expect(composed.value.host_id).toBe("router-only-live");
      expect([...composed.value.capabilities].sort()).toEqual([
        "l2-computable",
        "spoke-baseline",
      ]);
      expect([...composed.value.roles].sort()).toEqual([
        "checker",
        "data-store",
      ]);
      expect([...composed.value.namespaces].sort()).toEqual(["alpha", "beta"]);
      expect(composed.value.extensions.router).toEqual({ peers: ["peer-live"] });
    }
  });

  it("unions tools[] across connected peers (dedup by capability_id, lexicographic order)", async () => {
    const router = connectMultiPeerRouter();
    router.registerPeer(
      new FakePeer(
        "peer-a",
        manifest({
          host_id: "h-a",
          capabilities: [
            "spoke-baseline",
            "tools.math.add",
            "tools.echo.echo",
          ],
          tools: [ADD_DESCRIPTOR, ECHO_DESCRIPTOR],
        }),
      ),
    );
    router.registerPeer(
      new FakePeer(
        "peer-b",
        manifest({
          host_id: "h-b",
          capabilities: [
            "spoke-baseline",
            "tools.echo.echo",
            "tools.echo.boom",
          ],
          tools: [ECHO_DESCRIPTOR, BOOM_DESCRIPTOR],
        }),
      ),
    );

    const result = await router.getHostCapabilityManifest();

    expect(result.ok).toBe(true);
    if (result.ok) {
      // tools.echo.echo is shared → deduped; the union is ordered by
      // capability_id lexicographically (§6 stability, not first-seen).
      expect(result.value.tools?.map((descriptor) => descriptor.capability_id)).toEqual([
        "tools.echo.boom",
        "tools.echo.echo",
        "tools.math.add",
      ]);
    }
  });
});

describe("MultiPeerRouter tool routing (§6 frozen contract)", () => {
  it("routes invokeTool to the peer whose cached manifest capabilities include the exact tool capability (disjoint tools)", async () => {
    const router = connectMultiPeerRouter();
    const addPeer = new FakePeer(
      "peer-add",
      manifest({
        host_id: "h-add",
        capabilities: ["spoke-baseline", "tools.math.add"],
      }),
    );
    const echoPeer = new FakePeer(
      "peer-echo",
      manifest({
        host_id: "h-echo",
        capabilities: ["spoke-baseline", "tools.echo.echo"],
      }),
    );
    router.registerPeer(echoPeer);
    router.registerPeer(addPeer);

    const add = await router.invokeTool("tools.math.add", { a: 1, b: 2 });
    expect(add.ok).toBe(true);
    // The selected peer's adapter invokeTool ran — the router delegates,
    // never crafts envelopes itself (§6).
    expect(addPeer.calls).toEqual(["invokeTool"]);
    expect(echoPeer.calls).toEqual([]);

    const echo = await router.invokeTool("tools.echo.echo", { v: 1 });
    expect(echo.ok).toBe(true);
    expect(echoPeer.calls).toEqual(["invokeTool"]);
    expect(addPeer.calls).toEqual(["invokeTool"]);
  });

  it("rejects with the locked no_capable_peer reject when no peer's capabilities include the tool (§5, details.op = capability_id)", async () => {
    const router = connectMultiPeerRouter();
    const addPeer = new FakePeer(
      "peer-add",
      manifest({
        host_id: "h-add",
        capabilities: ["spoke-baseline", "tools.math.add"],
      }),
    );
    router.registerPeer(addPeer);

    const result = await router.invokeTool("tools.echo.boom", {});

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
      expect(result.details?.wire_code).toBe("no_capable_peer");
      expect(result.details?.kind).toBe("no_capable_peer");
      expect(result.details?.op).toBe("tools.echo.boom");
    }
    // Terminal: no delegate ran (no wrong-peer fallback).
    expect(addPeer.calls).toEqual([]);
  });

  it("breaks ties on the lowest peer_id when both peers offer the tool (§4)", async () => {
    const router = connectMultiPeerRouter();
    const peerB = new FakePeer(
      "peer-bbb",
      manifest({
        host_id: "h-b",
        capabilities: ["spoke-baseline", "tools.math.add"],
      }),
    );
    const peerA = new FakePeer(
      "peer-aaa",
      manifest({
        host_id: "h-a",
        capabilities: ["spoke-baseline", "tools.math.add"],
      }),
    );
    router.registerPeer(peerB);
    router.registerPeer(peerA);

    const result = await router.invokeTool("tools.math.add", { a: 1, b: 2 });

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value).toEqual({ served_by: "peer-aaa" });
    }
    expect(peerA.calls).toEqual(["invokeTool"]);
    expect(peerB.calls).toEqual([]);
  });

  it("fails fast with INVALID_INPUT on a non-tools. capability id before any peer selection (§6 grammar gate)", async () => {
    const router = connectMultiPeerRouter();
    const addPeer = new FakePeer(
      "peer-add",
      manifest({
        host_id: "h-add",
        capabilities: ["spoke-baseline", "tools.math.add"],
      }),
    );
    router.registerPeer(addPeer);

    // "upsert" is a baseline op — without the grammar gate the router
    // would select the baseline peer and delegate (or surface it as
    // no_capable_peer). The D13/D14 parity row promises a fail-fast
    // INVALID_INPUT with details.capability_id and no peer selection.
    const result = await router.invokeTool("upsert", {});

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details?.capability_id).toBe("upsert");
      // NOT the no_capable_peer / CAPABILITY_PORT_MISSING path.
      expect(result.details?.wire_code).not.toBe("no_capable_peer");
    }
    // Terminal: no peer was selected, no delegate ran.
    expect(addPeer.calls).toEqual([]);
  });

  it("fails fast with INVALID_INPUT on a malformed tools.* capability id before any peer selection (§6 grammar gate)", async () => {
    const router = connectMultiPeerRouter();
    const addPeer = new FakePeer(
      "peer-add",
      manifest({
        host_id: "h-add",
        capabilities: ["spoke-baseline", "tools.math.add"],
      }),
    );
    router.registerPeer(addPeer);

    for (const badId of ["tools.UPPER.x", "tools.onlyns"]) {
      const result = await router.invokeTool(badId, {});

      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
        expect(result.details?.capability_id).toBe(badId);
        expect(result.details?.wire_code).not.toBe("no_capable_peer");
      }
    }
    // Terminal: no peer was selected, no delegate ran.
    expect(addPeer.calls).toEqual([]);
  });
});

describe("MultiPeerRouter tool routing over loopback responders (§6 topology)", () => {
  // Router-registered adapters are LOCALLY-DIALED: router tool invokes
  // travel initiator→responder and are served by the PEER's responder-side
  // tool serving — the proof drives tool-serving responder doubles (the
  // minimal responder with the frozen §4 pipeline), not dialer-side
  // registerToolHandler alone.
  it("routes invokeTool to the serving responder by exact tool capability (disjoint tools)", async () => {
    const addCalls: { args: Record<string, unknown> }[] = [];
    const echoCalls: { args: Record<string, unknown> }[] = [];
    const { client: clientA, responder: responderA } = await dialToolPeer({
      responderSeedBase: 0xa1,
      tools: ["tools.math.add"],
    });
    const { client: clientB, responder: responderB } = await dialToolPeer({
      responderSeedBase: 0xb2,
      tools: ["tools.echo.echo"],
    });
    try {
      responderA.registerToolHandler("tools.math.add", addHandler(addCalls));
      responderB.registerToolHandler("tools.echo.echo", async (args) => {
        echoCalls.push({ args });
        return spokeOk(args);
      });

      const router = connectMultiPeerRouter();
      router.registerPeer(clientA);
      router.registerPeer(clientB);

      // tools.math.add is advertised + served only by responder A.
      const add = await router.invokeTool("tools.math.add", { a: 2, b: 3 });
      expect(add).toEqual({ ok: true, value: { sum: 5 } });
      expect(addCalls).toEqual([{ args: { a: 2, b: 3 } }]);
      expect(echoCalls).toEqual([]);

      // tools.echo.echo is advertised + served only by responder B.
      const echo = await router.invokeTool("tools.echo.echo", { v: 1 });
      expect(echo).toEqual({ ok: true, value: { v: 1 } });
      expect(echoCalls).toEqual([{ args: { v: 1 } }]);
      expect(addCalls).toHaveLength(1);
    } finally {
      clientA.close();
      clientB.close();
      responderA.close();
      responderB.close();
    }
  }, 15000);

  it("rejects with no_capable_peer when no registered peer's responder serves the tool", async () => {
    const { client: clientA, responder: responderA } = await dialToolPeer({
      responderSeedBase: 0xa1,
      tools: ["tools.math.add"],
    });
    const { client: clientB, responder: responderB } = await dialToolPeer({
      responderSeedBase: 0xb2,
      tools: ["tools.echo.echo"],
    });
    try {
      const router = connectMultiPeerRouter();
      router.registerPeer(clientA);
      router.registerPeer(clientB);

      const result = await router.invokeTool("tools.echo.boom", {});

      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
        expect(result.details?.wire_code).toBe("no_capable_peer");
        expect(result.details?.kind).toBe("no_capable_peer");
        expect(result.details?.op).toBe("tools.echo.boom");
      }
      // Terminal: neither responder served anything.
      expect(responderA.stats.handlersRun).toBe(0);
      expect(responderB.stats.handlersRun).toBe(0);
    } finally {
      clientA.close();
      clientB.close();
      responderA.close();
      responderB.close();
    }
  }, 15000);
});

/** The add handler used by loopback fixtures. */
function addHandler(calls: { args: Record<string, unknown> }[]): TestToolHandler {
  return async (args) => {
    calls.push({ args });
    return spokeOk({ sum: (args.a as number) + (args.b as number) });
  };
}
