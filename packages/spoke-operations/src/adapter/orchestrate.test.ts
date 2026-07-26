import type {
  AssembleRequest,
  CheckRequest,
  Finding,
  KnowledgeEntry,
  PromoteRequest,
  RelateRequest,
  Relation,
  Rule,
  Scope,
  TimelineEvent,
  UpsertRequest,
} from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import {
  SpokeRejectCode,
  orchestrateAssemble,
  orchestrateCheck,
  orchestratePromote,
  orchestrateRelate,
  orchestrateUpsert,
  spokeOk,
  spokeReject,
  type BaselinePorts,
  type CheckRunInput,
  type SpokeResult,
} from "../index.js";

function makeKnowledgeEntry(
  overrides: Partial<KnowledgeEntry> & Pick<KnowledgeEntry, "entry_id">,
): KnowledgeEntry {
  return {
    schema_version: 1,
    entry_type: "character",
    canonical_name: "Mira Vale",
    status: "provisional",
    body: { summary: "Protagonist" },
    extensions: {},
    ...overrides,
  };
}

function makeRelation(
  overrides: Partial<Relation> & Pick<Relation, "relation_id">,
): Relation {
  return {
    schema_version: 1,
    relation_type: "related_to",
    from_id: "kb_a",
    to_id: "kb_b",
    extensions: {},
    ...overrides,
  };
}

function makeFinding(
  overrides: Partial<Finding> & Pick<Finding, "finding_id">,
): Finding {
  return {
    schema_version: 1,
    severity: "warning",
    status: "open",
    title: "Issue",
    description: "Detected by mock checker",
    extensions: {},
    ...overrides,
  };
}

function makeRule(
  overrides: Partial<Rule> & Pick<Rule, "rule_id">,
): Rule {
  return {
    schema_version: 1,
    canonical_name: "No orphans",
    kind: "rule",
    extensions: {},
    ...overrides,
  };
}

function makeTimelineEvent(
  overrides: Partial<TimelineEvent> & Pick<TimelineEvent, "timeline_event_id">,
): TimelineEvent {
  return {
    schema_version: 1,
    canonical_name: "Arrival",
    extensions: {},
    ...overrides,
  };
}

/** In-memory BaselinePorts for orchestration tests (no I/O). */
function createMemoryBaselinePorts(seed?: {
  entries?: KnowledgeEntry[];
  relations?: Relation[];
  events?: TimelineEvent[];
  rules?: Rule[];
}): BaselinePorts & {
  store: {
    entries: Map<string, KnowledgeEntry>;
    relations: Map<string, Relation>;
    events: TimelineEvent[];
    rules: Map<string, Rule>;
    findings: Finding[];
  };
} {
  const entries = new Map(
    (seed?.entries ?? []).map((entry) => [entry.entry_id, entry]),
  );
  const relations = new Map(
    (seed?.relations ?? []).map((relation) => [relation.relation_id, relation]),
  );
  const events = [...(seed?.events ?? [])];
  const rules = new Map((seed?.rules ?? []).map((rule) => [rule.rule_id, rule]));
  const findings: Finding[] = [];

  const ports: BaselinePorts = {
    getKnowledgeEntry(entryId: string): SpokeResult<KnowledgeEntry> {
      const entry = entries.get(entryId);
      if (entry === undefined) {
        return spokeReject(
          SpokeRejectCode.KNOWLEDGE_ENTRY_NOT_FOUND,
          `KnowledgeEntry not found: ${entryId}`,
          { entry_id: entryId },
        );
      }
      return spokeOk(entry);
    },
    putKnowledgeEntry(entry: KnowledgeEntry): SpokeResult<KnowledgeEntry> {
      entries.set(entry.entry_id, entry);
      return spokeOk(entry);
    },
    putRelation(relation: Relation): SpokeResult<Relation> {
      relations.set(relation.relation_id, relation);
      return spokeOk(relation);
    },
    listKnowledgeEntries(_scope: Scope): SpokeResult<KnowledgeEntry[]> {
      return spokeOk([...entries.values()]);
    },
    listTimelineEvents(_scope: Scope): SpokeResult<TimelineEvent[]> {
      return spokeOk([...events]);
    },
    putFindings(next: Finding[]): SpokeResult<Finding[]> {
      findings.push(...next);
      return spokeOk(next);
    },
    listRules(ruleRefs: string[]): SpokeResult<Rule[]> {
      const resolved: Rule[] = [];
      for (const ref of ruleRefs) {
        const rule = rules.get(ref);
        if (rule === undefined) {
          return spokeReject(
            SpokeRejectCode.INVALID_INPUT,
            `Rule not found: ${ref}`,
            { rule_ref: ref },
          );
        }
        resolved.push(rule);
      }
      return spokeOk(resolved);
    },
  };

  return { ...ports, store: { entries, relations, events, rules, findings } };
}

describe("baseline orchestration", () => {
  it("orchestrateUpsert creates a KnowledgeEntry through ports", () => {
    const ports = createMemoryBaselinePorts();
    const candidate = makeKnowledgeEntry({ entry_id: "kb_new" });
    const request: UpsertRequest = { knowledge_entries: [candidate] };

    const result = orchestrateUpsert(ports, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value).toEqual({
      knowledge_entries: [candidate],
    });
    expect(ports.store.entries.get("kb_new")).toEqual(candidate);
  });

  it("orchestratePromote persists confirmed KnowledgeEntry", () => {
    const candidate = makeKnowledgeEntry({
      entry_id: "kb_promote",
      status: "provisional",
      revision: 0,
    });
    const ports = createMemoryBaselinePorts();
    const request: PromoteRequest = { candidate };

    const result = orchestratePromote(ports, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.knowledge_entry.status).toBe("confirmed");
    expect(result.value.knowledge_entry.revision).toBe(1);
    expect(ports.store.entries.get("kb_promote")?.status).toBe("confirmed");
  });

  it("orchestrateRelate persists a Relation", () => {
    const ports = createMemoryBaselinePorts();
    const relation = makeRelation({ relation_id: "rel_1" });
    const request: RelateRequest = { relation };

    const result = orchestrateRelate(ports, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value).toEqual({ relation });
    expect(ports.store.relations.get("rel_1")).toEqual(relation);
  });

  it("orchestrateCheck loads scope data, runs checker, and puts findings", () => {
    const entry = makeKnowledgeEntry({ entry_id: "kb_check" });
    const event = makeTimelineEvent({ timeline_event_id: "te_1" });
    const rule = makeRule({ rule_id: "rule_1" });
    const ports = createMemoryBaselinePorts({
      entries: [entry],
      events: [event],
      rules: [rule],
    });
    const request: CheckRequest = {
      scope: { scope_id: "world_1", entry_ids: ["kb_check"] },
      rule_refs: ["rule_1"],
    };
    const finding = makeFinding({ finding_id: "f_1", target_entry_id: "kb_check" });

    const result = orchestrateCheck(ports, request, (input: CheckRunInput) => {
      expect(input.request).toEqual(request);
      expect(input.entries).toEqual([entry]);
      expect(input.events).toEqual([event]);
      expect(input.rules).toEqual([rule]);
      return spokeOk([finding]);
    });

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value).toEqual({ findings: [finding] });
    expect(ports.store.findings).toEqual([finding]);
  });

  it("orchestrateAssemble builds a packet from scoped KnowledgeEntries", () => {
    const entry = makeKnowledgeEntry({
      entry_id: "kb_assemble",
      canonical_name: "Assemble Hero",
      body: { summary: "Context snippet" },
    });
    const ports = createMemoryBaselinePorts({ entries: [entry] });
    const request: AssembleRequest = {
      scope: { scope_id: "world_1", entry_ids: ["kb_assemble"] },
      max_entries: 10,
    };

    const result = orchestrateAssemble(ports, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.packet.packet_id).toBe("assemble:world_1");
    expect(result.value.packet.entries).toEqual([
      {
        entry_id: "kb_assemble",
        entry_type: "character",
        canonical_name: "Assemble Hero",
        snippet: "Context snippet",
      },
    ]);
  });
});
