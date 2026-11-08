import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import type {
  AssembleRequest,
  CheckRequest,
  CognitionSelection,
  ComputeRequest,
  ComputeResponse,
  Finding,
  HostCapabilityManifest,
  KnowledgeEntry,
  ProjectRequest,
  ProjectResponse,
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
  orchestrateCompute,
  orchestrateForkAssemble,
  orchestrateForkCheck,
  orchestrateProject,
  orchestratePromote,
  orchestrateRelate,
  orchestrateUpsert,
  spokeOk,
  spokeReject,
  type BaselinePorts,
  type CheckRunInput,
  type ComputablePorts,
  type ForkPorts,
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

/** Enforce OCC on in-memory put — mirrors adapter CAS contract. */
function putKnowledgeEntryWithOcc(
  entries: Map<string, KnowledgeEntry>,
  entry: KnowledgeEntry,
  expectedBaseRevision: number | null,
): SpokeResult<KnowledgeEntry> {
  const existing = entries.get(entry.entry_id);
  if (expectedBaseRevision === null) {
    if (existing !== undefined) {
      return spokeReject(
        SpokeRejectCode.REVISION_CONFLICT,
        `Entry already exists: ${entry.entry_id}`,
        { entry_id: entry.entry_id },
      );
    }
  } else {
    if (existing === undefined) {
      return spokeReject(
        SpokeRejectCode.STORED_REVISION_STALE,
        `KnowledgeEntry not found for update: ${entry.entry_id}`,
        { entry_id: entry.entry_id, expectedBaseRevision },
      );
    }
    const currentRevision = existing.revision ?? 0;
    if (currentRevision !== expectedBaseRevision) {
      return spokeReject(
        SpokeRejectCode.STORED_REVISION_STALE,
        `Store revision ${currentRevision} does not match expected base ${expectedBaseRevision}`,
        { expectedBaseRevision, storeRevision: currentRevision },
      );
    }
  }
  entries.set(entry.entry_id, entry);
  return spokeOk(entry);
}

/**
 * Enforce OCC on in-memory relation put — mirrors adapter CAS contract.
 * The adapter owns revision assignment (seed 1 on create, bump +1 on update),
 * matching the toy-world fixture store and the RelationPort parity contract.
 */
function putRelationWithOcc(
  relations: Map<string, Relation>,
  relation: Relation,
  expectedBaseRevision: number | null,
): SpokeResult<Relation> {
  const existing = relations.get(relation.relation_id);
  if (expectedBaseRevision === null) {
    if (existing !== undefined) {
      return spokeReject(
        SpokeRejectCode.RELATION_ALREADY_EXISTS,
        `Relation already exists: ${relation.relation_id}`,
        { relation_id: relation.relation_id },
      );
    }
    const created: Relation = { ...relation, revision: 1 };
    relations.set(relation.relation_id, created);
    return spokeOk(created);
  }
  if (existing === undefined) {
    return spokeReject(
      SpokeRejectCode.STORED_REVISION_STALE,
      `Relation not found for update: ${relation.relation_id}`,
      { relation_id: relation.relation_id, expectedBaseRevision },
    );
  }
  const currentRevision = existing.revision ?? 0;
  if (currentRevision !== expectedBaseRevision) {
    return spokeReject(
      SpokeRejectCode.STORED_REVISION_STALE,
      `Store revision ${currentRevision} does not match expected base ${expectedBaseRevision}`,
      { expectedBaseRevision, storeRevision: currentRevision },
    );
  }
  const updated: Relation = { ...relation, revision: currentRevision + 1 };
  relations.set(relation.relation_id, updated);
  return spokeOk(updated);
}

function makeMemorySelfManifest(): HostCapabilityManifest {
  return {
    schema_version: 1,
    host_id: "memory-baseline-host",
    roles: ["data-store"],
    capabilities: ["spoke-baseline"],
    namespaces: ["memory"],
    extensions: {},
  };
}

function normalizePeerManifests(
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

function createMemoryBaselinePorts(seed?: {
  entries?: KnowledgeEntry[];
  relations?: Relation[];
  events?: TimelineEvent[];
  rules?: Rule[];
  selfManifest?: HostCapabilityManifest;
  peerManifests?: HostCapabilityManifest[];
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
  const selfManifest = seed?.selfManifest ?? makeMemorySelfManifest();
  const peerManifests = seed?.peerManifests ?? [];

  const ports: BaselinePorts = {
    async getKnowledgeEntry(
      entryId: string,
    ): Promise<SpokeResult<KnowledgeEntry>> {
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
    async putKnowledgeEntry(
      entry: KnowledgeEntry,
      expectedBaseRevision: number | null,
    ): Promise<SpokeResult<KnowledgeEntry>> {
      return putKnowledgeEntryWithOcc(entries, entry, expectedBaseRevision);
    },
    async getRelation(relationId: string): Promise<SpokeResult<Relation>> {
      const relation = relations.get(relationId);
      if (relation === undefined) {
        return spokeReject(
          SpokeRejectCode.RELATION_NOT_FOUND,
          `Relation not found: ${relationId}`,
          { relation_id: relationId },
        );
      }
      return spokeOk(relation);
    },
    async putRelation(
      relation: Relation,
      expectedBaseRevision: number | null,
    ): Promise<SpokeResult<Relation>> {
      return putRelationWithOcc(relations, relation, expectedBaseRevision);
    },
    async listKnowledgeEntries(
      _scope: Scope,
    ): Promise<SpokeResult<KnowledgeEntry[]>> {
      return spokeOk([...entries.values()]);
    },
    async listTimelineEvents(
      _scope: Scope,
    ): Promise<SpokeResult<TimelineEvent[]>> {
      return spokeOk([...events]);
    },
    async putFindings(next: Finding[]): Promise<SpokeResult<Finding[]>> {
      findings.push(...next);
      return spokeOk(next);
    },
    async listRules(ruleRefs: string[]): Promise<SpokeResult<Rule[]>> {
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
    async getHostCapabilityManifest(): Promise<
      SpokeResult<HostCapabilityManifest>
    > {
      return spokeOk(selfManifest);
    },
    async listPeerHostCapabilityManifests(): Promise<
      SpokeResult<HostCapabilityManifest[]>
    > {
      return spokeOk(
        normalizePeerManifests(selfManifest.host_id, peerManifests),
      );
    },
  };

  return { ...ports, store: { entries, relations, events, rules, findings } };
}

describe("baseline orchestration", () => {
  it("orchestrateUpsert creates a KnowledgeEntry through ports", async () => {
    const ports = createMemoryBaselinePorts();
    const candidate = makeKnowledgeEntry({ entry_id: "kb_new" });
    const request: UpsertRequest = { knowledge_entries: [candidate] };

    const result = await orchestrateUpsert(ports, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value).toEqual({
      knowledge_entries: [candidate],
    });
    expect(ports.store.entries.get("kb_new")).toEqual(candidate);
  });

  it("orchestratePromote persists confirmed KnowledgeEntry", async () => {
    const candidate = makeKnowledgeEntry({
      entry_id: "kb_promote",
      status: "provisional",
      revision: 0,
    });
    const ports = createMemoryBaselinePorts();
    const request: PromoteRequest = { candidate };

    const result = await orchestratePromote(ports, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.knowledge_entry.status).toBe("confirmed");
    expect(result.value.knowledge_entry.revision).toBe(1);
    expect(ports.store.entries.get("kb_promote")?.status).toBe("confirmed");
  });

  it("orchestratePromote rejects when stored status is terminal", async () => {
    const stored = makeKnowledgeEntry({
      entry_id: "kb_promote_terminal",
      status: "merged",
      revision: 1,
    });
    const candidate = makeKnowledgeEntry({
      entry_id: "kb_promote_terminal",
      status: "provisional",
      revision: 1,
    });
    const ports = createMemoryBaselinePorts({ entries: [stored] });

    const result = await orchestratePromote(ports, { candidate });

    expect(result.ok).toBe(false);
    if (result.ok) {
      return;
    }
    expect(result.code).toBe(SpokeRejectCode.KNOWLEDGE_ENTRY_TERMINAL_STATUS);
  });

  it("orchestratePromote rejects on stored revision mismatch", async () => {
    const stored = makeKnowledgeEntry({
      entry_id: "kb_promote_rev",
      status: "provisional",
      revision: 3,
    });
    const candidate = makeKnowledgeEntry({
      entry_id: "kb_promote_rev",
      status: "provisional",
      revision: 1,
    });
    const ports = createMemoryBaselinePorts({ entries: [stored] });

    const result = await orchestratePromote(ports, { candidate });

    expect(result.ok).toBe(false);
    if (result.ok) {
      return;
    }
    expect(result.code).toBe(SpokeRejectCode.STORED_REVISION_STALE);
  });

  it("orchestratePromote succeeds when stored provisional matches revision", async () => {
    const stored = makeKnowledgeEntry({
      entry_id: "kb_promote_match",
      status: "provisional",
      revision: 2,
    });
    const candidate = makeKnowledgeEntry({
      entry_id: "kb_promote_match",
      status: "provisional",
      revision: 2,
    });
    const ports = createMemoryBaselinePorts({ entries: [stored] });

    const result = await orchestratePromote(ports, { candidate });

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.knowledge_entry.status).toBe("confirmed");
    expect(result.value.knowledge_entry.revision).toBe(3);
  });

  it("orchestratePromote forces persisted revision to stored.revision + 1 when stored exists", async () => {
    const stored = makeKnowledgeEntry({
      entry_id: "kb_promote_force_rev",
      status: "provisional",
      revision: 7,
    });
    const candidate = makeKnowledgeEntry({
      entry_id: "kb_promote_force_rev",
      status: "provisional",
      revision: 7,
    });
    const puts: { entry: KnowledgeEntry; expectedBaseRevision: number | null }[] = [];
    const baseline = createMemoryBaselinePorts({ entries: [stored] });
    const ports: BaselinePorts = {
      ...baseline,
      async putKnowledgeEntry(
        entry: KnowledgeEntry,
        expectedBaseRevision: number | null,
      ): Promise<SpokeResult<KnowledgeEntry>> {
        puts.push({ entry, expectedBaseRevision });
        return baseline.putKnowledgeEntry(entry, expectedBaseRevision);
      },
    };

    const result = await orchestratePromote(ports, { candidate });

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(puts).toHaveLength(1);
    expect(puts[0]?.expectedBaseRevision).toBe(7);
    expect(puts[0]?.entry.revision).toBe(8);
    expect(result.value.knowledge_entry.revision).toBe(8);
  });

  it("orchestratePromote propagates adapter OCC reject when concurrent promote advanced revision", async () => {
    const snapshot = makeKnowledgeEntry({
      entry_id: "kb_promote_occ",
      status: "provisional",
      revision: 2,
    });
    let storeRevision = 2;
    const baseline = createMemoryBaselinePorts({ entries: [snapshot] });
    const ports: BaselinePorts = {
      ...baseline,
      async getKnowledgeEntry(
        entryId: string,
      ): Promise<SpokeResult<KnowledgeEntry>> {
        const result = await baseline.getKnowledgeEntry(entryId);
        if (result.ok) {
          // Concurrent writer advances the store after our read snapshot.
          storeRevision = 3;
        }
        return result;
      },
      async putKnowledgeEntry(
        entry: KnowledgeEntry,
        expectedBaseRevision: number | null,
      ): Promise<SpokeResult<KnowledgeEntry>> {
        if (
          expectedBaseRevision !== null &&
          storeRevision !== expectedBaseRevision
        ) {
          return spokeReject(
            SpokeRejectCode.STORED_REVISION_STALE,
            `Store revision ${storeRevision} is ahead of expected base ${expectedBaseRevision}`,
            { expectedBaseRevision, storeRevision },
          );
        }
        storeRevision = entry.revision ?? 0;
        return baseline.putKnowledgeEntry(entry, expectedBaseRevision);
      },
    };
    const candidate = makeKnowledgeEntry({
      entry_id: "kb_promote_occ",
      status: "provisional",
      revision: 2,
    });

    const result = await orchestratePromote(ports, { candidate });

    expect(result.ok).toBe(false);
    if (result.ok) {
      return;
    }
    expect(result.code).toBe(SpokeRejectCode.STORED_REVISION_STALE);
  });

  it("orchestrateUpsert passes null expectedBaseRevision on create", async () => {
    const puts: { entry: KnowledgeEntry; expectedBaseRevision: number | null }[] = [];
    const baseline = createMemoryBaselinePorts();
    const ports: BaselinePorts = {
      ...baseline,
      async putKnowledgeEntry(
        entry: KnowledgeEntry,
        expectedBaseRevision: number | null,
      ): Promise<SpokeResult<KnowledgeEntry>> {
        puts.push({ entry, expectedBaseRevision });
        return baseline.putKnowledgeEntry(entry, expectedBaseRevision);
      },
    };
    const candidate = makeKnowledgeEntry({ entry_id: "kb_create_occ" });
    const request: UpsertRequest = { knowledge_entries: [candidate] };

    const result = await orchestrateUpsert(ports, request);

    expect(result.ok).toBe(true);
    expect(puts).toHaveLength(1);
    expect(puts[0]?.expectedBaseRevision).toBeNull();
  });

  it("orchestrateUpsert passes stored revision as expectedBaseRevision on update", async () => {
    const stored = makeKnowledgeEntry({
      entry_id: "kb_update_occ",
      revision: 4,
    });
    const puts: { entry: KnowledgeEntry; expectedBaseRevision: number | null }[] = [];
    const baseline = createMemoryBaselinePorts({ entries: [stored] });
    const ports: BaselinePorts = {
      ...baseline,
      async putKnowledgeEntry(
        entry: KnowledgeEntry,
        expectedBaseRevision: number | null,
      ): Promise<SpokeResult<KnowledgeEntry>> {
        puts.push({ entry, expectedBaseRevision });
        return baseline.putKnowledgeEntry(entry, expectedBaseRevision);
      },
    };
    const candidate = makeKnowledgeEntry({
      entry_id: "kb_update_occ",
      revision: 4,
      canonical_name: "Updated",
    });
    const request: UpsertRequest = { knowledge_entries: [candidate] };

    const result = await orchestrateUpsert(ports, request);

    expect(result.ok).toBe(true);
    expect(puts).toHaveLength(1);
    expect(puts[0]?.expectedBaseRevision).toBe(4);
  });

  it("orchestrateUpsert rejects concurrent update when expected base is stale", async () => {
    const stored = makeKnowledgeEntry({
      entry_id: "kb_upsert_occ",
      revision: 1,
    });
    let storeRevision = 1;
    const baseline = createMemoryBaselinePorts({ entries: [stored] });
    const ports: BaselinePorts = {
      ...baseline,
      async getKnowledgeEntry(
        entryId: string,
      ): Promise<SpokeResult<KnowledgeEntry>> {
        const result = await baseline.getKnowledgeEntry(entryId);
        if (result.ok) {
          storeRevision = 2;
        }
        return result;
      },
      async putKnowledgeEntry(
        entry: KnowledgeEntry,
        expectedBaseRevision: number | null,
      ): Promise<SpokeResult<KnowledgeEntry>> {
        if (
          expectedBaseRevision !== null &&
          storeRevision !== expectedBaseRevision
        ) {
          return spokeReject(
            SpokeRejectCode.STORED_REVISION_STALE,
            `Store revision ${storeRevision} does not match expected base ${expectedBaseRevision}`,
            { expectedBaseRevision, storeRevision },
          );
        }
        storeRevision = entry.revision ?? 0;
        return baseline.putKnowledgeEntry(entry, expectedBaseRevision);
      },
    };
    const candidate = makeKnowledgeEntry({
      entry_id: "kb_upsert_occ",
      revision: 1,
      canonical_name: "Raced",
    });

    const result = await orchestrateUpsert(ports, {
      knowledge_entries: [candidate],
    });

    expect(result.ok).toBe(false);
    if (result.ok) {
      return;
    }
    expect(result.code).toBe(SpokeRejectCode.STORED_REVISION_STALE);
  });

  it("orchestrateRelate persists a Relation (create seeds revision 1)", async () => {
    const ports = createMemoryBaselinePorts();
    const relation = makeRelation({ relation_id: "rel_1" });
    const request: RelateRequest = { relation };

    const result = await orchestrateRelate(ports, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.relation.relation_id).toBe("rel_1");
    expect(result.value.relation.revision).toBe(1);
    expect(ports.store.relations.get("rel_1")?.revision).toBe(1);
  });

  it("orchestrateRelate passes null expectedBaseRevision on create", async () => {
    const puts: {
      relation: Relation;
      expectedBaseRevision: number | null;
    }[] = [];
    const baseline = createMemoryBaselinePorts();
    const ports: BaselinePorts = {
      ...baseline,
      async putRelation(
        relation: Relation,
        expectedBaseRevision: number | null,
      ): Promise<SpokeResult<Relation>> {
        puts.push({ relation, expectedBaseRevision });
        return baseline.putRelation(relation, expectedBaseRevision);
      },
    };
    const relation = makeRelation({ relation_id: "rel_create_occ" });
    const request: RelateRequest = { relation };

    const result = await orchestrateRelate(ports, request);

    expect(result.ok).toBe(true);
    expect(puts).toHaveLength(1);
    expect(puts[0]?.expectedBaseRevision).toBeNull();
  });

  it("orchestrateRelate passes stored revision as expectedBaseRevision on update", async () => {
    const stored = makeRelation({ relation_id: "rel_update_occ", revision: 4 });
    const puts: {
      relation: Relation;
      expectedBaseRevision: number | null;
    }[] = [];
    const baseline = createMemoryBaselinePorts({ relations: [stored] });
    const ports: BaselinePorts = {
      ...baseline,
      async putRelation(
        relation: Relation,
        expectedBaseRevision: number | null,
      ): Promise<SpokeResult<Relation>> {
        puts.push({ relation, expectedBaseRevision });
        return baseline.putRelation(relation, expectedBaseRevision);
      },
    };
    const candidate = makeRelation({
      relation_id: "rel_update_occ",
      revision: 4,
    });
    const request: RelateRequest = { relation: candidate };

    const result = await orchestrateRelate(ports, request);

    expect(result.ok).toBe(true);
    expect(puts).toHaveLength(1);
    expect(puts[0]?.expectedBaseRevision).toBe(4);
  });

  it("orchestrateRelate rejects update when store revision is stale (ahead)", async () => {
    const stored = makeRelation({ relation_id: "rel_stale", revision: 5 });
    let storeRevision = 5;
    const baseline = createMemoryBaselinePorts({ relations: [stored] });
    const ports: BaselinePorts = {
      ...baseline,
      async getRelation(relationId: string): Promise<SpokeResult<Relation>> {
        const result = await baseline.getRelation(relationId);
        if (result.ok) {
          // Concurrent writer advances the store after our read snapshot.
          storeRevision = 6;
        }
        return result;
      },
      async putRelation(
        relation: Relation,
        expectedBaseRevision: number | null,
      ): Promise<SpokeResult<Relation>> {
        if (
          expectedBaseRevision !== null &&
          storeRevision !== expectedBaseRevision
        ) {
          return spokeReject(
            SpokeRejectCode.STORED_REVISION_STALE,
            `Store revision ${storeRevision} does not match expected base ${expectedBaseRevision}`,
            { expectedBaseRevision, storeRevision },
          );
        }
        storeRevision = (relation.revision ?? 0) + 1;
        return baseline.putRelation(relation, expectedBaseRevision);
      },
    };
    const candidate = makeRelation({
      relation_id: "rel_stale",
      revision: 5,
    });

    const result = await orchestrateRelate(ports, { relation: candidate });

    expect(result.ok).toBe(false);
    if (result.ok) {
      return;
    }
    expect(result.code).toBe(SpokeRejectCode.STORED_REVISION_STALE);
  });

  it("orchestrateRelate rejects update when candidate revision conflicts (ahead of store)", async () => {
    const stored = makeRelation({ relation_id: "rel_conflict", revision: 2 });
    const baseline = createMemoryBaselinePorts({ relations: [stored] });
    // Candidate claims revision 7 while store holds 2 → validator returns
    // REVISION_CONFLICT (candidate ahead of stored) before the put.
    const candidate = makeRelation({
      relation_id: "rel_conflict",
      revision: 7,
    });

    const result = await orchestrateRelate(baseline, { relation: candidate });

    expect(result.ok).toBe(false);
    if (result.ok) {
      return;
    }
    expect(result.code).toBe(SpokeRejectCode.REVISION_CONFLICT);
  });

  it("orchestrateRelate propagates RELATION_ALREADY_EXISTS when create races an existing id", async () => {
    // The create-path `RELATION_ALREADY_EXISTS` can only surface through a
    // read-then-put CAS race: the orchestrator's `getRelation` snapshot missed
    // a concurrently-inserted row, so validate routes to create and the adapter
    // rejects `putRelation(existing, null)`. The orchestrator must propagate it.
    const stored = makeRelation({ relation_id: "rel_race", revision: 1 });
    const baseline = createMemoryBaselinePorts({ relations: [stored] });
    const ports: BaselinePorts = {
      ...baseline,
      async getRelation(relationId: string): Promise<SpokeResult<Relation>> {
        return spokeReject(
          SpokeRejectCode.RELATION_NOT_FOUND,
          `Stale snapshot missed: ${relationId}`,
          { relation_id: relationId },
        );
      },
    };
    // Create candidate carries no revision so validate chooses the create path.
    const candidate = makeRelation({ relation_id: "rel_race" });
    const request: RelateRequest = { relation: candidate };

    const result = await orchestrateRelate(ports, request);

    expect(result.ok).toBe(false);
    if (result.ok) {
      return;
    }
    expect(result.code).toBe(SpokeRejectCode.RELATION_ALREADY_EXISTS);
  });

  it("orchestrateRelate persisted relation carries bumped revision on update", async () => {
    const stored = makeRelation({ relation_id: "rel_bump", revision: 3 });
    const baseline = createMemoryBaselinePorts({ relations: [stored] });
    const candidate = makeRelation({
      relation_id: "rel_bump",
      revision: 3,
      label: "updated-label",
    });

    const result = await orchestrateRelate(baseline, { relation: candidate });

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.relation.revision).toBe(4);
    expect(result.value.relation.label).toBe("updated-label");
    expect(baseline.store.relations.get("rel_bump")?.revision).toBe(4);
  });

  it("orchestrateCheck loads scope data, runs checker, and puts findings", async () => {
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

    const result = await orchestrateCheck(ports, request, (input: CheckRunInput) => {
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

  it("orchestrateCheck lets embedded rules win by rule_id over refs", async () => {
    const entry = makeKnowledgeEntry({ entry_id: "kb_check_merge" });
    const storedRule = makeRule({
      rule_id: "rule_shared",
      canonical_name: "Stored rule",
      statement: "from-store",
    });
    const otherStored = makeRule({
      rule_id: "rule_other",
      canonical_name: "Other stored",
      statement: "keep-me",
    });
    const embeddedOverride = makeRule({
      rule_id: "rule_shared",
      canonical_name: "Embedded wins",
      statement: "from-embed",
    });
    const embeddedNew = makeRule({
      rule_id: "rule_new",
      canonical_name: "New embed",
      statement: "append-me",
    });
    const ports = createMemoryBaselinePorts({
      entries: [entry],
      rules: [storedRule, otherStored],
    });
    const request: CheckRequest = {
      scope: { scope_id: "world_1", entry_ids: ["kb_check_merge"] },
      rule_refs: ["rule_shared", "rule_other"],
      rules: [embeddedOverride, embeddedNew],
    };

    const result = await orchestrateCheck(ports, request, (input: CheckRunInput) => {
      expect(input.rules).toHaveLength(3);
      expect(input.rules.map((rule) => rule.rule_id)).toEqual([
        "rule_shared",
        "rule_other",
        "rule_new",
      ]);
      expect(input.rules[0]).toEqual(embeddedOverride);
      expect(input.rules[1]).toEqual(otherStored);
      expect(input.rules[2]).toEqual(embeddedNew);
      return spokeOk([]);
    });

    expect(result.ok).toBe(true);
  });

  it("orchestrateAssemble builds a packet from scoped KnowledgeEntries", async () => {
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

    const result = await orchestrateAssemble(ports, request);

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

function withComputablePorts(
  baseline: BaselinePorts,
): ComputablePorts & { projected: ProjectRequest[]; computed: ComputeRequest[] } {
  const projected: ProjectRequest[] = [];
  const computed: ComputeRequest[] = [];

  const ports: ComputablePorts = {
    ...baseline,
    async project(request: ProjectRequest): Promise<SpokeResult<ProjectResponse>> {
      projected.push(request);
      return spokeOk({
        session_id: request.session_id,
        entry_id: request.entry_id,
        computable: { ...request.state, projected: true },
      });
    },
    async compute(request: ComputeRequest): Promise<SpokeResult<ComputeResponse>> {
      computed.push(request);
      return spokeOk({
        session_id: request.session_id,
        entry_id: request.entry_id,
        computable: request.computable,
        ...(request.settle === true
          ? { state: { ...request.computable } }
          : {}),
      });
    },
  };

  return { ...ports, projected, computed };
}

function withForkPorts(
  baseline: ReturnType<typeof createMemoryBaselinePorts>,
): ForkPorts & { forkListCalls: Scope[] } {
  const forkListCalls: Scope[] = [];

  const ports: ForkPorts = {
    ...baseline,
    async listForkTimelineEvents(
      scope: Scope & { fork_id: string },
    ): Promise<SpokeResult<TimelineEvent[]>> {
      forkListCalls.push(scope);
      return spokeOk(
        baseline.store.events.filter((event) => event.fork_id === scope.fork_id),
      );
    },
  };

  return { ...ports, forkListCalls };
}

describe("optional computable orchestration", () => {
  it("orchestrateProject validates then calls ComputablePort.project", async () => {
    const ports = withComputablePorts(createMemoryBaselinePorts());
    const request: ProjectRequest = {
      session_id: "sess_1",
      entry_id: "kb_1",
      state: { tide_level: 2.4 },
    };

    const result = await orchestrateProject(ports, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value).toEqual({
      session_id: "sess_1",
      entry_id: "kb_1",
      computable: { tide_level: 2.4, projected: true },
    });
    expect(ports.projected).toEqual([request]);
  });

  it("orchestrateCompute validates then calls ComputablePort.compute", async () => {
    const ports = withComputablePorts(createMemoryBaselinePorts());
    const request: ComputeRequest = {
      session_id: "sess_1",
      entry_id: "kb_1",
      computable: { tide_level: 3.1 },
      settle: true,
    };

    const result = await orchestrateCompute(ports, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value).toEqual({
      session_id: "sess_1",
      entry_id: "kb_1",
      computable: { tide_level: 3.1 },
      state: { tide_level: 3.1 },
    });
    expect(ports.computed).toEqual([request]);
  });

  it("returns CAPABILITY_PORT_MISSING when project is absent at a dynamic boundary", async () => {
    const baseline = createMemoryBaselinePorts();
    const ports = baseline as unknown as ComputablePorts;
    const request: ProjectRequest = {
      session_id: "sess_1",
      entry_id: "kb_1",
      state: {},
    };

    const result = await orchestrateProject(ports, request);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
    }
  });
});

describe("optional fork orchestration", () => {
  it("orchestrateForkCheck loads KE via listKnowledgeEntries and events via listForkTimelineEvents", async () => {
    const entry = makeKnowledgeEntry({ entry_id: "kb_fork" });
    const onFork = makeTimelineEvent({
      timeline_event_id: "te_fork",
      fork_id: "fork_a",
    });
    const otherFork = makeTimelineEvent({
      timeline_event_id: "te_other",
      fork_id: "fork_b",
    });
    const baseline = createMemoryBaselinePorts({
      entries: [entry],
      events: [onFork, otherFork],
    });
    const ports = withForkPorts(baseline);
    const request: CheckRequest = {
      scope: {
        scope_id: "world_1",
        entry_ids: ["kb_fork"],
        fork_id: "fork_a",
      },
    };
    const finding = makeFinding({ finding_id: "f_fork" });

    const result = await orchestrateForkCheck(ports, request, (input) => {
      expect(input.entries).toEqual([entry]);
      expect(input.events).toEqual([onFork]);
      return spokeOk([finding]);
    });

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value).toEqual({ findings: [finding] });
    expect(ports.forkListCalls).toEqual([request.scope]);
    expect(baseline.store.findings).toEqual([finding]);
  });

  it("orchestrateForkAssemble uses fork timeline port and builds a packet", async () => {
    const entry = makeKnowledgeEntry({
      entry_id: "kb_fork_assemble",
      canonical_name: "Fork Hero",
      body: { summary: "On branch" },
    });
    const baseline = createMemoryBaselinePorts({
      entries: [entry],
      events: [
        makeTimelineEvent({
          timeline_event_id: "te_1",
          fork_id: "fork_a",
        }),
      ],
    });
    const ports = withForkPorts(baseline);
    const request: AssembleRequest = {
      scope: {
        scope_id: "world_1",
        entry_ids: ["kb_fork_assemble"],
        fork_id: "fork_a",
      },
    };

    const result = await orchestrateForkAssemble(ports, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.packet.packet_id).toBe("assemble:world_1");
    expect(result.value.packet.entries[0]?.canonical_name).toBe("Fork Hero");
    expect(ports.forkListCalls).toHaveLength(1);
  });

  it("returns CAPABILITY_PORT_MISSING when listForkTimelineEvents is absent", async () => {
    const baseline = createMemoryBaselinePorts();
    const ports = baseline as unknown as ForkPorts;
    const request: CheckRequest = {
      scope: { scope_id: "world_1", fork_id: "fork_a" },
    };

    const result = await orchestrateForkCheck(ports, request, () => spokeOk([]));

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.CAPABILITY_PORT_MISSING);
    }
  });
});

/** Shared entry plus one holder-private entry per holder, seeded in that order. */
function makeViewpointEntries(): KnowledgeEntry[] {
  return [
    makeKnowledgeEntry({ entry_id: "kb_shared" }),
    makeKnowledgeEntry({
      entry_id: "kb_mine",
      owner: "kb_mira",
      disclosure: "owner-private",
    }),
    makeKnowledgeEntry({
      entry_id: "kb_theirs",
      owner: "kb_rival",
      disclosure: "owner-private",
    }),
  ];
}

describe("ownership viewpoint orchestration", () => {
  it("orchestrateCheck supplies shared and own-private entries to the checker", async () => {
    const ports = createMemoryBaselinePorts({ entries: makeViewpointEntries() });
    const request: CheckRequest = {
      scope: { scope_id: "world_1", viewpoint: "kb_mira" },
    };
    let supplied: string[] = [];

    const result = await orchestrateCheck(
      ports,
      request,
      (input: CheckRunInput) => {
        supplied = input.entries.map((entry) => entry.entry_id);
        return spokeOk([]);
      },
    );

    expect(result.ok).toBe(true);
    expect(supplied).toEqual(["kb_shared", "kb_mine"]);
  });

  it("orchestrateCheck withholds owner-private entries without a scope viewpoint", async () => {
    const ports = createMemoryBaselinePorts({ entries: makeViewpointEntries() });
    const request: CheckRequest = { scope: { scope_id: "world_1" } };
    let supplied: string[] = [];

    const result = await orchestrateCheck(
      ports,
      request,
      (input: CheckRunInput) => {
        supplied = input.entries.map((entry) => entry.entry_id);
        return spokeOk([]);
      },
    );

    expect(result.ok).toBe(true);
    expect(supplied).toEqual(["kb_shared"]);
  });

  it("orchestrateAssemble packs only entries visible to the scope viewpoint", async () => {
    const ports = createMemoryBaselinePorts({ entries: makeViewpointEntries() });
    const request: AssembleRequest = {
      scope: { scope_id: "world_1", viewpoint: "kb_mira" },
    };

    const result = await orchestrateAssemble(ports, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.packet.entries.map((entry) => entry.entry_id)).toEqual([
      "kb_shared",
      "kb_mine",
    ]);
  });

  it("orchestrateAssemble swaps the private entry when the viewpoint is foreign", async () => {
    const ports = createMemoryBaselinePorts({ entries: makeViewpointEntries() });
    const request: AssembleRequest = {
      scope: { scope_id: "world_1", viewpoint: "kb_rival" },
    };

    const result = await orchestrateAssemble(ports, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.packet.entries.map((entry) => entry.entry_id)).toEqual([
      "kb_shared",
      "kb_theirs",
    ]);
  });

  it("orchestrateForkCheck applies viewpoint filtering to scoped entries", async () => {
    const baseline = createMemoryBaselinePorts({
      entries: makeViewpointEntries(),
    });
    const ports = withForkPorts(baseline);
    const request: CheckRequest = {
      scope: { scope_id: "world_1", viewpoint: "kb_mira", fork_id: "fork_a" },
    };
    let supplied: string[] = [];

    const result = await orchestrateForkCheck(ports, request, (input) => {
      supplied = input.entries.map((entry) => entry.entry_id);
      return spokeOk([]);
    });

    expect(result.ok).toBe(true);
    expect(supplied).toEqual(["kb_shared", "kb_mine"]);
    expect(ports.forkListCalls).toHaveLength(1);
  });

  it("orchestrateForkAssemble packs only entries visible to the scope viewpoint", async () => {
    const baseline = createMemoryBaselinePorts({
      entries: makeViewpointEntries(),
    });
    const ports = withForkPorts(baseline);
    const request: AssembleRequest = {
      scope: { scope_id: "world_1", viewpoint: "kb_mira", fork_id: "fork_a" },
    };

    const result = await orchestrateForkAssemble(ports, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.packet.entries.map((entry) => entry.entry_id)).toEqual([
      "kb_shared",
      "kb_mine",
    ]);
    expect(ports.forkListCalls).toHaveLength(1);
  });
});

const assemblyCognitionModule = {
  records: [
    {
      kind: "belief",
      record_id: "rec_assemble",
      holder: { kind: "knowledge_entry", namespace: "world_a", id: "kb_mira" },
      target: {
        carrier: { kind: "knowledge_entry", namespace: "world_a", id: "kb_box" },
        target_id: "t_marble",
        kind: "proposition",
        text: "Mira hides the marble",
      },
      assessor: { kind: "principal", namespace: "world_a", id: "assessor_self" },
      assessor_role: "holder_self_report",
      basis: {
        kind: "evidence",
        evidence: [{ kind: "knowledge_entry", namespace: "world_a", id: "kb_box" }],
      },
      assessment_context: { context_id: "ctx_main", fork_id: "fork_alpha" },
      effective_at: "2026-07-23T09:00:00Z",
      assessed_at: "2026-07-23T09:05:00Z",
      view: "current",
      order: 1,
      q: {
        value: 0.9,
        assertion: "belief_attribution",
        method: "direct_report",
        calibration: "uncalibrated",
      },
      extensions: {},
    },
  ],
  world_facts: [],
  extensions: {},
};

function makeCognitionEntry(): KnowledgeEntry {
  return makeKnowledgeEntry({
    entry_id: "kb_mira",
    owner: "kb_mira",
    disclosure: "owner-private",
    modules: { cognition: assemblyCognitionModule },
  });
}

const assemblyCognitionSelection = {
  view: "current" as const,
  filter: {
    holder: { kind: "knowledge_entry", namespace: "world_a", id: "kb_mira" },
  },
  selection_policy: { policy_id: "pol_t2_first_input", policy_version: "v1" },
};

const assemblyPolicy = {
  select: ({ assessments }: { assessments: readonly { record_id: string }[] }) =>
    assessments[0]!.record_id,
};

describe("cognition assembly orchestration", () => {
  it("queries the port with baseline scope fields only", async () => {
    const baseline = createMemoryBaselinePorts({ entries: [makeCognitionEntry()] });
    const scopes: Scope[] = [];
    const ports: BaselinePorts = {
      ...baseline,
      async listKnowledgeEntries(scope: Scope) {
        scopes.push(scope);
        return baseline.listKnowledgeEntries(scope);
      },
    };
    const request: AssembleRequest = {
      scope: {
        scope_id: "world_1",
        viewpoint: "kb_mira",
        cognition: assemblyCognitionSelection,
      },
    };

    const result = await orchestrateAssemble(ports, request, assemblyPolicy);

    expect(result.ok).toBe(true);
    expect(scopes).toHaveLength(1);
    expect(scopes[0]).not.toHaveProperty("cognition");
    expect(scopes[0]?.viewpoint).toBe("kb_mira");
  });

  it("withholds private cognition from a foreign viewpoint", async () => {
    const ports = createMemoryBaselinePorts({ entries: [makeCognitionEntry()] });
    const request: AssembleRequest = {
      scope: {
        scope_id: "world_1",
        viewpoint: "kb_rival",
        cognition: assemblyCognitionSelection,
      },
    };

    const result = await orchestrateAssemble(ports, request, assemblyPolicy);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.packet.cognition).toEqual({
      view: "current",
      items: [],
      extensions: {},
    });
  });

  it("projects authorized current cognition with a traced policy selection", async () => {
    const ports = createMemoryBaselinePorts({ entries: [makeCognitionEntry()] });
    const request: AssembleRequest = {
      scope: {
        scope_id: "world_1",
        viewpoint: "kb_mira",
        cognition: assemblyCognitionSelection,
      },
    };

    const result = await orchestrateAssemble(ports, request, assemblyPolicy);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    const cognition = result.value.packet.cognition;
    expect(cognition?.items).toHaveLength(1);
    expect(cognition?.items[0]?.holder).toEqual({
      kind: "knowledge_entry",
      namespace: "world_a",
      id: "kb_mira",
    });
    expect(cognition?.items[0]?.target.carrier.id).toBe("kb_box");
    expect(cognition?.items[0]?.assessment_context).toEqual({
      context_id: "ctx_main",
      fork_id: "fork_alpha",
    });
    expect(cognition?.items[0]?.selection).toEqual({
      policy: { policy_id: "pol_t2_first_input", policy_version: "v1" },
      input_record_ids: ["rec_assemble"],
      selected_record_id: "rec_assemble",
    });
    // Slim packet entries are unchanged by the derivative projection.
    expect(result.value.packet.entries).toEqual([
      {
        entry_id: "kb_mira",
        entry_type: "character",
        canonical_name: "Mira Vale",
        snippet: "Protagonist",
      },
    ]);
  });

  it("rejects a current cognition selection without an injected policy", async () => {
    const ports = createMemoryBaselinePorts({ entries: [makeCognitionEntry()] });
    const request: AssembleRequest = {
      scope: {
        scope_id: "world_1",
        viewpoint: "kb_mira",
        cognition: assemblyCognitionSelection,
      },
    };

    const result = await orchestrateAssemble(ports, request);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });

  it("leaves the packet unchanged when Scope.cognition is absent", async () => {
    const ports = createMemoryBaselinePorts({ entries: [makeCognitionEntry()] });
    const request: AssembleRequest = {
      scope: { scope_id: "world_1", viewpoint: "kb_mira" },
    };

    const result = await orchestrateAssemble(ports, request);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.packet).not.toHaveProperty("cognition");
  });

  it("applies the same authorization-first cognition path on fork assemble", async () => {
    const baseline = createMemoryBaselinePorts({ entries: [makeCognitionEntry()] });
    const ports = withForkPorts(baseline);
    const request: AssembleRequest = {
      scope: {
        scope_id: "world_1",
        viewpoint: "kb_mira",
        fork_id: "fork_a",
        cognition: assemblyCognitionSelection,
      },
    };

    const result = await orchestrateForkAssemble(ports, request, assemblyPolicy);

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.packet.cognition?.items).toHaveLength(1);
    expect(ports.forkListCalls).toHaveLength(1);
  });
});

// Shared cross-language KE authorization/assembly contract (TS ↔ Rust): every
// vector runs the actual public operation with a recording baseline port, so a
// rejection is proven to happen before source output, checker or persistence.
const assemblyVectorFile = JSON.parse(
  readFileSync(
    join(
      dirname(fileURLToPath(import.meta.url)),
      "../../../../fixtures/cognition/assembly-vectors.json",
    ),
    "utf8",
  ),
) as { vectors: AssemblyVector[] };

interface AssemblyVectorCalls {
  listKnowledgeEntries: number;
  listTimelineEvents: number;
  listRules: number;
  putFindings: number;
  checker: number;
}

interface AssemblyVectorError {
  code: string;
  reason?: string;
  field?: string;
}

interface AssemblyItemExpectation {
  presence: "assessed" | "no_record";
  holder: [string, string, string];
  target: [[string, string, string], string, string];
  context: [string, string | null];
  assessments: string[];
  selected: string | null;
  policy: [string, string] | null;
  input_record_ids: string[] | null;
}

interface AssemblyVector {
  id: string;
  operation: "assemble" | "check";
  entries: KnowledgeEntry[];
  scope: Scope;
  max_entries?: number;
  policy_selects?: "first_input" | string | null;
  expected: {
    error?: AssemblyVectorError;
    cognition_absent?: true;
    view?: "current" | "recorded_at";
    items?: AssemblyItemExpectation[];
    entry_ids?: string[];
    checker_entry_ids?: string[];
    findings_persisted?: number;
  };
  expected_calls: AssemblyVectorCalls;
}

function projectionSummary(cognition: {
  view: string;
  items: readonly {
    presence: string;
    holder: { kind: string; namespace: string; id: string };
    target: {
      carrier: { kind: string; namespace: string; id: string };
      kind: string;
      target_id: string;
    };
    assessment_context: { context_id: string; fork_id?: string };
    assessments: readonly { record_id: string }[];
    selection?: {
      policy: { policy_id: string; policy_version: string };
      input_record_ids: readonly string[];
      selected_record_id: string;
    };
  }[];
}) {
  return {
    view: cognition.view,
    items: cognition.items.map((item) => ({
      presence: item.presence,
      holder: [item.holder.kind, item.holder.namespace, item.holder.id],
      target: [
        [
          item.target.carrier.kind,
          item.target.carrier.namespace,
          item.target.carrier.id,
        ],
        item.target.kind,
        item.target.target_id,
      ],
      context: [
        item.assessment_context.context_id,
        item.assessment_context.fork_id ?? null,
      ],
      assessments: item.assessments.map((record) => record.record_id),
      selected: item.selection?.selected_record_id ?? null,
      policy:
        item.selection === undefined
          ? null
          : [
              item.selection.policy.policy_id,
              item.selection.policy.policy_version,
            ],
      input_record_ids: item.selection?.input_record_ids ?? null,
    })),
  };
}

function vectorPolicy(selects: AssemblyVector["policy_selects"]) {
  if (selects === "first_input") {
    return {
      select: ({ assessments }: { assessments: readonly { record_id: string }[] }) =>
        assessments[0]!.record_id,
    };
  }
  if (typeof selects === "string") {
    return { select: () => selects };
  }
  return undefined;
}

/** Recording baseline port: the provider deliberately returns the raw vector set. */
function createVectorPorts(
  entries: KnowledgeEntry[],
  calls: AssemblyVectorCalls,
): BaselinePorts {
  const baseline = createMemoryBaselinePorts({ entries });
  return {
    ...baseline,
    async listKnowledgeEntries() {
      calls.listKnowledgeEntries += 1;
      return spokeOk(entries);
    },
    async listTimelineEvents() {
      calls.listTimelineEvents += 1;
      return spokeOk([]);
    },
    async listRules(ruleRefs) {
      calls.listRules += 1;
      return baseline.listRules(ruleRefs);
    },
    async putFindings(next) {
      calls.putFindings += 1;
      return spokeOk(next);
    },
  };
}

function expectVectorReject(
  result: SpokeResult<unknown>,
  expected: AssemblyVectorError,
): void {
  expect(result.ok).toBe(false);
  if (result.ok) {
    return;
  }
  expect(result.code).toBe(expected.code);
  if (expected.reason !== undefined || expected.field !== undefined) {
    expect(result.details).toMatchObject({
      ...(expected.reason === undefined ? {} : { reason: expected.reason }),
      ...(expected.field === undefined ? {} : { field: expected.field }),
    });
  }
}

function vectorCarrierRecords(entries: readonly KnowledgeEntry[]) {
  const records = [];
  for (const entry of entries) {
    const module = entry.modules?.cognition;
    if (module === undefined || Array.isArray(module)) {
      continue;
    }
    const moduleRecords = module.records;
    if (!Array.isArray(moduleRecords)) {
      continue;
    }
    records.push(...moduleRecords);
  }
  return records;
}

async function runAssemblyVector(vector: AssemblyVector): Promise<void> {
  const calls: AssemblyVectorCalls = {
    listKnowledgeEntries: 0,
    listTimelineEvents: 0,
    listRules: 0,
    putFindings: 0,
    checker: 0,
  };
  const ports = createVectorPorts(vector.entries, calls);

  if (vector.operation === "check") {
    const checkerEntryIds: string[] = [];
    const result = await orchestrateCheck(
      ports,
      { scope: vector.scope },
      (input) => {
        calls.checker += 1;
        checkerEntryIds.push(...input.entries.map((entry) => entry.entry_id));
        return spokeOk([]);
      },
    );

    expect(calls, vector.id).toEqual(vector.expected_calls);
    const error = vector.expected.error;
    if (error !== undefined) {
      expectVectorReject(result, error);
      return;
    }

    expect(result.ok, vector.id).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(checkerEntryIds, vector.id).toEqual(vector.expected.checker_entry_ids);
    expect(result.value.findings, vector.id).toHaveLength(
      vector.expected.findings_persisted ?? 0,
    );
    return;
  }

  const result = await orchestrateAssemble(
    ports,
    {
      scope: vector.scope,
      ...(vector.max_entries === undefined
        ? {}
        : { max_entries: vector.max_entries }),
    },
    vectorPolicy(vector.policy_selects),
  );

  expect(calls, vector.id).toEqual(vector.expected_calls);
  const error = vector.expected.error;
  if (error !== undefined) {
    expectVectorReject(result, error);
    return;
  }

  expect(result.ok, vector.id).toBe(true);
  if (!result.ok) {
    return;
  }

  const packet = result.value.packet;
  expect(
    packet.entries.map((entry) => entry.entry_id),
    vector.id,
  ).toEqual(vector.expected.entry_ids);

  if (vector.expected.cognition_absent !== undefined) {
    expect(packet.cognition, vector.id).toBeUndefined();
    return;
  }

  const projection = packet.cognition;
  expect(projection, vector.id).toBeDefined();
  if (projection === undefined) {
    return;
  }
  expect(projectionSummary(projection), vector.id).toEqual({
    view: vector.expected.view,
    items: vector.expected.items,
  });

  // Every retained assessment is the complete input record: holder, target,
  // assessor, basis, context and probability references survive verbatim.
  const inputRecords = vectorCarrierRecords(vector.entries);
  for (const item of projection.items) {
    for (const record of item.assessments) {
      expect(record, `${vector.id}:${record.record_id}`).toEqual(
        inputRecords.find(
          (candidate) => candidate.record_id === record.record_id,
        ),
      );
    }
  }
}

describe("cognition assembly shared vectors", () => {
  it("executes every assembly vector id exactly once through the public path", async () => {
    const executed = new Set<string>();
    for (const vector of assemblyVectorFile.vectors) {
      await runAssemblyVector(vector);
      expect(executed.has(vector.id)).toBe(false);
      executed.add(vector.id);
    }
    expect(executed.size).toBe(assemblyVectorFile.vectors.length);
  });
});

const cognitionEntryFor = (entryId: string, recordId: string): KnowledgeEntry =>
  makeKnowledgeEntry({
    entry_id: entryId,
    modules: {
      cognition: {
        ...assemblyCognitionModule,
        records: [
          {
            ...assemblyCognitionModule.records[0]!,
            record_id: recordId,
            holder: {
              kind: "knowledge_entry",
              namespace: "world_a",
              id: entryId,
            },
          },
        ],
      },
    },
  });

const recordedAtSelection = {
  view: "recorded_at" as const,
  filter: {
    holder: { kind: "knowledge_entry", namespace: "world_a", id: "kb_mira" },
  },
};

describe("cognition scope rejection boundaries", () => {
  it("rejects an unknown filter predicate before any source output", async () => {
    const calls: AssemblyVectorCalls = {
      listKnowledgeEntries: 0,
      listTimelineEvents: 0,
      listRules: 0,
      putFindings: 0,
      checker: 0,
    };
    const ports = createVectorPorts([makeCognitionEntry()], calls);
    const request = {
      scope: {
        scope_id: "world_1",
        viewpoint: "kb_mira",
        cognition: {
          view: "current",
          filter: { q_min: 0.5 },
          selection_policy: { policy_id: "pol_t2", policy_version: "v1" },
        },
      },
    } as unknown as AssembleRequest;

    const result = await orchestrateAssemble(ports, request, assemblyPolicy);

    expectVectorReject(result, {
      code: SpokeRejectCode.INVALID_INPUT,
      reason: "unsupported_cognition_requirement",
      field: "scope.cognition.filter.q_min",
    });
    expect(calls.listKnowledgeEntries).toBe(0);
    expect(calls.listTimelineEvents).toBe(0);
  });

  it("check enforces cognition locally on the authorized set", async () => {
    const baseline = createMemoryBaselinePorts({
      entries: [
        cognitionEntryFor("kb_mira", "rec_mira"),
        cognitionEntryFor("kb_theo", "rec_theo"),
      ],
    });
    const scopes: Scope[] = [];
    const ports: BaselinePorts = {
      ...baseline,
      async listKnowledgeEntries(scope: Scope) {
        scopes.push(scope);
        return baseline.listKnowledgeEntries(scope);
      },
    };
    let checkerEntries: string[] = [];

    const result = await orchestrateCheck(
      ports,
      {
        scope: {
          scope_id: "world_1",
          cognition: {
            view: "current",
            filter: {
              holder: {
                kind: "knowledge_entry",
                namespace: "world_a",
                id: "kb_theo",
              },
            },
            selection_policy: { policy_id: "pol_t2", policy_version: "v1" },
          },
        },
      },
      (input) => {
        checkerEntries = input.entries.map((entry) => entry.entry_id);
        return spokeOk([]);
      },
    );

    expect(result.ok).toBe(true);
    expect(scopes).toHaveLength(1);
    expect(scopes[0]).not.toHaveProperty("cognition");
    expect(checkerEntries).toEqual(["kb_theo"]);
  });

  it("fork check rejects the unavailable recorded-at view before any port call", async () => {
    const baseline = createMemoryBaselinePorts({
      entries: [makeCognitionEntry()],
    });
    const scopes: Scope[] = [];
    const recording: BaselinePorts = {
      ...baseline,
      async listKnowledgeEntries(scope: Scope) {
        scopes.push(scope);
        return baseline.listKnowledgeEntries(scope);
      },
    };
    const ports = withForkPorts(recording);
    let checkerCalls = 0;

    const result = await orchestrateForkCheck(
      ports,
      {
        scope: {
          scope_id: "world_1",
          fork_id: "fork_a",
          cognition: recordedAtSelection,
        },
      },
      () => {
        checkerCalls += 1;
        return spokeOk([]);
      },
    );

    expectVectorReject(result, {
      code: SpokeRejectCode.INVALID_INPUT,
      reason: "unsupported_cognition_requirement",
      field: "scope.cognition.view",
    });
    expect(scopes).toHaveLength(0);
    expect(ports.forkListCalls).toHaveLength(0);
    expect(checkerCalls).toBe(0);
  });

  it("fork assemble rejects the unavailable recorded-at view before any port call", async () => {
    const baseline = createMemoryBaselinePorts({
      entries: [makeCognitionEntry()],
    });
    const scopes: Scope[] = [];
    const recording: BaselinePorts = {
      ...baseline,
      async listKnowledgeEntries(scope: Scope) {
        scopes.push(scope);
        return baseline.listKnowledgeEntries(scope);
      },
    };
    const ports = withForkPorts(recording);

    const result = await orchestrateForkAssemble(ports, {
      scope: {
        scope_id: "world_1",
        fork_id: "fork_a",
        viewpoint: "kb_mira",
        cognition: recordedAtSelection,
      },
    });

    expectVectorReject(result, {
      code: SpokeRejectCode.INVALID_INPUT,
      reason: "unsupported_cognition_requirement",
      field: "scope.cognition.view",
    });
    expect(scopes).toHaveLength(0);
    expect(ports.forkListCalls).toHaveLength(0);
  });
});
