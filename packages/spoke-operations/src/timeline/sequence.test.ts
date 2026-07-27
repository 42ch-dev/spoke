import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import type { Relation, TimelineEvent } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import { SpokeRejectCode } from "../result.js";
import {
  filterTimelineEventsByMomentScale,
  orderTimelineEventsByIds,
  orderTimelineEventsByPrecedes,
} from "./sequence.js";

const fixtureRoot = join(
  dirname(fileURLToPath(import.meta.url)),
  "../../../../fixtures/toy-world",
);

function loadFixture<T>(filename: string): T {
  return JSON.parse(
    readFileSync(join(fixtureRoot, filename), "utf8"),
  ) as T;
}

function makeTimelineEvent(
  overrides: Partial<TimelineEvent> & Pick<TimelineEvent, "timeline_event_id">,
): TimelineEvent {
  return {
    schema_version: 1,
    canonical_name: "Event",
    extensions: {},
    ...overrides,
  };
}

function makeRelation(
  overrides: Partial<Relation> & Pick<Relation, "relation_id" | "from_id" | "to_id">,
): Relation {
  return {
    schema_version: 1,
    relation_type: "precedes",
    extensions: {},
    ...overrides,
  };
}

describe("filterTimelineEventsByMomentScale", () => {
  it("keeps only moment-scale events in input order", () => {
    const events = [
      makeTimelineEvent({
        timeline_event_id: "evt_1",
        timeline_scale: "moment",
      }),
      makeTimelineEvent({
        timeline_event_id: "evt_2",
        timeline_scale: "narrative",
      }),
      makeTimelineEvent({
        timeline_event_id: "evt_3",
        timeline_scale: "moment",
      }),
      makeTimelineEvent({ timeline_event_id: "evt_4" }),
    ];

    expect(filterTimelineEventsByMomentScale(events)).toEqual([
      events[0],
      events[2],
    ]);
  });

  it("returns an empty array for empty input", () => {
    expect(filterTimelineEventsByMomentScale([])).toEqual([]);
  });
});

describe("orderTimelineEventsByIds", () => {
  const events = [
    makeTimelineEvent({ timeline_event_id: "evt_a" }),
    makeTimelineEvent({ timeline_event_id: "evt_b" }),
    makeTimelineEvent({ timeline_event_id: "evt_c" }),
  ];

  it("orders by explicit id list and appends stable tail", () => {
    const result = orderTimelineEventsByIds(events, ["evt_c", "evt_a"]);
    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.map((event) => event.timeline_event_id)).toEqual([
      "evt_c",
      "evt_a",
      "evt_b",
    ]);
  });

  it("rejects unknown timeline_event_id values", () => {
    const result = orderTimelineEventsByIds(events, ["evt_a", "evt_missing"]);
    expect(result).toEqual({
      ok: false,
      code: SpokeRejectCode.INVALID_INPUT,
      message:
        "orderedIds contains timeline_event_id values not present in timelineEvents",
      details: { unknown_timeline_event_ids: ["evt_missing"] },
    });
  });

  it("rejects duplicate ids in orderedIds", () => {
    const result = orderTimelineEventsByIds(events, ["evt_a", "evt_a"]);
    expect(result.ok).toBe(false);
    if (result.ok) {
      return;
    }
    expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
  });

  it("rejects duplicate timeline_event_id values in timelineEvents", () => {
    const duplicateEvents = [
      makeTimelineEvent({ timeline_event_id: "evt_a" }),
      makeTimelineEvent({ timeline_event_id: "evt_a" }),
      makeTimelineEvent({ timeline_event_id: "evt_b" }),
    ];

    const result = orderTimelineEventsByIds(duplicateEvents, ["evt_a"]);
    expect(result).toEqual({
      ok: false,
      code: SpokeRejectCode.INVALID_INPUT,
      message: "timelineEvents contains duplicate timeline_event_id values",
      details: { duplicate_timeline_event_ids: ["evt_a"] },
    });
  });
});

describe("orderTimelineEventsByPrecedes", () => {
  it("orders Harbor moment beats via precedes relations on dual KE ids", () => {
    const events = [
      loadFixture<TimelineEvent>("evt_tw_harbor_berth_confirm.json"),
      loadFixture<TimelineEvent>("evt_tw_harbor_dawn.json"),
      loadFixture<TimelineEvent>("evt_tw_harbor_customs_gate.json"),
      loadFixture<TimelineEvent>("evt_tw_harbor_market_square.json"),
    ];
    const relations = [
      loadFixture<Relation>("rel_tw_harbor_precedes_dawn_to_market.json"),
      loadFixture<Relation>("rel_tw_harbor_precedes_market_to_customs.json"),
      loadFixture<Relation>("rel_tw_harbor_precedes_customs_to_berth.json"),
    ];

    const result = orderTimelineEventsByPrecedes(events, relations);
    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }

    expect(result.value.map((event) => event.timeline_event_id)).toEqual([
      "evt_tw_harbor_dawn",
      "evt_tw_harbor_market_square",
      "evt_tw_harbor_customs_gate",
      "evt_tw_harbor_berth_confirm",
    ]);
  });

  it("appends unlinked events after the linked ordered block in input order", () => {
    const linked = makeTimelineEvent({
      timeline_event_id: "evt_linked_b",
      extensions: { spoke: { timeline_entry_id: "kb_b" } },
    });
    const linkedEarlier = makeTimelineEvent({
      timeline_event_id: "evt_linked_a",
      extensions: { spoke: { timeline_entry_id: "kb_a" } },
    });
    const unlinkedFirst = makeTimelineEvent({ timeline_event_id: "evt_unlinked_1" });
    const unlinkedSecond = makeTimelineEvent({ timeline_event_id: "evt_unlinked_2" });

    const events = [unlinkedFirst, linkedEarlier, linked, unlinkedSecond];
    const relations = [
      makeRelation({
        relation_id: "rel_a_b",
        from_id: "kb_a",
        to_id: "kb_b",
      }),
    ];

    const result = orderTimelineEventsByPrecedes(events, relations);
    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }

    expect(result.value.map((event) => event.timeline_event_id)).toEqual([
      "evt_linked_a",
      "evt_linked_b",
      "evt_unlinked_1",
      "evt_unlinked_2",
    ]);
  });

  it("breaks ready-queue ties by ascending timeline_event_id", () => {
    const events = [
      makeTimelineEvent({
        timeline_event_id: "evt_z",
        extensions: { spoke: { timeline_entry_id: "kb_z" } },
      }),
      makeTimelineEvent({
        timeline_event_id: "evt_a",
        extensions: { spoke: { timeline_entry_id: "kb_a" } },
      }),
      makeTimelineEvent({
        timeline_event_id: "evt_m",
        extensions: { spoke: { timeline_entry_id: "kb_m" } },
      }),
    ];

    const result = orderTimelineEventsByPrecedes(events, []);
    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }

    expect(result.value.map((event) => event.timeline_event_id)).toEqual([
      "evt_a",
      "evt_m",
      "evt_z",
    ]);
  });

  it("ignores relations whose endpoints are outside the input link map", () => {
    const events = [
      makeTimelineEvent({
        timeline_event_id: "evt_a",
        extensions: { spoke: { timeline_entry_id: "kb_a" } },
      }),
      makeTimelineEvent({
        timeline_event_id: "evt_b",
        extensions: { spoke: { timeline_entry_id: "kb_b" } },
      }),
    ];
    const relations = [
      makeRelation({
        relation_id: "rel_external",
        from_id: "kb_a",
        to_id: "kb_outside",
      }),
    ];

    const result = orderTimelineEventsByPrecedes(events, relations);
    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }

    expect(result.value.map((event) => event.timeline_event_id)).toEqual([
      "evt_a",
      "evt_b",
    ]);
  });

  it("rejects cycles among linked events", () => {
    const events = [
      makeTimelineEvent({
        timeline_event_id: "evt_a",
        extensions: { spoke: { timeline_entry_id: "kb_a" } },
      }),
      makeTimelineEvent({
        timeline_event_id: "evt_b",
        extensions: { spoke: { timeline_entry_id: "kb_b" } },
      }),
      makeTimelineEvent({
        timeline_event_id: "evt_c",
        extensions: { spoke: { timeline_entry_id: "kb_c" } },
      }),
    ];
    const relations = [
      makeRelation({
        relation_id: "rel_a_b",
        from_id: "kb_a",
        to_id: "kb_b",
      }),
      makeRelation({
        relation_id: "rel_b_c",
        from_id: "kb_b",
        to_id: "kb_c",
      }),
      makeRelation({
        relation_id: "rel_c_a",
        from_id: "kb_c",
        to_id: "kb_a",
      }),
    ];

    const result = orderTimelineEventsByPrecedes(events, relations);
    expect(result.ok).toBe(false);
    if (result.ok) {
      return;
    }

    expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
    expect(result.details).toEqual({
      precedes_cycle: true,
      entry_ids: ["kb_a", "kb_b", "kb_c"],
    });
  });

  it("keeps all linked events when timeline_entry_id is shared", () => {
    const events = [
      makeTimelineEvent({
        timeline_event_id: "evt_b",
        extensions: { spoke: { timeline_entry_id: "kb_shared" } },
      }),
      makeTimelineEvent({
        timeline_event_id: "evt_a",
        extensions: { spoke: { timeline_entry_id: "kb_shared" } },
      }),
    ];

    const result = orderTimelineEventsByPrecedes(events, []);
    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value.map((event) => event.timeline_event_id)).toEqual([
      "evt_a",
      "evt_b",
    ]);
  });

  it("rejects self-loop precedes relations", () => {
    const events = [
      makeTimelineEvent({
        timeline_event_id: "evt_a",
        extensions: { spoke: { timeline_entry_id: "kb_a" } },
      }),
    ];
    const relations = [
      makeRelation({
        relation_id: "rel_self",
        from_id: "kb_a",
        to_id: "kb_a",
      }),
    ];

    const result = orderTimelineEventsByPrecedes(events, relations);
    expect(result).toEqual({
      ok: false,
      code: SpokeRejectCode.INVALID_INPUT,
      message:
        "precedes relation resolves both endpoints to the same timeline event",
      details: { precedes_cycle: true, entry_ids: ["kb_a"] },
    });
  });

  it("rejects duplicate timeline_event_id values in timelineEvents", () => {
    const events = [
      makeTimelineEvent({
        timeline_event_id: "evt_a",
        extensions: { spoke: { timeline_entry_id: "kb_a" } },
      }),
      makeTimelineEvent({
        timeline_event_id: "evt_a",
        extensions: { spoke: { timeline_entry_id: "kb_b" } },
      }),
    ];

    const result = orderTimelineEventsByPrecedes(events, []);
    expect(result).toEqual({
      ok: false,
      code: SpokeRejectCode.INVALID_INPUT,
      message: "timelineEvents contains duplicate timeline_event_id values",
      details: { duplicate_timeline_event_ids: ["evt_a"] },
    });
  });

  it("honors options.relationType when filtering relations", () => {
    const events = [
      makeTimelineEvent({
        timeline_event_id: "evt_a",
        extensions: { spoke: { timeline_entry_id: "kb_a" } },
      }),
      makeTimelineEvent({
        timeline_event_id: "evt_b",
        extensions: { spoke: { timeline_entry_id: "kb_b" } },
      }),
    ];
    const relations = [
      makeRelation({
        relation_id: "rel_custom",
        relation_type: "follows",
        from_id: "kb_b",
        to_id: "kb_a",
      }),
    ];

    const defaultResult = orderTimelineEventsByPrecedes(events, relations);
    expect(defaultResult.ok).toBe(true);
    if (!defaultResult.ok) {
      return;
    }
    expect(defaultResult.value.map((event) => event.timeline_event_id)).toEqual([
      "evt_a",
      "evt_b",
    ]);

    const customResult = orderTimelineEventsByPrecedes(events, relations, {
      relationType: "follows",
    });
    expect(customResult.ok).toBe(true);
    if (!customResult.ok) {
      return;
    }
    expect(customResult.value.map((event) => event.timeline_event_id)).toEqual([
      "evt_b",
      "evt_a",
    ]);
  });
});
