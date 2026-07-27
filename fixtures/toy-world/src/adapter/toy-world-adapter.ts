/**
 * Reference FullAdapter for the toy-world fixture graph.
 * Baseline ports use MemoryStore OCC; computable / fork return wire-valid stubs.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";

import type {
  ComputeRequest,
  ComputeResponse,
  Finding,
  ForkId,
  HostCapabilityManifest,
  KnowledgeEntry,
  ProjectRequest,
  ProjectResponse,
  Relation,
  Rule,
  Scope,
  TimelineEvent,
} from "@42ch/spoke-schemas";
import {
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  type BaselineAdapter,
  type FullAdapter,
  type SpokeResult,
} from "@42ch/spoke-operations";

import {
  MemoryStore,
  TOY_WORLD_FIXTURES_ROOT,
  type MemoryStoreSeed,
} from "./memory-store.js";

function loadOpFixture<T>(filename: string): T {
  const raw = readFileSync(join(TOY_WORLD_FIXTURES_ROOT, filename), "utf8");
  return JSON.parse(raw) as T;
}

const TOY_WORLD_SELF_MANIFEST: HostCapabilityManifest =
  loadOpFixture<HostCapabilityManifest>("host_tw_primary.json");

/** Product-seeded peer manifests held in adapter memory (static fixture graph). */
const TOY_WORLD_PEER_MANIFESTS: HostCapabilityManifest[] = [
  loadOpFixture<HostCapabilityManifest>("host_tw_peer.json"),
];

/** Peer-list normalization used by ToyWorldAdapter (exclude self, dedupe, sort). */
export function normalizePeerManifests(
  selfHostId: string,
  peers: HostCapabilityManifest[],
): HostCapabilityManifest[] {
  const byHostId = new Map<string, HostCapabilityManifest>();
  for (const peer of peers) {
    if (peer.host_id === selfHostId) {
      continue;
    }
    byHostId.set(peer.host_id, peer);
  }
  return [...byHostId.values()].sort((left, right) =>
    Buffer.compare(
      Buffer.from(left.host_id, "utf8"),
      Buffer.from(right.host_id, "utf8"),
    ),
  );
}

/**
 * Toy-world reference adapter — implements FullAdapter over an in-memory store.
 */
export class ToyWorldAdapter implements FullAdapter {
  readonly store: MemoryStore;

  constructor(storeOrSeed?: MemoryStore | MemoryStoreSeed) {
    if (storeOrSeed instanceof MemoryStore) {
      this.store = storeOrSeed;
    } else if (storeOrSeed !== undefined) {
      this.store = new MemoryStore(storeOrSeed);
    } else {
      this.store = new MemoryStore();
    }
  }

  /** Construct with committed kb / rel / evt / rule / fnd fixtures loaded. */
  static withCommittedFixtures(): ToyWorldAdapter {
    return new ToyWorldAdapter(MemoryStore.fromCommittedFixtures());
  }

  getKnowledgeEntry(entryId: string): SpokeResult<KnowledgeEntry> {
    return this.store.getKnowledgeEntry(entryId);
  }

  putKnowledgeEntry(
    entry: KnowledgeEntry,
    expectedBaseRevision: number | null,
  ): SpokeResult<KnowledgeEntry> {
    return this.store.putKnowledgeEntry(entry, expectedBaseRevision);
  }

  putRelation(relation: Relation): SpokeResult<Relation> {
    return this.store.putRelation(relation);
  }

  listKnowledgeEntries(_scope: Scope): SpokeResult<KnowledgeEntry[]> {
    return this.store.listKnowledgeEntries();
  }

  listTimelineEvents(_scope: Scope): SpokeResult<TimelineEvent[]> {
    return this.store.listTimelineEvents();
  }

  putFindings(findings: Finding[]): SpokeResult<Finding[]> {
    return this.store.putFindings(findings);
  }

  listRules(ruleRefs: string[]): SpokeResult<Rule[]> {
    return this.store.listRules(ruleRefs);
  }

  getHostCapabilityManifest(): SpokeResult<HostCapabilityManifest> {
    return spokeOk(TOY_WORLD_SELF_MANIFEST);
  }

  listPeerHostCapabilityManifests(): SpokeResult<HostCapabilityManifest[]> {
    return spokeOk(
      normalizePeerManifests(
        TOY_WORLD_SELF_MANIFEST.host_id,
        TOY_WORLD_PEER_MANIFESTS,
      ),
    );
  }

  /**
   * Minimal wire-valid ProjectResponse from committed op_tw_project_response.json.
   * Echoes request session_id / entry_id; computable shape comes from the fixture.
   * Error-envelope fixtures are rejected (parity with the Rust adapter).
   */
  project(request: ProjectRequest): SpokeResult<ProjectResponse> {
    const fixture = loadOpFixture<ProjectResponse>("op_tw_project_response.json");
    if ("error" in fixture && !("computable" in fixture)) {
      return spokeReject(
        SpokeRejectCode.INVALID_INPUT,
        `fixture project response is an error envelope: ${fixture.error.message}`,
      );
    }
    return spokeOk({
      session_id: request.session_id,
      entry_id: request.entry_id,
      computable: fixture.computable,
    });
  }

  /**
   * Minimal wire-valid ComputeResponse from committed settle response fixture.
   * When settle is true, include fixture `state`; otherwise omit it.
   */
  compute(request: ComputeRequest): SpokeResult<ComputeResponse> {
    const fixture = loadOpFixture<ComputeResponse>(
      "op_tw_compute_settle_response.json",
    );
    if (request.settle === true) {
      return spokeOk({
        session_id: request.session_id,
        entry_id: request.entry_id,
        computable: request.computable ?? fixture.computable,
        state: fixture.state ?? request.computable,
      });
    }
    return spokeOk({
      session_id: request.session_id,
      entry_id: request.entry_id,
      computable: request.computable ?? fixture.computable,
    });
  }

  /**
   * Fork timeline query — seeded events filtered by scope.fork_id.
   */
  listForkTimelineEvents(
    scope: Scope & { fork_id: ForkId },
  ): SpokeResult<TimelineEvent[]> {
    const events = this.store.events.filter(
      (event) => event.fork_id === scope.fork_id,
    );
    return spokeOk(events);
  }
}

/**
 * Baseline-only view of a ToyWorldAdapter — omits optional Full methods so
 * dynamic orchestrators surface CAPABILITY_PORT_MISSING.
 */
export function asBaselineOnly(adapter: ToyWorldAdapter): BaselineAdapter {
  return {
    getKnowledgeEntry: (entryId) => adapter.getKnowledgeEntry(entryId),
    putKnowledgeEntry: (entry, expectedBaseRevision) =>
      adapter.putKnowledgeEntry(entry, expectedBaseRevision),
    putRelation: (relation) => adapter.putRelation(relation),
    listKnowledgeEntries: (scope) => adapter.listKnowledgeEntries(scope),
    listTimelineEvents: (scope) => adapter.listTimelineEvents(scope),
    putFindings: (findings) => adapter.putFindings(findings),
    listRules: (ruleRefs) => adapter.listRules(ruleRefs),
    getHostCapabilityManifest: () => adapter.getHostCapabilityManifest(),
    listPeerHostCapabilityManifests: () =>
      adapter.listPeerHostCapabilityManifests(),
  };
}
