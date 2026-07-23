import type { KnowledgeEntry } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import { SpokeRejectCode } from "../result.js";
import { validateUpsertKnowledgeEntry } from "./validate.js";

function makeKnowledgeEntry(
  overrides: Partial<KnowledgeEntry> & Pick<KnowledgeEntry, "entry_id">,
): KnowledgeEntry {
  return {
    schema_version: 1,
    entry_type: "character",
    canonical_name: "Mira Vale",
    status: "provisional",
    body: { summary: "Protagonist" },
    extensions: {},
    ...overrides,
  };
}

describe("validateUpsertKnowledgeEntry", () => {
  it("accepts valid create without revision", () => {
    const candidate = makeKnowledgeEntry({ entry_id: "kb_new" });
    expect(validateUpsertKnowledgeEntry(candidate).ok).toBe(true);
  });

  it("accepts valid create with revision 0", () => {
    const candidate = makeKnowledgeEntry({ entry_id: "kb_new", revision: 0 });
    expect(validateUpsertKnowledgeEntry(candidate).ok).toBe(true);
  });

  it("rejects create with revision >= 1", () => {
    const candidate = makeKnowledgeEntry({ entry_id: "kb_new", revision: 1 });
    const result = validateUpsertKnowledgeEntry(candidate);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });

  it("rejects create with empty canonical_name", () => {
    const candidate = makeKnowledgeEntry({ entry_id: "kb_new", canonical_name: "" });
    const result = validateUpsertKnowledgeEntry(candidate);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.EMPTY_CANONICAL_NAME);
    }
  });

  it("rejects create with whitespace-only canonical_name", () => {
    const candidate = makeKnowledgeEntry({ entry_id: "kb_new", canonical_name: "   " });
    const result = validateUpsertKnowledgeEntry(candidate);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.EMPTY_CANONICAL_NAME);
    }
  });

  it("accepts valid update with matching revision", () => {
    const stored = makeKnowledgeEntry({
      entry_id: "kb_1",
      revision: 2,
      status: "confirmed",
    });
    const candidate = makeKnowledgeEntry({
      entry_id: "kb_1",
      revision: 2,
      status: "confirmed",
    });

    expect(validateUpsertKnowledgeEntry(candidate, { stored }).ok).toBe(true);
  });

  it("rejects update without revision", () => {
    const stored = makeKnowledgeEntry({ entry_id: "kb_1", revision: 1 });
    const candidate = makeKnowledgeEntry({ entry_id: "kb_1" });

    const result = validateUpsertKnowledgeEntry(candidate, { stored });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.MISSING_REQUIRED_FIELD);
    }
  });

  it("rejects update when stored revision is stale", () => {
    const stored = makeKnowledgeEntry({ entry_id: "kb_1", revision: 3 });
    const candidate = makeKnowledgeEntry({ entry_id: "kb_1", revision: 2 });

    const result = validateUpsertKnowledgeEntry(candidate, { stored });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.STORED_REVISION_STALE);
    }
  });

  it("rejects update when stored has terminal status", () => {
    const stored = makeKnowledgeEntry({
      entry_id: "kb_1",
      revision: 1,
      status: "merged",
    });
    const candidate = makeKnowledgeEntry({
      entry_id: "kb_1",
      revision: 1,
      status: "merged",
    });

    const result = validateUpsertKnowledgeEntry(candidate, { stored });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.KNOWLEDGE_ENTRY_TERMINAL_STATUS);
    }
  });

  it("rejects update path without stored via explicit mode", () => {
    const candidate = makeKnowledgeEntry({ entry_id: "kb_1", revision: 1 });
    const result = validateUpsertKnowledgeEntry(candidate, { mode: "update" });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.KNOWLEDGE_ENTRY_NOT_FOUND);
    }
  });

  it("rejects create path when stored is provided via explicit mode", () => {
    const stored = makeKnowledgeEntry({ entry_id: "kb_1", revision: 0 });
    const candidate = makeKnowledgeEntry({ entry_id: "kb_1" });
    const result = validateUpsertKnowledgeEntry(candidate, { stored, mode: "create" });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.KNOWLEDGE_ENTRY_ALREADY_EXISTS);
    }
  });

  it("rejects entry_id mismatch on update", () => {
    const stored = makeKnowledgeEntry({ entry_id: "kb_1", revision: 1 });
    const candidate = makeKnowledgeEntry({ entry_id: "kb_2", revision: 1 });

    const result = validateUpsertKnowledgeEntry(candidate, { stored });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });

  it("rejects update when candidate is missing required fields", () => {
    const stored = makeKnowledgeEntry({ entry_id: "kb_1", revision: 1 });
    const candidate = {
      entry_id: "kb_1",
      revision: 1,
    } as KnowledgeEntry;

    const result = validateUpsertKnowledgeEntry(candidate, { stored });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.MISSING_REQUIRED_FIELD);
    }
  });
});
