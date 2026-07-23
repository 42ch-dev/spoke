import type { Relation } from "@42ch/spoke-schemas";

import { spokeOk, spokeReject, type SpokeResult } from "../result.js";
import { SpokeRejectCode } from "../result.js";

function isNonEmptyTrimmedString(value: string | undefined): boolean {
  return typeof value === "string" && value.trim().length > 0;
}

/**
 * Validate Relation shape and lifecycle rules before persist.
 */
export function validateRelateRequest(relation: Relation): SpokeResult<void> {
  if (!isNonEmptyTrimmedString(relation.from_id)) {
    return spokeReject(
      SpokeRejectCode.RELATION_MISSING_ENDPOINT,
      "Relation from_id must be a non-empty trimmed string",
      { field: "from_id" },
    );
  }

  if (!isNonEmptyTrimmedString(relation.to_id)) {
    return spokeReject(
      SpokeRejectCode.RELATION_MISSING_ENDPOINT,
      "Relation to_id must be a non-empty trimmed string",
      { field: "to_id" },
    );
  }

  const fromId = relation.from_id.trim();
  const toId = relation.to_id.trim();

  if (fromId === toId) {
    return spokeReject(
      SpokeRejectCode.RELATION_SELF_EDGE,
      "Relation from_id must not equal to_id",
      { from_id: relation.from_id, to_id: relation.to_id },
    );
  }

  return spokeOk();
}
