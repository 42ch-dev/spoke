import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

import type {
  AssemblePacket,
  Event,
  Finding,
  Keyblock,
  PromoteRequest,
  Scope,
} from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import { buildAssemblePacket } from "../assemble/builder.js";
import { preserveExtensionMaps } from "../extensions/merge.js";
import { transitionFindingStatus } from "../finding/transition.js";
import { assertRevisionMatch } from "../occ/assert-revision.js";
import { validatePromoteRequest } from "../promote/acceptance.js";
import {
  eventMatchesScope,
  filterKeyblocksByScope,
  keyblockMatchesScope,
} from "../scope/match.js";
import {
  FIXTURE_SCHEMA_MAP,
  FIXTURES_ROOT,
  compileSchemaValidator,
  createSchemaValidator,
} from "./schema-validator.js";

function loadFixture<T>(filename: string): T {
  const raw = readFileSync(join(FIXTURES_ROOT, filename), "utf8");
  return JSON.parse(raw) as T;
}

describe("fixtures/toy-world schema conformance", () => {
  const ajv = createSchemaValidator();

  const fixtureFiles = readdirSync(FIXTURES_ROOT).filter((name) =>
    name.endsWith(".json"),
  );

  it.each(fixtureFiles)("validates %s against its schema", (filename) => {
    const schemaId = FIXTURE_SCHEMA_MAP[filename];

    expect(schemaId, `missing schema mapping for ${filename}`).toBeDefined();

    const validate = compileSchemaValidator(ajv, schemaId!);
    const data = JSON.parse(
      readFileSync(join(FIXTURES_ROOT, filename), "utf8"),
    );

    const valid = validate(data);

    expect(validate.errors, JSON.stringify(validate.errors, null, 2)).toBeNull();
    expect(valid).toBe(true);
  });

  it("covers every mapped fixture file", () => {
    expect(fixtureFiles.sort()).toEqual(Object.keys(FIXTURE_SCHEMA_MAP).sort());
  });

  it("resolves graph ids within the toy-world graph", () => {
    const mira = loadFixture<Keyblock>("kb_tw_mira.json");
    const harbor = loadFixture<Keyblock>("kb_tw_harbor.json");
    const relation = loadFixture<{ from_id: string; to_id: string }>(
      "rel_tw_mira_harbor.json",
    );
    const event = loadFixture<Event>("evt_tw_harbor_dawn.json");
    const finding = loadFixture<Finding>("fnd_tw_open.json");
    const packet = loadFixture<AssemblePacket>("pkt_tw_scope.json");

    expect(relation.from_id).toBe(mira.keyblock_id);
    expect(relation.to_id).toBe(harbor.keyblock_id);
    expect(event.participant_keyblock_ids).toEqual(
      expect.arrayContaining([mira.keyblock_id, harbor.keyblock_id]),
    );
    expect(finding.target_keyblock_id).toBe(mira.keyblock_id);
    expect(packet.entries.map((entry) => entry.keyblock_id)).toEqual([
      mira.keyblock_id,
      harbor.keyblock_id,
    ]);
  });

  it("preserves unknown extension keys on Mira Keyblock", () => {
    const mira = loadFixture<Keyblock>("kb_tw_mira.json");
    const preserved = preserveExtensionMaps(mira.extensions, {});

    expect(preserved.nexus).toEqual({
      world_id: "wld_toy_nexus_001",
      fork_hint: "baseline",
    });
    expect(preserved.creader).toEqual({
      book_id: "bk_toy_creader_001",
      chapter_slug: "harbor-arrival",
    });
  });
});

describe("fixtures/toy-world ops exercise", () => {
  const mira = () => loadFixture<Keyblock>("kb_tw_mira.json");
  const harbor = () => loadFixture<Keyblock>("kb_tw_harbor.json");
  const event = () => loadFixture<Event>("evt_tw_harbor_dawn.json");
  const finding = () => loadFixture<Finding>("fnd_tw_open.json");

  it("passes promote gate for provisional Mira candidate", () => {
    const candidate: Keyblock = {
      ...mira(),
      status: "provisional",
    };
    const request: PromoteRequest = { candidate };

    expect(validatePromoteRequest(request).ok).toBe(true);
  });

  it("matches Scope toy-scope-001 refinements", () => {
    const scope: Scope = {
      scope_id: "toy-scope-001",
      keyblock_ids: ["kb_tw_mira", "kb_tw_harbor"],
      block_types: ["character", "location"],
    };

    expect(keyblockMatchesScope(mira(), scope)).toBe(true);
    expect(keyblockMatchesScope(harbor(), scope)).toBe(true);
    expect(filterKeyblocksByScope([mira(), harbor()], scope)).toHaveLength(2);
    expect(
      keyblockMatchesScope(mira(), {
        scope_id: "toy-scope-001",
        source_id: "manuscript:tw-ch1",
      }),
    ).toBe(true);
    expect(
      eventMatchesScope(event(), {
        scope_id: "toy-scope-001",
        event_ids: ["evt_tw_harbor_dawn"],
        timeline_scale: "moment",
      }),
    ).toBe(true);
  });

  it("asserts revision match on fixture Keyblocks", () => {
    expect(assertRevisionMatch(1, 1).ok).toBe(true);
    expect(assertRevisionMatch(1, 0).ok).toBe(false);
  });

  it("builds AssemblePacket from fixture Keyblocks", () => {
    const result = buildAssemblePacket({
      packetId: "pkt_tw_scope_built",
      keyblocks: [mira(), harbor()],
    });

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.entries).toHaveLength(2);
      expect(result.value.packet_id).toBe("pkt_tw_scope_built");
    }
  });

  it("transitions open finding to resolved", () => {
    const result = transitionFindingStatus(finding(), "resolved");

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.status).toBe("resolved");
    }
  });
});
