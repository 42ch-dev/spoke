import type { KnowledgeEntry, PromoteRequest } from "@42ch/spoke-schemas";

import { spokeOk, spokeReject, type SpokeResult } from "../result.js";
import { SpokeRejectCode } from "../result.js";

const TERMINAL_KNOWLEDGE_ENTRY_STATUSES = new Set(["merged", "deleted"]);

const REQUIRED_KNOWLEDGE_ENTRY_FIELDS = [
  "schema_version",
  "entry_id",
  "entry_type",
  "canonical_name",
  "status",
  "body",
  "extensions",
] as const satisfies ReadonlyArray<keyof KnowledgeEntry>;

function validateRevision(candidate: KnowledgeEntry): SpokeResult<void> {
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
      "KnowledgeEntry revision must be a non-negative integer or omitted",
      { revision: candidate.revision },
    );
  }

  return spokeOk();
}

function validateKnowledgeEntryShape(candidate: KnowledgeEntry): SpokeResult<void> {
  for (const field of REQUIRED_KNOWLEDGE_ENTRY_FIELDS) {
    const value = candidate[field];

    if (value === undefined || value === null) {
      return spokeReject(
        SpokeRejectCode.MISSING_REQUIRED_FIELD,
        `Missing required KnowledgeEntry field: ${field}`,
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
      "KnowledgeEntry schema_version must be an integer >= 1",
      { schema_version: candidate.schema_version },
    );
  }

  if (
    typeof candidate.entry_id !== "string" ||
    candidate.entry_id.length === 0
  ) {
    return spokeReject(
      SpokeRejectCode.MISSING_REQUIRED_FIELD,
      "KnowledgeEntry entry_id must be a non-empty string",
      { field: "entry_id" },
    );
  }

  if (typeof candidate.entry_type !== "string" || candidate.entry_type.length === 0) {
    return spokeReject(
      SpokeRejectCode.MISSING_REQUIRED_FIELD,
      "KnowledgeEntry entry_type must be a non-empty string",
      { field: "entry_type" },
    );
  }

  if (typeof candidate.body !== "object" || candidate.body === null || Array.isArray(candidate.body)) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "KnowledgeEntry body must be an object",
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
      "KnowledgeEntry extensions must be an object",
      { field: "extensions" },
    );
  }

  if (typeof candidate.canonical_name !== "string") {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "KnowledgeEntry canonical_name must be a string",
      { field: "canonical_name" },
    );
  }

  if (candidate.canonical_name.trim().length === 0) {
    return spokeReject(
      SpokeRejectCode.EMPTY_CANONICAL_NAME,
      "KnowledgeEntry canonical_name must be non-empty",
    );
  }

  const revisionResult = validateRevision(candidate);
  if (!revisionResult.ok) {
    return revisionResult;
  }

  if (TERMINAL_KNOWLEDGE_ENTRY_STATUSES.has(candidate.status)) {
    return spokeReject(
      SpokeRejectCode.CANDIDATE_TERMINAL_STATUS,
      `Candidate KnowledgeEntry has terminal status: ${candidate.status}`,
      { status: candidate.status },
    );
  }

  if (candidate.status !== "provisional") {
    return spokeReject(
      SpokeRejectCode.CANDIDATE_NOT_PROVISIONAL,
      `Candidate KnowledgeEntry status must be provisional (got ${candidate.status})`,
      { status: candidate.status },
    );
  }

  return spokeOk();
}

function requestTargetEqualsCandidate(
  candidate: KnowledgeEntry,
  targetId: string | undefined,
): boolean {
  return targetId !== undefined && targetId === candidate.entry_id;
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

  const shapeResult = validateKnowledgeEntryShape(request.candidate);
  if (!shapeResult.ok) {
    return shapeResult;
  }

  const { candidate, target_entry_id: targetKnowledgeEntryId } = request;

  if (requestTargetEqualsCandidate(candidate, targetKnowledgeEntryId)) {
    return spokeReject(
      SpokeRejectCode.MERGE_TARGET_SELF,
      "target_entry_id must not equal candidate.entry_id",
      {
        entry_id: candidate.entry_id,
        target_entry_id: targetKnowledgeEntryId,
      },
    );
  }

  return spokeOk();
}

function nextRevision(candidate: KnowledgeEntry): number {
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
 * Return promoted KnowledgeEntry view (status confirmed, revision bumped); does not persist.
 */
export function applyPromoteAcceptance(
  request: PromoteRequest,
): SpokeResult<KnowledgeEntry> {
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
