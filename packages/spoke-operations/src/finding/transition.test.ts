import type { Finding } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import { SpokeRejectCode } from "../result.js";
import {
  isValidFindingStatusTransition,
  transitionFindingStatus,
} from "./transition.js";

function makeFinding(status: string): Finding {
  return {
    schema_version: 1,
    finding_id: "fnd_1",
    severity: "warning",
    status,
    title: "Title",
    description: "Description",
    extensions: {},
  };
}

describe("isValidFindingStatusTransition", () => {
  it.each([
    ["open", "resolved"],
    ["open", "dismissed"],
    ["resolved", "open"],
    ["dismissed", "open"],
    ["open", "open"],
    ["resolved", "resolved"],
    ["dismissed", "dismissed"],
  ] as const)("allows %s -> %s", (from, to) => {
    expect(isValidFindingStatusTransition(from, to)).toBe(true);
  });

  it.each([
    ["resolved", "dismissed"],
    ["dismissed", "resolved"],
    ["open", "invalid"],
    ["invalid", "open"],
  ] as const)("rejects %s -> %s", (from, to) => {
    expect(isValidFindingStatusTransition(from, to)).toBe(false);
  });
});

describe("transitionFindingStatus", () => {
  it("accepts allowed transitions and sets updated_at", () => {
    const finding = makeFinding("open");
    const result = transitionFindingStatus(finding, "resolved");

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.status).toBe("resolved");
      expect(result.value.updated_at).toMatch(/^\d{4}-\d{2}-\d{2}T/);
      expect(finding.status).toBe("open");
    }
  });

  it("accepts no-op same-status transition", () => {
    const finding = makeFinding("dismissed");
    const result = transitionFindingStatus(finding, "dismissed");

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.status).toBe("dismissed");
    }
  });

  it("rejects invalid target status", () => {
    const result = transitionFindingStatus(makeFinding("open"), "bogus");

    expect(result).toEqual({
      ok: false,
      code: SpokeRejectCode.INVALID_STATUS,
      message: "Invalid finding status: bogus",
      details: { status: "bogus" },
    });
  });

  it("rejects disallowed transition", () => {
    const result = transitionFindingStatus(makeFinding("resolved"), "dismissed");

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_STATUS_TRANSITION);
      expect(result.details).toEqual({ from: "resolved", to: "dismissed" });
    }
  });
});
