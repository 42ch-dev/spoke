import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

import { preserveExtensionMaps } from "@42ch/spoke-operations";
import type { AssemblePacket, Event, Finding, Keyblock } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

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

  const fixtureFiles = readdirSync(FIXTURES_ROOT).filter(
    (name) => name.endsWith(".json") && name !== "package.json",
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
