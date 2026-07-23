import type { KnowledgeEntry } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import { SpokeRejectCode } from "../result.js";
import {
  buildAssemblePacket,
  knowledgeEntryToAssembleEntry,
} from "./builder.js";

function makeKnowledgeEntry(overrides: Partial<KnowledgeEntry> = {}): KnowledgeEntry {
  return {
    schema_version: 1,
    knowledge_entry_id: "kb_1",
    entry_type: "character",
    canonical_name: "Mira Vale",
    status: "confirmed",
    body: {},
    extensions: {},
    ...overrides,
  };
}

describe("knowledgeEntryToAssembleEntry", () => {
  it("maps core fields", () => {
    const entry = knowledgeEntryToAssembleEntry(
      makeKnowledgeEntry({ body: { summary: "  Hero  " } }),
    );

    expect(entry).toEqual({
      knowledge_entry_id: "kb_1",
      entry_type: "character",
      canonical_name: "Mira Vale",
      snippet: "Hero",
    });
  });

  it("omits snippet for non-string body.summary", () => {
    const entry = knowledgeEntryToAssembleEntry(
      makeKnowledgeEntry({ body: { summary: 42 } }),
    );

    expect(entry).not.toHaveProperty("snippet");
  });

  it("omits snippet for whitespace-only summary", () => {
    const entry = knowledgeEntryToAssembleEntry(
      makeKnowledgeEntry({ body: { summary: "   " } }),
    );

    expect(entry).not.toHaveProperty("snippet");
  });
});

describe("buildAssemblePacket", () => {
  it("builds packet with empty knowledge entry list", () => {
    const result = buildAssemblePacket({ packetId: "pkt_1", knowledgeEntries: [] });

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value).toEqual({
        schema_version: 1,
        packet_id: "pkt_1",
        entries: [],
        extensions: {},
      });
    }
  });

  it("truncates entries in input order with maxEntries", () => {
    const knowledgeEntries = [
      makeKnowledgeEntry({ knowledge_entry_id: "kb_a", canonical_name: "A" }),
      makeKnowledgeEntry({ knowledge_entry_id: "kb_b", canonical_name: "B" }),
      makeKnowledgeEntry({ knowledge_entry_id: "kb_c", canonical_name: "C" }),
    ];

    const result = buildAssemblePacket({
      packetId: "pkt_2",
      knowledgeEntries,
      maxEntries: 2,
    });

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.entries.map((entry) => entry.knowledge_entry_id)).toEqual([
        "kb_a",
        "kb_b",
      ]);
    }
  });

  it("passes extensions through", () => {
    const extensions = { nexus: { profile: "chat" } };
    const result = buildAssemblePacket({
      packetId: "pkt_3",
      knowledgeEntries: [],
      extensions,
    });

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.extensions).toEqual(extensions);
    }
  });

  it("rejects empty packetId", () => {
    const result = buildAssemblePacket({ packetId: "  ", knowledgeEntries: [] });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_PACKET_INPUT);
    }
  });

  it("rejects negative maxEntries", () => {
    const result = buildAssemblePacket({
      packetId: "pkt_4",
      knowledgeEntries: [],
      maxEntries: -1,
    });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_PACKET_INPUT);
    }
  });

  it("rejects knowledge entry with null body (F-002)", () => {
    const result = buildAssemblePacket({
      packetId: "pkt_5",
      knowledgeEntries: [
        makeKnowledgeEntry({ body: null as unknown as KnowledgeEntry["body"] }),
      ],
    });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_PACKET_INPUT);
    }
  });

  it("rejects non-string canonical_name (F-004)", () => {
    const result = buildAssemblePacket({
      packetId: "pkt_6",
      knowledgeEntries: [
        makeKnowledgeEntry({ canonical_name: 99 as unknown as string }),
      ],
    });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_PACKET_INPUT);
    }
  });
});
