import type { Keyblock } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import { SpokeRejectCode } from "../result.js";
import {
  isValidKeyblockStatusTransition,
  transitionKeyblockStatus,
} from "./transition.js";

function makeKeyblock(status: string, overrides: Partial<Keyblock> = {}): Keyblock {
  return {
    schema_version: 1,
    keyblock_id: "kb_1",
    block_type: "character",
    canonical_name: "Mira Vale",
    status,
    body: { summary: "Protagonist" },
    extensions: {},
    ...overrides,
  };
}

describe("isValidKeyblockStatusTransition", () => {
  it.each([
    ["provisional", "confirmed"],
    ["provisional", "deprecated"],
    ["provisional", "merged"],
    ["provisional", "deleted"],
    ["confirmed", "deprecated"],
    ["confirmed", "merged"],
    ["confirmed", "deleted"],
    ["deprecated", "confirmed"],
    ["deprecated", "deleted"],
    ["provisional", "provisional"],
    ["confirmed", "confirmed"],
    ["deprecated", "deprecated"],
    ["merged", "merged"],
    ["deleted", "deleted"],
  ] as const)("allows %s -> %s", (from, to) => {
    expect(isValidKeyblockStatusTransition(from, to)).toBe(true);
  });

  it.each([
    ["deprecated", "merged"],
    ["merged", "confirmed"],
    ["deleted", "provisional"],
    ["confirmed", "provisional"],
    ["merged", "deleted"],
    ["deleted", "confirmed"],
    ["provisional", "bogus"],
    ["bogus", "confirmed"],
  ] as const)("rejects %s -> %s", (from, to) => {
    expect(isValidKeyblockStatusTransition(from, to)).toBe(false);
  });
});

describe("transitionKeyblockStatus", () => {
  it("updates status on allowed transition without mutating input", () => {
    const keyblock = makeKeyblock("provisional");
    const result = transitionKeyblockStatus(keyblock, "confirmed");

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.status).toBe("confirmed");
      expect(keyblock.status).toBe("provisional");
    }
  });

  it("accepts no-op same-status transition", () => {
    const keyblock = makeKeyblock("merged");
    const result = transitionKeyblockStatus(keyblock, "merged");

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.status).toBe("merged");
    }
  });

  it("rejects invalid target status", () => {
    const result = transitionKeyblockStatus(makeKeyblock("provisional"), "bogus");

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_KEYBLOCK_STATUS);
      expect(result.details).toEqual({ status: "bogus" });
    }
  });

  it("rejects invalid current status", () => {
    const result = transitionKeyblockStatus(makeKeyblock("bogus"), "confirmed");

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_KEYBLOCK_STATUS);
      expect(result.details).toEqual({ status: "bogus" });
    }
  });

  it("rejects disallowed transition with from/to details", () => {
    const result = transitionKeyblockStatus(makeKeyblock("deprecated"), "merged");

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_KEYBLOCK_STATUS_TRANSITION);
      expect(result.details).toEqual({ from: "deprecated", to: "merged" });
    }
  });

  it("rejects terminal outbound transition", () => {
    const result = transitionKeyblockStatus(makeKeyblock("deleted"), "provisional");

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_KEYBLOCK_STATUS_TRANSITION);
      expect(result.details).toEqual({ from: "deleted", to: "provisional" });
    }
  });
});
