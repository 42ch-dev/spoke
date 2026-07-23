import type { KnowledgeEntry } from "@42ch/spoke-schemas";

import { spokeOk, spokeReject, type SpokeResult } from "../result.js";
import { SpokeRejectCode } from "../result.js";

const ACTIVE_KNOWLEDGE_ENTRY_STATUSES = new Set(["provisional", "confirmed"]);

function isActiveKnowledgeEntryStatus(status: string): boolean {
  return ACTIVE_KNOWLEDGE_ENTRY_STATUSES.has(status);
}

export type AssertUniqueActiveKnowledgeEntryInput = {
  scope_key: string;
  entry_type: string;
  canonical_name: string;
  candidate: KnowledgeEntry;
  existing: KnowledgeEntry[];
};

/**
 * Reject duplicate active triple among caller-supplied KnowledgeEntries; scope_key is opaque.
 */
export function assertUniqueActiveKnowledgeEntry({
  scope_key,
  entry_type,
  canonical_name,
  candidate,
  existing,
}: AssertUniqueActiveKnowledgeEntryInput): SpokeResult<void> {
  if (candidate.entry_type !== entry_type || candidate.canonical_name !== canonical_name) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "entry_type and canonical_name must match candidate wire fields",
      {
        entry_type,
        canonical_name,
        candidate_entry_type: candidate.entry_type,
        candidate_canonical_name: candidate.canonical_name,
      },
    );
  }

  if (!isActiveKnowledgeEntryStatus(candidate.status)) {
    return spokeOk();
  }

  for (const knowledgeEntry of existing) {
    if (!isActiveKnowledgeEntryStatus(knowledgeEntry.status)) {
      continue;
    }
    if (knowledgeEntry.entry_type !== entry_type) {
      continue;
    }
    if (knowledgeEntry.canonical_name !== canonical_name) {
      continue;
    }
    if (knowledgeEntry.entry_id === candidate.entry_id) {
      continue;
    }

    return spokeReject(
      SpokeRejectCode.DUPLICATE_ACTIVE_KNOWLEDGE_ENTRY,
      `Duplicate active knowledge entry for (${scope_key}, ${entry_type}, ${canonical_name})`,
      {
        scope_key,
        entry_type,
        canonical_name,
        conflicting_entry_id: knowledgeEntry.entry_id,
      },
    );
  }

  return spokeOk();
}
