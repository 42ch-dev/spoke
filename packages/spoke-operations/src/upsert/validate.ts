import type { Keyblock } from "@42ch/spoke-schemas";

import { assertRevisionMatch } from "../occ/assert-revision.js";
import { spokeOk, spokeReject, type SpokeResult } from "../result.js";
import { SpokeRejectCode } from "../result.js";

const TERMINAL_KEYBLOCK_STATUSES = new Set(["merged", "deleted"]);

const REQUIRED_KEYBLOCK_FIELDS = [
  "schema_version",
  "keyblock_id",
  "block_type",
  "canonical_name",
  "status",
  "body",
  "extensions",
] as const satisfies ReadonlyArray<keyof Keyblock>;

export type ValidateUpsertKeyblockContext = {
  stored?: Keyblock;
  mode?: "create" | "update";
};

function validateRequiredKeyblockFields(candidate: Keyblock): SpokeResult<void> {
  for (const field of REQUIRED_KEYBLOCK_FIELDS) {
    const value = candidate[field];

    if (value === undefined || value === null) {
      return spokeReject(
        SpokeRejectCode.MISSING_REQUIRED_FIELD,
        `Missing required Keyblock field: ${field}`,
        { field },
      );
    }
  }

  if (
    typeof candidate.schema_version !== "number" ||
    !Number.isInteger(candidate.schema_version) ||
    candidate.schema_version < 1
  ) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "Keyblock schema_version must be an integer >= 1",
      { schema_version: candidate.schema_version },
    );
  }

  if (typeof candidate.keyblock_id !== "string" || candidate.keyblock_id.length === 0) {
    return spokeReject(
      SpokeRejectCode.MISSING_REQUIRED_FIELD,
      "Keyblock keyblock_id must be a non-empty string",
      { field: "keyblock_id" },
    );
  }

  if (typeof candidate.block_type !== "string" || candidate.block_type.length === 0) {
    return spokeReject(
      SpokeRejectCode.MISSING_REQUIRED_FIELD,
      "Keyblock block_type must be a non-empty string",
      { field: "block_type" },
    );
  }

  if (typeof candidate.body !== "object" || candidate.body === null || Array.isArray(candidate.body)) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "Keyblock body must be an object",
      { field: "body" },
    );
  }

  if (
    typeof candidate.extensions !== "object" ||
    candidate.extensions === null ||
    Array.isArray(candidate.extensions)
  ) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "Keyblock extensions must be an object",
      { field: "extensions" },
    );
  }

  return spokeOk();
}

function validateCreateRevision(candidate: Keyblock): SpokeResult<void> {
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
      "Keyblock revision must be a non-negative integer, 0, or omitted on create",
      { revision: candidate.revision },
    );
  }

  if (candidate.revision >= 1) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "Keyblock revision must be absent, undefined, or 0 on create",
      { revision: candidate.revision },
    );
  }

  return spokeOk();
}

function validateCreatePath(candidate: Keyblock): SpokeResult<void> {
  const required = validateRequiredKeyblockFields(candidate);
  if (!required.ok) {
    return required;
  }

  return validateCreateRevision(candidate);
}

function validateUpdatePath(candidate: Keyblock, stored: Keyblock): SpokeResult<void> {
  if (candidate.keyblock_id !== stored.keyblock_id) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "Candidate keyblock_id must match stored keyblock_id on update",
      {
        candidate_keyblock_id: candidate.keyblock_id,
        stored_keyblock_id: stored.keyblock_id,
      },
    );
  }

  if (TERMINAL_KEYBLOCK_STATUSES.has(stored.status)) {
    return spokeReject(
      SpokeRejectCode.KEYBLOCK_TERMINAL_STATUS,
      `Stored Keyblock has terminal status: ${stored.status}`,
      { status: stored.status },
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
 * Validate Keyblock upsert before persist; create vs update inferred from stored presence.
 */
export function validateUpsertKeyblock(
  candidate: Keyblock,
  context: ValidateUpsertKeyblockContext = {},
): SpokeResult<void> {
  const { stored, mode } = context;

  if (mode === "update" && stored === undefined) {
    return spokeReject(
      SpokeRejectCode.KEYBLOCK_NOT_FOUND,
      "Update path requires a stored Keyblock",
      { keyblock_id: candidate.keyblock_id },
    );
  }

  if (mode === "create" && stored !== undefined) {
    return spokeReject(
      SpokeRejectCode.KEYBLOCK_ALREADY_EXISTS,
      "Create path must not include a stored Keyblock",
      { keyblock_id: stored.keyblock_id },
    );
  }

  if (stored !== undefined) {
    return validateUpdatePath(candidate, stored);
  }

  return validateCreatePath(candidate);
}
