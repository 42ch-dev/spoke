import type { Relation } from "@42ch/spoke-schemas";

import { assertRevisionMatch } from "../occ/assert-revision.js";
import { spokeOk, spokeReject, type SpokeResult } from "../result.js";
import { SpokeRejectCode } from "../result.js";

export type ValidateRelateRequestContext = {
  stored?: Relation;
};

function isNonEmptyTrimmedString(value: string | undefined): boolean {
  return typeof value === "string" && value.trim().length > 0;
}

function validateCreateRevision(candidate: Relation): SpokeResult<void> {
  if (candidate.revision === undefined || candidate.revision === 0) {
    return spokeOk();
  }

  if (
    typeof candidate.revision !== "number" ||
    !Number.isInteger(candidate.revision) ||
    candidate.revision < 0
  ) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "Relation revision must be a non-negative integer, 0, or omitted on create",
      { revision: candidate.revision },
    );
  }

  if (candidate.revision >= 1) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "Relation revision must be absent, undefined, or 0 on create",
      { revision: candidate.revision },
    );
  }

  return spokeOk();
}

function validateUpdatePath(
  candidate: Relation,
  stored: Relation,
): SpokeResult<void> {
  if (candidate.relation_id !== stored.relation_id) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "Candidate relation_id must match stored relation_id on update",
      {
        candidate_relation_id: candidate.relation_id,
        stored_relation_id: stored.relation_id,
      },
    );
  }

  if (candidate.revision === undefined) {
    return spokeReject(
      SpokeRejectCode.MISSING_REQUIRED_FIELD,
      "Candidate revision is required on update",
      { field: "revision" },
    );
  }

  if (
    typeof candidate.revision !== "number" ||
    !Number.isInteger(candidate.revision) ||
    candidate.revision < 0
  ) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "Candidate revision must be a non-negative integer on update",
      { revision: candidate.revision },
    );
  }

  return assertRevisionMatch(candidate.revision, stored.revision ?? 0);
}

/**
 * Validate Relation shape and lifecycle rules before persist; create vs update
 * inferred from stored presence. Mirrors validateUpsertKnowledgeEntry.
 */
export function validateRelateRequest(
  relation: Relation,
  context: ValidateRelateRequestContext = {},
): SpokeResult<void> {
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

  if (context.stored !== undefined) {
    return validateUpdatePath(relation, context.stored);
  }

  return validateCreateRevision(relation);
}
