import type { Keyblock } from "@42ch/spoke-schemas";

import { spokeOk, spokeReject, type SpokeResult } from "../result.js";
import { SpokeRejectCode } from "../result.js";

const ACTIVE_KEYBLOCK_STATUSES = new Set(["provisional", "confirmed"]);

function isActiveKeyblockStatus(status: string): boolean {
  return ACTIVE_KEYBLOCK_STATUSES.has(status);
}

export type AssertUniqueActiveKeyblockInput = {
  scope_key: string;
  block_type: string;
  canonical_name: string;
  candidate: Keyblock;
  existing: Keyblock[];
};

/**
 * Reject duplicate active triple among caller-supplied Keyblocks; scope_key is opaque.
 */
export function assertUniqueActiveKeyblock({
  scope_key,
  block_type,
  canonical_name,
  candidate,
  existing,
}: AssertUniqueActiveKeyblockInput): SpokeResult<void> {
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

  if (!isActiveKeyblockStatus(candidate.status)) {
    return spokeOk();
  }

  for (const keyblock of existing) {
    if (!isActiveKeyblockStatus(keyblock.status)) {
      continue;
    }
    if (keyblock.block_type !== block_type) {
      continue;
    }
    if (keyblock.canonical_name !== canonical_name) {
      continue;
    }
    if (keyblock.keyblock_id === candidate.keyblock_id) {
      continue;
    }

    return spokeReject(
      SpokeRejectCode.DUPLICATE_ACTIVE_KEYBLOCK,
      `Duplicate active keyblock for (${scope_key}, ${block_type}, ${canonical_name})`,
      {
        scope_key,
        block_type,
        canonical_name,
        conflicting_keyblock_id: keyblock.keyblock_id,
      },
    );
  }

  return spokeOk();
}
