import type { Finding } from "@42ch/spoke-schema";

import { spokeOk, spokeReject, type SpokeResult } from "../result.js";
import { SpokeRejectCode } from "../result.js";

const CORE_FINDING_STATUSES = new Set(["open", "resolved", "dismissed"]);

const ALLOWED_TRANSITIONS: ReadonlyMap<string, ReadonlySet<string>> = new Map([
  ["open", new Set(["resolved", "dismissed", "open"])],
  ["resolved", new Set(["open", "resolved"])],
  ["dismissed", new Set(["open", "dismissed"])],
]);

function isCoreFindingStatus(status: string): boolean {
  return CORE_FINDING_STATUSES.has(status);
}

/**
 * Returns whether a Finding status transition is allowed by the cross-product table.
 */
export function isValidFindingStatusTransition(
  from: string,
  to: string,
): boolean {
  if (!isCoreFindingStatus(from) || !isCoreFindingStatus(to)) {
    return false;
  }

  const allowedTargets = ALLOWED_TRANSITIONS.get(from);
  return allowedTargets?.has(to) ?? false;
}

/**
 * Apply a Finding status transition; returns updated status and updated_at on success.
 */
export function transitionFindingStatus(
  finding: Finding,
  to: string,
): SpokeResult<Finding> {
  if (!isCoreFindingStatus(to)) {
    return spokeReject(
      SpokeRejectCode.INVALID_STATUS,
      `Invalid finding status: ${to}`,
      { status: to },
    );
  }

  if (!isCoreFindingStatus(finding.status)) {
    return spokeReject(
      SpokeRejectCode.INVALID_STATUS,
      `Invalid current finding status: ${finding.status}`,
      { status: finding.status },
    );
  }

  if (!isValidFindingStatusTransition(finding.status, to)) {
    return spokeReject(
      SpokeRejectCode.INVALID_STATUS_TRANSITION,
      `Disallowed finding status transition: ${finding.status} -> ${to}`,
      { from: finding.status, to },
    );
  }

  return spokeOk({
    ...finding,
    status: to,
    updated_at: new Date().toISOString(),
  });
}
