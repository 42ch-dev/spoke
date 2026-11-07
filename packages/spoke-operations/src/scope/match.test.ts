import type { KnowledgeEntry, Scope, TimelineEvent } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import { SpokeRejectCode } from "../result.js";
import {
  assertSupportedCognitionSelection,
  cognitionRecordsOfAuthorizedEntries,
  filterKnowledgeEntriesByScope,
  filterKnowledgeEntriesByScopeCognition,
  filterTimelineEventsByScope,
  knowledgeEntryPassesScopeAuthorization,
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

/** Run the served list filter and unwrap; an unexpected rejection fails the test. */
function filteredEntries(
  knowledgeEntries: KnowledgeEntry[],
  scope: Scope,
): KnowledgeEntry[] {
  const result = filterKnowledgeEntriesByScope(knowledgeEntries, scope);
  expect(result.ok).toBe(true);
  if (!result.ok) {
    throw new Error(`unexpected scope rejection: ${result.code}`);
  }
  return result.value;
}

describe("knowledgeEntryPassesScopeAuthorization", () => {
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
    expect(knowledgeEntryPassesScopeAuthorization(knowledgeEntry, baseScope)).toBe(true);
  });

  it("matches entry_ids refinement", () => {
    expect(
      knowledgeEntryPassesScopeAuthorization(knowledgeEntry, {
        ...baseScope,
        entry_ids: ["kb_1", "kb_2"],
      }),
    ).toBe(true);
    expect(
      knowledgeEntryPassesScopeAuthorization(knowledgeEntry, {
        ...baseScope,
        entry_ids: ["kb_2"],
      }),
    ).toBe(false);
  });

  it("matches entry_types refinement", () => {
    expect(
      knowledgeEntryPassesScopeAuthorization(knowledgeEntry, {
        ...baseScope,
        entry_types: ["character"],
      }),
    ).toBe(true);
    expect(
      knowledgeEntryPassesScopeAuthorization(knowledgeEntry, {
        ...baseScope,
        entry_types: ["location"],
      }),
    ).toBe(false);
  });

  it("matches source_id refinement", () => {
    expect(
      knowledgeEntryPassesScopeAuthorization(knowledgeEntry, {
        ...baseScope,
        source_id: "manuscript_1",
      }),
    ).toBe(true);
    expect(
      knowledgeEntryPassesScopeAuthorization(knowledgeEntry, { ...baseScope, source_id: "other" }),
    ).toBe(false);
  });

  it("ignores timeline event refinements on KnowledgeEntry", () => {
    expect(
      knowledgeEntryPassesScopeAuthorization(knowledgeEntry, {
        ...baseScope,
        timeline_event_ids: ["evt_missing"],
        timeline_scale: "brief",
      }),
    ).toBe(true);
  });

  it("requires all present refinements (AND)", () => {
    expect(
      knowledgeEntryPassesScopeAuthorization(knowledgeEntry, {
        ...baseScope,
        entry_ids: ["kb_1"],
        entry_types: ["location"],
      }),
    ).toBe(false);
  });

  it("leaves an entry without disclosure visible to a viewpoint", () => {
    expect(
      knowledgeEntryPassesScopeAuthorization(knowledgeEntry, {
        ...baseScope,
        viewpoint: "kb_other",
      }),
    ).toBe(true);
  });

  it("matches an owner-private entry against its own scope viewpoint", () => {
    const privateEntry = makeKnowledgeEntry({
      entry_id: "kb_1",
      owner: "kb_mira",
      disclosure: "owner-private",
    });

    expect(
      knowledgeEntryPassesScopeAuthorization(privateEntry, {
        ...baseScope,
        viewpoint: "kb_mira",
      }),
    ).toBe(true);
  });

  it("excludes an owner-private entry from a scope without a viewpoint", () => {
    const privateEntry = makeKnowledgeEntry({
      entry_id: "kb_1",
      owner: "kb_mira",
      disclosure: "owner-private",
    });

    expect(knowledgeEntryPassesScopeAuthorization(privateEntry, baseScope)).toBe(false);
  });

  it("excludes an owner-private entry from a foreign scope viewpoint", () => {
    const privateEntry = makeKnowledgeEntry({
      entry_id: "kb_1",
      owner: "kb_mira",
      disclosure: "owner-private",
    });

    expect(
      knowledgeEntryPassesScopeAuthorization(privateEntry, {
        ...baseScope,
        viewpoint: "kb_other",
      }),
    ).toBe(false);
  });

  it("composes the disclosure predicate with entry refinements (AND)", () => {
    const privateEntry = makeKnowledgeEntry({
      entry_id: "kb_1",
      owner: "kb_mira",
      disclosure: "owner-private",
    });

    expect(
      knowledgeEntryPassesScopeAuthorization(privateEntry, {
        ...baseScope,
        entry_ids: ["kb_1"],
        viewpoint: "kb_mira",
      }),
    ).toBe(true);
    // entry_ids would admit the entry; the disclosure predicate still excludes it.
    expect(
      knowledgeEntryPassesScopeAuthorization(privateEntry, {
        ...baseScope,
        entry_ids: ["kb_1"],
        viewpoint: "kb_other",
      }),
    ).toBe(false);
    // viewpoint would admit the entry; the entry refinement still excludes it.
    expect(
      knowledgeEntryPassesScopeAuthorization(privateEntry, {
        ...baseScope,
        entry_ids: ["kb_missing"],
        viewpoint: "kb_mira",
      }),
    ).toBe(false);
  });

  it("excludes an unknown disclosure value under any scope viewpoint", () => {
    const groupEntry = makeKnowledgeEntry({
      entry_id: "kb_1",
      owner: "kb_mira",
      disclosure: "group-private",
    });

    expect(
      knowledgeEntryPassesScopeAuthorization(groupEntry, {
        ...baseScope,
        viewpoint: "kb_mira",
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

    const filtered = filteredEntries(knowledgeEntries, {
      ...baseScope,
      entry_types: ["character"],
    });

    expect(filtered).toHaveLength(1);
    expect(filtered[0]?.entry_id).toBe("kb_1");
  });

  it("keeps shared entries and the viewpoint's own private entries", () => {
    const knowledgeEntries = [
      makeKnowledgeEntry({ entry_id: "kb_shared" }),
      makeKnowledgeEntry({
        entry_id: "kb_mine",
        owner: "kb_mira",
        disclosure: "owner-private",
      }),
      makeKnowledgeEntry({
        entry_id: "kb_theirs",
        owner: "kb_rival",
        disclosure: "owner-private",
      }),
      makeKnowledgeEntry({
        entry_id: "kb_group",
        owner: "kb_mira",
        disclosure: "group-private",
      }),
    ];

    const filtered = filteredEntries(knowledgeEntries, {
      ...baseScope,
      viewpoint: "kb_mira",
    });

    expect(filtered.map((entry) => entry.entry_id)).toEqual([
      "kb_shared",
      "kb_mine",
    ]);
  });

  it("keeps only shared entries when the scope has no viewpoint", () => {
    const knowledgeEntries = [
      makeKnowledgeEntry({ entry_id: "kb_shared" }),
      makeKnowledgeEntry({
        entry_id: "kb_mine",
        owner: "kb_mira",
        disclosure: "owner-private",
      }),
    ];

    const filtered = filteredEntries(knowledgeEntries, baseScope);

    expect(filtered.map((entry) => entry.entry_id)).toEqual(["kb_shared"]);
  });

  it("applies a current cognition selection after authorization", () => {
    const result = filterKnowledgeEntriesByScope(
      [
        entryWithCognition("kb_mira", "rec_mira"),
        entryWithCognition("kb_theo", "rec_theo"),
        makeKnowledgeEntry({ entry_id: "kb_plain" }),
      ],
      { scope_id: "world_1", cognition: cognitionSelection },
    );

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.map((entry) => entry.entry_id)).toEqual(["kb_mira"]);
    }
  });

  it("rejects an unavailable cognition view instead of returning the baseline set", () => {
    const result = filterKnowledgeEntriesByScope(
      [entryWithCognition("kb_mira", "rec_mira")],
      {
        scope_id: "world_1",
        cognition: { view: "recorded_at", filter: cognitionSelection.filter },
      },
    );

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toMatchObject({
        reason: "unsupported_cognition_requirement",
        field: "scope.cognition.view",
      });
    }
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

  it("matches fork_id refinement when equal", () => {
    const forkedEvent = makeTimelineEvent({
      timeline_event_id: "evt_fork",
      fork_id: "fork_mainline_a",
    });

    expect(
      timelineEventMatchesScope(forkedEvent, {
        ...baseScope,
        fork_id: "fork_mainline_a",
      }),
    ).toBe(true);
  });

  it("misses fork_id refinement when unequal", () => {
    const forkedEvent = makeTimelineEvent({
      timeline_event_id: "evt_fork",
      fork_id: "fork_mainline_a",
    });

    expect(
      timelineEventMatchesScope(forkedEvent, {
        ...baseScope,
        fork_id: "fork_what_if_b",
      }),
    ).toBe(false);
  });

  it("misses fork_id refinement when event lacks fork_id", () => {
    expect(
      timelineEventMatchesScope(timelineEvent, {
        ...baseScope,
        fork_id: "fork_mainline_a",
      }),
    ).toBe(false);
  });

  it("does not filter on parent_fork_id", () => {
    const forkedEvent = makeTimelineEvent({
      timeline_event_id: "evt_fork",
      parent_fork_id: "fork_mainline_a",
    });

    expect(
      timelineEventMatchesScope(forkedEvent, {
        ...baseScope,
        fork_id: "fork_mainline_a",
      }),
    ).toBe(false);
  });

  it("passes when scope omits fork_id", () => {
    const forkedEvent = makeTimelineEvent({
      timeline_event_id: "evt_fork",
      fork_id: "fork_mainline_a",
    });

    expect(timelineEventMatchesScope(forkedEvent, baseScope)).toBe(true);
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

  it("filters by fork_id", () => {
    const timelineEvents = [
      makeTimelineEvent({ timeline_event_id: "evt_1", fork_id: "fork_mainline_a" }),
      makeTimelineEvent({ timeline_event_id: "evt_2", fork_id: "fork_what_if_b" }),
      makeTimelineEvent({ timeline_event_id: "evt_3" }),
    ];

    const filtered = filterTimelineEventsByScope(timelineEvents, {
      ...baseScope,
      fork_id: "fork_mainline_a",
    });

    expect(filtered).toHaveLength(1);
    expect(filtered[0]?.timeline_event_id).toBe("evt_1");
  });
});

describe("Scope.extensions is opaque to matchers", () => {
  // Product-scoped query metadata lives under extensions.<namespace>; protocol matchers
  // match core Scope fields only and must neither consume nor alter extensions.
  const extensions: NonNullable<Scope["extensions"]> = {
    toy: { branch_id: "fork_mainline_a" },
    nexus: { text_search: "harbor", limit: 10 },
  };

  it("matches on core fields identically whether or not extensions are present", () => {
    const matching = makeKnowledgeEntry({
      entry_id: "kb_1",
      entry_type: "character",
      source_anchor: {
        schema_version: 1,
        source_id: "manuscript_1",
        extensions: {},
      },
    });
    const nonMatching = makeKnowledgeEntry({
      entry_id: "kb_other",
      entry_type: "location",
    });

    const coreRefinements = {
      ...baseScope,
      entry_ids: ["kb_1"],
      entry_types: ["character"],
      source_id: "manuscript_1",
    } satisfies Scope;

    expect(knowledgeEntryPassesScopeAuthorization(matching, coreRefinements)).toBe(true);
    expect(knowledgeEntryPassesScopeAuthorization(nonMatching, coreRefinements)).toBe(false);

    // Same core refinements, now carrying product query metadata in extensions.
    const withExtensions: Scope = { ...coreRefinements, extensions };

    expect(knowledgeEntryPassesScopeAuthorization(matching, withExtensions)).toBe(
      knowledgeEntryPassesScopeAuthorization(matching, coreRefinements),
    );
    expect(knowledgeEntryPassesScopeAuthorization(nonMatching, withExtensions)).toBe(
      knowledgeEntryPassesScopeAuthorization(nonMatching, coreRefinements),
    );

    // Timeline matchers also ignore extensions.
    const event = makeTimelineEvent({
      timeline_event_id: "evt_1",
      timeline_scale: "narrative",
    });
    const timelineCore: Scope = { ...baseScope, timeline_scale: "narrative" };
    const timelineWithExtensions: Scope = { ...timelineCore, extensions };

    expect(timelineEventMatchesScope(event, timelineWithExtensions)).toBe(
      timelineEventMatchesScope(event, timelineCore),
    );
  });

  it("preserves extensions verbatim (not consumed or altered) across match + filter", () => {
    const knowledgeEntries = [
      makeKnowledgeEntry({ entry_id: "kb_1", entry_type: "character" }),
      makeKnowledgeEntry({ entry_id: "kb_2", entry_type: "location" }),
    ];
    const timelineEvents = [
      makeTimelineEvent({ timeline_event_id: "evt_1", timeline_scale: "narrative" }),
    ];

    const scope: Scope = {
      ...baseScope,
      entry_types: ["character"],
      timeline_scale: "narrative",
      extensions,
    };

    // Drive every matcher + filter surface.
    knowledgeEntryPassesScopeAuthorization(knowledgeEntries[0]!, scope);
    expect(filterKnowledgeEntriesByScope(knowledgeEntries, scope).ok).toBe(true);
    timelineEventMatchesScope(timelineEvents[0]!, scope);
    filterTimelineEventsByScope(timelineEvents, scope);

    // Referential identity + deep equality: the bag survived untouched.
    expect(scope.extensions).toBe(extensions);
    expect(scope.extensions).toEqual({
      toy: { branch_id: "fork_mainline_a" },
      nexus: { text_search: "harbor", limit: 10 },
    });
  });
});

const cognitionModule = {
  records: [
    {
      kind: "belief",
      record_id: "rec_scope_1",
      holder: { kind: "knowledge_entry", namespace: "world_a", id: "kb_mira" },
      target: {
        carrier: { kind: "knowledge_entry", namespace: "world_a", id: "kb_box" },
        target_id: "t_marble",
        kind: "proposition",
      },
      assessor: { kind: "principal", namespace: "world_a", id: "assessor_self" },
      assessor_role: "holder_self_report",
      basis: {
        kind: "evidence",
        evidence: [{ kind: "knowledge_entry", namespace: "world_a", id: "kb_box" }],
      },
      assessment_context: { context_id: "ctx_main", fork_id: "fork_alpha" },
      effective_at: "2026-07-23T09:00:00Z",
      assessed_at: "2026-07-23T09:05:00Z",
      view: "current",
      order: 1,
      q: {
        value: 0.9,
        assertion: "belief_attribution",
        method: "direct_report",
        calibration: "uncalibrated",
      },
      extensions: {},
    },
  ],
  world_facts: [],
  extensions: {},
};

describe("cognitionRecordsOfAuthorizedEntries", () => {
  it("extracts records only from entries the caller already authorized", () => {
    const authorized = filteredEntries(
      [
        makeKnowledgeEntry({
          entry_id: "kb_mira",
          owner: "kb_mira",
          disclosure: "owner-private",
          modules: { cognition: cognitionModule },
        }),
      ],
      { scope_id: "world_1", viewpoint: "kb_mira" },
    );

    const result = cognitionRecordsOfAuthorizedEntries(authorized);

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.map((record) => record.record_id)).toEqual([
        "rec_scope_1",
      ]);
    }
  });

  it("never exposes a private holder's cognition to a foreign viewpoint", () => {
    const authorized = filteredEntries(
      [
        makeKnowledgeEntry({
          entry_id: "kb_mira",
          owner: "kb_mira",
          disclosure: "owner-private",
          modules: { cognition: cognitionModule },
        }),
      ],
      { scope_id: "world_1", viewpoint: "kb_rival" },
    );

    const result = cognitionRecordsOfAuthorizedEntries(authorized);

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value).toEqual([]);
    }
  });

  it("ignores entries without a cognition carrier", () => {
    const result = cognitionRecordsOfAuthorizedEntries([
      makeKnowledgeEntry({ entry_id: "kb_plain" }),
    ]);

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value).toEqual([]);
    }
  });

  it("rejects a malformed carrier module instead of dropping records", () => {
    const result = cognitionRecordsOfAuthorizedEntries([
      makeKnowledgeEntry({
        entry_id: "kb_bad",
        modules: {
          cognition: { records: [], extensions: {} },
        },
      }),
    ]);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toMatchObject({
        reason: "invalid_cognition_record",
        entry_id: "kb_bad",
        field: "modules.cognition",
      });
    }
  });
});

const cognitionSelection = {
  view: "current" as const,
  filter: {
    holder: { kind: "knowledge_entry", namespace: "world_a", id: "kb_mira" },
  },
  selection_policy: { policy_id: "pol_scope", policy_version: "v1" },
};

const entryWithCognition = (
  entryId: string,
  recordId: string,
): KnowledgeEntry =>
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

describe("filterKnowledgeEntriesByScopeCognition", () => {
  it("keeps only entries whose carried record satisfies the predicate", () => {
    const result = filterKnowledgeEntriesByScopeCognition(
      [
        entryWithCognition("kb_mira", "rec_mira"),
        entryWithCognition("kb_theo", "rec_theo"),
        makeKnowledgeEntry({ entry_id: "kb_plain" }),
      ],
      { scope_id: "world_1", cognition: cognitionSelection },
      "scope.cognition",
    );

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.map((entry) => entry.entry_id)).toEqual(["kb_mira"]);
    }
  });

  it("returns the authorized set unchanged when Scope.cognition is absent", () => {
    const entries = [makeKnowledgeEntry({ entry_id: "kb_plain" })];
    const result = filterKnowledgeEntriesByScopeCognition(
      entries,
      { scope_id: "world_1" },
      "scope.cognition",
    );

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value).toEqual(entries);
    }
  });

  it("rejects the unavailable recorded-at view instead of matching nothing", () => {
    const result = filterKnowledgeEntriesByScopeCognition(
      [entryWithCognition("kb_mira", "rec_mira")],
      {
        scope_id: "world_1",
        cognition: { view: "recorded_at", filter: cognitionSelection.filter },
      },
      "scope.cognition",
    );

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      expect(result.details).toMatchObject({
        reason: "unsupported_cognition_requirement",
        field: "scope.cognition.view",
      });
    }
  });

  it("rejects a malformed authorized carrier", () => {
    const result = filterKnowledgeEntriesByScopeCognition(
      [
        makeKnowledgeEntry({
          entry_id: "kb_bad",
          modules: { cognition: { records: [], extensions: {} } },
        }),
      ],
      { scope_id: "world_1", cognition: cognitionSelection },
      "scope.cognition",
    );

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.details).toMatchObject({
        reason: "invalid_cognition_record",
        field: "modules.cognition",
      });
    }
  });
});

describe("assertSupportedCognitionSelection", () => {
  it("accepts an absent selection and a current view", () => {
    expect(
      assertSupportedCognitionSelection(undefined, "scope.cognition").ok,
    ).toBe(true);
    expect(
      assertSupportedCognitionSelection(cognitionSelection, "scope.cognition")
        .ok,
    ).toBe(true);
  });

  it("rejects an unknown predicate at the request field path", () => {
    const result = assertSupportedCognitionSelection(
      {
        view: "current",
        filter: { q_min: 0.5 },
        selection_policy: { policy_id: "p", policy_version: "v" },
      } as never,
      "scope.cognition",
    );

    expect(result.ok).toBe(false);
    if (!result.ok) {
      // C5 names the relative member; this carrier only prefixes the root.
      expect(result.details).toMatchObject({
        reason: "unsupported_cognition_requirement",
        field: "scope.cognition.filter.q_min",
      });
    }
  });

  it("joins an empty member without an extra dot", () => {
    const result = assertSupportedCognitionSelection(
      {
        view: "current",
        filter: { "": 1 },
        selection_policy: { policy_id: "p", policy_version: "v" },
      } as never,
      "scope.cognition",
    );

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.details).toMatchObject({
        reason: "unsupported_cognition_requirement",
        field: 'scope.cognition.filter[""]',
      });
    }
  });
});
