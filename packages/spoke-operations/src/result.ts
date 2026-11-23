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
   * note: Unexpected internal / server-side failure (HTTP 500 class). Distinct
   * from the 4xx validation codes; hosts raise it for storage or runtime errors
   * that are not the caller's fault.
   */
  INTERNAL_ERROR: "INTERNAL_ERROR",
  // note: Memory-system vocabulary (spoke-memory, §14.1 of the memory protocol).
  /** note: Memory op target does not exist (or is hidden from the principal). */
  NOT_FOUND: "NOT_FOUND",
  /** note: Principal lacks coverage; also used when the provider declines to reveal existence. */
  NOT_AUTHORIZED: "NOT_AUTHORIZED",
  /** note: The presented grant exists but is revoked/expired (distinct from coverage-absence NOT_AUTHORIZED). */
  GRANT_REVOKED: "GRANT_REVOKED",
  /** note: Space is frozen for maintenance/migration; op is outside the §8.2 allowlist. */
  SPACE_FROZEN: "SPACE_FROZEN",
  /** note: Subject (or context) is fenced; writes are rejected, reads allowed. */
  SUBJECT_FENCED: "SUBJECT_FENCED",
  /** note: Candidate was already consumed (§8.1 terminal state). */
  CANDIDATE_CONSUMED: "CANDIDATE_CONSUMED",
  /** note: Reflection admission failed: a required profile kind is absent or empty. */
  PROFILE_INCOMPLETE: "PROFILE_INCOMPLETE",
  /** note: Reflection admission failed: provider/engine data-sufficiency policy gate. */
  INSUFFICIENT_DATA: "INSUFFICIENT_DATA",
  /** note: Same idempotency key replayed with a different payload digest. */
  IDEMPOTENCY_CONFLICT: "IDEMPOTENCY_CONFLICT",
  /** note: Commit state unknown (§14.3); resolve via same-key replay or memory.receipts.get. */
  OUTCOME_UNCERTAIN: "OUTCOME_UNCERTAIN",
  /** note: Export bundle is past export_retention_seconds. */
  EXPORT_EXPIRED: "EXPORT_EXPIRED",
  /** note: Staged import verification failed, or the import left the staged state. */
  IMPORT_CONFLICT: "IMPORT_CONFLICT",
  /** note: Receipt body was evicted; details.last_known_status from the dedup tombstone. */
  RECEIPT_EXPIRED: "RECEIPT_EXPIRED",
  /** note: Pagination cursor invalidated by state or grant change (§7.0.7). */
  CURSOR_INVALID: "CURSOR_INVALID",
  // note: History-family vocabulary (spoke-history contract §Refusal vocabulary).
  /** note: The requested revision existed below the current retained floor and its checkpoint history was discarded (details.floor_revision). */
  HISTORY_TRUNCATED: "HISTORY_TRUNCATED",
  /** note: The request passed the spoke-history connect gate, but the host does not serve the requested entity_kind (details.entity_kind). */
  HISTORY_ENTITY_UNSUPPORTED: "HISTORY_ENTITY_UNSUPPORTED",
  /** note: The requested revision is at or above the floor but cannot be reconstructed — an implementation-declared retention gap (details.retention_reason). */
  HISTORY_NOT_RETAINED: "HISTORY_NOT_RETAINED",
  // note: Adoption-family vocabulary (spoke-knowledge-adoption contract §Refusal vocabulary).
  /** note: A normal put attempted to change or remove an existing destination adoption_anchor (details.entry_id). */
  ADOPTION_ANCHOR_IMMUTABLE: "ADOPTION_ANCHOR_IMMUTABLE",
  /** note: Retirement-chain resolution hit a cycle, ambiguity, deleted terminal, or the 32-edge limit (details.retirement_chain). */
  SOURCE_RETIRED: "SOURCE_RETIRED",
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
