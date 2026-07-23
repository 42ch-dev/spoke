/**
 * Unified success/reject envelope for all SPOKE operations helpers.
 */

export const SpokeRejectCode = {
  INVALID_INPUT: "INVALID_INPUT",
  INVALID_STATUS: "INVALID_STATUS",
  INVALID_STATUS_TRANSITION: "INVALID_STATUS_TRANSITION",
  CANDIDATE_NOT_PROVISIONAL: "CANDIDATE_NOT_PROVISIONAL",
  CANDIDATE_TERMINAL_STATUS: "CANDIDATE_TERMINAL_STATUS",
  EMPTY_CANONICAL_NAME: "EMPTY_CANONICAL_NAME",
  MERGE_TARGET_SELF: "MERGE_TARGET_SELF",
  MISSING_REQUIRED_FIELD: "MISSING_REQUIRED_FIELD",
  INVALID_PACKET_INPUT: "INVALID_PACKET_INPUT",
  REVISION_CONFLICT: "REVISION_CONFLICT",
  STORED_REVISION_STALE: "STORED_REVISION_STALE",
} as const;

export type SpokeRejectCode =
  (typeof SpokeRejectCode)[keyof typeof SpokeRejectCode];

export type SpokeOk<T = void> = [T] extends [void]
  ? { ok: true }
  : { ok: true; value: T };

export type SpokeReject = {
  ok: false;
  code: SpokeRejectCode;
  message: string;
  details?: Record<string, unknown>;
};

export type SpokeResult<T = void> = SpokeOk<T> | SpokeReject;

export function spokeOk(): SpokeOk<void>;
export function spokeOk<T>(value: T): SpokeOk<T>;
export function spokeOk<T>(value?: T): SpokeOk<T> | SpokeOk<void> {
  if (value === undefined) {
    return { ok: true };
  }
  return { ok: true, value } as SpokeOk<T>;
}

export function spokeReject(
  code: SpokeRejectCode,
  message: string,
  details?: Record<string, unknown>,
): SpokeReject {
  return details === undefined
    ? { ok: false, code, message }
    : { ok: false, code, message, details };
}
