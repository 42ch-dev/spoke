import type { KnowledgeEntry } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import { SpokeRejectCode } from "../result.js";
import { assertUniqueActiveKnowledgeEntry } from "./uniqueness.js";

function makeKnowledgeEntry(
  overrides: Partial<KnowledgeEntry> & Pick<KnowledgeEntry, "entry_id">,
): KnowledgeEntry {
  return {
    schema_version: 1,
    entry_type: "character",
    canonical_name: "Mira Vale",
    status: "confirmed",
    body: { summary: "Protagonist" },
    extensions: {},
    ...overrides,
  };
}

const baseInput = {
  scope_key: "world_1",
  entry_type: "character",
  canonical_name: "Mira Vale",
};

describe("assertUniqueActiveKnowledgeEntry", () => {
  it("accepts when no conflicting active knowledge entry exists", () => {
    const candidate = makeKnowledgeEntry({
      entry_id: "kb_new",
      status: "provisional",
    });
    const result = assertUniqueActiveKnowledgeEntry({
      ...baseInput,
      candidate,
      existing: [
        makeKnowledgeEntry({
          entry_id: "kb_other",
          entry_type: "location",
          canonical_name: "Harbor",
        }),
      ],
    });

    expect(result.ok).toBe(true);
  });

  it("rejects duplicate active triple for a different entry_id", () => {
    const candidate = makeKnowledgeEntry({
      entry_id: "kb_new",
      status: "provisional",
    });
    const result = assertUniqueActiveKnowledgeEntry({
      ...baseInput,
      candidate,
      existing: [
        makeKnowledgeEntry({ entry_id: "kb_existing", status: "confirmed" }),
      ],
    });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.DUPLICATE_ACTIVE_KNOWLEDGE_ENTRY);
      expect(result.details).toEqual({
        scope_key: "world_1",
        entry_type: "character",
        canonical_name: "Mira Vale",
        conflicting_entry_id: "kb_existing",
      });
    }
  });

  it("allows same entry_id update in place", () => {
    const candidate = makeKnowledgeEntry({
      entry_id: "kb_1",
      status: "confirmed",
    });
    const result = assertUniqueActiveKnowledgeEntry({
      ...baseInput,
      candidate,
      existing: [makeKnowledgeEntry({ entry_id: "kb_1", status: "confirmed" })],
    });

    expect(result.ok).toBe(true);
  });

  it("ignores inactive existing knowledge entries", () => {
    const candidate = makeKnowledgeEntry({
      entry_id: "kb_new",
      status: "provisional",
    });
    const result = assertUniqueActiveKnowledgeEntry({
      ...baseInput,
      candidate,
      existing: [
        makeKnowledgeEntry({ entry_id: "kb_deprecated", status: "deprecated" }),
        makeKnowledgeEntry({ entry_id: "kb_merged", status: "merged" }),
        makeKnowledgeEntry({ entry_id: "kb_deleted", status: "deleted" }),
      ],
    });

    expect(result.ok).toBe(true);
  });

  it("passes when candidate is inactive", () => {
    const candidate = makeKnowledgeEntry({
      entry_id: "kb_new",
      status: "deprecated",
    });
    const result = assertUniqueActiveKnowledgeEntry({
      ...baseInput,
      candidate,
      existing: [
        makeKnowledgeEntry({ entry_id: "kb_existing", status: "confirmed" }),
      ],
    });

    expect(result.ok).toBe(true);
  });

  it("rejects when entry_type or canonical_name do not match candidate wire fields (R1)", () => {
    const candidate = makeKnowledgeEntry({
      entry_id: "kb_new",
      status: "provisional",
    });

    const blockTypeMismatch = assertUniqueActiveKnowledgeEntry({
      ...baseInput,
      entry_type: "location",
      candidate,
      existing: [],
    });
    expect(blockTypeMismatch.ok).toBe(false);
    if (!blockTypeMismatch.ok) {
      expect(blockTypeMismatch.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }

    const nameMismatch = assertUniqueActiveKnowledgeEntry({
      ...baseInput,
      canonical_name: "Other Name",
      candidate,
      existing: [],
    });
    expect(nameMismatch.ok).toBe(false);
    if (!nameMismatch.ok) {
      expect(nameMismatch.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });
});
