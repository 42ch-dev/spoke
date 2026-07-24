import type {
  ComputableFieldMap,
  ComputableLogChange,
  ComputableLogEntry,
  ComputeRequest,
  ProjectRequest,
} from "@42ch/spoke-schemas";

import { spokeOk, spokeReject, type SpokeResult } from "../result.js";
import { SpokeRejectCode } from "../result.js";

const RFC3339_PATTERN =
  /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/;

function isNonEmptyTrimmedString(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function validateExtensionMap(
  extensions: unknown,
  field: string,
): SpokeResult<void> {
  if (!isPlainObject(extensions)) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      `${field} must be an object`,
      { field },
    );
  }

  return spokeOk();
}

function isRfc3339Timestamp(value: string): boolean {
  return RFC3339_PATTERN.test(value);
}

/**
 * Shape gate for body.state / body.computable and op ComputableFieldMap payloads.
 */
export function validateComputableFieldMap(
  value: ComputableFieldMap | unknown,
): SpokeResult<void> {
  if (!isPlainObject(value)) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "ComputableFieldMap must be a non-null plain object",
      { field: "computable_field_map" },
    );
  }

  return spokeOk();
}

function validateComputableLogChange(
  change: ComputableLogChange,
  index: number,
): SpokeResult<void> {
  if (!isNonEmptyTrimmedString(change.path)) {
    return spokeReject(
      SpokeRejectCode.MISSING_REQUIRED_FIELD,
      "ComputableLogChange path must be a non-empty string",
      { field: "changes", index },
    );
  }

  return spokeOk();
}

/**
 * Shape gate for TimelineEvent.computable_logs[] items.
 */
export function validateComputableLogEntry(
  entry: ComputableLogEntry,
): SpokeResult<void> {
  if (
    !isNonEmptyTrimmedString(entry.logged_at) ||
    !isRfc3339Timestamp(entry.logged_at.trim())
  ) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "ComputableLogEntry logged_at must be a valid RFC 3339 timestamp",
      { field: "logged_at" },
    );
  }

  if (!isNonEmptyTrimmedString(entry.entry_id)) {
    return spokeReject(
      SpokeRejectCode.MISSING_REQUIRED_FIELD,
      "ComputableLogEntry entry_id must be a non-empty string",
      { field: "entry_id" },
    );
  }

  if (!Array.isArray(entry.changes)) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "ComputableLogEntry changes must be an array",
      { field: "changes" },
    );
  }

  for (let index = 0; index < entry.changes.length; index += 1) {
    const changeResult = validateComputableLogChange(entry.changes[index]!, index);
    if (!changeResult.ok) {
      return changeResult;
    }
  }

  if (
    entry.session_id !== undefined &&
    !isNonEmptyTrimmedString(entry.session_id)
  ) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "ComputableLogEntry session_id must be a non-empty string when present",
      { field: "session_id" },
    );
  }

  return spokeOk();
}

/**
 * Required-field gate for project op request wire shape.
 */
export function validateProjectRequest(
  request: ProjectRequest,
): SpokeResult<void> {
  if (!isNonEmptyTrimmedString(request.session_id)) {
    return spokeReject(
      SpokeRejectCode.MISSING_REQUIRED_FIELD,
      "ProjectRequest session_id must be a non-empty string",
      { field: "session_id" },
    );
  }

  if (!isNonEmptyTrimmedString(request.entry_id)) {
    return spokeReject(
      SpokeRejectCode.MISSING_REQUIRED_FIELD,
      "ProjectRequest entry_id must be a non-empty string",
      { field: "entry_id" },
    );
  }

  const stateResult = validateComputableFieldMap(request.state);
  if (!stateResult.ok) {
    return stateResult;
  }

  if (request.extensions !== undefined) {
    const extensionsResult = validateExtensionMap(
      request.extensions,
      "extensions",
    );
    if (!extensionsResult.ok) {
      return extensionsResult;
    }
  }

  return spokeOk();
}

/**
 * Required-field gate for compute op request wire shape.
 */
export function validateComputeRequest(
  request: ComputeRequest,
): SpokeResult<void> {
  if (!isNonEmptyTrimmedString(request.session_id)) {
    return spokeReject(
      SpokeRejectCode.MISSING_REQUIRED_FIELD,
      "ComputeRequest session_id must be a non-empty string",
      { field: "session_id" },
    );
  }

  if (!isNonEmptyTrimmedString(request.entry_id)) {
    return spokeReject(
      SpokeRejectCode.MISSING_REQUIRED_FIELD,
      "ComputeRequest entry_id must be a non-empty string",
      { field: "entry_id" },
    );
  }

  const computableResult = validateComputableFieldMap(request.computable);
  if (!computableResult.ok) {
    return computableResult;
  }

  if (request.settle !== undefined && typeof request.settle !== "boolean") {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "ComputeRequest settle must be a boolean when present",
      { field: "settle" },
    );
  }

  if (request.extensions !== undefined) {
    const extensionsResult = validateExtensionMap(
      request.extensions,
      "extensions",
    );
    if (!extensionsResult.ok) {
      return extensionsResult;
    }
  }

  return spokeOk();
}
