import { describe, expect, it } from "vitest";

import type { MemoryGrant } from "@42ch/spoke-schemas";

import {
  grantCovers,
  validateGrantIssuance,
  type GrantRequirement,
  type IssuedGrant,
} from "./grants.js";

const SPACE = "sp_1";
const GRANTEE = "p1";
// Fixed "now" mirroring the Rust tests.
const NOW = Date.parse("2026-06-01T00:00:00Z");

function makeGrant(overrides: Record<string, unknown>): MemoryGrant {
  return {
    schema_version: 1,
    grant_id: "g1",
    space_id: SPACE,
    principal_id: GRANTEE,
    capabilities: ["capture"],
    created_at: "2026-01-01T00:00:00Z",
    revision: 1,
    extensions: {},
    ...overrides,
  } as MemoryGrant;
}

function requirement(capability: string): GrantRequirement {
  return {
    capability,
    subjectId: null,
    contextIds: [],
    readsShared: false,
    subjectAll: false,
  };
}

function issued(capabilities: string[]): IssuedGrant {
  return {
    capabilities,
    subjectId: null,
    contextIds: [],
    sharedAccess: false,
    expiresAt: null,
  };
}

function expectNotAuthorized(result: { ok: boolean; code?: string }): void {
  expect(result.ok).toBe(false);
  expect(result.code).toBe("NOT_AUTHORIZED");
}

describe("grantCovers (§6.5)", () => {
  it("matches capability and admin implies all", () => {
    const grant = makeGrant({});
    expect(grantCovers(grant, GRANTEE, SPACE, NOW, requirement("capture"))).toBe(true);
    expect(grantCovers(grant, GRANTEE, SPACE, NOW, requirement("recall"))).toBe(false);

    const admin = makeGrant({ capabilities: ["admin"] });
    expect(grantCovers(admin, GRANTEE, SPACE, NOW, requirement("recall"))).toBe(true);
  });

  it("rejects wrong principal, wrong space, expired, revoked", () => {
    const grant = makeGrant({});
    expect(grantCovers(grant, "p_other", SPACE, NOW, requirement("capture"))).toBe(false);
    expect(grantCovers(grant, GRANTEE, "sp_other", NOW, requirement("capture"))).toBe(false);

    const expired = makeGrant({ expires_at: "2026-01-02T00:00:00Z" });
    expect(grantCovers(expired, GRANTEE, SPACE, NOW, requirement("capture"))).toBe(false);

    const revoked = makeGrant({ revoked_at: "2026-01-02T00:00:00Z" });
    expect(grantCovers(revoked, GRANTEE, SPACE, NOW, requirement("capture"))).toBe(false);

    const future = makeGrant({ expires_at: "2027-01-01T00:00:00Z" });
    expect(grantCovers(future, GRANTEE, SPACE, NOW, requirement("capture"))).toBe(true);
  });

  it("matches subject scope and rejects cross-subject", () => {
    const subjectGrant = makeGrant({ subject_id: "s1" });
    expect(
      grantCovers(subjectGrant, GRANTEE, SPACE, NOW, { ...requirement("capture"), subjectId: "s1" }),
    ).toBe(true);
    expect(
      grantCovers(subjectGrant, GRANTEE, SPACE, NOW, { ...requirement("capture"), subjectId: "s2" }),
    ).toBe(false);

    const spaceWide = makeGrant({});
    expect(
      grantCovers(spaceWide, GRANTEE, SPACE, NOW, { ...requirement("capture"), subjectId: "s2" }),
    ).toBe(true);
  });

  it("space-wide ops require space-wide grants", () => {
    // §6.5: a subject-scoped grant never covers space-wide operations.
    const subjectGrant = makeGrant({ subject_id: "s1" });
    expect(grantCovers(subjectGrant, GRANTEE, SPACE, NOW, requirement("capture"))).toBe(false);

    const spaceWide = makeGrant({});
    expect(grantCovers(spaceWide, GRANTEE, SPACE, NOW, requirement("capture"))).toBe(true);
  });

  it("covers op-named contexts only when within grant coverage", () => {
    const restricted = makeGrant({ context_ids: ["c1"] });
    const opC1 = { ...requirement("capture"), contextIds: ["c1"] };
    const opC2 = { ...requirement("capture"), contextIds: ["c2"] };
    expect(grantCovers(restricted, GRANTEE, SPACE, NOW, opC1)).toBe(true);
    expect(grantCovers(restricted, GRANTEE, SPACE, NOW, opC2)).toBe(false);

    const unrestricted = makeGrant({});
    expect(grantCovers(unrestricted, GRANTEE, SPACE, NOW, opC2)).toBe(true);

    const restrictedTwo = makeGrant({ context_ids: ["c1", "c2"] });
    expect(
      grantCovers(restrictedTwo, GRANTEE, SPACE, NOW, { ...requirement("capture"), contextIds: ["c1", "c2"] }),
    ).toBe(true);
  });

  it("shared-visibility reads require unrestricted or shared_access", () => {
    const reqShared = { ...requirement("records_read"), readsShared: true };

    const noShared = makeGrant({ capabilities: ["records_read"], context_ids: ["c1"] });
    expect(grantCovers(noShared, GRANTEE, SPACE, NOW, reqShared)).toBe(false);

    const withShared = makeGrant({
      capabilities: ["records_read"],
      context_ids: ["c1"],
      shared_access: true,
    });
    expect(grantCovers(withShared, GRANTEE, SPACE, NOW, reqShared)).toBe(true);

    const unrestricted = makeGrant({ capabilities: ["records_read"] });
    expect(grantCovers(unrestricted, GRANTEE, SPACE, NOW, reqShared)).toBe(true);
  });

  it("subject_all never passes context-limited grants", () => {
    const reqAll = { ...requirement("records_read"), subjectAll: true };

    const restricted = makeGrant({
      capabilities: ["records_read"],
      context_ids: ["c1"],
      shared_access: true,
    });
    expect(grantCovers(restricted, GRANTEE, SPACE, NOW, reqAll)).toBe(false);

    const unrestricted = makeGrant({ capabilities: ["records_read"] });
    expect(grantCovers(unrestricted, GRANTEE, SPACE, NOW, reqAll)).toBe(true);
  });
});

describe("validateGrantIssuance (§7.9, fix-round semantics)", () => {
  it("requires an applicable admin grant", () => {
    // capture-only principal can never issue.
    const captureOnly = [makeGrant({ capabilities: ["capture"] })];
    expectNotAuthorized(validateGrantIssuance(captureOnly, GRANTEE, SPACE, NOW, issued(["capture"])));

    // admin implies every capability within its coverage.
    const admin = [makeGrant({ capabilities: ["admin"] })];
    expect(validateGrantIssuance(admin, GRANTEE, SPACE, NOW, issued(["recall"])).ok).toBe(true);
  });

  it("rejects subject scope amplification", () => {
    const granter = [makeGrant({ capabilities: ["admin"], subject_id: "s1" })];
    expectNotAuthorized(
      validateGrantIssuance(granter, GRANTEE, SPACE, NOW, { ...issued(["capture"]), subjectId: "s2" }),
    );
    expect(
      validateGrantIssuance(granter, GRANTEE, SPACE, NOW, { ...issued(["capture"]), subjectId: "s1" }).ok,
    ).toBe(true);
    const spaceWide = [makeGrant({ capabilities: ["admin"] })];
    expect(
      validateGrantIssuance(spaceWide, GRANTEE, SPACE, NOW, { ...issued(["capture"]), subjectId: "s2" }).ok,
    ).toBe(true);
  });

  it("rejects context amplification and unrestricted escalation", () => {
    const granter = [makeGrant({ capabilities: ["admin"], context_ids: ["c1"] })];
    expectNotAuthorized(
      validateGrantIssuance(granter, GRANTEE, SPACE, NOW, {
        ...issued(["capture"]),
        contextIds: ["c1", "c2"],
      }),
    );
    // Critical: restricted granter can never issue an unrestricted grant.
    expectNotAuthorized(validateGrantIssuance(granter, GRANTEE, SPACE, NOW, issued(["capture"])));

    expect(
      validateGrantIssuance(granter, GRANTEE, SPACE, NOW, {
        ...issued(["capture"]),
        contextIds: ["c1"],
      }).ok,
    ).toBe(true);
    const unrestricted = [makeGrant({ capabilities: ["admin"] })];
    expect(
      validateGrantIssuance(unrestricted, GRANTEE, SPACE, NOW, {
        ...issued(["capture"]),
        contextIds: ["c1", "c2"],
      }).ok,
    ).toBe(true);
  });

  it("never unions disjoint grants", () => {
    // admin for c1 + capture for c2 cannot jointly authorize a c2 issuance.
    const granter = [
      makeGrant({ capabilities: ["admin"], context_ids: ["c1"] }),
      makeGrant({ capabilities: ["capture"], context_ids: ["c2"] }),
    ];
    expectNotAuthorized(
      validateGrantIssuance(granter, GRANTEE, SPACE, NOW, {
        ...issued(["capture"]),
        contextIds: ["c2"],
      }),
    );

    const disjointAdmins = [
      makeGrant({ capabilities: ["admin"], context_ids: ["c1"] }),
      makeGrant({ capabilities: ["admin"], context_ids: ["c2"] }),
    ];
    expectNotAuthorized(
      validateGrantIssuance(disjointAdmins, GRANTEE, SPACE, NOW, {
        ...issued(["capture"]),
        contextIds: ["c1", "c2"],
      }),
    );
  });

  it("rejects shared_access without granter shared_access", () => {
    const granter = [makeGrant({ capabilities: ["admin"], context_ids: ["c1"] })];
    const shared = { ...issued(["records_read"]), contextIds: ["c1"], sharedAccess: true };
    expectNotAuthorized(validateGrantIssuance(granter, GRANTEE, SPACE, NOW, shared));

    const withShared = [
      makeGrant({ capabilities: ["admin"], context_ids: ["c1"], shared_access: true }),
    ];
    expect(validateGrantIssuance(withShared, GRANTEE, SPACE, NOW, shared).ok).toBe(true);
  });

  it("bounds issuance by the earliest applicable admin expiry", () => {
    const granter = [
      makeGrant({ capabilities: ["admin"], expires_at: "2027-01-01T00:00:00Z" }),
      makeGrant({ capabilities: ["admin"], expires_at: "2028-01-01T00:00:00Z" }),
    ];

    const at2028 = { ...issued(["capture"]), expiresAt: Date.parse("2028-01-01T00:00:00Z") };
    expectNotAuthorized(validateGrantIssuance(granter, GRANTEE, SPACE, NOW, at2028));

    const at2027 = { ...issued(["capture"]), expiresAt: Date.parse("2027-01-01T00:00:00Z") };
    expect(validateGrantIssuance(granter, GRANTEE, SPACE, NOW, at2027).ok).toBe(true);

    // Non-expiring issuance rejected while any applicable admin grant expires.
    expectNotAuthorized(validateGrantIssuance(granter, GRANTEE, SPACE, NOW, issued(["capture"])));

    // Non-expiring granter may issue any expiry.
    const eternal = [makeGrant({ capabilities: ["admin"] })];
    expect(validateGrantIssuance(eternal, GRANTEE, SPACE, NOW, at2028).ok).toBe(true);
  });

  it("ignores inapplicable grants when bounding expiry", () => {
    // Expiring admin(s1) must not bound s2 issuance.
    const granter = [
      makeGrant({ capabilities: ["admin"], subject_id: "s2" }),
      makeGrant({
        capabilities: ["admin"],
        subject_id: "s1",
        expires_at: "2027-01-01T00:00:00Z",
      }),
    ];
    const s2NonExpiring = { ...issued(["capture"]), subjectId: "s2" };
    expect(validateGrantIssuance(granter, GRANTEE, SPACE, NOW, s2NonExpiring).ok).toBe(true);
    const s2At2028 = { ...s2NonExpiring, expiresAt: Date.parse("2028-01-01T00:00:00Z") };
    expect(validateGrantIssuance(granter, GRANTEE, SPACE, NOW, s2At2028).ok).toBe(true);
    const s1NonExpiring = { ...issued(["capture"]), subjectId: "s1" };
    expectNotAuthorized(validateGrantIssuance(granter, GRANTEE, SPACE, NOW, s1NonExpiring));

    // Context-disjoint expiring admin(c1) must not bound a c2 issuance.
    const contextGranter = [
      makeGrant({ capabilities: ["admin"], context_ids: ["c2"] }),
      makeGrant({
        capabilities: ["admin"],
        context_ids: ["c1"],
        expires_at: "2027-01-01T00:00:00Z",
      }),
    ];
    const c2NonExpiring = { ...issued(["capture"]), contextIds: ["c2"] };
    expect(validateGrantIssuance(contextGranter, GRANTEE, SPACE, NOW, c2NonExpiring).ok).toBe(true);
    const c1NonExpiring = { ...issued(["capture"]), contextIds: ["c1"] };
    expectNotAuthorized(validateGrantIssuance(contextGranter, GRANTEE, SPACE, NOW, c1NonExpiring));
  });

  it("ignores other principals and spaces", () => {
    const granter = [
      makeGrant({ principal_id: "p9", capabilities: ["admin"] }),
      makeGrant({ space_id: "sp_9", capabilities: ["admin"] }),
    ];
    expectNotAuthorized(validateGrantIssuance(granter, GRANTEE, SPACE, NOW, issued(["capture"])));
  });

  it("fails closed on malformed grant timestamps", () => {
    // Malformed expires_at → treated as expired → coverage denied.
    const badExpiry = makeGrant({ expires_at: "not-a-date" });
    expect(grantCovers(badExpiry, GRANTEE, SPACE, NOW, requirement("capture"))).toBe(false);

    // Date-only (RFC 3339 requires a full date-time with Z) → invalid.
    const dateOnly = makeGrant({ expires_at: "2027-01-01" });
    expect(grantCovers(dateOnly, GRANTEE, SPACE, NOW, requirement("capture"))).toBe(false);

    // Calendar-invalid date must not be normalized by Date.parse.
    const impossible = makeGrant({ expires_at: "2027-02-30T00:00:00Z" });
    expect(grantCovers(impossible, GRANTEE, SPACE, NOW, requirement("capture"))).toBe(false);

    // Malformed revoked_at → treated as revoked → coverage denied.
    const badRevoked = makeGrant({ revoked_at: "not-a-date" });
    expect(grantCovers(badRevoked, GRANTEE, SPACE, NOW, requirement("capture"))).toBe(false);

    // Admin grant with malformed expiry is excluded from applicability, so
    // issuance fails NOT_AUTHORIZED (no silent unbounded coverage).
    const malformedAdmin = [makeGrant({ capabilities: ["admin"], expires_at: "garbage" })];
    expectNotAuthorized(
      validateGrantIssuance(malformedAdmin, GRANTEE, SPACE, NOW, issued(["capture"])),
    );
    expectNotAuthorized(
      validateGrantIssuance(malformedAdmin, GRANTEE, SPACE, NOW, {
        ...issued(["capture"]),
        expiresAt: Date.parse("2027-01-01T00:00:00Z"),
      }),
    );

    // Date-only expiry on the only admin grant also fails closed.
    const dateOnlyAdmin = [makeGrant({ capabilities: ["admin"], expires_at: "2027-01-01" })];
    expectNotAuthorized(
      validateGrantIssuance(dateOnlyAdmin, GRANTEE, SPACE, NOW, issued(["capture"])),
    );

    // Malformed revoked_at on the only admin grant also fails closed.
    const revokedAdmin = [makeGrant({ capabilities: ["admin"], revoked_at: "garbage" })];
    expectNotAuthorized(validateGrantIssuance(revokedAdmin, GRANTEE, SPACE, NOW, issued(["capture"])));
  });

  it("rejects non-finite issued expiries before the bound comparison", () => {
    const granter = [makeGrant({ capabilities: ["admin"], expires_at: "2027-01-01T00:00:00Z" })];

    expectNotAuthorized(
      validateGrantIssuance(granter, GRANTEE, SPACE, NOW, {
        ...issued(["capture"]),
        expiresAt: Number.NaN,
      }),
    );
    expectNotAuthorized(
      validateGrantIssuance(granter, GRANTEE, SPACE, NOW, {
        ...issued(["capture"]),
        expiresAt: Number.POSITIVE_INFINITY,
      }),
    );

    // Valid equal and earlier expiries remain accepted.
    expect(
      validateGrantIssuance(granter, GRANTEE, SPACE, NOW, {
        ...issued(["capture"]),
        expiresAt: Date.parse("2027-01-01T00:00:00Z"),
      }).ok,
    ).toBe(true);
    expect(
      validateGrantIssuance(granter, GRANTEE, SPACE, NOW, {
        ...issued(["capture"]),
        expiresAt: NOW + 30 * 24 * 60 * 60 * 1000,
      }).ok,
    ).toBe(true);
  });
});
