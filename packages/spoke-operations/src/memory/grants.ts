import type { MemoryGrant } from "@42ch/spoke-schemas";
import { spokeOk, spokeReject, type SpokeResult } from "../result.js";

/** What one op needs from a single grant (§6.5, §7 tables). */
export interface GrantRequirement {
  capability: string;
  subjectId: string | null;
  contextIds: string[];
  readsShared: boolean;
  subjectAll: boolean;
}

interface GrantView {
  capabilities: string[];
  subjectId: string | null;
  contextIds: string[];
  sharedAccess: boolean;
  expiresAt: number | null;
  revokedAt: number | null;
  principalId: string;
  spaceId: string;
}

const RFC3339_UTC =
  /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d+)?Z$/;

function parseTimestamp(value: string | undefined): number | null {
  if (value === undefined) {
    return null;
  }
  const match = RFC3339_UTC.exec(value);
  if (match === null) {
    return Number.NaN;
  }
  const parsed = Date.parse(value);
  if (!Number.isFinite(parsed)) {
    return Number.NaN;
  }
  const date = new Date(parsed);
  const sameInstant =
    date.getUTCFullYear() === Number(match[1]) &&
    date.getUTCMonth() + 1 === Number(match[2]) &&
    date.getUTCDate() === Number(match[3]) &&
    date.getUTCHours() === Number(match[4]) &&
    date.getUTCMinutes() === Number(match[5]) &&
    date.getUTCSeconds() === Number(match[6]);
  return sameInstant ? parsed : Number.NaN;
}

function isLive(view: GrantView, now: number): boolean {
  if (view.revokedAt !== null) {
    return false;
  }
  return view.expiresAt === null || view.expiresAt > now;
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
  if (!isLive(view, now)) {
    return false;
  }
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
  capabilities: string[];
  subjectId: string | null;
  contextIds: string[];
  sharedAccess: boolean;
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
  const applicable = granterGrants
    .map(viewOf)
    .filter(
      (view) =>
        view.principalId === principalId &&
        view.spaceId === spaceId &&
        isLive(view, now) &&
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

  if (issued.expiresAt !== null && !Number.isFinite(issued.expiresAt)) {
    return notAuthorized("issued expires_at is not a finite timestamp");
  }

  const expiries = applicable
    .map((view) => view.expiresAt)
    .filter(
      (expiresAt): expiresAt is number =>
        expiresAt !== null && Number.isFinite(expiresAt),
    );
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
