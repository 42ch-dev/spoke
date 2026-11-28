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
  parseToolCapabilityId,
  spokeOk,
  spokeReject,
  type BaselinePorts,
  type SpokeReject,
  type SpokeResult,
} from "@42ch/spoke-operations";

import {
  CAPABILITY_SPOKE_COGNITION,
  capabilitySatisfies,
  cognitionRequirementPresent,
  requiredCapability as coreRequiredCapability,
} from "../core/dispatch.js";
import type { RemoteAdapterState } from "./remote-adapter.js";
import {
  CAPABILITY_KE_OWNERSHIP,
  PORT_OP_CAPABILITY_REQUIREMENTS,
  scopeOpRequiresOwnershipCapability,
} from "./responder.js";

/** The router's own identity when the consumer configures none (contract §8). */
const DEFAULT_ROUTER_HOST_ID = "multi-peer-router";

// ── Locked op → selection-input tables (contract §2 / §3) ────────────────

/**
 * The eight orchestrated op names (contract §2 — locked). Selection resolves
 * each through the shared core `requiredCapability` table; `port.*` rows are
 * deliberately not repeated here: selection reads the responder's product
 * catalogue (`PORT_OP_CAPABILITY_REQUIREMENTS`), so routing and serving
 * share one map instead of drifting. Product-defined ops are
 * product-documented and in neither table; selection REJECTS ops outside
 * both (`no_capable_peer`) — an op with no gate must not fall through
 * ungated. Memory-family ops stay rejected the same way: the core
 * table resolves them, but they are not orchestrated, so the Set membership
 * check in `requiredCapability` below rejects them exactly as before.
 */
const ORCHESTRATED_OPS: ReadonlySet<string> = new Set([
  "upsert",
  "promote",
  "relate",
  "check",
  "assemble",
  "project",
  "compute",
  "extract",
]);

/**
 * Preferred role per op (contract §3 — locked, SOFT preference only: it
 * reorders candidates, never rejects a capable peer for lacking the role).
 * `upsert` / `promote` / `relate` and `port.*` baseline ops carry no role
 * preference — capability + namespace are the discriminators.
 */
const PREFERRED_ROLE: Readonly<Record<string, string>> = {
  check: "checker",
  assemble: "assembler",
  project: "l2-computable",
  compute: "l2-computable",
};

function requiredCapability(op: string): string | undefined {
  if (ORCHESTRATED_OPS.has(op) || op.startsWith("tools.")) {
    return coreRequiredCapability(op);
  }
  return PORT_OP_CAPABILITY_REQUIREMENTS[op];
}

// ── Payload-derived selection inputs (contract §2 / §3) ──────────────────

function requestNamespace(payload: Record<string, unknown>): string | undefined {
  const topLevel = payload["namespace"];
  if (typeof topLevel === "string") {
    return topLevel;
  }
  const scope = payload["scope"];
  if (typeof scope === "object" && scope !== null && "namespace" in scope) {
    const nested = scope.namespace;
    if (typeof nested === "string") {
      return nested;
    }
  }
  return undefined;
}

function requestScopeKey(payload: Record<string, unknown>): string | undefined {
  const topLevel = payload["scope_key"];
  if (typeof topLevel === "string") {
    return topLevel;
  }
  const scope = payload["scope"];
  if (typeof scope === "object" && scope !== null && "scope_key" in scope) {
    const nested = scope.scope_key;
    if (typeof nested === "string") {
      return nested;
    }
  }
  return undefined;
}

const textEncoder = new TextEncoder();

function compareUtf8PeerIds(a: string, b: string): number {
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

function unionOf(arrays: readonly (readonly string[])[]): [string, ...string[]] {
  const seen = new Set<string>();
  for (const array of arrays) {
    for (const item of array) {
      seen.add(item);
    }
  }
  return [...seen] as [string, ...string[]];
}

function unionTools(
  arrays: readonly (readonly ToolDescriptor[])[],
): ToolDescriptor[] {
  const byId = new Map<string, ToolDescriptor>();
  for (const array of arrays) {
    for (const descriptor of array) {
      if (!byId.has(descriptor.capability_id)) {
        byId.set(descriptor.capability_id, descriptor);
      }
    }
  }
  return [...byId.values()].sort((a, b) =>
    compareUtf8PeerIds(a.capability_id, b.capability_id),
  );
}

function noCapablePeer(op: string, reason: string): SpokeReject {
  return spokeReject(
    SpokeRejectCode.CAPABILITY_PORT_MISSING,
    `no capable peer for ${op}: ${reason}`,
    { wire_code: "no_capable_peer", kind: "no_capable_peer", op },
  );
}

// ── Pure selection (contract §3 + §4 + §5) ───────────────────────────────

/** One selection candidate: a registered peer's id + cached manifest (§2). */
export interface SelectablePeer {
  peerId: string;
  manifest: HostCapabilityManifest;
}

/**
 * The locked §3 selection algorithm over an explicit candidate set:
 *
 * 1. Capability filter (hard): the peer's `capabilities` MUST include the
 *    required capability for the op (§2 mapping table). Ops outside the
 *    mapping table are rejected outright — no gate to run, never an
 *    ungated fall-through.
 * 2. Namespace filter (hard): when the request payload carries a namespace,
 *    the peer's `namespaces` MUST include it (exact match; skipped when the
 *    request carries none; no wildcard).
 * 3. Authority filter (hard when both sides declare): a peer whose
 *    `authority.scope_key` is present AND mismatches the request's scope key
 *    is excluded; when only one side (or neither) declares, the filter is
 *    skipped for that peer.
 * 4. Role preference (soft): peers with the op's preferred role in `roles[]`
 *    are preferred over peers without it; never rejects for lacking it.
 * 5. Deterministic tie-break: lowest `peer_id` in lexicographic UTF-8 byte
 *    order (§4) — no clock, no random, no health score.
 *
 * Returns a `no_capable_peer` reject (§5) when the op has no capability
 * mapping or no candidate survives the hard gates — terminal, stable, no
 * wrong-peer fallback.
 */
export function selectPeerForOp(
  candidates: readonly SelectablePeer[],
  op: string,
  payload: Record<string, unknown>,
): SpokeResult<SelectablePeer> {
  if (candidates.length === 0) {
    return noCapablePeer(op, "no established peer registered");
  }

  const required = requiredCapability(op);
  if (required === undefined) {
    return noCapablePeer(op, `no capability mapping for unknown op "${op}"`);
  }

  let survivors = candidates.filter((candidate) =>
    capabilitySatisfies(candidate.manifest.capabilities, required),
  );
  if (survivors.length === 0) {
    return noCapablePeer(
      op,
      `no peer advertises capability "${required}"`,
    );
  }

  if (scopeOpRequiresOwnershipCapability(op, payload)) {
    survivors = survivors.filter((candidate) =>
      candidate.manifest.capabilities.includes(CAPABILITY_KE_OWNERSHIP),
    );
    if (survivors.length === 0) {
      return noCapablePeer(
        op,
        `no peer advertises capability "${CAPABILITY_KE_OWNERSHIP}"`,
      );
    }
  }

  const namespace = requestNamespace(payload);
  if (namespace !== undefined) {
    survivors = survivors.filter((candidate) =>
      candidate.manifest.namespaces.includes(namespace),
    );
    if (survivors.length === 0) {
      return noCapablePeer(op, `no peer advertises namespace "${namespace}"`);
    }
  }

  const scopeKey = requestScopeKey(payload);
  if (scopeKey !== undefined) {
    survivors = survivors.filter((candidate) => {
      const declared = candidate.manifest.authority?.scope_key;
      return declared === undefined || declared === scopeKey;
    });
    if (survivors.length === 0) {
      return noCapablePeer(
        op,
        `no peer authority scope key matches "${scopeKey}"`,
      );
    }
  }

  if (cognitionRequirementPresent(op, payload)) {
    survivors = survivors.filter((candidate) =>
      candidate.manifest.capabilities.includes(CAPABILITY_SPOKE_COGNITION),
    );
    if (survivors.length === 0) {
      return noCapablePeer(
        op,
        `no peer advertises capability "${CAPABILITY_SPOKE_COGNITION}"`,
      );
    }
  }

  const preferredRole = PREFERRED_ROLE[op];
  if (preferredRole !== undefined) {
    const roleMatched = survivors.filter((candidate) =>
      candidate.manifest.roles.includes(preferredRole),
    );
    if (roleMatched.length > 0) {
      survivors = roleMatched;
    }
  }

  survivors.sort((a, b) => compareUtf8PeerIds(a.peerId, b.peerId));
  return spokeOk(survivors[0]);
}

export interface RoutedRemoteAdapter extends BaselinePorts {
  readonly state: RemoteAdapterState;
  readonly remotePeerId: string;
  readonly remoteManifest: HostCapabilityManifest;
  invokeTool(
    capabilityId: string,
    args: Record<string, unknown>,
  ): Promise<SpokeResult<unknown>>;
}

export interface MultiPeerRouterOptions {
  hostId?: string;
}

interface RegisteredPeer {
  adapter: RoutedRemoteAdapter;
  /** Per-peer manifest cache, captured at registration (contract §1/§7.4). */
  manifest: HostCapabilityManifest;
}

interface SelectedPeer {
  peerId: string;
  adapter: RoutedRemoteAdapter;
}

export class MultiPeerRouter implements BaselinePorts {
  readonly #hostId: string;
  #peers = new Map<string, RegisteredPeer>();

  constructor(options: MultiPeerRouterOptions = {}) {
    this.#hostId = options.hostId || DEFAULT_ROUTER_HOST_ID;
  }

  // ── Registry (contract §7.4) ───────────────────────────────────────────

  registerPeer(adapter: RoutedRemoteAdapter): string {
    const peerId = adapter.remotePeerId;
    if (peerId.length === 0) {
      throw new Error(
        "registerPeer requires an adapter with an established connect session (verified remote peer id unavailable)",
      );
    }
    let manifest: HostCapabilityManifest;
    try {
      manifest = adapter.remoteManifest;
    } catch {
      throw new Error(
        "registerPeer requires an adapter with an established connect session (remote capability manifest unavailable)",
      );
    }
    this.#peers.set(peerId, { adapter, manifest });
    return peerId;
  }

  unregisterPeer(peerId: string): void {
    this.#peers.delete(peerId);
  }

  listPeers(): string[] {
    return [...this.#peers.keys()];
  }

  // see contract §1 / §8 — select a peer per call, then delegate

  async getKnowledgeEntry(entryId: string): Promise<SpokeResult<KnowledgeEntry>> {
    return this.#delegate("port.knowledge.get", {}, (peer) =>
      peer.getKnowledgeEntry(entryId),
    );
  }

  async putKnowledgeEntry(
    entry: KnowledgeEntry,
    expectedBaseRevision: number | null,
  ): Promise<SpokeResult<KnowledgeEntry>> {
    return this.#delegate("port.knowledge.put", {}, (peer) =>
      peer.putKnowledgeEntry(entry, expectedBaseRevision),
    );
  }

  async getRelation(relationId: string): Promise<SpokeResult<Relation>> {
    return this.#delegate("port.relation.get", {}, (peer) =>
      peer.getRelation(relationId),
    );
  }

  async putRelation(
    relation: Relation,
    expectedBaseRevision: number | null,
  ): Promise<SpokeResult<Relation>> {
    return this.#delegate("port.relation.put", {}, (peer) =>
      peer.putRelation(relation, expectedBaseRevision),
    );
  }

  async listKnowledgeEntries(
    scope: Scope,
  ): Promise<SpokeResult<KnowledgeEntry[]>> {
    return this.#delegate("port.scope.list_knowledge_entries", { scope }, (peer) =>
      peer.listKnowledgeEntries(scope),
    );
  }

  async listTimelineEvents(scope: Scope): Promise<SpokeResult<TimelineEvent[]>> {
    return this.#delegate("port.scope.list_timeline_events", { scope }, (peer) =>
      peer.listTimelineEvents(scope),
    );
  }

  async putFindings(findings: Finding[]): Promise<SpokeResult<Finding[]>> {
    return this.#delegate("port.finding.put", {}, (peer) =>
      peer.putFindings(findings),
    );
  }

  async listRules(ruleRefs: string[]): Promise<SpokeResult<Rule[]>> {
    return this.#delegate("port.rule.list", {}, (peer) =>
      peer.listRules(ruleRefs),
    );
  }

  // see contract §6 — tool routing

  async invokeTool(
    capabilityId: string,
    args: Record<string, unknown>,
  ): Promise<SpokeResult<unknown>> {
    const parsed = parseToolCapabilityId(capabilityId);
    if (!parsed.ok) {
      return parsed;
    }
    const selected = this.#selectPeerForOp(capabilityId, {});
    if (!selected.ok) {
      return selected;
    }
    return selected.value.adapter.invokeTool(capabilityId, args);
  }

  // ── HostManifestPort — aggregated locally (contract §6, no round-trip) ──

  async getHostCapabilityManifest(): Promise<SpokeResult<HostCapabilityManifest>> {
    const connected = this.#connectedPeers();
    const composed: HostCapabilityManifest = {
      schema_version: 1,
      host_id: this.#hostId,
      capabilities: unionOf(connected.map((peer) => peer.manifest.capabilities)),
      roles: unionOf(connected.map((peer) => peer.manifest.roles)),
      namespaces: unionOf(connected.map((peer) => peer.manifest.namespaces)),
      tools: unionTools(connected.map((peer) => peer.manifest.tools ?? [])),
      extensions: { router: { peers: connected.map((peer) => peer.peerId) } },
    };
    return spokeOk(composed);
  }

  async listPeerHostCapabilityManifests(): Promise<
    SpokeResult<HostCapabilityManifest[]>
  > {
    return spokeOk(
      this.#connectedPeers().map((peer) => structuredClone(peer.manifest)),
    );
  }

  #connectedPeers(): Array<{ peerId: string; manifest: HostCapabilityManifest }> {
    const connected: Array<{
      peerId: string;
      manifest: HostCapabilityManifest;
    }> = [];
    for (const [peerId, entry] of this.#peers) {
      if (entry.adapter.state === "Established") {
        connected.push({ peerId, manifest: entry.manifest });
      }
    }
    connected.sort((a, b) => compareUtf8PeerIds(a.peerId, b.peerId));
    return connected;
  }

  #selectPeerForOp(
    op: string,
    payload: Record<string, unknown>,
  ): SpokeResult<SelectedPeer> {
    const candidates: SelectablePeer[] = [];
    for (const [peerId, entry] of this.#peers) {
      if (entry.adapter.state === "Established") {
        candidates.push({ peerId, manifest: entry.manifest });
      }
    }
    const selection = selectPeerForOp(candidates, op, payload);
    if (!selection.ok) {
      return selection;
    }
    const entry = this.#peers.get(selection.value.peerId);
    if (entry === undefined) {
      return noCapablePeer(
        op,
        `selected peer ${selection.value.peerId} is no longer registered`,
      );
    }
    return spokeOk({ peerId: selection.value.peerId, adapter: entry.adapter });
  }

  async #delegate<T>(
    op: string,
    payload: Record<string, unknown>,
    invoke: (adapter: RoutedRemoteAdapter) => Promise<SpokeResult<T>>,
  ): Promise<SpokeResult<T>> {
    const selected = this.#selectPeerForOp(op, payload);
    if (!selected.ok) {
      return selected;
    }
    return invoke(selected.value.adapter);
  }
}

/**
 * Construct a multi-peer capability router (contract §8). No dial options —
 * the router starts with zero peers; consumers dial each peer's
 * `RemoteAdapter` themselves and register the established adapter.
 */
export function connectMultiPeerRouter(
  options?: MultiPeerRouterOptions,
): MultiPeerRouter {
  return new MultiPeerRouter(options);
}
