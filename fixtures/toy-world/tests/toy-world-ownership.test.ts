import { readFileSync } from "node:fs";
import { join } from "node:path";

import {
  filterKnowledgeEntriesByScope,
  getKnowledgeEntryOwner,
  knowledgeEntryMatchesScope,
  knowledgeEntryVisibleToViewpoint,
} from "@42ch/spoke-operations";
import type {
  CheckRequest,
  KnowledgeEntry,
  MindState,
} from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import {
  FIXTURE_SCHEMA_MAP,
  FIXTURES_ROOT,
  SCHEMA_IDS,
  compileSchemaValidator,
  createSchemaValidator,
} from "./schema-validator.js";

function loadFixture<T>(filename: string): T {
  const raw = readFileSync(join(FIXTURES_ROOT, filename), "utf8");
  return JSON.parse(raw) as T;
}

describe("fixtures/toy-world ke-ownership samples", () => {
  const ajv = createSchemaValidator();

  const mira = loadFixture<KnowledgeEntry>("kb_tw_mira.json");
  const harbor = loadFixture<KnowledgeEntry>("kb_tw_harbor.json");
  const privateReading = loadFixture<KnowledgeEntry>(
    "kb_tw_owner_private.json",
  );
  const request = loadFixture<CheckRequest>(
    "op_tw_ownership_check_request.json",
  );
  // A second holder already present in the graph: the MindState holder of the
  // false-belief pair. Read from its fixture rather than restated here.
  const foreignHolder =
    loadFixture<MindState>("mind_tw_bo_pre_transfer.json").holder_entry_id;

  it("validates both ownership samples against their registered schemas", () => {
    const samples = [
      ["kb_tw_owner_private.json", SCHEMA_IDS.knowledgeEntry],
      ["op_tw_ownership_check_request.json", SCHEMA_IDS.checkRequest],
    ] as const;

    for (const [filename, schemaId] of samples) {
      expect(FIXTURE_SCHEMA_MAP[filename]).toBe(schemaId);

      const validate = compileSchemaValidator(ajv, schemaId);
      const valid = validate(loadFixture<unknown>(filename));

      expect(validate.errors, JSON.stringify(validate.errors, null, 2)).toBeNull();
      expect(valid).toBe(true);
    }
  });

  it("references the holder KnowledgeEntry by its actual entry_id", () => {
    // The holder is the existing Mira KnowledgeEntry; both samples must point at
    // its real entry_id, which is read from the fixture, not derived from names.
    expect(privateReading.owner).toBe(mira.entry_id);
    expect(getKnowledgeEntryOwner(privateReading)).toBe(mira.entry_id);
    expect(request.scope.viewpoint).toBe(mira.entry_id);
    expect(request.scope.entry_ids).toEqual([
      privateReading.entry_id,
      harbor.entry_id,
    ]);

    // The anchor entries stay field-absent: ownership is opt-in, and baseline
    // fixtures keep their untouched shape.
    expect(mira.owner).toBeUndefined();
    expect(mira.disclosure).toBeUndefined();
    expect(harbor.owner).toBeUndefined();
    expect(harbor.disclosure).toBeUndefined();
  });

  it("filters own versus foreign viewpoint over the same samples", () => {
    const candidates = [privateReading, harbor];

    expect(privateReading.disclosure).toBe("owner-private");
    expect(foreignHolder).not.toBe(mira.entry_id);

    // Owner viewpoint: the private reading and the shared entry both survive.
    expect(request.scope.viewpoint).toBe(privateReading.owner);
    expect(knowledgeEntryVisibleToViewpoint(privateReading, mira.entry_id)).toBe(
      true,
    );
    expect(knowledgeEntryMatchesScope(privateReading, request.scope)).toBe(true);
    expect(
      filterKnowledgeEntriesByScope(candidates, request.scope).map(
        (entry) => entry.entry_id,
      ),
    ).toEqual([privateReading.entry_id, harbor.entry_id]);

    // Foreign viewpoint over the identical fixture: only the field-absent
    // baseline entry is shared; the owner-private reading drops out.
    const foreignScope = { ...request.scope, viewpoint: foreignHolder };

    expect(
      knowledgeEntryVisibleToViewpoint(privateReading, foreignHolder),
    ).toBe(false);
    expect(knowledgeEntryMatchesScope(privateReading, foreignScope)).toBe(false);
    expect(knowledgeEntryVisibleToViewpoint(harbor, foreignHolder)).toBe(true);
    expect(
      filterKnowledgeEntriesByScope(candidates, foreignScope).map(
        (entry) => entry.entry_id,
      ),
    ).toEqual([harbor.entry_id]);
  });

  it("keeps unknown disclosure vocabulary schema-valid and not implicitly shared", () => {
    const unknownDisclosure: KnowledgeEntry = {
      ...privateReading,
      disclosure: "circle-of-trust",
    };

    // Open vocabulary: a product-defined disclosure value still validates.
    const validate = compileSchemaValidator(ajv, SCHEMA_IDS.knowledgeEntry);
    const valid = validate(unknownDisclosure);

    expect(validate.errors, JSON.stringify(validate.errors, null, 2)).toBeNull();
    expect(valid).toBe(true);

    // ...but the core predicate never reinterprets it as shared — not even for
    // the owner viewpoint, nor for a viewpoint spelling the value verbatim.
    expect(
      knowledgeEntryVisibleToViewpoint(unknownDisclosure, mira.entry_id),
    ).toBe(false);
    expect(
      knowledgeEntryVisibleToViewpoint(unknownDisclosure, "circle-of-trust"),
    ).toBe(false);
    expect(
      knowledgeEntryMatchesScope(unknownDisclosure, request.scope),
    ).toBe(false);
    expect(
      filterKnowledgeEntriesByScope([unknownDisclosure], request.scope),
    ).toEqual([]);
  });
});
