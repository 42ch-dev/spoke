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
  INVALID_KNOWLEDGE_ENTRY_STATUS: "INVALID_KNOWLEDGE_ENTRY_STATUS",
  INVALID_KNOWLEDGE_ENTRY_STATUS_TRANSITION:
    "INVALID_KNOWLEDGE_ENTRY_STATUS_TRANSITION",
  DUPLICATE_ACTIVE_KNOWLEDGE_ENTRY: "DUPLICATE_ACTIVE_KNOWLEDGE_ENTRY",
  KNOWLEDGE_ENTRY_NOT_FOUND: "KNOWLEDGE_ENTRY_NOT_FOUND",
  KNOWLEDGE_ENTRY_ALREADY_EXISTS: "KNOWLEDGE_ENTRY_ALREADY_EXISTS",
  KNOWLEDGE_ENTRY_TERMINAL_STATUS: "KNOWLEDGE_ENTRY_TERMINAL_STATUS",
  RELATION_SELF_EDGE: "RELATION_SELF_EDGE",
  RELATION_MISSING_ENDPOINT: "RELATION_MISSING_ENDPOINT",
  RELATION_NOT_FOUND: "RELATION_NOT_FOUND",
  RELATION_ALREADY_EXISTS: "RELATION_ALREADY_EXISTS",
  CAPABILITY_PORT_MISSING: "CAPABILITY_PORT_MISSING",
  /**
   * Unexpected internal / server-side failure (HTTP 500 class). Distinct from
   * the 4xx validation codes; hosts raise it for storage or runtime errors
   * that are not the caller's fault.
   */
  INTERNAL_ERROR: "INTERNAL_ERROR",
  // Memory-system vocabulary (spoke-memory, §14.1 of the memory protocol).
  /** Memory op target does not exist (or is hidden from the principal). */
  NOT_FOUND: "NOT_FOUND",
  /** Principal lacks coverage; also used when the provider declines to reveal existence. */
  NOT_AUTHORIZED: "NOT_AUTHORIZED",
  /** The presented grant exists but is revoked/expired (distinct from coverage-absence NOT_AUTHORIZED). */
  GRANT_REVOKED: "GRANT_REVOKED",
  /** Space is frozen for maintenance/migration; op is outside the §8.2 allowlist. */
  SPACE_FROZEN: "SPACE_FROZEN",
  /** Subject (or context) is fenced; writes are rejected, reads allowed. */
  SUBJECT_FENCED: "SUBJECT_FENCED",
  /** Candidate was already consumed (§8.1 terminal state). */
  CANDIDATE_CONSUMED: "CANDIDATE_CONSUMED",
  /** Reflection admission failed: a required profile kind is absent or empty. */
  PROFILE_INCOMPLETE: "PROFILE_INCOMPLETE",
  /** Reflection admission failed: provider/engine data-sufficiency policy gate. */
  INSUFFICIENT_DATA: "INSUFFICIENT_DATA",
  /** Same idempotency key replayed with a different payload digest. */
  IDEMPOTENCY_CONFLICT: "IDEMPOTENCY_CONFLICT",
  /** Commit state unknown (§14.3); resolve via same-key replay or memory.receipts.get. */
  OUTCOME_UNCERTAIN: "OUTCOME_UNCERTAIN",
  /** Export bundle is past export_retention_seconds. */
  EXPORT_EXPIRED: "EXPORT_EXPIRED",
  /** Staged import verification failed, or the import left the staged state. */
  IMPORT_CONFLICT: "IMPORT_CONFLICT",
  /** Receipt body was evicted; details.last_known_status from the dedup tombstone. */
  RECEIPT_EXPIRED: "RECEIPT_EXPIRED",
  /** Pagination cursor invalidated by state or grant change (§7.0.7). */
  CURSOR_INVALID: "CURSOR_INVALID",
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
