/**
 * `spoke-history` family helpers: the port result type and the
 * canonical-JSON snapshot comparison (spoke-history contract: two providers
 * returning the same snapshot at the same revision MUST produce identical
 * bytes under RFC 8785 canonical JSON).
 *
 * Rust parity: `crates/spoke-operations/src/history.rs`.
 */

import canonicalize from "canonicalize";

import type { RevisionSnapshot } from "@42ch/spoke-schemas";

import { spokeOk, spokeReject, type SpokeResult } from "./result.js";

/**
 * `history.revisions.prune` success value (spoke-history contract
 * §Checkpoints, pruning, and `floor_revision`): a plain object — the schema
 * carries no named `HistoryPruneResult` type.
 *
 * `deleted` counts checkpoints deleted by this call; it excludes the live
 * row and does not count derived revisions. `floor_revision` is
 * `newest_deleted_checkpoint_revision + 1` when at least one checkpoint was
 * deleted; `null` on a no-op, including `keep` >= the available checkpoint
 * count.
 */
export interface HistoryPruneResult {
  /** Number of checkpoints deleted by this call. */
  deleted: number;
  /** Newest deleted checkpoint revision + 1, or `null` on a no-op. The current retained floor for subsequent replay. */
  floor_revision: number | null;
}

/**
 * RFC 8785 canonical JSON bytes (as a string) of `value`, or `null` when the
 * value cannot be canonicalized (mirrors `serde_jcs::to_vec` failure).
 */
function canonicalJson(value: unknown): string | undefined {
  try {
    return canonicalize(value);
  } catch {
    return undefined;
  }
}

/**
 * Whether two `RevisionSnapshot`s are byte-identical under RFC 8785
 * canonical JSON — the contract's provider-equality rule.
 *
 * Rejects with `INVALID_INPUT` when either snapshot cannot be serialized to
 * canonical bytes.
 */
export function revisionSnapshotsEqualCanonical(
  left: RevisionSnapshot,
  right: RevisionSnapshot,
): SpokeResult<boolean> {
  const leftBytes = canonicalJson(left);
  if (leftBytes === undefined) {
    return spokeReject(
      "INVALID_INPUT",
      "JCS canonicalization failed: value is not JSON-canonicalizable (RFC 8785)",
    );
  }
  const rightBytes = canonicalJson(right);
  if (rightBytes === undefined) {
    return spokeReject(
      "INVALID_INPUT",
      "JCS canonicalization failed: value is not JSON-canonicalizable (RFC 8785)",
    );
  }
  return spokeOk(leftBytes === rightBytes);
}
