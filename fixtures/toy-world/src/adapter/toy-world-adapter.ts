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
  ToolDescriptor,
} from "@42ch/spoke-schemas";
import {
  SpokeRejectCode,
  findTool,
  listTools,
  parseToolCapabilityId,
  spokeOk,
  spokeReject,
  validateToolArguments,
  type BaselineAdapter,
  type FullAdapter,
  type SpokeResult,
} from "@42ch/spoke-operations";

import {
  MemoryStore,
  TOY_WORLD_FIXTURES_ROOT,
  type MemoryStoreSeed,
} from "./memory-store.js";
import {
  defaultToyWorldToolHandlers,
  type ToolHandler,
} from "./toy-world-tools.js";

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

function cloneHostCapabilityManifest(
  manifest: HostCapabilityManifest,
): HostCapabilityManifest {
  return structuredClone(manifest);
}

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
    byHostId.set(peer.host_id, cloneHostCapabilityManifest(peer));
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

  /**
   * Tool-handler registry (plan-3 serving surface): `registerToolHandler`
   * fills it; `invokeTool` dispatches by exact capability id. The registry
   * MUST NOT mutate the manifest — descriptor truth for discovery stays in
   * the manifest's `tools[]`; a registry/manifest mismatch is a provider
   * bug caught by `validateManifestTools`.
   */
  readonly #toolHandlers: Map<string, ToolHandler>;

  constructor(
    storeOrSeed?: MemoryStore | MemoryStoreSeed,
    options: { handlers?: Map<string, ToolHandler> } = {},
  ) {
    if (storeOrSeed instanceof MemoryStore) {
      this.store = storeOrSeed;
    } else if (storeOrSeed !== undefined) {
      this.store = new MemoryStore(storeOrSeed);
    } else {
      this.store = new MemoryStore();
    }
    // Explicit handler registry (advanced/test use): pass an empty map to
    // build a provider whose manifest declares tools but serves none — the
    // declared-but-unregistered provider-bug state. Defaults to the two
    // frozen toy-world handlers.
    this.#toolHandlers = options.handlers ?? defaultToyWorldToolHandlers(this.store);
  }

  /** Construct with committed kb / rel / evt / rule / fnd fixtures loaded. */
  static withCommittedFixtures(): ToyWorldAdapter {
    return new ToyWorldAdapter(MemoryStore.fromCommittedFixtures());
  }

  async getKnowledgeEntry(entryId: string): Promise<SpokeResult<KnowledgeEntry>> {
    return this.store.getKnowledgeEntry(entryId);
  }

  async putKnowledgeEntry(
    entry: KnowledgeEntry,
    expectedBaseRevision: number | null,
  ): Promise<SpokeResult<KnowledgeEntry>> {
    return this.store.putKnowledgeEntry(entry, expectedBaseRevision);
  }

  async getRelation(relationId: string): Promise<SpokeResult<Relation>> {
    return this.store.getRelation(relationId);
  }

  async putRelation(
    relation: Relation,
    expectedBaseRevision: number | null,
  ): Promise<SpokeResult<Relation>> {
    return this.store.putRelation(relation, expectedBaseRevision);
  }

  async listKnowledgeEntries(_scope: Scope): Promise<SpokeResult<KnowledgeEntry[]>> {
    return this.store.listKnowledgeEntries();
  }

  async listTimelineEvents(_scope: Scope): Promise<SpokeResult<TimelineEvent[]>> {
    return this.store.listTimelineEvents();
  }

  async putFindings(findings: Finding[]): Promise<SpokeResult<Finding[]>> {
    return this.store.putFindings(findings);
  }

  async listRules(ruleRefs: string[]): Promise<SpokeResult<Rule[]>> {
    return this.store.listRules(ruleRefs);
  }

  async getHostCapabilityManifest(): Promise<SpokeResult<HostCapabilityManifest>> {
    return spokeOk(cloneHostCapabilityManifest(TOY_WORLD_SELF_MANIFEST));
  }

  async listPeerHostCapabilityManifests(): Promise<
    SpokeResult<HostCapabilityManifest[]>
  > {
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
  async project(request: ProjectRequest): Promise<SpokeResult<ProjectResponse>> {
    const fixture = loadOpFixture<ProjectResponse>("op_tw_project_response.json");
    if ("error" in fixture) {
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
  async compute(request: ComputeRequest): Promise<SpokeResult<ComputeResponse>> {
    const fixture = loadOpFixture<ComputeResponse>(
      "op_tw_compute_settle_response.json",
    );
    if ("error" in fixture) {
      return spokeReject(
        SpokeRejectCode.INVALID_INPUT,
        `fixture compute response is an error envelope: ${fixture.error.message}`,
      );
    }
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
  async listForkTimelineEvents(
    scope: Scope & { fork_id: ForkId },
  ): Promise<SpokeResult<TimelineEvent[]>> {
    const events = this.store.events.filter(
      (event) => event.fork_id === scope.fork_id,
    );
    return spokeOk(events);
  }

  // ── Tool serving (reference provider surface) ──────────────────────────

  /**
   * List the tools this provider declares, in manifest order. Returns a
   * defensive copy (parity with `listTools` — mutating the result does not
   * mutate the manifest).
   */
  toolDescriptors(): ToolDescriptor[] {
    return listTools(TOY_WORLD_SELF_MANIFEST);
  }

  /**
   * Register a handler for a `tools.<ns>.<tool_id>` capability (plan-3
   * surface parity). Grammar-asserted: a non-`tools.` id throws (provider
   * misuse, mirroring `RemoteAdapter.registerToolHandler`). Duplicate
   * registration OVERWRITES the previous handler (last-wins, documented).
   * The registry does NOT mutate the manifest.
   */
  registerToolHandler(capabilityId: string, handler: ToolHandler): void {
    const parsed = parseToolCapabilityId(capabilityId);
    if (!parsed.ok) {
      throw new Error(parsed.message);
    }
    this.#toolHandlers.set(capabilityId, handler);
  }

  /**
   * Invoke a declared tool by capability id. Grammar gate, then descriptor
   * lookup in the self manifest, then the structural argument gate
   * (`validateToolArguments`), then dispatch to the registered handler.
   * A tool that is not listed in the manifest rejects with
   * `CAPABILITY_PORT_MISSING` (no silent success); a declared tool without
   * a registered handler is a provider bug and rejects the same way.
   */
  async invokeTool(
    capabilityId: string,
    args: Record<string, unknown>,
  ): Promise<SpokeResult<unknown>> {
    const parsed = parseToolCapabilityId(capabilityId);
    if (!parsed.ok) {
      return parsed;
    }
    const descriptor = findTool(TOY_WORLD_SELF_MANIFEST, capabilityId);
    if (descriptor === undefined) {
      return spokeReject(
        SpokeRejectCode.CAPABILITY_PORT_MISSING,
        `Tool "${capabilityId}" is not declared in the toy-world manifest (tools[])`,
        { capability: capabilityId },
      );
    }
    const argsCheck = validateToolArguments(descriptor, args);
    if (!argsCheck.ok) {
      return argsCheck;
    }
    const handler = this.#toolHandlers.get(capabilityId);
    if (handler === undefined) {
      return spokeReject(
        SpokeRejectCode.CAPABILITY_PORT_MISSING,
        `Tool "${capabilityId}" is declared but has no registered handler`,
        { capability: capabilityId },
      );
    }
    return handler(args);
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
    getRelation: (relationId) => adapter.getRelation(relationId),
    putRelation: (relation, expectedBaseRevision) =>
      adapter.putRelation(relation, expectedBaseRevision),
    listKnowledgeEntries: (scope) => adapter.listKnowledgeEntries(scope),
    listTimelineEvents: (scope) => adapter.listTimelineEvents(scope),
    putFindings: (findings) => adapter.putFindings(findings),
    listRules: (ruleRefs) => adapter.listRules(ruleRefs),
    getHostCapabilityManifest: () => adapter.getHostCapabilityManifest(),
    listPeerHostCapabilityManifests: () =>
      adapter.listPeerHostCapabilityManifests(),
  };
}
