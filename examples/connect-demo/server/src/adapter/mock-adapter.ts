/**
 * BaselinePorts adapter backed by the deterministic mock inference engine.
 *
 * Serves the demo server's `spoke-baseline` capability families: knowledge /
 * relation persistence with OCC, scope query (demo-harbor namespace), finding
 * persistence, rule query, and the host manifest surface. Port shapes mirror
 * the toy-world reference adapter; the engine owns all storage and
 * derivation.
 */

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
  filterKnowledgeEntriesByScope,
  filterTimelineEventsByScope,
  spokeOk,
  type BaselinePorts,
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
 * Server self-manifest (verbatim per plan) — served by
 * getHostCapabilityManifest. The tool capability ids are listed so the
 * client's reverse-invoked tools are negotiated (the negotiated set is the
 * intersection of both manifests' capabilities); the host declares the same
 * descriptors the client serves, and `validateManifestTools` passes on this
 * manifest.
 */
export const DEMO_SERVER_MANIFEST: HostCapabilityManifest = {
  schema_version: 1,
  host_id: "demo-inference-host",
  roles: ["checker", "assembler"],
  capabilities: [
    "spoke-baseline",
    TOY_WORLD_ROLL_DICE_ID,
    TOY_WORLD_LORE_LOOKUP_ID,
  ],
  namespaces: [DEMO_SCOPE_ID, TOY_WORLD_NAMESPACE],
  tools: [ROLL_DICE_DESCRIPTOR, LORE_LOOKUP_DESCRIPTOR],
  extensions: {},
};

export class MockAdapter implements BaselinePorts {
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

  async putFindings(findings: Finding[]): Promise<SpokeResult<Finding[]>> {
    return this.engine.putFindings(findings);
  }

  async listRules(ruleRefs: string[]): Promise<SpokeResult<Rule[]>> {
    return this.engine.listRules(ruleRefs);
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
