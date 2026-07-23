import type { KnowledgeEntry } from "@42ch/spoke-schemas";

import { assertRevisionMatch } from "../occ/assert-revision.js";
import { spokeOk, spokeReject, type SpokeResult } from "../result.js";
import { SpokeRejectCode } from "../result.js";

const TERMINAL_KNOWLEDGE_ENTRY_STATUSES = new Set(["merged", "deleted"]);

const REQUIRED_KNOWLEDGE_ENTRY_FIELDS = [
  "schema_version",
  "knowledge_entry_id",
  "block_type",
  "canonical_name",
  "status",
  "body",
  "extensions",
] as const satisfies ReadonlyArray<keyof KnowledgeEntry>;

export type ValidateUpsertKnowledgeEntryContext = {
  stored?: KnowledgeEntry;
  mode?: "create" | "update";
};

function validateRequiredKnowledgeEntryFields(
  candidate: KnowledgeEntry,
): SpokeResult<void> {
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
    typeof candidate.knowledge_entry_id !== "string" ||
    candidate.knowledge_entry_id.length === 0
  ) {
    return spokeReject(
      SpokeRejectCode.MISSING_REQUIRED_FIELD,
      "KnowledgeEntry knowledge_entry_id must be a non-empty string",
      { field: "knowledge_entry_id" },
    );
  }

  if (typeof candidate.block_type !== "string" || candidate.block_type.length === 0) {
    return spokeReject(
      SpokeRejectCode.MISSING_REQUIRED_FIELD,
      "KnowledgeEntry block_type must be a non-empty string",
      { field: "block_type" },
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

  return spokeOk();
}

function validateCreateRevision(candidate: KnowledgeEntry): SpokeResult<void> {
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
      "KnowledgeEntry revision must be a non-negative integer, 0, or omitted on create",
      { revision: candidate.revision },
    );
  }

  if (candidate.revision >= 1) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "KnowledgeEntry revision must be absent, undefined, or 0 on create",
      { revision: candidate.revision },
    );
  }

  return spokeOk();
}

function validateCreatePath(candidate: KnowledgeEntry): SpokeResult<void> {
  const required = validateRequiredKnowledgeEntryFields(candidate);
  if (!required.ok) {
    return required;
  }

  return validateCreateRevision(candidate);
}

function validateUpdatePath(
  candidate: KnowledgeEntry,
  stored: KnowledgeEntry,
): SpokeResult<void> {
  const required = validateRequiredKnowledgeEntryFields(candidate);
  if (!required.ok) {
    return required;
  }

  if (candidate.knowledge_entry_id !== stored.knowledge_entry_id) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "Candidate knowledge_entry_id must match stored knowledge_entry_id on update",
      {
        candidate_knowledge_entry_id: candidate.knowledge_entry_id,
        stored_knowledge_entry_id: stored.knowledge_entry_id,
      },
    );
  }

  if (TERMINAL_KNOWLEDGE_ENTRY_STATUSES.has(stored.status)) {
    return spokeReject(
      SpokeRejectCode.KNOWLEDGE_ENTRY_TERMINAL_STATUS,
      `Stored KnowledgeEntry has terminal status: ${stored.status}`,
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
 * Validate KnowledgeEntry upsert before persist; create vs update inferred from stored presence.
 */
export function validateUpsertKnowledgeEntry(
  candidate: KnowledgeEntry,
  context: ValidateUpsertKnowledgeEntryContext = {},
): SpokeResult<void> {
  const { stored, mode } = context;

  if (mode === "update" && stored === undefined) {
    return spokeReject(
      SpokeRejectCode.KNOWLEDGE_ENTRY_NOT_FOUND,
      "Update path requires a stored KnowledgeEntry",
      { knowledge_entry_id: candidate.knowledge_entry_id },
    );
  }

  if (mode === "create" && stored !== undefined) {
    return spokeReject(
      SpokeRejectCode.KNOWLEDGE_ENTRY_ALREADY_EXISTS,
      "Create path must not include a stored KnowledgeEntry",
      { knowledge_entry_id: stored.knowledge_entry_id },
    );
  }

  if (stored !== undefined) {
    return validateUpdatePath(candidate, stored);
  }

  return validateCreatePath(candidate);
}
