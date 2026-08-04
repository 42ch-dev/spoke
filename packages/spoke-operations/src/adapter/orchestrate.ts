/**
 * Injection orchestration entrypoints — compose pure helpers with port I/O.
 */

import type {
  AssembleRequest,
  AssembleResponse,
  CheckRequest,
  CheckResponse,
  ComputeRequest,
  ComputeResponse,
  Finding,
  ForkId,
  KnowledgeEntry,
  ProjectRequest,
  ProjectResponse,
  PromoteRequest,
  PromoteResponse,
  RelateRequest,
  RelateResponse,
  Relation,
  Rule,
  Scope,
  TimelineEvent,
  UpsertRequest,
  UpsertResponse,
} from "@42ch/spoke-schemas";

import { buildAssemblePacket } from "../assemble/builder.js";
import {
  validateComputeRequest,
  validateProjectRequest,
} from "../computable/validate.js";
import { assertUniqueActiveKnowledgeEntry } from "../knowledge-entry/uniqueness.js";
import { isValidKnowledgeEntryStatusTransition } from "../knowledge-entry/transition.js";
import { assertRevisionMatch } from "../occ/assert-revision.js";
import {
  applyPromoteAcceptance,
  validatePromoteRequest,
} from "../promote/acceptance.js";
import { validateRelateRequest } from "../relate/validate.js";
import {
  SpokeRejectCode,
  spokeOk,
  spokeReject,
  type SpokeResult,
} from "../result.js";
import {
  filterKnowledgeEntriesByScope,
  filterTimelineEventsByScope,
} from "../scope/match.js";
import { validateUpsertKnowledgeEntry } from "../upsert/validate.js";

const TERMINAL_KNOWLEDGE_ENTRY_STATUSES = new Set(["merged", "deleted"]);

import type {
  BaselinePorts,
  ComputablePorts,
  ForkPorts,
  KnowledgeEntryPort,
  RelationPort,
} from "./ports.js";

/** Checker callback input after ports load scoped data and rules. */
export type CheckRunInput = {
  request: CheckRequest;
  entries: KnowledgeEntry[];
  events: TimelineEvent[];
  rules: Rule[];
};

async function loadStoredKnowledgeEntry(
  ports: KnowledgeEntryPort,
  entryId: string,
): Promise<SpokeResult<KnowledgeEntry | undefined>> {
  const result = await ports.getKnowledgeEntry(entryId);
  if (!result.ok) {
    if (result.code === SpokeRejectCode.KNOWLEDGE_ENTRY_NOT_FOUND) {
      // spokeOk(undefined) collapses to void SpokeOk; keep explicit value for absence.
      return { ok: true, value: undefined };
    }
    return result;
  }
  return spokeOk(result.value);
}

async function loadStoredRelation(
  ports: RelationPort,
  relationId: string,
): Promise<SpokeResult<Relation | undefined>> {
  const result = await ports.getRelation(relationId);
  if (!result.ok) {
    if (result.code === SpokeRejectCode.RELATION_NOT_FOUND) {
      return { ok: true, value: undefined };
    }
    return result;
  }
  return spokeOk(result.value);
}

function assertStatusTransitionWhenApplicable(
  candidate: KnowledgeEntry,
  stored: KnowledgeEntry | undefined,
): SpokeResult<void> {
  if (stored === undefined || stored.status === candidate.status) {
    return spokeOk();
  }

  if (
    !isValidKnowledgeEntryStatusTransition(stored.status, candidate.status)
  ) {
    return spokeReject(
      SpokeRejectCode.INVALID_KNOWLEDGE_ENTRY_STATUS_TRANSITION,
      `Disallowed knowledge entry status transition: ${stored.status} -> ${candidate.status}`,
      { from: stored.status, to: candidate.status },
    );
  }

  return spokeOk();
}

function assertUniquenessWhenApplicable(
  candidate: KnowledgeEntry,
  existing: KnowledgeEntry[],
): SpokeResult<void> {
  return assertUniqueActiveKnowledgeEntry({
    scope_key: "upsert",
    entry_type: candidate.entry_type,
    canonical_name: candidate.canonical_name,
    candidate,
    existing,
  });
}

/**
 * Upsert KnowledgeEntries: load context, validate, optional status/uniqueness, put.
 */
export async function orchestrateUpsert(
  ports: BaselinePorts,
  request: UpsertRequest,
): Promise<SpokeResult<UpsertResponse>> {
  const persisted: KnowledgeEntry[] = [];
  const batchPeers: KnowledgeEntry[] = [];

  for (const candidate of request.knowledge_entries) {
    const storedResult = await loadStoredKnowledgeEntry(
      ports,
      candidate.entry_id,
    );
    if (!storedResult.ok) {
      return storedResult;
    }
    const stored = storedResult.value;

    const validation = validateUpsertKnowledgeEntry(candidate, { stored });
    if (!validation.ok) {
      return validation;
    }

    const statusGate = assertStatusTransitionWhenApplicable(candidate, stored);
    if (!statusGate.ok) {
      return statusGate;
    }

    const uniqueness = assertUniquenessWhenApplicable(candidate, [
      ...batchPeers,
      ...(stored !== undefined ? [stored] : []),
    ]);
    if (!uniqueness.ok) {
      return uniqueness;
    }

    const expectedBaseRevision =
      stored === undefined ? null : (stored.revision ?? 0);

    const put = await ports.putKnowledgeEntry(candidate, expectedBaseRevision);
    if (!put.ok) {
      return put;
    }

    persisted.push(put.value);
    batchPeers.push(put.value);
  }

  return spokeOk({ knowledge_entries: persisted });
}

/**
 * Promote a provisional candidate, then persist via KnowledgeEntryPort.
 */
export async function orchestratePromote(
  ports: BaselinePorts,
  request: PromoteRequest,
): Promise<SpokeResult<PromoteResponse>> {
  const storedResult = await loadStoredKnowledgeEntry(
    ports,
    request.candidate.entry_id,
  );
  if (!storedResult.ok) {
    return storedResult;
  }
  const stored = storedResult.value;
  if (stored !== undefined) {
    if (TERMINAL_KNOWLEDGE_ENTRY_STATUSES.has(stored.status)) {
      return spokeReject(
        SpokeRejectCode.KNOWLEDGE_ENTRY_TERMINAL_STATUS,
        `Stored KnowledgeEntry has terminal status: ${stored.status}`,
        { status: stored.status },
      );
    }

    const revisionGate = assertRevisionMatch(
      request.candidate.revision ?? 0,
      stored.revision ?? 0,
    );
    if (!revisionGate.ok) {
      return revisionGate;
    }
  }

  const validation = validatePromoteRequest(request);
  if (!validation.ok) {
    return validation;
  }

  const accepted = applyPromoteAcceptance(request);
  if (!accepted.ok) {
    return accepted;
  }

  // When stored exists, base the persisted revision on stored — do not trust
  // candidate-only bump if it could diverge from the loaded OCC base.
  const toPersist =
    stored !== undefined
      ? {
          ...accepted.value,
          revision: (stored.revision ?? 0) + 1,
        }
      : accepted.value;

  const expectedBaseRevision =
    stored === undefined ? null : (stored.revision ?? 0);

  const put = await ports.putKnowledgeEntry(toPersist, expectedBaseRevision);
  if (!put.ok) {
    return put;
  }

  return spokeOk({
    knowledge_entry: put.value,
    ...(request.target_entry_id !== undefined
      ? { superseded_id: request.target_entry_id }
      : {}),
  });
}

/**
 * Validate and persist a Relation: load stored, validate (create vs update),
 * run OCC-aware put. Mirrors orchestrateUpsert.
 */
export async function orchestrateRelate(
  ports: BaselinePorts,
  request: RelateRequest,
): Promise<SpokeResult<RelateResponse>> {
  const storedResult = await loadStoredRelation(
    ports,
    request.relation.relation_id,
  );
  if (!storedResult.ok) {
    return storedResult;
  }
  const stored = storedResult.value;

  const validation = validateRelateRequest(request.relation, {
    stored,
    mode: stored === undefined ? "create" : "update",
  });
  if (!validation.ok) {
    return validation;
  }

  const expectedBaseRevision =
    stored === undefined ? null : (stored.revision ?? 0);

  const put = await ports.putRelation(request.relation, expectedBaseRevision);
  if (!put.ok) {
    return put;
  }

  return spokeOk({ relation: put.value });
}

async function resolveCheckRules(
  ports: BaselinePorts,
  request: CheckRequest,
): Promise<SpokeResult<Rule[]>> {
  const embedded = request.rules ?? [];
  if (request.rule_refs === undefined || request.rule_refs.length === 0) {
    return spokeOk([...embedded]);
  }

  const resolved = await ports.listRules(request.rule_refs);
  if (!resolved.ok) {
    return resolved;
  }

  // Start from resolved refs; embedded rules win by rule_id (replace or append).
  const merged = [...resolved.value];
  for (const rule of embedded) {
    const index = merged.findIndex((item) => item.rule_id === rule.rule_id);
    if (index >= 0) {
      merged[index] = rule;
    } else {
      merged.push(rule);
    }
  }
  return spokeOk(merged);
}

/**
 * Load scoped data and rules, invoke product checker callback, persist findings.
 */
export async function orchestrateCheck(
  ports: BaselinePorts,
  request: CheckRequest,
  runChecker: (input: CheckRunInput) => SpokeResult<Finding[]>,
): Promise<SpokeResult<CheckResponse>> {
  const rulesResult = await resolveCheckRules(ports, request);
  if (!rulesResult.ok) {
    return rulesResult;
  }

  const entriesResult = await ports.listKnowledgeEntries(request.scope);
  if (!entriesResult.ok) {
    return entriesResult;
  }

  const eventsResult = await ports.listTimelineEvents(request.scope);
  if (!eventsResult.ok) {
    return eventsResult;
  }

  const entries = filterKnowledgeEntriesByScope(
    entriesResult.value,
    request.scope,
  );
  const events = filterTimelineEventsByScope(eventsResult.value, request.scope);

  const checkResult = runChecker({
    request,
    entries,
    events,
    rules: rulesResult.value,
  });
  if (!checkResult.ok) {
    return checkResult;
  }

  const put = await ports.putFindings(checkResult.value);
  if (!put.ok) {
    return put;
  }

  return spokeOk({ findings: put.value });
}

/**
 * Query scoped KnowledgeEntries/events, apply scope helpers, build AssemblePacket.
 */
export async function orchestrateAssemble(
  ports: BaselinePorts,
  request: AssembleRequest,
): Promise<SpokeResult<AssembleResponse>> {
  const entriesResult = await ports.listKnowledgeEntries(request.scope);
  if (!entriesResult.ok) {
    return entriesResult;
  }

  const eventsResult = await ports.listTimelineEvents(request.scope);
  if (!eventsResult.ok) {
    return eventsResult;
  }

  // Events are loaded/filtered for sequence parity; packet builders use entries only.
  filterTimelineEventsByScope(eventsResult.value, request.scope);

  const entries = filterKnowledgeEntriesByScope(
    entriesResult.value,
    request.scope,
  );

  const packetResult = buildAssemblePacket({
    packetId: `assemble:${request.scope.scope_id}`,
    knowledgeEntries: entries,
    maxEntries: request.max_entries,
    extensions: request.extensions,
  });
  if (!packetResult.ok) {
    return packetResult;
  }

  return spokeOk({ packet: packetResult.value });
}

function requirePortMethod(
  ports: object,
  method: string,
): SpokeResult<void> {
  const candidate = (ports as Record<string, unknown>)[method];
  if (typeof candidate !== "function") {
    return spokeReject(
      SpokeRejectCode.CAPABILITY_PORT_MISSING,
      `Optional port method missing: ${method}`,
      { method },
    );
  }
  return spokeOk();
}

function requireForkScope(
  scope: Scope,
): SpokeResult<Scope & { fork_id: ForkId }> {
  if (
    typeof scope.fork_id !== "string" ||
    scope.fork_id.trim().length === 0
  ) {
    return spokeReject(
      SpokeRejectCode.MISSING_REQUIRED_FIELD,
      "Fork orchestration requires scope.fork_id",
      { field: "scope.fork_id" },
    );
  }

  return spokeOk({ ...scope, fork_id: scope.fork_id });
}

/**
 * Project Session computable view via ComputablePort after request validation.
 */
export async function orchestrateProject(
  ports: ComputablePorts,
  request: ProjectRequest,
): Promise<SpokeResult<ProjectResponse>> {
  const capability = requirePortMethod(ports, "project");
  if (!capability.ok) {
    return capability;
  }

  const validation = validateProjectRequest(request);
  if (!validation.ok) {
    return validation;
  }

  return await ports.project(request);
}

/**
 * Compute Session updates via ComputablePort after request validation.
 * Settled-state persistence remains an explicit adapter step.
 */
export async function orchestrateCompute(
  ports: ComputablePorts,
  request: ComputeRequest,
): Promise<SpokeResult<ComputeResponse>> {
  const capability = requirePortMethod(ports, "compute");
  if (!capability.ok) {
    return capability;
  }

  const validation = validateComputeRequest(request);
  if (!validation.ok) {
    return validation;
  }

  return await ports.compute(request);
}

/**
 * Fork-aware check: KE via ScopeQueryPort; timeline via ForkTimelineQueryPort.
 */
export async function orchestrateForkCheck(
  ports: ForkPorts,
  request: CheckRequest,
  runChecker: (input: CheckRunInput) => SpokeResult<Finding[]>,
): Promise<SpokeResult<CheckResponse>> {
  const capability = requirePortMethod(ports, "listForkTimelineEvents");
  if (!capability.ok) {
    return capability;
  }

  const forkScope = requireForkScope(request.scope);
  if (!forkScope.ok) {
    return forkScope;
  }

  const rulesResult = await resolveCheckRules(ports, request);
  if (!rulesResult.ok) {
    return rulesResult;
  }

  const entriesResult = await ports.listKnowledgeEntries(request.scope);
  if (!entriesResult.ok) {
    return entriesResult;
  }

  const eventsResult = await ports.listForkTimelineEvents(forkScope.value);
  if (!eventsResult.ok) {
    return eventsResult;
  }

  const entries = filterKnowledgeEntriesByScope(
    entriesResult.value,
    request.scope,
  );
  const events = filterTimelineEventsByScope(eventsResult.value, request.scope);

  const checkResult = runChecker({
    request,
    entries,
    events,
    rules: rulesResult.value,
  });
  if (!checkResult.ok) {
    return checkResult;
  }

  const put = await ports.putFindings(checkResult.value);
  if (!put.ok) {
    return put;
  }

  return spokeOk({ findings: put.value });
}

/**
 * Fork-aware assemble: KE via ScopeQueryPort; timeline via ForkTimelineQueryPort.
 */
export async function orchestrateForkAssemble(
  ports: ForkPorts,
  request: AssembleRequest,
): Promise<SpokeResult<AssembleResponse>> {
  const capability = requirePortMethod(ports, "listForkTimelineEvents");
  if (!capability.ok) {
    return capability;
  }

  const forkScope = requireForkScope(request.scope);
  if (!forkScope.ok) {
    return forkScope;
  }

  const entriesResult = await ports.listKnowledgeEntries(request.scope);
  if (!entriesResult.ok) {
    return entriesResult;
  }

  const eventsResult = await ports.listForkTimelineEvents(forkScope.value);
  if (!eventsResult.ok) {
    return eventsResult;
  }

  // Events are loaded/filtered for sequence parity; packet builders use entries only.
  filterTimelineEventsByScope(eventsResult.value, request.scope);

  const entries = filterKnowledgeEntriesByScope(
    entriesResult.value,
    request.scope,
  );

  const packetResult = buildAssemblePacket({
    packetId: `assemble:${request.scope.scope_id}`,
    knowledgeEntries: entries,
    maxEntries: request.max_entries,
    extensions: request.extensions,
  });
  if (!packetResult.ok) {
    return packetResult;
  }

  return spokeOk({ packet: packetResult.value });
}
