import type { Event, Keyblock, Scope } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import {
  eventMatchesScope,
  filterEventsByScope,
  filterKeyblocksByScope,
  keyblockMatchesScope,
} from "./match.js";

function makeKeyblock(
  overrides: Partial<Keyblock> & Pick<Keyblock, "keyblock_id">,
): Keyblock {
  return {
    schema_version: 1,
    block_type: "character",
    canonical_name: "Mira Vale",
    status: "confirmed",
    body: { summary: "Protagonist" },
    extensions: {},
    ...overrides,
  };
}

function makeEvent(overrides: Partial<Event> & Pick<Event, "event_id">): Event {
  return {
    schema_version: 1,
    canonical_name: "Battle of Harbor",
    extensions: {},
    ...overrides,
  };
}

const baseScope: Scope = { scope_id: "world_1" };

describe("keyblockMatchesScope", () => {
  const keyblock = makeKeyblock({
    keyblock_id: "kb_1",
    block_type: "character",
    source_anchor: {
      schema_version: 1,
      source_id: "manuscript_1",
      extensions: {},
    },
  });

  it("passes when only scope_id is set", () => {
    expect(keyblockMatchesScope(keyblock, baseScope)).toBe(true);
  });

  it("matches keyblock_ids refinement", () => {
    expect(
      keyblockMatchesScope(keyblock, { ...baseScope, keyblock_ids: ["kb_1", "kb_2"] }),
    ).toBe(true);
    expect(keyblockMatchesScope(keyblock, { ...baseScope, keyblock_ids: ["kb_2"] })).toBe(
      false,
    );
  });

  it("matches block_types refinement", () => {
    expect(keyblockMatchesScope(keyblock, { ...baseScope, block_types: ["character"] })).toBe(
      true,
    );
    expect(keyblockMatchesScope(keyblock, { ...baseScope, block_types: ["location"] })).toBe(
      false,
    );
  });

  it("matches source_id refinement", () => {
    expect(keyblockMatchesScope(keyblock, { ...baseScope, source_id: "manuscript_1" })).toBe(
      true,
    );
    expect(keyblockMatchesScope(keyblock, { ...baseScope, source_id: "other" })).toBe(false);
  });

  it("ignores event refinements on Keyblock", () => {
    expect(
      keyblockMatchesScope(keyblock, {
        ...baseScope,
        event_ids: ["evt_missing"],
        timeline_scale: "brief",
      }),
    ).toBe(true);
  });

  it("requires all present refinements (AND)", () => {
    expect(
      keyblockMatchesScope(keyblock, {
        ...baseScope,
        keyblock_ids: ["kb_1"],
        block_types: ["location"],
      }),
    ).toBe(false);
  });
});

describe("filterKeyblocksByScope", () => {
  it("filters by combined refinements", () => {
    const keyblocks = [
      makeKeyblock({ keyblock_id: "kb_1", block_type: "character" }),
      makeKeyblock({ keyblock_id: "kb_2", block_type: "location" }),
    ];

    const filtered = filterKeyblocksByScope(keyblocks, {
      ...baseScope,
      block_types: ["character"],
    });

    expect(filtered).toHaveLength(1);
    expect(filtered[0]?.keyblock_id).toBe("kb_1");
  });
});

describe("eventMatchesScope", () => {
  const event = makeEvent({
    event_id: "evt_1",
    timeline_scale: "narrative",
  });

  it("passes when only scope_id is set", () => {
    expect(eventMatchesScope(event, baseScope)).toBe(true);
  });

  it("matches event_ids refinement", () => {
    expect(eventMatchesScope(event, { ...baseScope, event_ids: ["evt_1"] })).toBe(true);
    expect(eventMatchesScope(event, { ...baseScope, event_ids: ["evt_2"] })).toBe(false);
  });

  it("matches timeline_scale refinement", () => {
    expect(eventMatchesScope(event, { ...baseScope, timeline_scale: "narrative" })).toBe(
      true,
    );
    expect(eventMatchesScope(event, { ...baseScope, timeline_scale: "brief" })).toBe(false);
  });

  it("ignores keyblock refinements on Event", () => {
    expect(
      eventMatchesScope(event, {
        ...baseScope,
        keyblock_ids: ["kb_missing"],
        block_types: ["character"],
        source_id: "manuscript_1",
      }),
    ).toBe(true);
  });
});

describe("filterEventsByScope", () => {
  it("filters by timeline_scale", () => {
    const events = [
      makeEvent({ event_id: "evt_1", timeline_scale: "brief" }),
      makeEvent({ event_id: "evt_2", timeline_scale: "narrative" }),
    ];

    const filtered = filterEventsByScope(events, {
      ...baseScope,
      timeline_scale: "narrative",
    });

    expect(filtered).toHaveLength(1);
    expect(filtered[0]?.event_id).toBe("evt_2");
  });
});
