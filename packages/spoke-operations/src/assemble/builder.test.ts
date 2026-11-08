import type { CognitionSelection, KnowledgeEntry } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import { SpokeRejectCode } from "../result.js";
import {
  buildAssemblePacket,
  knowledgeEntryToAssembleEntry,
} from "./builder.js";

function makeKnowledgeEntry(overrides: Partial<KnowledgeEntry> = {}): KnowledgeEntry {
  return {
    schema_version: 1,
    entry_id: "kb_1",
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
      entry_id: "kb_1",
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
      makeKnowledgeEntry({ entry_id: "kb_a", canonical_name: "A" }),
      makeKnowledgeEntry({ entry_id: "kb_b", canonical_name: "B" }),
      makeKnowledgeEntry({ entry_id: "kb_c", canonical_name: "C" }),
    ];

    const result = buildAssemblePacket({
      packetId: "pkt_2",
      knowledgeEntries,
      maxEntries: 2,
    });

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.entries.map((entry) => entry.entry_id)).toEqual([
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

// Cognition projection is a derivative of the already-authorized carrier set:
// the builder never invents a source view, and `max_entries` only truncates
// slim context entries.

const cognitionModule = {
  records: [
    {
      kind: "belief" as const,
      record_id: "rec_a",
      holder: { kind: "knowledge_entry" as const, namespace: "world_a", id: "kb_a" },
      target: {
        carrier: { kind: "knowledge_entry" as const, namespace: "world_a", id: "kb_box" },
        target_id: "t_marble",
        kind: "proposition" as const,
        text: "Mira hides the marble",
      },
      assessor: { kind: "principal" as const, namespace: "world_a", id: "assessor_self" },
      assessor_role: "holder_self_report" as const,
      basis: {
        kind: "evidence" as const,
        evidence: [{ kind: "knowledge_entry" as const, namespace: "world_a", id: "kb_box" }],
      },
      assessment_context: { context_id: "ctx_main", fork_id: "fork_alpha" },
      effective_at: "2026-07-23T09:00:00Z",
      assessed_at: "2026-07-23T09:05:00Z",
      view: "current" as const,
      order: 1,
      q: {
        value: 0.9,
        assertion: "belief_attribution" as const,
        method: "direct_report" as const,
        calibration: "uncalibrated" as const,
      },
      extensions: {},
    },
  ],
  world_facts: [],
  extensions: {},
};

const carrierEntry = (entryId: string, recordId: string): KnowledgeEntry =>
  makeKnowledgeEntry({
    entry_id: entryId,
    modules: {
      cognition: {
        ...cognitionModule,
        records: [
          {
            ...cognitionModule.records[0]!,
            record_id: recordId,
            holder: { kind: "knowledge_entry", namespace: "world_a", id: entryId },
          },
        ],
      },
    },
  });

const currentSelection: CognitionSelection = {
  view: "current",
  filter: {},
  selection_policy: { policy_id: "pol_first", policy_version: "v1" },
};

const firstInputPolicy = {
  select: ({ assessments }: { assessments: readonly { record_id: string }[] }) =>
    assessments[0]!.record_id,
};

describe("buildAssemblePacket cognition availability", () => {
  it("rejects the unavailable recorded-at view instead of an empty projection", () => {
    const result = buildAssemblePacket({
      packetId: "pkt_recorded",
      knowledgeEntries: [carrierEntry("kb_a", "rec_a")],
      cognitionSelection: {
        view: "recorded_at",
        filter: { holder: { kind: "knowledge_entry", namespace: "world_a", id: "kb_a" } },
      },
    });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toMatchObject({
        reason: "unsupported_cognition_requirement",
        field: "cognitionSelection.view",
      });
    }
  });

  it("projects the authorized source set before max_entries truncation", () => {
    const result = buildAssemblePacket({
      packetId: "pkt_limit",
      knowledgeEntries: [
        carrierEntry("kb_a", "rec_a"),
        carrierEntry("kb_b", "rec_b"),
        carrierEntry("kb_c", "rec_c"),
      ],
      maxEntries: 1,
      cognitionSelection: currentSelection,
      currentViewPolicy: firstInputPolicy,
    });

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.entries.map((entry) => entry.entry_id)).toEqual(["kb_a"]);
    expect(result.value.cognition?.items.map((item) => item.selection?.selected_record_id)).toEqual([
      "rec_a",
      "rec_b",
      "rec_c",
    ]);
  });
});
