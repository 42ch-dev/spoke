/**
 * In-memory OCC store for the toy-world reference adapter.
 * Optional seed from committed fixture JSON under fixtures/toy-world/.
 */

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import type {
  Finding,
  KnowledgeEntry,
  Relation,
  Rule,
  TimelineEvent,
} from "@42ch/spoke-schemas";
import {
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  type SpokeResult,
} from "@42ch/spoke-operations";

const FIXTURES_ROOT = join(
  dirname(fileURLToPath(import.meta.url)),
  "../..",
);

function loadJson<T>(filename: string): T {
  const raw = readFileSync(join(FIXTURES_ROOT, filename), "utf8");
  return JSON.parse(raw) as T;
}

/** Seed payload for constructing a MemoryStore. */
export type MemoryStoreSeed = {
  entries?: KnowledgeEntry[];
  relations?: Relation[];
  events?: TimelineEvent[];
  rules?: Rule[];
  findings?: Finding[];
};

/**
 * Mutable in-memory maps with KnowledgeEntry OCC on put.
 */
export class MemoryStore {
  readonly entries: Map<string, KnowledgeEntry>;
  readonly relations: Map<string, Relation>;
  readonly events: TimelineEvent[];
  readonly rules: Map<string, Rule>;
  readonly findings: Finding[];

  constructor(seed?: MemoryStoreSeed) {
    this.entries = new Map(
      (seed?.entries ?? []).map((entry) => [entry.entry_id, entry]),
    );
    this.relations = new Map(
      (seed?.relations ?? []).map((relation) => [
        relation.relation_id,
        relation,
      ]),
    );
    this.events = [...(seed?.events ?? [])];
    this.rules = new Map(
      (seed?.rules ?? []).map((rule) => [rule.rule_id, rule]),
    );
    this.findings = [...(seed?.findings ?? [])];
  }

  /**
   * Seed from committed toy-world JSON (kb / rel / evt / rule / fnd).
   */
  static fromCommittedFixtures(): MemoryStore {
    return new MemoryStore({
      entries: [
        loadJson<KnowledgeEntry>("kb_tw_mira.json"),
        loadJson<KnowledgeEntry>("kb_tw_harbor.json"),
        loadJson<KnowledgeEntry>("kb_tw_harbor_dawn_event.json"),
        loadJson<KnowledgeEntry>("kb_tw_harbor_market_square_event.json"),
        loadJson<KnowledgeEntry>("kb_tw_harbor_customs_gate_beat.json"),
        loadJson<KnowledgeEntry>("kb_tw_harbor_berth_confirm_event.json"),
      ],
      relations: [
        loadJson<Relation>("rel_tw_mira_harbor.json"),
        loadJson<Relation>("rel_tw_harbor_precedes_dawn_to_market.json"),
        loadJson<Relation>("rel_tw_harbor_precedes_market_to_customs.json"),
        loadJson<Relation>("rel_tw_harbor_precedes_customs_to_berth.json"),
      ],
      events: [
        loadJson<TimelineEvent>("evt_tw_harbor_dawn.json"),
        loadJson<TimelineEvent>("evt_tw_harbor_market_square.json"),
        loadJson<TimelineEvent>("evt_tw_harbor_customs_gate.json"),
        loadJson<TimelineEvent>("evt_tw_harbor_berth_confirm.json"),
        loadJson<TimelineEvent>("evt_tw_harbor_storm_delay.json"),
      ],
      rules: [loadJson<Rule>("rule_tw_consistency.json")],
      findings: [loadJson<Finding>("fnd_tw_open.json")],
    });
  }

  getKnowledgeEntry(entryId: string): SpokeResult<KnowledgeEntry> {
    const entry = this.entries.get(entryId);
    if (entry === undefined) {
      return spokeReject(
        SpokeRejectCode.KNOWLEDGE_ENTRY_NOT_FOUND,
        `KnowledgeEntry not found: ${entryId}`,
        { entry_id: entryId },
      );
    }
    return spokeOk(entry);
  }

  /**
   * Conditional put — `null` expectedBaseRevision means create (must be absent).
   */
  putKnowledgeEntry(
    entry: KnowledgeEntry,
    expectedBaseRevision: number | null,
  ): SpokeResult<KnowledgeEntry> {
    const existing = this.entries.get(entry.entry_id);
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
    this.entries.set(entry.entry_id, entry);
    return spokeOk(entry);
  }

  putRelation(relation: Relation): SpokeResult<Relation> {
    this.relations.set(relation.relation_id, relation);
    return spokeOk(relation);
  }

  listKnowledgeEntries(): SpokeResult<KnowledgeEntry[]> {
    return spokeOk([...this.entries.values()]);
  }

  listTimelineEvents(): SpokeResult<TimelineEvent[]> {
    return spokeOk([...this.events]);
  }

  putFindings(next: Finding[]): SpokeResult<Finding[]> {
    this.findings.push(...next);
    return spokeOk(next);
  }

  listRules(ruleRefs: string[]): SpokeResult<Rule[]> {
    const resolved: Rule[] = [];
    for (const ref of ruleRefs) {
      const rule = this.rules.get(ref);
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
  }
}

export { FIXTURES_ROOT as TOY_WORLD_FIXTURES_ROOT };
