import type { KnowledgeEntry, Scope, TimelineEvent } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import {
  filterKnowledgeEntriesByScope,
  filterTimelineEventsByScope,
  knowledgeEntryMatchesScope,
  timelineEventMatchesScope,
} from "./match.js";

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

function makeTimelineEvent(
  overrides: Partial<TimelineEvent> & Pick<TimelineEvent, "timeline_event_id">,
): TimelineEvent {
  return {
    schema_version: 1,
    canonical_name: "Battle of Harbor",
    extensions: {},
    ...overrides,
  };
}

const baseScope: Scope = { scope_id: "world_1" };

describe("knowledgeEntryMatchesScope", () => {
  const knowledgeEntry = makeKnowledgeEntry({
    entry_id: "kb_1",
    entry_type: "character",
    source_anchor: {
      schema_version: 1,
      source_id: "manuscript_1",
      extensions: {},
    },
  });

  it("passes when only scope_id is set", () => {
    expect(knowledgeEntryMatchesScope(knowledgeEntry, baseScope)).toBe(true);
  });

  it("matches entry_ids refinement", () => {
    expect(
      knowledgeEntryMatchesScope(knowledgeEntry, {
        ...baseScope,
        entry_ids: ["kb_1", "kb_2"],
      }),
    ).toBe(true);
    expect(
      knowledgeEntryMatchesScope(knowledgeEntry, {
        ...baseScope,
        entry_ids: ["kb_2"],
      }),
    ).toBe(false);
  });

  it("matches entry_types refinement", () => {
    expect(
      knowledgeEntryMatchesScope(knowledgeEntry, {
        ...baseScope,
        entry_types: ["character"],
      }),
    ).toBe(true);
    expect(
      knowledgeEntryMatchesScope(knowledgeEntry, {
        ...baseScope,
        entry_types: ["location"],
      }),
    ).toBe(false);
  });

  it("matches source_id refinement", () => {
    expect(
      knowledgeEntryMatchesScope(knowledgeEntry, {
        ...baseScope,
        source_id: "manuscript_1",
      }),
    ).toBe(true);
    expect(
      knowledgeEntryMatchesScope(knowledgeEntry, { ...baseScope, source_id: "other" }),
    ).toBe(false);
  });

  it("ignores timeline event refinements on KnowledgeEntry", () => {
    expect(
      knowledgeEntryMatchesScope(knowledgeEntry, {
        ...baseScope,
        timeline_event_ids: ["evt_missing"],
        timeline_scale: "brief",
      }),
    ).toBe(true);
  });

  it("requires all present refinements (AND)", () => {
    expect(
      knowledgeEntryMatchesScope(knowledgeEntry, {
        ...baseScope,
        entry_ids: ["kb_1"],
        entry_types: ["location"],
      }),
    ).toBe(false);
  });
});

describe("filterKnowledgeEntriesByScope", () => {
  it("filters by combined refinements", () => {
    const knowledgeEntries = [
      makeKnowledgeEntry({ entry_id: "kb_1", entry_type: "character" }),
      makeKnowledgeEntry({ entry_id: "kb_2", entry_type: "location" }),
    ];

    const filtered = filterKnowledgeEntriesByScope(knowledgeEntries, {
      ...baseScope,
      entry_types: ["character"],
    });

    expect(filtered).toHaveLength(1);
    expect(filtered[0]?.entry_id).toBe("kb_1");
  });
});

describe("timelineEventMatchesScope", () => {
  const timelineEvent = makeTimelineEvent({
    timeline_event_id: "evt_1",
    timeline_scale: "narrative",
  });

  it("passes when only scope_id is set", () => {
    expect(timelineEventMatchesScope(timelineEvent, baseScope)).toBe(true);
  });

  it("matches timeline_event_ids refinement", () => {
    expect(
      timelineEventMatchesScope(timelineEvent, {
        ...baseScope,
        timeline_event_ids: ["evt_1"],
      }),
    ).toBe(true);
    expect(
      timelineEventMatchesScope(timelineEvent, {
        ...baseScope,
        timeline_event_ids: ["evt_2"],
      }),
    ).toBe(false);
  });

  it("matches timeline_scale refinement", () => {
    expect(
      timelineEventMatchesScope(timelineEvent, {
        ...baseScope,
        timeline_scale: "narrative",
      }),
    ).toBe(true);
    expect(
      timelineEventMatchesScope(timelineEvent, { ...baseScope, timeline_scale: "brief" }),
    ).toBe(false);
  });

  it("ignores knowledge entry refinements on TimelineEvent", () => {
    expect(
      timelineEventMatchesScope(timelineEvent, {
        ...baseScope,
        entry_ids: ["kb_missing"],
        entry_types: ["character"],
        source_id: "manuscript_1",
      }),
    ).toBe(true);
  });
});

describe("filterTimelineEventsByScope", () => {
  it("filters by timeline_scale", () => {
    const timelineEvents = [
      makeTimelineEvent({ timeline_event_id: "evt_1", timeline_scale: "brief" }),
      makeTimelineEvent({ timeline_event_id: "evt_2", timeline_scale: "narrative" }),
    ];

    const filtered = filterTimelineEventsByScope(timelineEvents, {
      ...baseScope,
      timeline_scale: "narrative",
    });

    expect(filtered).toHaveLength(1);
    expect(filtered[0]?.timeline_event_id).toBe("evt_2");
  });
});
