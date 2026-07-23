import type { KnowledgeEntry } from "@42ch/spoke-schemas";

import { spokeOk, spokeReject, type SpokeResult } from "../result.js";
import { SpokeRejectCode } from "../result.js";

const ACTIVE_KNOWLEDGE_ENTRY_STATUSES = new Set(["provisional", "confirmed"]);

function isActiveKnowledgeEntryStatus(status: string): boolean {
  return ACTIVE_KNOWLEDGE_ENTRY_STATUSES.has(status);
}

export type AssertUniqueActiveKnowledgeEntryInput = {
  scope_key: string;
  block_type: string;
  canonical_name: string;
  candidate: KnowledgeEntry;
  existing: KnowledgeEntry[];
};

/**
 * Reject duplicate active triple among caller-supplied KnowledgeEntries; scope_key is opaque.
 */
export function assertUniqueActiveKnowledgeEntry({
  scope_key,
  block_type,
  canonical_name,
  candidate,
  existing,
}: AssertUniqueActiveKnowledgeEntryInput): SpokeResult<void> {
  if (candidate.block_type !== block_type || candidate.canonical_name !== canonical_name) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "block_type and canonical_name must match candidate wire fields",
      {
        block_type,
        canonical_name,
        candidate_block_type: candidate.block_type,
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
    if (knowledgeEntry.block_type !== block_type) {
      continue;
    }
    if (knowledgeEntry.canonical_name !== canonical_name) {
      continue;
    }
    if (knowledgeEntry.knowledge_entry_id === candidate.knowledge_entry_id) {
      continue;
    }

    return spokeReject(
      SpokeRejectCode.DUPLICATE_ACTIVE_KNOWLEDGE_ENTRY,
      `Duplicate active knowledge entry for (${scope_key}, ${block_type}, ${canonical_name})`,
      {
        scope_key,
        block_type,
        canonical_name,
        conflicting_knowledge_entry_id: knowledgeEntry.knowledge_entry_id,
      },
    );
  }

  return spokeOk();
}
