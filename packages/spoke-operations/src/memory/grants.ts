/**
 * Pure grant-coverage evaluation (§6.5, MEM-R006/R017) and the
 * non-amplification checks for `memory.grants.grant` (§7.9) — TS parity
 * with `crates/spoke-operations/src/memory/grants.rs`, including the
 * fix-round semantics (space-wide scope matching, admin-only single-grant
 * issuance, earliest-*applicable*-expiry bound).
 *
 * Time is an explicit epoch-milliseconds input (`now`), mirroring the Rust
 * helper's explicit `DateTime<Utc>` parameter; grant `expires_at` /
 * `revoked_at` ISO-8601 strings are parsed for comparison.
 */

import type { MemoryGrant } from "@42ch/spoke-schemas";
import { spokeOk, spokeReject, type SpokeResult } from "../result.js";

/** What one op needs from a single grant (§6.5, §7 tables). */
export interface GrantRequirement {
  /** Required capability from the §7 grant table (`admin` implies all within coverage). */
  capability: string;
  /** Subject the op names; `null` for space-wide ops. */
  subjectId: string | null;
  /** Placement contexts the op names; empty = the op names no contexts. */
  contextIds: string[];
  /** The op reads shared visibility. */
  readsShared: boolean;
  /** The op uses a `subject_all` selection — only unrestricted grants pass. */
  subjectAll: boolean;
}

interface GrantView {
  capabilities: string[];
  subjectId: string | null;
  /** Empty = unrestricted within the grant's subject/space scope (§6.5). */
  contextIds: string[];
  sharedAccess: boolean;
  expiresAt: number | null;
  revokedAt: number | null;
  principalId: string;
  spaceId: string;
}

function parseTimestamp(value: string | undefined): number | null {
  if (value === undefined) {
    return null;
  }
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? null : parsed;
}

function viewOf(grant: MemoryGrant): GrantView {
  const base = {
    capabilities: grant.capabilities,
    subjectId: grant.subject_id ?? null,
    sharedAccess: "shared_access" in grant ? (grant.shared_access ?? false) : false,
    expiresAt: parseTimestamp(grant.expires_at),
    revokedAt: parseTimestamp(grant.revoked_at),
    principalId: grant.principal_id,
    spaceId: grant.space_id,
  };
  if ("context_ids" in grant) {
    return { ...base, contextIds: [...grant.context_ids] };
  }
  return { ...base, contextIds: [] };
}

/** §6.5 grant evaluation (MEM-R006/R017): is `grant` sufficient for `requirement`? */
export function grantCovers(
  grant: MemoryGrant,
  principalId: string,
  spaceId: string,
  now: number,
  requirement: GrantRequirement,
): boolean {
  const view = viewOf(grant);
  if (view.principalId !== principalId || view.spaceId !== spaceId) {
    return false;
  }
  if (view.revokedAt !== null) {
    return false;
  }
  if (view.expiresAt !== null && view.expiresAt <= now) {
    return false;
  }
  // Subject scope (§6.5): a subject-scoped grant never covers another
  // subject, and never covers space-wide ops.
  if (requirement.subjectId !== null) {
    if (view.subjectId !== null && view.subjectId !== requirement.subjectId) {
      return false;
    }
  } else if (view.subjectId !== null) {
    return false;
  }
  const hasCapability = view.capabilities.some(
    (capability) => capability === requirement.capability || capability === "admin",
  );
  if (!hasCapability) {
    return false;
  }
  const unrestricted = view.contextIds.length === 0;
  if (
    requirement.contextIds.length > 0 &&
    !unrestricted &&
    !requirement.contextIds.every((context) => view.contextIds.includes(context))
  ) {
    return false;
  }
  if (requirement.readsShared && !unrestricted && !view.sharedAccess) {
    return false;
  }
  if (requirement.subjectAll && !unrestricted) {
    return false;
  }
  return true;
}

/** The grant being issued via `memory.grants.grant`. */
export interface IssuedGrant {
  /** Requested capabilities (§7.9 `minItems: 1`, `uniqueItems: true` — callers validate). */
  capabilities: string[];
  /** `null` = space-wide issuance; otherwise the subject scope. */
  subjectId: string | null;
  /** Requested placement-context restriction; empty = unrestricted. */
  contextIds: string[];
  /** Requested shared visibility access (meaningful only when restricted). */
  sharedAccess: boolean;
  /** Requested expiry (epoch ms); `null` = non-expiring. */
  expiresAt: number | null;
}

function notAuthorized(reason: string): SpokeResult<void> {
  return spokeReject(
    "NOT_AUTHORIZED",
    `Grant issuance would amplify granter coverage: ${reason}`,
  );
}

/**
 * §7.9 non-amplification, two stages (parity with the Rust helper):
 *
 * 1. **Applicability** — a valid (unrevoked, unexpired) `admin` grant for
 *    the principal and space whose coverage matches the issued grant's
 *    facets (subject scope, context scope, `shared_access`). Empty set →
 *    `NOT_AUTHORIZED`; the matching grant is the §6.5 single-grant coverage
 *    (disjoint grants are never unioned).
 * 2. **Expiry bound** — over the *applicable* set only: the issued
 *    `expires_at` must be ≤ the earliest applicable admin expiry, and a
 *    non-expiring issuance requires that no applicable admin grant expires.
 */
export function validateGrantIssuance(
  granterGrants: MemoryGrant[],
  principalId: string,
  spaceId: string,
  now: number,
  issued: IssuedGrant,
): SpokeResult<void> {
  // Stage 1: applicable admin grants.
  const applicable = granterGrants
    .map(viewOf)
    .filter(
      (view) =>
        view.principalId === principalId &&
        view.spaceId === spaceId &&
        view.revokedAt === null &&
        (view.expiresAt === null || view.expiresAt > now) &&
        view.capabilities.includes("admin"),
    )
    .filter((view) => {
      const subjectOk =
        issued.subjectId === null
          ? view.subjectId === null
          : view.subjectId === null || view.subjectId === issued.subjectId;
      const contextsOk =
        issued.contextIds.length === 0
          ? view.contextIds.length === 0
          : view.contextIds.length === 0 ||
            issued.contextIds.every((context) => view.contextIds.includes(context));
      const sharedOk =
        !issued.sharedAccess || view.contextIds.length === 0 || view.sharedAccess;
      return subjectOk && contextsOk && sharedOk;
    });

  if (applicable.length === 0) {
    return notAuthorized(
      "no applicable admin grant matches the issued subject, contexts, and shared_access",
    );
  }

  // Stage 2: expiry bound over the applicable set only.
  const expiries = applicable
    .map((view) => view.expiresAt)
    .filter((expiresAt): expiresAt is number => expiresAt !== null);
  const earliestApplicableExpiry = expiries.length === 0 ? null : Math.min(...expiries);
  if (issued.expiresAt !== null) {
    if (earliestApplicableExpiry !== null && issued.expiresAt > earliestApplicableExpiry) {
      return notAuthorized(
        "expires_at is later than the earliest applicable admin grant expiry",
      );
    }
  } else if (earliestApplicableExpiry !== null) {
    return notAuthorized(
      "non-expiring issuance requires a non-expiring applicable admin grant",
    );
  }
  return spokeOk();
}
