/**
 * FullPorts adapter backed by the deterministic mock inference engine.
 *
 * Serves the demo server's `spoke-baseline` capability families: knowledge /
 * relation persistence with OCC, scope query (demo-harbor namespace), finding
 * persistence, rule query, and the host manifest surface — plus the optional
 * `l2-computable` (project / compute sessions) and `l5-fork`
 * (listForkTimelineEvents over the seeded storm fork) families. Port shapes
 * mirror the toy-world reference adapter; the engine owns all storage and
 * derivation.
 *
 * The adapter also serves the whole-operation `ke-extraction` face: `extract`
 * runs the library `orchestrateExtract` over a host-local deterministic
 * loader and extractor. It is a service on the injected ports object, not a
 * `port.*` catalogue method.
 */

import type {
  ComputeRequest,
  ComputeResponse,
  ExtractRequest,
  ExtractResponse,
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
  filterKnowledgeEntriesByScope,
  filterTimelineEventsByScope,
  orchestrateExtract,
  spokeOk,
  type ExtractionPort,
  type FullPorts,
  type RunExtractor,
  type SpokeResult,
} from "@42ch/spoke-operations";

import { MockEngine } from "../engine/mock-engine.js";
import { DEMO_SCOPE_ID } from "../engine/seed-corpus.js";
import {
  LORE_LOOKUP_DESCRIPTOR,
  ROLL_DICE_DESCRIPTOR,
  TOY_WORLD_LORE_LOOKUP_ID,
  TOY_WORLD_NAMESPACE,
  TOY_WORLD_ROLL_DICE_ID,
} from "../tools/toy-world-tools.js";

/**
 * Loader-only sentinel: a value that exists solely inside the host-local
 * loaded input. It never crosses the wire, and the integration suite scans
 * the captured request/response frames for it.
 */
export const DEMO_EXTRACTION_CANARY = "demo-loader-only-canary";

/** Advisory extraction method name carried in `run.method` (open vocabulary). */
export const DEMO_EXTRACTION_METHOD = "demo.rule-based";

/** Host-local loaded value — never a request/response wire object. */
interface DemoLoadedExtractionInput {
  canary: string;
  resolved_source_ids: string[];
}

/**
 * Deterministic candidate id: a pure function of the request (`run_id` plus
 * the source index), so a repeated request yields identical candidates and
 * nothing depends on wall-clock or random state.
 */
export function demoExtractCandidateEntryId(
  runId: string,
  index: number,
): string {
  return `demo-harbor/extracted/${runId}/${index + 1}`;
}

/**
 * Demo source loader. The demo holds no external source store, so resolving
 * the referenced sources is the deterministic mapping of the request anchors
 * onto their `source_id`s; a product loader fetches the referenced material
 * here. The returned value stays host-local to the `ExtractionPort`.
 */
const DEMO_EXTRACTION_PORT: ExtractionPort = {
  async loadExtractionInput(request) {
    const loaded: DemoLoadedExtractionInput = {
      canary: DEMO_EXTRACTION_CANARY,
      resolved_source_ids: request.sources.map((anchor) => anchor.source_id),
    };
    return spokeOk(loaded);
  },
};

/**
 * Demo extractor: one provisional candidate per referenced source, derived
 * from the loaded reference list plus the request's `run_id`. Client-supplied
 * content is never echoed — a candidate carries the request anchor as a
 * pointer and the loaded canary never enters a candidate.
 */
const runDemoExtractor: RunExtractor = async ({ request, input }) => {
  const loaded = input as DemoLoadedExtractionInput;
  const candidates: KnowledgeEntry[] = request.sources.map((anchor, index) => ({
    schema_version: 1,
    entry_id: demoExtractCandidateEntryId(request.run_id, index),
    entry_type: "note",
    canonical_name: anchor.label ?? anchor.source_id,
    status: "provisional",
    body: {
      summary: `Candidate ${index + 1} resolved from ${loaded.resolved_source_ids[index]} (run ${request.run_id}).`,
    },
    source_anchor: anchor,
    extensions: {},
  }));

  return spokeOk({ candidates, method: DEMO_EXTRACTION_METHOD });
};

/**
 * Server self-manifest (verbatim per plan) — served by
 * getHostCapabilityManifest. The tool capability ids are listed so the
 * client's reverse-invoked tools are negotiated (the negotiated set is the
 * intersection of both manifests' capabilities); the host declares the same
 * descriptors the client serves, and `validateManifestTools` passes on this
 * manifest. The optional `l2-computable` / `l5-fork` families are declared
 * because the provider serves them through the ports face (the e2e's
 * undeclared-capability deny uses a variant of this manifest). `ke-extraction`
 * and `ke-ownership` are declared because the host serves the extract service
 * and honors viewpoint-bearing Scope requests; `input-source` records the
 * offering role for extraction. Roles are not capabilities — the two flags are
 * independent of them.
 */
export const DEMO_SERVER_MANIFEST: HostCapabilityManifest = {
  schema_version: 1,
  host_id: "demo-inference-host",
  roles: ["checker", "assembler", "input-source"],
  capabilities: [
    "spoke-baseline",
    TOY_WORLD_ROLL_DICE_ID,
    TOY_WORLD_LORE_LOOKUP_ID,
    "l2-computable",
    "l5-fork",
    "ke-extraction",
    "ke-ownership",
  ],
  namespaces: [DEMO_SCOPE_ID, TOY_WORLD_NAMESPACE],
  tools: [ROLL_DICE_DESCRIPTOR, LORE_LOOKUP_DESCRIPTOR],
  extensions: {},
};

export class MockAdapter implements FullPorts {
  readonly engine: MockEngine;

  constructor(engine?: MockEngine) {
    this.engine = engine ?? new MockEngine();
  }

  async getKnowledgeEntry(entryId: string): Promise<SpokeResult<KnowledgeEntry>> {
    return this.engine.getKnowledgeEntry(entryId);
  }

  async putKnowledgeEntry(
    entry: KnowledgeEntry,
    expectedBaseRevision: number | null,
  ): Promise<SpokeResult<KnowledgeEntry>> {
    return this.engine.putKnowledgeEntry(entry, expectedBaseRevision);
  }

  async getRelation(relationId: string): Promise<SpokeResult<Relation>> {
    return this.engine.getRelation(relationId);
  }

  async putRelation(
    relation: Relation,
    expectedBaseRevision: number | null,
  ): Promise<SpokeResult<Relation>> {
    return this.engine.putRelation(relation, expectedBaseRevision);
  }

  async listKnowledgeEntries(
    scope: Scope,
  ): Promise<SpokeResult<KnowledgeEntry[]>> {
    if (scope.scope_id !== undefined && scope.scope_id !== DEMO_SCOPE_ID) {
      return spokeOk([]);
    }
    return spokeOk(
      filterKnowledgeEntriesByScope(this.engine.listKnowledgeEntries(), scope),
    );
  }

  async listTimelineEvents(scope: Scope): Promise<SpokeResult<TimelineEvent[]>> {
    if (scope.scope_id !== undefined && scope.scope_id !== DEMO_SCOPE_ID) {
      return spokeOk([]);
    }
    return spokeOk(
      filterTimelineEventsByScope(this.engine.listTimelineEvents(), scope),
    );
  }

  // ── Optional families (served through the same ports face) ─────────────

  /**
   * l2-computable projection — materialize the session's computable view
   * from static state (engine-owned session store).
   */
  async project(request: ProjectRequest): Promise<SpokeResult<ProjectResponse>> {
    return this.engine.projectComputable(request);
  }

  /**
   * l2-computable apply/settle — merge the computable delta into the
   * session view; `settle: true` merges the view back into static state.
   */
  async compute(request: ComputeRequest): Promise<SpokeResult<ComputeResponse>> {
    return this.engine.computeComputable(request);
  }

  /**
   * l5-fork timeline query — the fork_id-scoped refinement of
   * `listTimelineEvents`, served through the library scope matcher (no
   * protocol-rule reimplementation). One provider satisfies both the
   * ScopeQueryPort and ForkTimelineQueryPort contracts.
   */
  async listForkTimelineEvents(
    scope: Scope & { fork_id: ForkId },
  ): Promise<SpokeResult<TimelineEvent[]>> {
    if (scope.scope_id !== undefined && scope.scope_id !== DEMO_SCOPE_ID) {
      return spokeOk([]);
    }
    return spokeOk(
      filterTimelineEventsByScope(this.engine.listTimelineEvents(), scope),
    );
  }

  async putFindings(findings: Finding[]): Promise<SpokeResult<Finding[]>> {
    return this.engine.putFindings(findings);
  }

  async listRules(ruleRefs: string[]): Promise<SpokeResult<Rule[]>> {
    return this.engine.listRules(ruleRefs);
  }

  // ── ke-extraction (whole-operation service, not a `port.*` method) ─────

  /**
   * Host-local extraction service: the library `orchestrateExtract` gates the
   * request, loads the referenced sources through the host-local port, runs
   * the deterministic extractor once, and assembles the response. The loaded
   * value stays here — neither the loader nor the extractor is reachable from
   * a peer.
   */
  async extract(
    request: ExtractRequest,
  ): Promise<SpokeResult<ExtractResponse>> {
    return orchestrateExtract(DEMO_EXTRACTION_PORT, request, runDemoExtractor);
  }

  async getHostCapabilityManifest(): Promise<SpokeResult<HostCapabilityManifest>> {
    return spokeOk(structuredClone(DEMO_SERVER_MANIFEST));
  }

  async listPeerHostCapabilityManifests(): Promise<
    SpokeResult<HostCapabilityManifest[]>
  > {
    // The demo inference host knows no peers; an empty list is valid.
    return spokeOk([]);
  }
}
