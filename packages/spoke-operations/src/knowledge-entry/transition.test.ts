import type { KnowledgeEntry } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import { SpokeRejectCode } from "../result.js";
import {
  isValidKnowledgeEntryStatusTransition,
  transitionKnowledgeEntryStatus,
} from "./transition.js";

function makeKnowledgeEntry(
  status: string,
  overrides: Partial<KnowledgeEntry> = {},
): KnowledgeEntry {
  return {
    schema_version: 1,
    knowledge_entry_id: "kb_1",
    block_type: "character",
    canonical_name: "Mira Vale",
    status,
    body: { summary: "Protagonist" },
    extensions: {},
    ...overrides,
  };
}

describe("isValidKnowledgeEntryStatusTransition", () => {
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
    expect(isValidKnowledgeEntryStatusTransition(from, to)).toBe(true);
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
    expect(isValidKnowledgeEntryStatusTransition(from, to)).toBe(false);
  });
});

describe("transitionKnowledgeEntryStatus", () => {
  it("updates status on allowed transition without mutating input", () => {
    const knowledgeEntry = makeKnowledgeEntry("provisional");
    const result = transitionKnowledgeEntryStatus(knowledgeEntry, "confirmed");

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.status).toBe("confirmed");
      expect(knowledgeEntry.status).toBe("provisional");
    }
  });

  it("accepts no-op same-status transition", () => {
    const knowledgeEntry = makeKnowledgeEntry("merged");
    const result = transitionKnowledgeEntryStatus(knowledgeEntry, "merged");

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.status).toBe("merged");
    }
  });

  it("rejects invalid target status", () => {
    const result = transitionKnowledgeEntryStatus(makeKnowledgeEntry("provisional"), "bogus");

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_KNOWLEDGE_ENTRY_STATUS);
      expect(result.details).toEqual({ status: "bogus" });
    }
  });

  it("rejects invalid current status", () => {
    const result = transitionKnowledgeEntryStatus(makeKnowledgeEntry("bogus"), "confirmed");

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_KNOWLEDGE_ENTRY_STATUS);
      expect(result.details).toEqual({ status: "bogus" });
    }
  });

  it("rejects disallowed transition with from/to details", () => {
    const result = transitionKnowledgeEntryStatus(makeKnowledgeEntry("deprecated"), "merged");

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_KNOWLEDGE_ENTRY_STATUS_TRANSITION);
      expect(result.details).toEqual({ from: "deprecated", to: "merged" });
    }
  });

  it("rejects terminal outbound transition", () => {
    const result = transitionKnowledgeEntryStatus(makeKnowledgeEntry("deleted"), "provisional");

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_KNOWLEDGE_ENTRY_STATUS_TRANSITION);
      expect(result.details).toEqual({ from: "deleted", to: "provisional" });
    }
  });
});
