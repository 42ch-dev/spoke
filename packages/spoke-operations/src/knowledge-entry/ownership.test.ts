import type { KnowledgeEntry } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import {
  getKnowledgeEntryOwner,
  knowledgeEntryVisibleToViewpoint,
} from "./ownership.js";

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

describe("getKnowledgeEntryOwner", () => {
  it("returns the holder entry id when ownership is present", () => {
    const entry = makeKnowledgeEntry({ entry_id: "kb_secret", owner: "kb_mira" });

    expect(getKnowledgeEntryOwner(entry)).toBe("kb_mira");
  });

  it("returns undefined when ownership is unspecified", () => {
    const entry = makeKnowledgeEntry({ entry_id: "kb_shared" });

    expect(getKnowledgeEntryOwner(entry)).toBeUndefined();
  });
});

describe("knowledgeEntryVisibleToViewpoint", () => {
  it("includes an entry without disclosure for any viewpoint", () => {
    const entry = makeKnowledgeEntry({ entry_id: "kb_shared" });

    expect(knowledgeEntryVisibleToViewpoint(entry, "kb_mira")).toBe(true);
    expect(knowledgeEntryVisibleToViewpoint(entry, "kb_other")).toBe(true);
    expect(knowledgeEntryVisibleToViewpoint(entry, undefined)).toBe(true);
  });

  it("includes an entry that carries an owner but no disclosure", () => {
    const entry = makeKnowledgeEntry({
      entry_id: "kb_attributed",
      owner: "kb_mira",
    });

    expect(knowledgeEntryVisibleToViewpoint(entry, "kb_other")).toBe(true);
    expect(knowledgeEntryVisibleToViewpoint(entry)).toBe(true);
  });

  it("includes an owner-private entry for its own viewpoint", () => {
    const entry = makeKnowledgeEntry({
      entry_id: "kb_secret",
      owner: "kb_mira",
      disclosure: "owner-private",
    });

    expect(knowledgeEntryVisibleToViewpoint(entry, "kb_mira")).toBe(true);
  });

  it("excludes an owner-private entry for a foreign viewpoint", () => {
    const entry = makeKnowledgeEntry({
      entry_id: "kb_secret",
      owner: "kb_mira",
      disclosure: "owner-private",
    });

    expect(knowledgeEntryVisibleToViewpoint(entry, "kb_other")).toBe(false);
  });

  it("excludes an owner-private entry when no viewpoint is supplied", () => {
    const entry = makeKnowledgeEntry({
      entry_id: "kb_secret",
      owner: "kb_mira",
      disclosure: "owner-private",
    });

    expect(knowledgeEntryVisibleToViewpoint(entry, undefined)).toBe(false);
    expect(knowledgeEntryVisibleToViewpoint(entry)).toBe(false);
  });

  it("excludes a malformed owner-private entry with no owner", () => {
    const entry = makeKnowledgeEntry({
      entry_id: "kb_orphan",
      disclosure: "owner-private",
    });

    expect(knowledgeEntryVisibleToViewpoint(entry, "kb_mira")).toBe(false);
    expect(knowledgeEntryVisibleToViewpoint(entry, undefined)).toBe(false);
  });

  it("excludes a malformed owner-private entry with an empty owner", () => {
    const entry = makeKnowledgeEntry({
      entry_id: "kb_orphan",
      owner: "",
      disclosure: "owner-private",
    });

    expect(knowledgeEntryVisibleToViewpoint(entry, "")).toBe(false);
    expect(knowledgeEntryVisibleToViewpoint(entry, "kb_mira")).toBe(false);
  });

  it("excludes an unknown disclosure value even for a matching owner viewpoint", () => {
    const entry = makeKnowledgeEntry({
      entry_id: "kb_group",
      owner: "kb_mira",
      disclosure: "group-private",
    });

    expect(knowledgeEntryVisibleToViewpoint(entry, "kb_mira")).toBe(false);
    expect(knowledgeEntryVisibleToViewpoint(entry, undefined)).toBe(false);
  });

  it("compares identifiers exactly without normalizing case or whitespace", () => {
    const entry = makeKnowledgeEntry({
      entry_id: "kb_secret",
      owner: "kb_mira",
      disclosure: "owner-private",
    });

    expect(knowledgeEntryVisibleToViewpoint(entry, " kb_mira")).toBe(false);
    expect(knowledgeEntryVisibleToViewpoint(entry, "KB_MIRA")).toBe(false);
  });
});
