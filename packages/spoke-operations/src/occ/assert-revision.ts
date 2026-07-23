import { spokeOk, spokeReject, type SpokeResult } from "../result.js";
import { SpokeRejectCode } from "../result.js";

function isValidRevision(value: number): boolean {
  return Number.isInteger(value) && value >= 0;
}

/**
 * Compare caller-supplied revisions before persist; library performs no storage I/O.
 */
export function assertRevisionMatch(
  expectedRevision: number,
  actualRevision: number,
): SpokeResult<void> {
  if (!isValidRevision(expectedRevision) || !isValidRevision(actualRevision)) {
    return spokeReject(
      SpokeRejectCode.INVALID_INPUT,
      "Revisions must be non-negative integers",
      { expectedRevision, actualRevision },
    );
  }

  if (expectedRevision === actualRevision) {
    return spokeOk();
  }

  if (actualRevision > expectedRevision) {
    return spokeReject(
      SpokeRejectCode.STORED_REVISION_STALE,
      `Stored revision ${actualRevision} is ahead of expected ${expectedRevision}`,
      { expectedRevision, actualRevision },
    );
  }

  return spokeReject(
    SpokeRejectCode.REVISION_CONFLICT,
    `Expected revision ${expectedRevision} is ahead of actual ${actualRevision}`,
    { expectedRevision, actualRevision },
  );
}
