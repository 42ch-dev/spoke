import type { Keyblock, PromoteRequest } from "@42ch/spoke-schema";

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

function validateRevision(candidate: Keyblock): SpokeResult<void> {
  if (candidate.revision === undefined) {
    return spokeOk();
  }

  if (
    typeof candidate.revision !== "number" ||
    !Number.isInteger(candidate.revision) ||
    candidate.revision < 0
  ) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "Keyblock revision must be a non-negative integer or omitted",
      { revision: candidate.revision },
    );
  }

  return spokeOk();
}

function validateKeyblockShape(candidate: Keyblock): SpokeResult<void> {
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

  if (typeof candidate.canonical_name !== "string") {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "Keyblock canonical_name must be a string",
      { field: "canonical_name" },
    );
  }

  if (candidate.canonical_name.trim().length === 0) {
    return spokeReject(
      SpokeRejectCode.EMPTY_CANONICAL_NAME,
      "Keyblock canonical_name must be non-empty",
    );
  }

  const revisionResult = validateRevision(candidate);
  if (!revisionResult.ok) {
    return revisionResult;
  }

  if (TERMINAL_KEYBLOCK_STATUSES.has(candidate.status)) {
    return spokeReject(
      SpokeRejectCode.CANDIDATE_TERMINAL_STATUS,
      `Candidate Keyblock has terminal status: ${candidate.status}`,
      { status: candidate.status },
    );
  }

  if (candidate.status !== "provisional") {
    return spokeReject(
      SpokeRejectCode.CANDIDATE_NOT_PROVISIONAL,
      `Candidate Keyblock status must be provisional (got ${candidate.status})`,
      { status: candidate.status },
    );
  }

  return spokeOk();
}

function requestTargetEqualsCandidate(
  candidate: Keyblock,
  targetId: string | undefined,
): boolean {
  return targetId !== undefined && targetId === candidate.keyblock_id;
}

function validatePromoteLifecycle(
  request: PromoteRequest,
): SpokeResult<void> {
  if (!request.candidate) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "PromoteRequest candidate is required",
    );
  }

  const shapeResult = validateKeyblockShape(request.candidate);
  if (!shapeResult.ok) {
    return shapeResult;
  }

  const { candidate, target_keyblock_id: targetKeyblockId } = request;

  if (requestTargetEqualsCandidate(candidate, targetKeyblockId)) {
    return spokeReject(
      SpokeRejectCode.MERGE_TARGET_SELF,
      "target_keyblock_id must not equal candidate.keyblock_id",
      {
        keyblock_id: candidate.keyblock_id,
        target_keyblock_id: targetKeyblockId,
      },
    );
  }

  return spokeOk();
}

function nextRevision(candidate: Keyblock): number {
  if (candidate.revision === undefined) {
    return 1;
  }

  return candidate.revision + 1;
}

/**
 * Validate promote request shape and lifecycle rules.
 */
export function validatePromoteRequest(
  request: PromoteRequest,
): SpokeResult<void> {
  return validatePromoteLifecycle(request);
}

/**
 * Return promoted Keyblock view (status confirmed, revision bumped); does not persist.
 */
export function applyPromoteAcceptance(
  request: PromoteRequest,
): SpokeResult<Keyblock> {
  const validation = validatePromoteLifecycle(request);
  if (!validation.ok) {
    return validation;
  }

  const { candidate } = request;

  return spokeOk({
    ...candidate,
    status: "confirmed",
    revision: nextRevision(candidate),
  });
}
