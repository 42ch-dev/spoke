/**
 * Injection orchestration entrypoints — compose pure helpers with port I/O.
 */

import type {
  AssembleRequest,
  AssembleResponse,
  CheckRequest,
  CheckResponse,
  Finding,
  KnowledgeEntry,
  PromoteRequest,
  PromoteResponse,
  RelateRequest,
  RelateResponse,
  Rule,
  TimelineEvent,
  UpsertRequest,
  UpsertResponse,
} from "@42ch/spoke-schemas";

import { buildAssemblePacket } from "../assemble/builder.js";
import { assertUniqueActiveKnowledgeEntry } from "../knowledge-entry/uniqueness.js";
import { isValidKnowledgeEntryStatusTransition } from "../knowledge-entry/transition.js";
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

import type { BaselinePorts, KnowledgeEntryPort } from "./ports.js";

/** Checker callback input after ports load scoped data and rules. */
export type CheckRunInput = {
  request: CheckRequest;
  entries: KnowledgeEntry[];
  events: TimelineEvent[];
  rules: Rule[];
};

function loadStoredKnowledgeEntry(
  ports: KnowledgeEntryPort,
  entryId: string,
): SpokeResult<KnowledgeEntry | undefined> {
  const result = ports.getKnowledgeEntry(entryId);
  if (!result.ok) {
    if (result.code === SpokeRejectCode.KNOWLEDGE_ENTRY_NOT_FOUND) {
      // spokeOk(undefined) collapses to void SpokeOk; keep explicit value for absence.
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
export function orchestrateUpsert(
  ports: BaselinePorts,
  request: UpsertRequest,
): SpokeResult<UpsertResponse> {
  const persisted: KnowledgeEntry[] = [];
  const batchPeers: KnowledgeEntry[] = [];

  for (const candidate of request.knowledge_entries) {
    const storedResult = loadStoredKnowledgeEntry(ports, candidate.entry_id);
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

    const put = ports.putKnowledgeEntry(candidate);
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
export function orchestratePromote(
  ports: BaselinePorts,
  request: PromoteRequest,
): SpokeResult<PromoteResponse> {
  const validation = validatePromoteRequest(request);
  if (!validation.ok) {
    return validation;
  }

  const accepted = applyPromoteAcceptance(request);
  if (!accepted.ok) {
    return accepted;
  }

  const put = ports.putKnowledgeEntry(accepted.value);
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
 * Validate and persist a Relation.
 */
export function orchestrateRelate(
  ports: BaselinePorts,
  request: RelateRequest,
): SpokeResult<RelateResponse> {
  const validation = validateRelateRequest(request.relation);
  if (!validation.ok) {
    return validation;
  }

  const put = ports.putRelation(request.relation);
  if (!put.ok) {
    return put;
  }

  return spokeOk({ relation: put.value });
}

function resolveCheckRules(
  ports: BaselinePorts,
  request: CheckRequest,
): SpokeResult<Rule[]> {
  const embedded = request.rules ?? [];
  if (request.rule_refs === undefined || request.rule_refs.length === 0) {
    return spokeOk([...embedded]);
  }

  const resolved = ports.listRules(request.rule_refs);
  if (!resolved.ok) {
    return resolved;
  }

  return spokeOk([...resolved.value, ...embedded]);
}

/**
 * Load scoped data and rules, invoke product checker callback, persist findings.
 */
export function orchestrateCheck(
  ports: BaselinePorts,
  request: CheckRequest,
  runChecker: (input: CheckRunInput) => SpokeResult<Finding[]>,
): SpokeResult<CheckResponse> {
  const rulesResult = resolveCheckRules(ports, request);
  if (!rulesResult.ok) {
    return rulesResult;
  }

  const entriesResult = ports.listKnowledgeEntries(request.scope);
  if (!entriesResult.ok) {
    return entriesResult;
  }

  const eventsResult = ports.listTimelineEvents(request.scope);
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

  const put = ports.putFindings(checkResult.value);
  if (!put.ok) {
    return put;
  }

  return spokeOk({ findings: put.value });
}

/**
 * Query scoped KnowledgeEntries/events, apply scope helpers, build AssemblePacket.
 */
export function orchestrateAssemble(
  ports: BaselinePorts,
  request: AssembleRequest,
): SpokeResult<AssembleResponse> {
  const entriesResult = ports.listKnowledgeEntries(request.scope);
  if (!entriesResult.ok) {
    return entriesResult;
  }

  const eventsResult = ports.listTimelineEvents(request.scope);
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
