/**
 * Capability-sliced adapter port contracts for injection orchestration.
 * Synchronous SpokeResult surface — adapters own async I/O behind this boundary.
 */

import type {
  ComputeRequest,
  ComputeResponse,
  Finding,
  ForkId,
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
  putKnowledgeEntry(entry: KnowledgeEntry): SpokeResult<KnowledgeEntry>;
}

/** Relation persistence. */
export interface RelationPort {
  putRelation(relation: Relation): SpokeResult<Relation>;
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
  RuleQueryPort;

/** Baseline plus optional computable capability. */
export type ComputablePorts = BaselinePorts & ComputablePort;

/** Baseline plus optional fork timeline capability. */
export type ForkPorts = BaselinePorts & ForkTimelineQueryPort;

/** Full composition of baseline, computable, and fork ports. */
export type FullPorts = BaselinePorts & ComputablePort & ForkTimelineQueryPort;
