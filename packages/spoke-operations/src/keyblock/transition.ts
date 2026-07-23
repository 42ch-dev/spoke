import type { Keyblock } from "@42ch/spoke-schemas";

import { spokeOk, spokeReject, type SpokeResult } from "../result.js";
import { SpokeRejectCode } from "../result.js";

const CORE_KEYBLOCK_STATUSES = new Set([
  "provisional",
  "confirmed",
  "deprecated",
  "merged",
  "deleted",
]);

const ALLOWED_TRANSITIONS: ReadonlyMap<string, ReadonlySet<string>> = new Map([
  [
    "provisional",
    new Set(["confirmed", "deprecated", "merged", "deleted", "provisional"]),
  ],
  ["confirmed", new Set(["deprecated", "merged", "deleted", "confirmed"])],
  ["deprecated", new Set(["confirmed", "deleted", "deprecated"])],
  ["merged", new Set(["merged"])],
  ["deleted", new Set(["deleted"])],
]);

function isCoreKeyblockStatus(status: string): boolean {
  return CORE_KEYBLOCK_STATUSES.has(status);
}

/**
 * Returns whether a Keyblock status transition is allowed by the cross-product table.
 */
export function isValidKeyblockStatusTransition(
  from: string,
  to: string,
): boolean {
  if (!isCoreKeyblockStatus(from) || !isCoreKeyblockStatus(to)) {
    return false;
  }

  const allowedTargets = ALLOWED_TRANSITIONS.get(from);
  return allowedTargets?.has(to) ?? false;
}

/**
 * Apply a Keyblock status transition; returns updated status on success without mutating input.
 */
export function transitionKeyblockStatus(
  keyblock: Keyblock,
  to: string,
): SpokeResult<Keyblock> {
  if (!isCoreKeyblockStatus(to)) {
    return spokeReject(
      SpokeRejectCode.INVALID_KEYBLOCK_STATUS,
      `Invalid keyblock status: ${to}`,
      { status: to },
    );
  }

  if (!isCoreKeyblockStatus(keyblock.status)) {
    return spokeReject(
      SpokeRejectCode.INVALID_KEYBLOCK_STATUS,
      `Invalid current keyblock status: ${keyblock.status}`,
      { status: keyblock.status },
    );
  }

  if (!isValidKeyblockStatusTransition(keyblock.status, to)) {
    return spokeReject(
      SpokeRejectCode.INVALID_KEYBLOCK_STATUS_TRANSITION,
      `Disallowed keyblock status transition: ${keyblock.status} -> ${to}`,
      { from: keyblock.status, to },
    );
  }

  return spokeOk({
    ...keyblock,
    status: to,
  });
}
