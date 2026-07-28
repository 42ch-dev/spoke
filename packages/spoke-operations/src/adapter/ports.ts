/**
 * Capability-sliced adapter port contracts for injection orchestration.
 * Synchronous SpokeResult surface — adapters own async I/O behind this boundary.
 */

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

import type { SpokeResult } from "../result.js";

/** Knowledge entry persistence — get / put by entry id. */
export interface KnowledgeEntryPort {
  getKnowledgeEntry(entryId: string): SpokeResult<KnowledgeEntry>;
  /**
   * Persist a KnowledgeEntry with optimistic concurrency control.
   *
   * Adapters MUST treat `expectedBaseRevision` as the store’s required current
   * revision before accepting the write (conditional put / OCC / CAS).
   * `null` means the entry must be absent (create). A non-null value means the
   * store’s current revision for `entry.entry_id` MUST equal
   * `expectedBaseRevision`; otherwise reject with `STORED_REVISION_STALE` or
   * `REVISION_CONFLICT`. True concurrent safety requires atomic compare-and-put
   * in the adapter; the library stays I/O-free.
   */
  putKnowledgeEntry(
    entry: KnowledgeEntry,
    expectedBaseRevision: number | null,
  ): SpokeResult<KnowledgeEntry>;
}

/** Relation persistence — get / put by relation id. */
export interface RelationPort {
  getRelation(relationId: string): SpokeResult<Relation>;
  /**
   * Persist a Relation with optimistic concurrency control.
   *
   * Adapters MUST treat `expectedBaseRevision` as the store’s required current
   * revision before accepting the write (conditional put / OCC / CAS).
   * `null` means the relation must be absent (create). A non-null value means the
   * store’s current revision for `relation.relation_id` MUST equal
   * `expectedBaseRevision`; otherwise reject with `STORED_REVISION_STALE` or
   * `REVISION_CONFLICT`. True concurrent safety requires atomic compare-and-put
   * in the adapter; the library stays I/O-free.
   */
  putRelation(
    relation: Relation,
    expectedBaseRevision: number | null,
  ): SpokeResult<Relation>;
}

/** Scope query for check / assemble — knowledge entries and timeline events. */
export interface ScopeQueryPort {
  listKnowledgeEntries(scope: Scope): SpokeResult<KnowledgeEntry[]>;
  listTimelineEvents(scope: Scope): SpokeResult<TimelineEvent[]>;
}

/** Finding persistence. */
export interface FindingPort {
  putFindings(findings: Finding[]): SpokeResult<Finding[]>;
}

/** Rule query by reference list. */
export interface RuleQueryPort {
  listRules(ruleRefs: string[]): SpokeResult<Rule[]>;
}

/**
 * Host collaboration metadata — self manifest and product-known peer manifests.
 * Integrators call explicitly; orchestrators do not auto-fetch manifests.
 */
export interface HostManifestPort {
  getHostCapabilityManifest(): SpokeResult<HostCapabilityManifest>;
  listPeerHostCapabilityManifests(): SpokeResult<HostCapabilityManifest[]>;
}

/** Optional l2-computable session — project / compute. */
export interface ComputablePort {
  project(request: ProjectRequest): SpokeResult<ProjectResponse>;
  compute(request: ComputeRequest): SpokeResult<ComputeResponse>;
}

/**
 * Optional l5-fork timeline query.
 * Capability-specific refinement of ScopeQueryPort; one object MAY satisfy both.
 */
export interface ForkTimelineQueryPort {
  listForkTimelineEvents(
    scope: Scope & { fork_id: ForkId },
  ): SpokeResult<TimelineEvent[]>;
}

/** Ports required for spoke-baseline orchestration. */
export type BaselinePorts = KnowledgeEntryPort &
  RelationPort &
  ScopeQueryPort &
  FindingPort &
  RuleQueryPort &
  HostManifestPort;

/** Baseline plus optional computable capability. */
export type ComputablePorts = BaselinePorts & ComputablePort;

/** Baseline plus optional fork timeline capability. */
export type ForkPorts = BaselinePorts & ForkTimelineQueryPort;

/** Full composition of baseline, computable, and fork ports. */
export type FullPorts = BaselinePorts & ComputablePort & ForkTimelineQueryPort;

/** Ergonomic alias for baseline adapter composition. */
export type BaselineAdapter = BaselinePorts;

/** Ergonomic alias for baseline plus computable adapter composition. */
export type ComputableAdapter = ComputablePorts;

/** Ergonomic alias for baseline plus fork adapter composition. */
export type ForkAdapter = ForkPorts;

/** Ergonomic alias for full adapter composition. */
export type FullAdapter = FullPorts;
