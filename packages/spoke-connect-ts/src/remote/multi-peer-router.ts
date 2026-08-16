/**
 * `MultiPeerRouter` — capability-selected multi-peer async `BaselinePorts`
 * over N registered per-peer `RemoteAdapter` instances (frozen multi-peer
 * routing contract; normative design intent in the RemoteAdapter spec
 * "Multi-peer registry/composer" staged follow-on).
 *
 * The router sits ABOVE per-peer `RemoteAdapter`s and BELOW `orchestrate*`:
 * consumers dial and register established adapters, then keep calling
 * `orchestrate*(router, req)` with no per-op `peer_id`. Each `BaselinePorts`
 * call selects exactly one peer by the locked §3 algorithm — capability →
 * namespace → authority hard filters, role soft partition, deterministic
 * lexicographic UTF-8 `peer_id` tie-break (§4) — and delegates. The selected
 * peer's underlying `SpokeResult` reject is returned as-is (§7.2: no
 * automatic alternate-retry, no remap). `getHostCapabilityManifest` /
 * `listPeerHostCapabilityManifests` are aggregated locally from the cached
 * per-peer manifests (§6: composed view / per-peer array — no round-trip).
 *
 * Public surface (§8): `connectMultiPeerRouter(options)`, `registerPeer`,
 * `unregisterPeer`, `listPeers`, the async `BaselinePorts` six families,
 * and the tool-invoke face `invokeTool(capabilityId, arguments)` (D14 —
 * exact-capability hard filter, lowest-`peer_id` tie-break, terminal
 * `no_capable_peer` reject, composed-manifest `tools[]` union).
 * Per-peer adapters are encapsulated — the router never exposes them after
 * registration.
 */

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

import type { RemoteAdapterState } from "./remote-adapter.js";

/** The router's own identity when the consumer configures none (contract §8). */
const DEFAULT_ROUTER_HOST_ID = "multi-peer-router";

// ── Locked op → selection-input tables (contract §2 / §3) ────────────────

/**
 * Required capability per op family (contract §2 — locked). Orchestrated
 * baseline families and the `port.*` baseline ops require `spoke-baseline`;
 * the computable families require `l2-computable`. Product-defined ops are
 * product-documented and have no row here; selection REJECTS ops outside
 * this table (`no_capable_peer`) — an op with no gate must not fall through
 * ungated (QC2 S-1). The router's fixed six-family surface only ever
 * queries the `port.*` rows (mirrors the RemoteAdapter `PORT_OPS`
 * catalogue).
 */
const REQUIRED_CAPABILITY: Readonly<Record<string, string>> = {
  // Orchestrated op families.
  upsert: "spoke-baseline",
  promote: "spoke-baseline",
  relate: "spoke-baseline",
  check: "spoke-baseline",
  assemble: "spoke-baseline",
  project: "l2-computable",
  compute: "l2-computable",
  // port.* baseline ops.
  "port.knowledge.get": "spoke-baseline",
  "port.knowledge.put": "spoke-baseline",
  "port.relation.get": "spoke-baseline",
  "port.relation.put": "spoke-baseline",
  "port.scope.list_knowledge_entries": "spoke-baseline",
  "port.scope.list_timeline_events": "spoke-baseline",
  "port.finding.put": "spoke-baseline",
  "port.rule.list": "spoke-baseline",
  // Optional l2-computable port ops (not delegated by the baseline surface).
  "port.computable.project": "l2-computable",
  "port.computable.compute": "l2-computable",
};

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

/**
 * Required capability for an op (contract §2 + frozen §6 `tools.` prefix
 * rule — the selection table gains the same widening as core dispatch):
 * a `tools.<ns>.<tool_id>` op is self-describing — the required capability
 * IS the op string itself (no registry, no umbrella flag); every other op
 * resolves through the locked table above. Ops outside both resolve to
 * `undefined` and are rejected outright by selection (no ungated
 * fall-through, QC2 S-1).
 */
function requiredCapability(op: string): string | undefined {
  if (op.startsWith("tools.")) {
    return op;
  }
  return REQUIRED_CAPABILITY[op];
}

// ── Payload-derived selection inputs (contract §2 / §3) ──────────────────

/**
 * The request's collaboration namespace, derived from the op payload when it
 * carries one (contract §2 — "derived from `Scope` when the op payload
 * carries one"). Products surface the namespace either at the payload top
 * level (`namespace`) or on the payload's `Scope` (`scope.namespace`). When
 * neither is present the namespace filter is skipped (§3 step 3). No
 * wildcard in v1: a literal `"*"` is the literal string, never a match-all.
 */
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

/**
 * The request's authority scope key, when the payload carries one (contract
 * §3 step 4 — the authority filter applies when the peer manifest AND the
 * request both declare a scope key). Same carrier shapes as the namespace.
 */
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

/**
 * Lexicographic UTF-8 byte order on `peer_id` strings (contract §4). Plain
 * JS `<` compares UTF-16 code units, which diverges from UTF-8 byte order
 * for non-BMP strings — encode to bytes for the faithful order (peer ids are
 * base58btc ASCII in practice; the byte order is what the contract locks).
 */
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

/**
 * Set-union preserving first-seen order across the inputs, cast to the
 * schema's minItems-1 tuple type (contract §6). The cast covers the
 * legitimate empty union: a router with zero connected peers composes empty
 * `capabilities` / `roles` / `namespaces`, which the generated wire type
 * (JSON Schema `minItems: 1`) cannot express directly — same cast as the
 * golden fixture.
 */
function unionOf(arrays: readonly (readonly string[])[]): [string, ...string[]] {
  const seen = new Set<string>();
  for (const array of arrays) {
    for (const item of array) {
      seen.add(item);
    }
  }
  return [...seen] as [string, ...string[]];
}

/**
 * Tool-descriptor union across the connected peers' `tools[]` (frozen §6):
 * dedup by `capability_id` (first occurrence wins) and sort by
 * `capability_id` in lexicographic UTF-8 byte order — stability across
 * registration order, unlike the first-seen string unions above.
 */
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

/** The locked §5 no-capable-peer reject: terminal, no retry, no fallback. */
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
 *    ungated fall-through (QC2 S-1).
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
    // Unknown ops are rejected outright: the §3 capability gate is step 1 of
    // every selection, and an op outside the locked mapping table has no gate
    // to run — falling through would select an arbitrary Established peer
    // (QC2 S-1). Same terminal §5 reject shape as a no-match.
    return noCapablePeer(op, `no capability mapping for unknown op "${op}"`);
  }
  let survivors = candidates.filter((candidate) =>
    candidate.manifest.capabilities.includes(required),
  );
  if (survivors.length === 0) {
    return noCapablePeer(
      op,
      `no peer advertises capability "${required}"`,
    );
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
      // Both sides declare → exact match required; only one side declares →
      // filter skipped for this peer.
      return declared === undefined || declared === scopeKey;
    });
    if (survivors.length === 0) {
      return noCapablePeer(
        op,
        `no peer authority scope key matches "${scopeKey}"`,
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

// ── Router ───────────────────────────────────────────────────────────────

/**
 * The per-peer adapter surface the router composes — satisfied by
 * `RemoteAdapter` (its read-only session getters + async `BaselinePorts`).
 * The router reads `state` (candidate gate), `remotePeerId` (registry key +
 * tie-break), and `remoteManifest` (cached at registration); it delegates
 * `BaselinePorts` calls to the selected adapter. Envelope auth stays
 * enforced inside each per-peer session — selection is a routing decision
 * above the adapters and adds no bypass.
 */
export interface RoutedRemoteAdapter extends BaselinePorts {
  readonly state: RemoteAdapterState;
  readonly remotePeerId: string;
  readonly remoteManifest: HostCapabilityManifest;
  /**
   * Forward tool-invoke face (frozen §6) — satisfied by
   * `RemoteAdapter.invokeTool`; the router delegates tool invokes to the
   * selected peer's adapter (the router never crafts envelopes itself).
   */
  invokeTool(
    capabilityId: string,
    args: Record<string, unknown>,
  ): Promise<SpokeResult<unknown>>;
}

export interface MultiPeerRouterOptions {
  /**
   * The router's own host identity — the composed view's `host_id` (contract
   * §6: the local node's identity, NOT a peer's). Optional: defaults to
   * `"multi-peer-router"`; an empty string is treated as unset (the schema's
   * `host_id` requires a non-empty string), mirroring Rust.
   */
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

/**
 * Multi-peer capability router: registry (§7.4) + locked §3 selection +
 * async `BaselinePorts` delegation (six families) + §6 HostManifest
 * aggregation. Construct via `connectMultiPeerRouter` — the router starts
 * with zero peers; consumers dial each peer's `RemoteAdapter` and register
 * the established adapter.
 */
export class MultiPeerRouter implements BaselinePorts {
  readonly #hostId: string;
  /** Registered peers by `peer_id` (dynamic insert/delete; registration order). */
  #peers = new Map<string, RegisteredPeer>();

  constructor(options: MultiPeerRouterOptions = {}) {
    // Empty-string hostId is treated as unset (the schema's `host_id`
    // requires a non-empty string) — same defaulting as Rust's
    // `host_id.filter(|id| !id.is_empty())` (§8 constructor options).
    this.#hostId = options.hostId || DEFAULT_ROUTER_HOST_ID;
  }

  // ── Registry (contract §7.4) ───────────────────────────────────────────

  /**
   * Register an adapter under its verified remote peer id. Idempotent on
   * `peer_id`: re-registering the same id replaces the stored adapter and
   * re-caches its manifest. Returns the `peer_id`. Throws when the adapter
   * has no established session (no verified peer id / no cached manifest) —
   * consumers dial first (§8: "consumers dial each peer's RemoteAdapter
   * themselves and register the established adapter"). Does NOT close the
   * adapter. A Closed adapter may remain registered — it is simply excluded
   * from the candidate set; the consumer MAY call `unregisterPeer` to evict.
   */
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

  /**
   * Remove a peer from the registry. No-op if not registered. Does NOT close
   * the adapter (the consumer owns the adapter lifecycle, §7.4).
   */
  unregisterPeer(peerId: string): void {
    this.#peers.delete(peerId);
  }

  /** Registered peer ids, in registration order (the registry, not selection). */
  listPeers(): string[] {
    return [...this.#peers.keys()];
  }

  // ── BaselinePorts — select a peer per call, then delegate (§1/§8) ──────

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
    // The selection payload carries the request Scope so the §2 namespace /
    // §3 authority derivations see it (the same Scope the peer receives).
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

  // ── Tool routing (frozen §6) ────────────────────────────────────────────

  /**
   * Tool-invoke face (frozen §6): fails fast on a non-`tools.` capability
   * id (the op string IS the capability string; a non-`tools.` id is a
   * programming error) with `INVALID_INPUT` + `details.capability_id`
   * before any peer selection — no wire traffic (D13/D14 parity with
   * `RemoteAdapter.invokeTool`). Otherwise select the peer whose cached
   * hello manifest `capabilities[]` contains the EXACT tool capability
   * string (the selection table's `tools.` prefix rule resolves the
   * required capability to the op itself), then delegate to the selected
   * peer's adapter `invokeTool` — the router never crafts envelopes
   * itself. No namespace/authority/role filters for tools (the capability
   * string is ns-scoped; tool payloads carry no `Scope`); deterministic
   * tie-break = lowest `peer_id`; none → the existing terminal
   * `no_capable_peer` reject (`details.op = capabilityId`). The selected
   * peer's underlying `SpokeResult` reject is returned as-is (§7.2 — no
   * alternate-retry).
   */
  async invokeTool(
    capabilityId: string,
    args: Record<string, unknown>,
  ): Promise<SpokeResult<unknown>> {
    // Fail fast on a non-tool capability id (the op string IS the
    // capability string; a non-`tools.` id is a programming error).
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

  /**
   * Composed view: set-union of all CONNECTED (Established) peers'
   * capabilities / roles / namespaces, the router's own `host_id`, NO
   * `authority` (a composed view does not synthesize an authority scope),
   * and `extensions.router.peers` listing the contributing peer ids in
   * lexicographic UTF-8 byte order. For consumer introspection ONLY — never
   * used for routing (§6 aggregation vs routing non-conflation).
   */
  async getHostCapabilityManifest(): Promise<SpokeResult<HostCapabilityManifest>> {
    const connected = this.#connectedPeers();
    const composed: HostCapabilityManifest = {
      schema_version: 1,
      host_id: this.#hostId,
      capabilities: unionOf(connected.map((peer) => peer.manifest.capabilities)),
      roles: unionOf(connected.map((peer) => peer.manifest.roles)),
      namespaces: unionOf(connected.map((peer) => peer.manifest.namespaces)),
      // Frozen §6: `tools[]` unions across connected peers, deduped by
      // `capability_id` in lexicographic order (stability, not first-seen).
      tools: unionTools(connected.map((peer) => peer.manifest.tools ?? [])),
      extensions: { router: { peers: connected.map((peer) => peer.peerId) } },
    };
    return spokeOk(composed);
  }

  /**
   * Per-peer array of all connected peers' cached manifests (the `host`
   * field from their authenticated hello, cached at registration), ordered
   * lexicographically by `peer_id` (UTF-8 byte order). Zero connected peers
   * is valid and returns `[]`.
   */
  async listPeerHostCapabilityManifests(): Promise<
    SpokeResult<HostCapabilityManifest[]>
  > {
    return spokeOk(
      this.#connectedPeers().map((peer) => structuredClone(peer.manifest)),
    );
  }

  // ── Selection + delegation internals ───────────────────────────────────

  /** Connected (Established) peers with their cached manifests, peer_id-sorted. */
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
    // Candidate set: Established peers only (Closed / Disconnected /
    // Handshaking excluded, contract §3 step 1 / §7.4).
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
      // Unreachable: candidates are built from the registry keys above.
      return noCapablePeer(
        op,
        `selected peer ${selection.value.peerId} is no longer registered`,
      );
    }
    return spokeOk({ peerId: selection.value.peerId, adapter: entry.adapter });
  }

  /**
   * I/O-thin delegation: select a peer for `op` + selection payload, then
   * await the peer's port method. The selected peer's underlying
   * `SpokeResult` reject (transport / session failure, envelope-auth kinds)
   * is returned AS-IS — never remapped to `no_capable_peer`, never retried
   * on an alternate peer (contract §7.2; the consumer owns retry because
   * only the consumer knows the op's idempotency semantics).
   */
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
