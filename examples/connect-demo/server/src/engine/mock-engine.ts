/**
 * Mock inference engine — in-memory store + deterministic derivation.
 *
 * Determinism is a HARD constraint: derived artifact ids, bodies, and
 * revisions are pure functions of store history — no wall-clock, no random.
 * The seed corpus loads at construction; every accepted entry/relation
 * mutation re-derives the reserved-id artifacts:
 *
 *   - `derived/world-digest` KnowledgeEntry — entry_type counts + sorted id
 *     roster over the user corpus (derived ids excluded, so the digest is a
 *     stable function of user history; its revision is the derivation count);
 *   - `derived/isolated-entry/<entry_id>` findings — one per user entry that
 *     participates in no relation.
 *
 * Revisions are store-owned (seed 1 on create, current + 1 on accepted
 * update), mirroring the toy-world reference store's relation OCC. The
 * `derived/` id namespace is reserved: user puts into it are rejected.
 */

import type {
  ComputeRequest,
  ComputeResponse,
  ComputableFieldMap,
  Finding,
  KnowledgeEntry,
  ProjectRequest,
  ProjectResponse,
  Relation,
  Rule,
  TimelineEvent,
} from "@42ch/spoke-schemas";
import {
  assertRevisionMatch,
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  type SpokeResult,
} from "@42ch/spoke-operations";

import {
  DEMO_SEED_ENTRIES,
  DEMO_SEED_RELATIONS,
  DEMO_SEED_RULES,
  DEMO_SEED_TIMELINE_EVENTS,
} from "./seed-corpus.js";

/** Reserved id prefix for engine-derived artifacts (never user-writable). */
const DERIVED_ID_PREFIX = "derived/";

/** entry_id of the derived world-digest KnowledgeEntry. */
export const DERIVED_WORLD_DIGEST_ENTRY_ID = `${DERIVED_ID_PREFIX}world-digest`;

/**
 * One l2-computable session: the dynamic computable view plus the static
 * state it settles into. Sessions are keyed by the opaque `session_id` the
 * client owns (products own session stores; this mock keeps one in memory).
 */
interface ComputableSession {
  entry_id: string;
  computable: ComputableFieldMap;
  state: ComputableFieldMap;
}

/** UTF-8 lexicographic compare (mirrors the repo's peer-manifest sort). */
function compareUtf8(left: string, right: string): number {
  return Buffer.compare(
    Buffer.from(left, "utf8"),
    Buffer.from(right, "utf8"),
  );
}

export class MockEngine {
  private readonly entries = new Map<string, KnowledgeEntry>();
  private readonly relations = new Map<string, Relation>();
  private readonly rules = new Map<string, Rule>();
  private readonly events: TimelineEvent[] = [];
  private readonly userFindings: Finding[] = [];
  private derivedFindings: Finding[] = [];
  /**
   * l2-computable sessions, keyed by client-owned `session_id`. Demo-only:
   * retained unbounded (simplify: no eviction — production stores must expire).
   */
  private readonly computableSessions = new Map<string, ComputableSession>();
  /** Number of derivations performed — the digest revision equals this. */
  private derivationCount = 0;

  constructor() {
    for (const entry of DEMO_SEED_ENTRIES) {
      this.entries.set(entry.entry_id, entry);
    }
    for (const relation of DEMO_SEED_RELATIONS) {
      this.relations.set(relation.relation_id, relation);
    }
    for (const rule of DEMO_SEED_RULES) {
      this.rules.set(rule.rule_id, rule);
    }
    for (const event of DEMO_SEED_TIMELINE_EVENTS) {
      this.events.push(event);
    }
    this.derive();
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
   * Conditional put — `null` expectedBaseRevision means create (must be
   * absent). Non-null is a compare-and-swap: the store's current revision
   * must equal the expected base (stale base → `STORED_REVISION_STALE`,
   * impossible future base → `REVISION_CONFLICT`, per
   * spoke-operations.md §5). Revisions are store-owned.
   */
  putKnowledgeEntry(
    entry: KnowledgeEntry,
    expectedBaseRevision: number | null,
  ): SpokeResult<KnowledgeEntry> {
    const guard = this.assertNotDerivedId(entry.entry_id);
    if (!guard.ok) {
      return guard;
    }

    const existing = this.entries.get(entry.entry_id);
    let stored: KnowledgeEntry;
    if (expectedBaseRevision === null) {
      if (existing !== undefined) {
        return spokeReject(
          SpokeRejectCode.REVISION_CONFLICT,
          `Entry already exists: ${entry.entry_id}`,
          { entry_id: entry.entry_id },
        );
      }
      stored = { ...entry, revision: 1 };
    } else {
      if (existing === undefined) {
        return spokeReject(
          SpokeRejectCode.STORED_REVISION_STALE,
          `KnowledgeEntry not found for update: ${entry.entry_id}`,
          { entry_id: entry.entry_id, expectedBaseRevision },
        );
      }
      const currentRevision = existing.revision ?? 0;
      const gate = assertRevisionMatch(expectedBaseRevision, currentRevision);
      if (!gate.ok) {
        return gate;
      }
      stored = { ...entry, revision: currentRevision + 1 };
    }

    this.entries.set(stored.entry_id, stored);
    this.derive();
    return spokeOk(stored);
  }

  getRelation(relationId: string): SpokeResult<Relation> {
    const relation = this.relations.get(relationId);
    if (relation === undefined) {
      return spokeReject(
        SpokeRejectCode.RELATION_NOT_FOUND,
        `Relation not found: ${relationId}`,
        { relation_id: relationId },
      );
    }
    return spokeOk(relation);
  }

  /**
   * Conditional put with the same OCC contract as putKnowledgeEntry, plus the
   * reference-store duplicate-create code `RELATION_ALREADY_EXISTS`.
   */
  putRelation(
    relation: Relation,
    expectedBaseRevision: number | null,
  ): SpokeResult<Relation> {
    const existing = this.relations.get(relation.relation_id);
    let stored: Relation;
    if (expectedBaseRevision === null) {
      if (existing !== undefined) {
        return spokeReject(
          SpokeRejectCode.RELATION_ALREADY_EXISTS,
          `Relation already exists: ${relation.relation_id}`,
          { relation_id: relation.relation_id },
        );
      }
      stored = { ...relation, revision: 1 };
    } else {
      if (existing === undefined) {
        return spokeReject(
          SpokeRejectCode.STORED_REVISION_STALE,
          `Relation not found for update: ${relation.relation_id}`,
          { relation_id: relation.relation_id, expectedBaseRevision },
        );
      }
      const currentRevision = existing.revision ?? 0;
      const gate = assertRevisionMatch(expectedBaseRevision, currentRevision);
      if (!gate.ok) {
        return gate;
      }
      stored = { ...relation, revision: currentRevision + 1 };
    }

    this.relations.set(stored.relation_id, stored);
    this.derive();
    return spokeOk(stored);
  }

  /** Raw entries (seed + user + derived digest); the adapter applies scope. */
  listKnowledgeEntries(): KnowledgeEntry[] {
    return [...this.entries.values()];
  }

  listTimelineEvents(): TimelineEvent[] {
    return [...this.events];
  }

  putFindings(findings: Finding[]): SpokeResult<Finding[]> {
    this.userFindings.push(...findings);
    return spokeOk(findings);
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

  /** Current `isolated_entry` derived findings (sorted by finding_id). */
  listDerivedFindings(): Finding[] {
    return [...this.derivedFindings];
  }

  /**
   * l2-computable projection (ComputablePort.project): materialize the
   * session's dynamic computable view from the request's static state.
   * Deterministic — the view is a pure copy of the request state; the
   * session is recorded so later computes settle against it.
   */
  projectComputable(request: ProjectRequest): SpokeResult<ProjectResponse> {
    const view: ComputableFieldMap = { ...request.state };
    this.computableSessions.set(request.session_id, {
      entry_id: request.entry_id,
      computable: view,
      state: { ...request.state },
    });
    return spokeOk({
      session_id: request.session_id,
      entry_id: request.entry_id,
      computable: view,
    });
  }

  /**
   * l2-computable apply/settle (ComputablePort.compute): merge the
   * request's computable delta into the session view (starting from the
   * delta when no session was projected), and when `settle` is true merge
   * the view back into the session's static state. Deterministic — pure
   * merges of request data, no wall-clock or random.
   */
  computeComputable(request: ComputeRequest): SpokeResult<ComputeResponse> {
    const existing = this.computableSessions.get(request.session_id);
    const view: ComputableFieldMap = {
      ...(existing?.computable ?? {}),
      ...request.computable,
    };
    const state: ComputableFieldMap = { ...(existing?.state ?? {}) };
    if (request.settle === true) {
      Object.assign(state, view);
    }
    this.computableSessions.set(request.session_id, {
      entry_id: request.entry_id,
      computable: view,
      state,
    });
    return request.settle === true
      ? spokeOk({
          session_id: request.session_id,
          entry_id: request.entry_id,
          computable: view,
          state,
        })
      : spokeOk({
          session_id: request.session_id,
          entry_id: request.entry_id,
          computable: view,
        });
  }

  private assertNotDerivedId(entryId: string): SpokeResult<void> {
    if (entryId.startsWith(DERIVED_ID_PREFIX)) {
      return spokeReject(
        SpokeRejectCode.INVALID_INPUT,
        `Reserved derived id: ${entryId}`,
        { entry_id: entryId },
      );
    }
    return spokeOk();
  }

  /**
   * Re-derive the reserved-id artifacts from the current store. Runs once at
   * construction and after every accepted entry/relation put.
   */
  private derive(): void {
    this.derivationCount += 1;

    const userEntries = [...this.entries.values()].filter(
      (entry) => !entry.entry_id.startsWith(DERIVED_ID_PREFIX),
    );

    const entryTypeCounts: Record<string, number> = {};
    for (const entry of userEntries) {
      entryTypeCounts[entry.entry_type] =
        (entryTypeCounts[entry.entry_type] ?? 0) + 1;
    }

    const sortedIds = userEntries
      .map((entry) => entry.entry_id)
      .sort(compareUtf8);

    const digest: KnowledgeEntry = {
      schema_version: 1,
      entry_id: DERIVED_WORLD_DIGEST_ENTRY_ID,
      entry_type: "note",
      canonical_name: "World Digest",
      status: "confirmed",
      body: {
        summary: `Digest of ${userEntries.length} knowledge entries in demo-harbor.`,
        computable: {
          entry_type_counts: entryTypeCounts,
          entry_ids_sorted: sortedIds,
        },
      },
      revision: this.derivationCount,
      extensions: {},
    };
    this.entries.set(DERIVED_WORLD_DIGEST_ENTRY_ID, digest);

    const isolated: Finding[] = [];
    for (const entry of userEntries) {
      const hasRelation = [...this.relations.values()].some(
        (relation) =>
          relation.from_id === entry.entry_id ||
          relation.to_id === entry.entry_id,
      );
      if (!hasRelation) {
        isolated.push({
          schema_version: 1,
          finding_id: `${DERIVED_ID_PREFIX}isolated-entry/${entry.entry_id}`,
          severity: "warning",
          status: "open",
          title: `Isolated entry: ${entry.canonical_name}`,
          description: `Entry ${entry.entry_id} participates in no relations.`,
          target_entry_id: entry.entry_id,
          extensions: {},
        });
      }
    }
    isolated.sort((left, right) => compareUtf8(left.finding_id, right.finding_id));
    this.derivedFindings = isolated;
  }
}
