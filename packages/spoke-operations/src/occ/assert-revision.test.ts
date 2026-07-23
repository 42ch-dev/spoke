import { describe, expect, it } from "vitest";

import { SpokeRejectCode } from "../result.js";
import { assertRevisionMatch } from "./assert-revision.js";

describe("assertRevisionMatch", () => {
  it("accepts equal non-negative integer revisions", () => {
    expect(assertRevisionMatch(0, 0).ok).toBe(true);
    expect(assertRevisionMatch(3, 3).ok).toBe(true);
  });

  it("rejects when actual revision is greater than expected", () => {
    const result = assertRevisionMatch(2, 5);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.STORED_REVISION_STALE);
    }
  });

  it("rejects when actual revision is less than expected", () => {
    const result = assertRevisionMatch(5, 2);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.REVISION_CONFLICT);
    }
  });

  it.each([
    ["negative expected", -1, 0],
    ["negative actual", 0, -1],
    ["non-integer expected", 1.5, 1],
    ["non-integer actual", 1, 1.5],
    ["NaN expected", Number.NaN, 0],
    ["NaN actual", 0, Number.NaN],
  ] as const)("rejects invalid input: %s", (_label, expected, actual) => {
    const result = assertRevisionMatch(expected, actual);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });
});
