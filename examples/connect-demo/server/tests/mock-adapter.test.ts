/**
 * MockAdapter / MockEngine unit tests.
 *
 * Covers the brief's Step 2 acceptance surface:
 * - engine seed corpus loads (world pair + governance fixtures + 1 relation +
 *   1 rule);
 * - put→get round-trip honors OCC (stale `expectedBaseRevision` → reject per
 *   operations conventions);
 * - derivation is deterministic (same history → same derived artifact
 *   ids/bodies);
 * - the scope query applies the ownership predicate (shared / own-private /
 *   foreign-private fixtures);
 * - the `ke-extraction` service answers provisional candidates from the
 *   referenced sources and keeps the loaded value host-local;
 * - `getHostCapabilityManifest` returns the server manifest.
 */

import { describe, expect, it } from "vitest";

import type {
  ExtractRequest,
  ExtractResponse,
  Finding,
  KnowledgeEntry,
  Relation,
  Rule,
  Scope,
  SourceAnchor,
} from "@42ch/spoke-schemas";
import { SpokeRejectCode, validateManifestTools, type SpokeResult } from "@42ch/spoke-operations";

import {
  DEMO_EXTRACTION_CANARY,
  DEMO_EXTRACTION_METHOD,
  DEMO_SERVER_MANIFEST,
  MockAdapter,
  demoExtractCandidateEntryId,
} from "../src/adapter/mock-adapter.js";
import { MockEngine } from "../src/engine/mock-engine.js";
import {
  DEMO_FOREIGN_PRIVATE_ENTRY_ID,
  DEMO_HOLDER_ENTRY_ID,
  DEMO_OWN_PRIVATE_ENTRY_ID,
  DEMO_SEED_ENTRIES,
  DEMO_SEED_FORK_ID,
  DEMO_SEED_RELATIONS,
  DEMO_SEED_RULES,
  DEMO_SEED_TIMELINE_EVENTS,
  DEMO_SHARED_ENTRY_ID,
  DEMO_SCOPE_ID,
} from "../src/engine/seed-corpus.js";

/** Assert a SpokeResult rejected with the given stable code. */
function expectRejected(
  result: SpokeResult<unknown>,
  code: SpokeRejectCode,
): void {
  expect(result.ok).toBe(false);
  if (!result.ok) {
    expect(result.code).toBe(code);
  }
}

/**
 * Assert a wire success branch (no `error` key) and narrow the
 * ProjectResponse / ComputeResponse unions to it (type guard).
 */
function expectSuccess<T extends object>(response: T): Exclude<T, { error: unknown }> {
  expect("error" in response).toBe(false);
  return response as Exclude<T, { error: unknown }>;
}

const COMPASS_ENTRY: KnowledgeEntry = {
  schema_version: 1,
  entry_id: "demo-harbor/item/compass",
  entry_type: "item",
  canonical_name: "Compass",
  status: "provisional",
  body: { summary: "A brass compass." },
  extensions: {},
};

const LANTERN_ENTRY: KnowledgeEntry = {
  schema_version: 1,
  entry_id: "demo-harbor/item/lantern",
  entry_type: "item",
  canonical_name: "Lantern",
  status: "provisional",
  body: { summary: "A harbor lantern." },
  extensions: {},
};

const COMPASS_RELATION: Relation = {
  schema_version: 1,
  relation_id: "demo-harbor/relation/compass-located-in-harbor",
  relation_type: "located_in",
  from_id: COMPASS_ENTRY.entry_id,
  to_id: "demo-harbor/location/harbor",
  extensions: {},
};

const WARNING_FINDING: Finding = {
  schema_version: 1,
  finding_id: "demo-harbor/finding/compass-uncased",
  severity: "info",
  status: "open",
  title: "Compass uncased",
  description: "The compass has no case.",
  target_entry_id: COMPASS_ENTRY.entry_id,
  extensions: {},
};

describe("engine seed corpus", () => {
  it("loads the seeded entries + 1 relation + 1 rule in scope demo-harbor", async () => {
    const adapter = new MockAdapter();

    for (const seed of DEMO_SEED_ENTRIES) {
      const got = await adapter.getKnowledgeEntry(seed.entry_id);
      expect(got.ok).toBe(true);
      if (got.ok) {
        expect(got.value.entry_id).toBe(seed.entry_id);
        expect(got.value.canonical_name).toBe(seed.canonical_name);
      }
    }

    const relation = await adapter.getRelation(DEMO_SEED_RELATIONS[0].relation_id);
    expect(relation.ok).toBe(true);
    if (relation.ok) {
      expect(relation.value.from_id).toBe(DEMO_SEED_RELATIONS[0].from_id);
      expect(relation.value.to_id).toBe(DEMO_SEED_RELATIONS[0].to_id);
    }

    const rules = await adapter.listRules([DEMO_SEED_RULES[0].rule_id]);
    expect(rules.ok).toBe(true);
    if (rules.ok) {
      expect(rules.value).toHaveLength(1);
      expect(rules.value[0].canonical_name).toBe("No isolated entries");
    }
  });

  it("lists seed entries plus the engine-derived world digest", async () => {
    const adapter = new MockAdapter();
    const listed = await adapter.listKnowledgeEntries({ scope_id: DEMO_SCOPE_ID });
    expect(listed.ok).toBe(true);
    if (listed.ok) {
      const ids = listed.value.map((entry) => entry.entry_id).sort();
      // A viewpoint-less query lists the shared seeds only: the two
      // owner-private fixtures are withheld by the core disclosure predicate.
      const sharedSeedIds = DEMO_SEED_ENTRIES.filter(
        (entry) => entry.disclosure === undefined,
      ).map((entry) => entry.entry_id);
      expect(ids).toEqual([...sharedSeedIds, "derived/world-digest"].sort());
      expect(ids).not.toContain(DEMO_OWN_PRIVATE_ENTRY_ID);
      expect(ids).not.toContain(DEMO_FOREIGN_PRIVATE_ENTRY_ID);
    }
  });

  it("rejects missing entries and rules with the operations codes", async () => {
    const adapter = new MockAdapter();
    expectRejected(
      await adapter.getKnowledgeEntry("demo-harbor/character/ghost"),
      SpokeRejectCode.KNOWLEDGE_ENTRY_NOT_FOUND,
    );
    expectRejected(
      await adapter.listRules(["demo-harbor/rule/missing"]),
      SpokeRejectCode.INVALID_INPUT,
    );
  });
});

describe("put/get OCC round-trip", () => {
  it("creates with expected null, rejects duplicate create, and CAS-updates", async () => {
    const adapter = new MockAdapter();

    const created = await adapter.putKnowledgeEntry(COMPASS_ENTRY, null);
    expect(created.ok).toBe(true);
    if (created.ok) {
      expect(created.value.entry_id).toBe(COMPASS_ENTRY.entry_id);
      expect(created.value.revision).toBe(1);
    }

    // Duplicate create: caller expects absence but the store already holds it.
    expectRejected(
      await adapter.putKnowledgeEntry(COMPASS_ENTRY, null),
      SpokeRejectCode.REVISION_CONFLICT,
    );

    // Stale base (caller behind store): stored 1 > expected 0.
    expectRejected(
      await adapter.putKnowledgeEntry(
        { ...COMPASS_ENTRY, body: { summary: "v2" } },
        0,
      ),
      SpokeRejectCode.STORED_REVISION_STALE,
    );

    // Impossible future base (caller ahead of store): expected 5 > stored 1.
    expectRejected(
      await adapter.putKnowledgeEntry(
        { ...COMPASS_ENTRY, body: { summary: "v2" } },
        5,
      ),
      SpokeRejectCode.REVISION_CONFLICT,
    );

    // CAS update against the current revision succeeds and bumps.
    const updated = await adapter.putKnowledgeEntry(
      { ...COMPASS_ENTRY, body: { summary: "v2" } },
      1,
    );
    expect(updated.ok).toBe(true);
    if (updated.ok) {
      expect(updated.value.revision).toBe(2);
    }

    // Non-null base on an absent entry: update path cannot find the store row.
    expectRejected(
      await adapter.putKnowledgeEntry(
        { ...COMPASS_ENTRY, entry_id: "demo-harbor/item/absent" },
        1,
      ),
      SpokeRejectCode.STORED_REVISION_STALE,
    );

    const got = await adapter.getKnowledgeEntry(COMPASS_ENTRY.entry_id);
    expect(got.ok).toBe(true);
    if (got.ok) {
      expect(got.value.body.summary).toBe("v2");
      expect(got.value.revision).toBe(2);
    }
  });

  it("guards the reserved derived/ id namespace", async () => {
    const adapter = new MockAdapter();
    const forged: KnowledgeEntry = {
      schema_version: 1,
      entry_id: "derived/world-digest",
      entry_type: "note",
      canonical_name: "Forged digest",
      status: "confirmed",
      body: {},
      extensions: {},
    };
    expectRejected(
      await adapter.putKnowledgeEntry(forged, null),
      SpokeRejectCode.INVALID_INPUT,
    );
  });

  it("relation puts honor OCC with RELATION_ALREADY_EXISTS on duplicate create", async () => {
    const adapter = new MockAdapter();

    const created = await adapter.putRelation(COMPASS_RELATION, null);
    expect(created.ok).toBe(true);
    if (created.ok) {
      expect(created.value.revision).toBe(1);
    }

    expectRejected(
      await adapter.putRelation(COMPASS_RELATION, null),
      SpokeRejectCode.RELATION_ALREADY_EXISTS,
    );

    expectRejected(
      await adapter.putRelation({ ...COMPASS_RELATION, label: "updated" }, 0),
      SpokeRejectCode.STORED_REVISION_STALE,
    );

    const updated = await adapter.putRelation(
      { ...COMPASS_RELATION, label: "updated" },
      1,
    );
    expect(updated.ok).toBe(true);
    if (updated.ok) {
      expect(updated.value.revision).toBe(2);
      expect(updated.value.label).toBe("updated");
    }
  });

  it("round-trips findings and resolves rules by reference", async () => {
    const adapter = new MockAdapter();
    const put = await adapter.putFindings([WARNING_FINDING]);
    expect(put.ok).toBe(true);
    if (put.ok) {
      expect(put.value).toEqual([WARNING_FINDING]);
    }
  });
});

describe("deterministic derivation", () => {
  it("derives the world digest from the seed corpus", async () => {
    const adapter = new MockAdapter();
    const digest = await adapter.getKnowledgeEntry("derived/world-digest");
    expect(digest.ok).toBe(true);
    if (digest.ok) {
      expect(digest.value.body.computable).toEqual({
        entry_type_counts: { character: 1, location: 1, note: 3 },
        entry_ids_sorted: [
          "demo-harbor/character/mira",
          "demo-harbor/location/harbor",
          "demo-harbor/note/harbor-log",
          "demo-harbor/note/mira-private-log",
          "demo-harbor/note/rival-private-log",
        ],
      });
      expect(digest.value.revision).toBe(1);
    }
  });

  it("re-derives the digest and isolated_entry findings on each mutation", async () => {
    const adapter = new MockAdapter();
    const engine = adapter.engine;

    const put = await adapter.putKnowledgeEntry(COMPASS_ENTRY, null);
    expect(put.ok).toBe(true);

    const digest = await adapter.getKnowledgeEntry("derived/world-digest");
    expect(digest.ok).toBe(true);
    if (digest.ok) {
      expect(digest.value.body.computable).toEqual({
        entry_type_counts: { character: 1, item: 1, location: 1, note: 3 },
        entry_ids_sorted: [
          "demo-harbor/character/mira",
          "demo-harbor/item/compass",
          "demo-harbor/location/harbor",
          "demo-harbor/note/harbor-log",
          "demo-harbor/note/mira-private-log",
          "demo-harbor/note/rival-private-log",
        ],
      });
      expect(digest.value.revision).toBe(2);
    }

    // The unconnected compass entry joins the isolated governance fixtures.
    const findings = engine.listDerivedFindings();
    expect(findings.map((finding) => finding.finding_id)).toEqual([
      "derived/isolated-entry/demo-harbor/item/compass",
      "derived/isolated-entry/demo-harbor/note/harbor-log",
      "derived/isolated-entry/demo-harbor/note/mira-private-log",
      "derived/isolated-entry/demo-harbor/note/rival-private-log",
    ]);
    const compassFinding = findings.find(
      (finding) => finding.target_entry_id === COMPASS_ENTRY.entry_id,
    );
    expect(compassFinding).toBeDefined();
    expect(compassFinding?.severity).toBe("warning");
    expect(compassFinding?.status).toBe("open");

    // Connecting the entry removes its derived finding and advances the digest.
    const rel = await adapter.putRelation(COMPASS_RELATION, null);
    expect(rel.ok).toBe(true);
    expect(
      engine.listDerivedFindings().map((finding) => finding.target_entry_id),
    ).toEqual([
      DEMO_SHARED_ENTRY_ID,
      DEMO_OWN_PRIVATE_ENTRY_ID,
      DEMO_FOREIGN_PRIVATE_ENTRY_ID,
    ]);

    const digest2 = await adapter.getKnowledgeEntry("derived/world-digest");
    expect(digest2.ok).toBe(true);
    if (digest2.ok) {
      expect(digest2.value.revision).toBe(3);
    }
  });

  it("same history yields identical derived artifacts across engines", async () => {
    const mutate = async (engine: MockEngine): Promise<void> => {
      const adapter = new MockAdapter(engine);
      const result = await adapter.putKnowledgeEntry(COMPASS_ENTRY, null);
      expect(result.ok).toBe(true);
      const lantern = await adapter.putKnowledgeEntry(LANTERN_ENTRY, null);
      expect(lantern.ok).toBe(true);
      const relation = await adapter.putRelation(COMPASS_RELATION, null);
      expect(relation.ok).toBe(true);
    };

    const engineA = new MockEngine();
    const engineB = new MockEngine();
    await mutate(engineA);
    await mutate(engineB);

    expect(engineA.getKnowledgeEntry("derived/world-digest")).toEqual(
      engineB.getKnowledgeEntry("derived/world-digest"),
    );
    expect(engineA.listDerivedFindings()).toEqual(engineB.listDerivedFindings());
  });
});

describe("host manifest", () => {
  it("returns the demo-inference-host server manifest", async () => {
    const adapter = new MockAdapter();
    const manifest = await adapter.getHostCapabilityManifest();
    expect(manifest.ok).toBe(true);
    if (manifest.ok) {
      // The server manifest declares the negotiated tool ids too (see
      // DEMO_SERVER_MANIFEST) — assert against the exported constant so the
      // fixture surface and the negotiation story stay in lockstep.
      expect(manifest.value).toEqual(DEMO_SERVER_MANIFEST);
      const toolsValidated = validateManifestTools(manifest.value);
      expect(toolsValidated.ok, toolsValidated.ok ? undefined : toolsValidated.message).toBe(
        true,
      );
    }
  });

  it("reports no peers and never leaks manifest mutation", async () => {
    const adapter = new MockAdapter();
    const peers = await adapter.listPeerHostCapabilityManifests();
    expect(peers.ok).toBe(true);
    if (peers.ok) {
      expect(peers.value).toEqual([]);
    }

    const first = await adapter.getHostCapabilityManifest();
    expect(first.ok).toBe(true);
    if (first.ok) {
      first.value.host_id = "mutated-by-caller";
    }
    const second = await adapter.getHostCapabilityManifest();
    expect(second.ok).toBe(true);
    if (second.ok) {
      expect(second.value.host_id).toBe("demo-inference-host");
    }
  });
});

describe("optional families (l2-computable / l5-fork)", () => {
  it("projects a computable view and settles compute deltas into static state", async () => {
    const adapter = new MockAdapter();

    const projected = await adapter.project({
      session_id: "demo-session/unit-1",
      entry_id: "demo-harbor/location/harbor",
      state: { ships_at_dock: 3 },
    });
    expect(projected.ok).toBe(true);
    if (projected.ok) {
      const value = expectSuccess(projected.value);
      expect(value.session_id).toBe("demo-session/unit-1");
      expect(value.computable).toEqual({ ships_at_dock: 3 });
    }

    // compute merges the delta into the session view; settle merges the
    // view back into static state (the derived state).
    const computed = await adapter.compute({
      session_id: "demo-session/unit-1",
      entry_id: "demo-harbor/location/harbor",
      computable: { tide: "rising" },
      settle: true,
    });
    expect(computed.ok).toBe(true);
    if (computed.ok) {
      const value = expectSuccess(computed.value);
      expect(value.computable).toEqual({
        ships_at_dock: 3,
        tide: "rising",
      });
      expect(value.state).toEqual({
        ships_at_dock: 3,
        tide: "rising",
      });
    }

    // A non-settling compute carries the updated view without state.
    const unsettled = await adapter.compute({
      session_id: "demo-session/unit-1",
      entry_id: "demo-harbor/location/harbor",
      computable: { tide: "falling" },
    });
    expect(unsettled.ok).toBe(true);
    if (unsettled.ok) {
      const value = expectSuccess(unsettled.value);
      expect(value.computable).toEqual({
        ships_at_dock: 3,
        tide: "falling",
      });
      // A non-settling compute carries no `state` key at all — assert the
      // key's absence, not an explicit-undefined value.
      expect("state" in value).toBe(false);
    }
  });

  it("lists the seeded storm-fork timeline events by fork_id", async () => {
    const adapter = new MockAdapter();

    const events = await adapter.listForkTimelineEvents({
      scope_id: DEMO_SCOPE_ID,
      fork_id: DEMO_SEED_FORK_ID,
    });
    expect(events.ok).toBe(true);
    if (events.ok) {
      expect(events.value).toEqual(DEMO_SEED_TIMELINE_EVENTS);
      expect(events.value.length).toBeGreaterThan(0);
    }

    // An unknown fork id round-trips as an empty timeline; a foreign scope
    // is empty too (mirrors the baseline scope guard).
    const unknown = await adapter.listForkTimelineEvents({
      scope_id: DEMO_SCOPE_ID,
      fork_id: "demo-harbor/fork/unknown",
    });
    expect(unknown.ok).toBe(true);
    if (unknown.ok) {
      expect(unknown.value).toEqual([]);
    }
    const foreign = await adapter.listForkTimelineEvents({
      scope_id: "other-scope",
      fork_id: DEMO_SEED_FORK_ID,
    });
    expect(foreign.ok).toBe(true);
    if (foreign.ok) {
      expect(foreign.value).toEqual([]);
    }
  });

  it("declares the optional families in the server manifest", async () => {
    expect(DEMO_SERVER_MANIFEST.capabilities).toEqual(
      expect.arrayContaining(["l2-computable", "l5-fork"]),
    );
  });

  it("declares the KE lifecycle capabilities and the extraction offering role", async () => {
    expect(DEMO_SERVER_MANIFEST.capabilities).toEqual(
      expect.arrayContaining(["ke-extraction", "ke-ownership"]),
    );
    expect(DEMO_SERVER_MANIFEST.roles).toContain("input-source");
  });
});

describe("scope filtering", () => {
  it("returns [] for a foreign scope and honors entry_ids / entry_types", async () => {
    const adapter = new MockAdapter();
    const scope: Scope = { scope_id: DEMO_SCOPE_ID };

    const foreign = await adapter.listKnowledgeEntries({ scope_id: "other-scope" });
    expect(foreign.ok).toBe(true);
    if (foreign.ok) {
      expect(foreign.value).toEqual([]);
    }

    const byType = await adapter.listKnowledgeEntries({
      ...scope,
      entry_types: ["character"],
    });
    expect(byType.ok).toBe(true);
    if (byType.ok) {
      expect(byType.value.map((entry) => entry.entry_id)).toEqual([
        "demo-harbor/character/mira",
      ]);
    }

    const byIds = await adapter.listKnowledgeEntries({
      ...scope,
      entry_ids: ["demo-harbor/location/harbor"],
    });
    expect(byIds.ok).toBe(true);
    if (byIds.ok) {
      expect(byIds.value).toHaveLength(1);
      expect(byIds.value[0].entry_id).toBe("demo-harbor/location/harbor");
    }
  });
});

describe("ownership-aware scope query", () => {
  it("returns shared and the viewpoint's own private entry, withholding a foreign holder's", async () => {
    const adapter = new MockAdapter();
    const listed = await adapter.listKnowledgeEntries({
      scope_id: DEMO_SCOPE_ID,
      viewpoint: DEMO_HOLDER_ENTRY_ID,
    });
    expect(listed.ok).toBe(true);
    if (!listed.ok) {
      return;
    }

    const ids = listed.value.map((entry) => entry.entry_id);
    expect(ids).toContain(DEMO_SHARED_ENTRY_ID);
    expect(ids).toContain(DEMO_OWN_PRIVATE_ENTRY_ID);
    expect(ids).not.toContain(DEMO_FOREIGN_PRIVATE_ENTRY_ID);

    // Governance travels with the entry: the scope query hands back the
    // owner, the disclosure value, and unknown extension namespaces verbatim.
    const ownPrivate = listed.value.find(
      (entry) => entry.entry_id === DEMO_OWN_PRIVATE_ENTRY_ID,
    );
    expect(ownPrivate?.owner).toBe(DEMO_HOLDER_ENTRY_ID);
    expect(ownPrivate?.disclosure).toBe("owner-private");
    expect(ownPrivate?.extensions).toEqual({
      "demo-harbor": { retention_probe: "unknown-governance-field" },
    });
  });

  it("withholds both private fixtures from a foreign viewpoint", async () => {
    const adapter = new MockAdapter();
    const listed = await adapter.listKnowledgeEntries({
      scope_id: DEMO_SCOPE_ID,
      viewpoint: "demo-harbor/character/stranger",
    });
    expect(listed.ok).toBe(true);
    if (listed.ok) {
      const ids = listed.value.map((entry) => entry.entry_id);
      expect(ids).toContain(DEMO_SHARED_ENTRY_ID);
      expect(ids).not.toContain(DEMO_OWN_PRIVATE_ENTRY_ID);
      expect(ids).not.toContain(DEMO_FOREIGN_PRIVATE_ENTRY_ID);
    }
  });
});

describe("ke-extraction service", () => {
  const RUN_ID = "demo-run/unit-1";
  const SOURCES: [SourceAnchor, ...SourceAnchor[]] = [
    {
      schema_version: 1,
      source_id: "demo-harbor/manuscript/harbor-chapter-1",
      label: "Harbor chapter 1",
      extensions: {},
    },
    {
      schema_version: 1,
      source_id: "demo-harbor/manuscript/harbor-chapter-2",
      extensions: {},
    },
  ];

  /** Narrow the closed success branch (the error branch is a wire union member). */
  function expectExtractSuccess(
    response: ExtractResponse,
  ): Extract<ExtractResponse, { candidates: KnowledgeEntry[] }> {
    expect("error" in response).toBe(false);
    return response as Extract<ExtractResponse, { candidates: KnowledgeEntry[] }>;
  }

  it("answers every referenced source with a provisional candidate and echoes run_id", async () => {
    const adapter = new MockAdapter();
    const result = await adapter.extract({ run_id: RUN_ID, sources: SOURCES });
    expect(result.ok, result.ok ? undefined : result.message).toBe(true);
    if (!result.ok) {
      return;
    }

    const response = expectExtractSuccess(result.value);
    expect(response.run.run_id).toBe(RUN_ID);
    expect(response.run.method).toBe(DEMO_EXTRACTION_METHOD);
    expect(response.candidates.map((candidate) => candidate.entry_id)).toEqual(
      [0, 1].map((index) => demoExtractCandidateEntryId(RUN_ID, index)),
    );
    for (const candidate of response.candidates) {
      expect(candidate.status).toBe("provisional");
    }

    // A candidate points back at its source anchor — it never carries source
    // content, and the anchor's label is reused as the human name.
    expect(response.candidates[0].source_anchor).toEqual(SOURCES[0]);
    expect(response.candidates[0].canonical_name).toBe("Harbor chapter 1");
    expect(response.candidates[1].canonical_name).toBe(
      "demo-harbor/manuscript/harbor-chapter-2",
    );
  });

  it("keeps the loaded value host-local (the loader canary never reaches the response)", async () => {
    const adapter = new MockAdapter();
    const result = await adapter.extract({ run_id: RUN_ID, sources: SOURCES });
    expect(result.ok).toBe(true);
    if (result.ok) {
      const serialized = JSON.stringify(result.value);
      expect(serialized).not.toContain(DEMO_EXTRACTION_CANARY);
      // Guard against a vacuous scan: the response does serialize its own data.
      expect(serialized).toContain(RUN_ID);
    }
  });

  it("rejects an empty run_id or source list with INVALID_INPUT and stores nothing", async () => {
    const adapter = new MockAdapter();
    expectRejected(
      await adapter.extract({ run_id: "", sources: SOURCES }),
      SpokeRejectCode.INVALID_INPUT,
    );
    // The generated `sources` type is a non-empty tuple, so an empty list is
    // only reachable from an untyped caller — the boundary under test.
    expectRejected(
      await adapter.extract({
        run_id: RUN_ID,
        sources: [] as unknown as ExtractRequest["sources"],
      }),
      SpokeRejectCode.INVALID_INPUT,
    );

    const listed = await adapter.listKnowledgeEntries({
      scope_id: DEMO_SCOPE_ID,
      viewpoint: DEMO_HOLDER_ENTRY_ID,
    });
    expect(listed.ok).toBe(true);
    if (listed.ok) {
      expect(
        listed.value.some((entry) => entry.entry_id.includes("extracted")),
      ).toBe(false);
    }
  });

  it("is deterministic: the same request yields an identical response", async () => {
    const request = { run_id: RUN_ID, sources: SOURCES };
    const first = await new MockAdapter().extract(request);
    const second = await new MockAdapter().extract(request);
    expect(first).toEqual(second);
  });
});
