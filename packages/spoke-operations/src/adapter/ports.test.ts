import { describe, expect, it } from "vitest";

import type {
  Finding,
  KnowledgeEntry,
  Relation,
  Rule,
  Scope,
  TimelineEvent,
  ProjectRequest,
  ProjectResponse,
  ComputeRequest,
  ComputeResponse,
  ForkId,
} from "@42ch/spoke-schemas";

import {
  SpokeRejectCode,
  spokeOk,
  type SpokeResult,
  type KnowledgeEntryPort,
  type RelationPort,
  type ScopeQueryPort,
  type FindingPort,
  type RuleQueryPort,
  type ComputablePort,
  type ForkTimelineQueryPort,
  type BaselinePorts,
  type ComputablePorts,
  type ForkPorts,
  type FullPorts,
  type BaselineAdapter,
  type ComputableAdapter,
  type ForkAdapter,
  type FullAdapter,
} from "../index.js";

describe("adapter port exports", () => {
  it("exports CAPABILITY_PORT_MISSING as the 20th SpokeRejectCode", () => {
    expect(SpokeRejectCode.CAPABILITY_PORT_MISSING).toBe(
      "CAPABILITY_PORT_MISSING",
    );
    expect(Object.keys(SpokeRejectCode)).toHaveLength(20);
  });

  it("BaselinePorts accepts an object implementing all five baseline families", () => {
    const ports: BaselinePorts = {
      getKnowledgeEntry(_entryId: string): SpokeResult<KnowledgeEntry> {
        return spokeOk({} as KnowledgeEntry);
      },
      putKnowledgeEntry(
        entry: KnowledgeEntry,
        _expectedBaseRevision: number | null,
      ): SpokeResult<KnowledgeEntry> {
        return spokeOk(entry);
      },
      putRelation(relation: Relation): SpokeResult<Relation> {
        return spokeOk(relation);
      },
      listKnowledgeEntries(_scope: Scope): SpokeResult<KnowledgeEntry[]> {
        return spokeOk([]);
      },
      listTimelineEvents(_scope: Scope): SpokeResult<TimelineEvent[]> {
        return spokeOk([]);
      },
      putFindings(findings: Finding[]): SpokeResult<Finding[]> {
        return spokeOk(findings);
      },
      listRules(_ruleRefs: string[]): SpokeResult<Rule[]> {
        return spokeOk([]);
      },
    };

    expect(typeof ports.getKnowledgeEntry).toBe("function");
    expect(typeof ports.putRelation).toBe("function");
    expect(typeof ports.listKnowledgeEntries).toBe("function");
    expect(typeof ports.putFindings).toBe("function");
    expect(typeof ports.listRules).toBe("function");
  });

  it("ComputablePorts / ForkPorts / FullPorts compose optional families", () => {
    const baseline: BaselinePorts = {
      getKnowledgeEntry: () => spokeOk({} as KnowledgeEntry),
      putKnowledgeEntry: (entry, _expectedBaseRevision) => spokeOk(entry),
      putRelation: (relation) => spokeOk(relation),
      listKnowledgeEntries: () => spokeOk([]),
      listTimelineEvents: () => spokeOk([]),
      putFindings: (findings) => spokeOk(findings),
      listRules: () => spokeOk([]),
    };

    const computable: ComputablePort = {
      project(request: ProjectRequest): SpokeResult<ProjectResponse> {
        return spokeOk({} as ProjectResponse);
      },
      compute(request: ComputeRequest): SpokeResult<ComputeResponse> {
        return spokeOk({} as ComputeResponse);
      },
    };

    const fork: ForkTimelineQueryPort = {
      listForkTimelineEvents(
        _scope: Scope & { fork_id: ForkId },
      ): SpokeResult<TimelineEvent[]> {
        return spokeOk([]);
      },
    };

    const computablePorts: ComputablePorts = { ...baseline, ...computable };
    const forkPorts: ForkPorts = { ...baseline, ...fork };
    const fullPorts: FullPorts = { ...baseline, ...computable, ...fork };

    expect(typeof computablePorts.project).toBe("function");
    expect(typeof forkPorts.listForkTimelineEvents).toBe("function");
    expect(typeof fullPorts.compute).toBe("function");
    expect(typeof fullPorts.listForkTimelineEvents).toBe("function");
  });

  it("*Adapter aliases are assignable from matching *Ports compositions", () => {
    const baseline: BaselinePorts = {
      getKnowledgeEntry: () => spokeOk({} as KnowledgeEntry),
      putKnowledgeEntry: (entry, _expectedBaseRevision) => spokeOk(entry),
      putRelation: (relation) => spokeOk(relation),
      listKnowledgeEntries: () => spokeOk([]),
      listTimelineEvents: () => spokeOk([]),
      putFindings: (findings) => spokeOk(findings),
      listRules: () => spokeOk([]),
    };

    const computable: ComputablePort = {
      project: () => spokeOk({} as ProjectResponse),
      compute: () => spokeOk({} as ComputeResponse),
    };

    const fork: ForkTimelineQueryPort = {
      listForkTimelineEvents: () => spokeOk([]),
    };

    const baselineAdapter: BaselineAdapter = baseline;
    const computableAdapter: ComputableAdapter = { ...baseline, ...computable };
    const forkAdapter: ForkAdapter = { ...baseline, ...fork };
    const fullAdapter: FullAdapter = { ...baseline, ...computable, ...fork };

    expect(typeof baselineAdapter.getKnowledgeEntry).toBe("function");
    expect(typeof computableAdapter.project).toBe("function");
    expect(typeof forkAdapter.listForkTimelineEvents).toBe("function");
    expect(typeof fullAdapter.compute).toBe("function");
  });

  it("individual port interfaces are assignable by method shape", () => {
    const knowledge: KnowledgeEntryPort = {
      getKnowledgeEntry: () => spokeOk({} as KnowledgeEntry),
      putKnowledgeEntry: (entry, _expectedBaseRevision) => spokeOk(entry),
    };
    const relation: RelationPort = {
      putRelation: (r) => spokeOk(r),
    };
    const scope: ScopeQueryPort = {
      listKnowledgeEntries: () => spokeOk([]),
      listTimelineEvents: () => spokeOk([]),
    };
    const finding: FindingPort = {
      putFindings: (f) => spokeOk(f),
    };
    const rule: RuleQueryPort = {
      listRules: () => spokeOk([]),
    };

    expect(knowledge).toBeDefined();
    expect(relation).toBeDefined();
    expect(scope).toBeDefined();
    expect(finding).toBeDefined();
    expect(rule).toBeDefined();
  });
});
