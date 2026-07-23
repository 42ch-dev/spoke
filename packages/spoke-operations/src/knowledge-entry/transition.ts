import type { KnowledgeEntry } from "@42ch/spoke-schemas";

import { spokeOk, spokeReject, type SpokeResult } from "../result.js";
import { SpokeRejectCode } from "../result.js";

const CORE_KNOWLEDGE_ENTRY_STATUSES = new Set([
  "provisional",
  "confirmed",
  "deprecated",
  "merged",
  "deleted",
]);

const ALLOWED_TRANSITIONS: ReadonlyMap<string, ReadonlySet<string>> = new Map([
  [
    "provisional",
    new Set(["confirmed", "deprecated", "merged", "deleted", "provisional"]),
  ],
  ["confirmed", new Set(["deprecated", "merged", "deleted", "confirmed"])],
  ["deprecated", new Set(["confirmed", "deleted", "deprecated"])],
  ["merged", new Set(["merged"])],
  ["deleted", new Set(["deleted"])],
]);

function isCoreKnowledgeEntryStatus(status: string): boolean {
  return CORE_KNOWLEDGE_ENTRY_STATUSES.has(status);
}

/**
 * Returns whether a KnowledgeEntry status transition is allowed by the cross-product table.
 */
export function isValidKnowledgeEntryStatusTransition(
  from: string,
  to: string,
): boolean {
  if (!isCoreKnowledgeEntryStatus(from) || !isCoreKnowledgeEntryStatus(to)) {
    return false;
  }

  const allowedTargets = ALLOWED_TRANSITIONS.get(from);
  return allowedTargets?.has(to) ?? false;
}

/**
 * Apply a KnowledgeEntry status transition; returns updated status on success without mutating input.
 */
export function transitionKnowledgeEntryStatus(
  knowledgeEntry: KnowledgeEntry,
  to: string,
): SpokeResult<KnowledgeEntry> {
  if (!isCoreKnowledgeEntryStatus(to)) {
    return spokeReject(
      SpokeRejectCode.INVALID_KNOWLEDGE_ENTRY_STATUS,
      `Invalid knowledge entry status: ${to}`,
      { status: to },
    );
  }

  if (!isCoreKnowledgeEntryStatus(knowledgeEntry.status)) {
    return spokeReject(
      SpokeRejectCode.INVALID_KNOWLEDGE_ENTRY_STATUS,
      `Invalid current knowledge entry status: ${knowledgeEntry.status}`,
      { status: knowledgeEntry.status },
    );
  }

  if (!isValidKnowledgeEntryStatusTransition(knowledgeEntry.status, to)) {
    return spokeReject(
      SpokeRejectCode.INVALID_KNOWLEDGE_ENTRY_STATUS_TRANSITION,
      `Disallowed knowledge entry status transition: ${knowledgeEntry.status} -> ${to}`,
      { from: knowledgeEntry.status, to },
    );
  }

  return spokeOk({
    ...knowledgeEntry,
    status: to,
  });
}
