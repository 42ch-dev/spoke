/**
 * MockAdapter / MockEngine unit tests.
 *
 * Covers the brief's Step 2 acceptance surface:
 * - engine seed corpus loads (2 entries + 1 relation + 1 rule);
 * - put→get round-trip honors OCC (stale `expectedBaseRevision` → reject per
 *   operations conventions);
 * - derivation is deterministic (same history → same derived artifact
 *   ids/bodies);
 * - `getHostCapabilityManifest` returns the server manifest.
 */

import { describe, expect, it } from "vitest";

import type {
  Finding,
  KnowledgeEntry,
  Relation,
  Rule,
  Scope,
} from "@42ch/spoke-schemas";
import { SpokeRejectCode, type SpokeResult } from "@42ch/spoke-operations";

import { MockAdapter } from "../src/adapter/mock-adapter.js";
import { MockEngine } from "../src/engine/mock-engine.js";
import {
  DEMO_SEED_ENTRIES,
  DEMO_SEED_RELATIONS,
  DEMO_SEED_RULES,
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
  it("loads 2 entries + 1 relation + 1 rule in scope demo-harbor", async () => {
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
      expect(ids).toEqual([
        ...DEMO_SEED_ENTRIES.map((entry) => entry.entry_id).sort(),
        "derived/world-digest",
      ]);
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
        entry_type_counts: { character: 1, location: 1 },
        entry_ids_sorted: [
          "demo-harbor/character/mira",
          "demo-harbor/location/harbor",
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
        entry_type_counts: { character: 1, item: 1, location: 1 },
        entry_ids_sorted: [
          "demo-harbor/character/mira",
          "demo-harbor/item/compass",
          "demo-harbor/location/harbor",
        ],
      });
      expect(digest.value.revision).toBe(2);
    }

    // The unconnected compass entry is isolated → derived finding appears.
    const findings = engine.listDerivedFindings();
    expect(findings.map((finding) => finding.finding_id)).toEqual([
      "derived/isolated-entry/demo-harbor/item/compass",
    ]);
    expect(findings[0].target_entry_id).toBe(COMPASS_ENTRY.entry_id);
    expect(findings[0].severity).toBe("warning");
    expect(findings[0].status).toBe("open");

    // Connecting the entry removes the derived finding and advances the digest.
    const rel = await adapter.putRelation(COMPASS_RELATION, null);
    expect(rel.ok).toBe(true);
    expect(engine.listDerivedFindings()).toEqual([]);

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
      expect(manifest.value).toEqual({
        schema_version: 1,
        host_id: "demo-inference-host",
        roles: ["checker", "assembler"],
        capabilities: ["spoke-baseline"],
        namespaces: ["demo-harbor"],
        extensions: {},
      });
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
