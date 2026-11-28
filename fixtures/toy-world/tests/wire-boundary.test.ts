import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import { isStrictTimestamp } from "@42ch/spoke-operations";

import { isRevisionInteger } from "../../../packages/spoke-connect-ts/src/remote/payload.js";
import { compileSchemaValidator, createSchemaValidator } from "./schema-validator.js";

// Shared boundary contract (spoke-data-model §Shared wire scalars):
// `SchemaVersion` is `[1, 2^53-1]`; `2^53` is one past the ceiling.
// `Timestamp` is the strict RFC 3339 grammar (leap second `:60` admitted on
// any date; space-separated and offsetless spellings refused). Revision
// integers are `[0, 2^53]` inclusive; the fixture's refused `2^53+2` is the
// first representable JavaScript value above that ceiling because `2^53+1`
// rounds to `2^53` in a JS JSON parse (spoke-data-model.md:525).
//
// Each domain is asserted where its verdict is produced: the schema side
// through AJV (SchemaVersion, Timestamp via the registered strict format,
// Revision through the owning `Relation` schema) and the runtime side
// through the shared predicates (qc3-08).
const boundaryCases = JSON.parse(
  readFileSync(new URL("../../wire-boundary-cases.json", import.meta.url), "utf8"),
) as {
  schema_version: { accept: number[]; reject: number[] };
  revision: { accept: number[]; reject: number[] };
  timestamp: { accept: string[]; reject: string[] };
};

const SCHEMA_VERSION_ID =
  "https://spoke42.invalid/schemas/common/common.schema.json#/definitions/SchemaVersion";
const TIMESTAMP_ID =
  "https://spoke42.invalid/schemas/common/common.schema.json#/definitions/Timestamp";
const RELATION_ID = "https://spoke42.invalid/schemas/data/relation.schema.json";

describe("common.SchemaVersion boundary through AJV", () => {
  const validate = compileSchemaValidator(createSchemaValidator(), SCHEMA_VERSION_ID);

  it.each(boundaryCases.schema_version.accept)("accepts %s", (value) => {
    expect(validate(value)).toBe(true);
  });

  it.each(boundaryCases.schema_version.reject)("refuses %s", (value) => {
    expect(validate(value)).toBe(false);
  });
});

describe("common.Timestamp boundary through AJV", () => {
  const validate = compileSchemaValidator(createSchemaValidator(), TIMESTAMP_ID);

  it.each(boundaryCases.timestamp.accept)("accepts %s", (value) => {
    expect(validate(value)).toBe(true);
  });

  it.each(boundaryCases.timestamp.reject)("refuses %s", (value) => {
    expect(validate(value)).toBe(false);
  });
});

describe("registered date-time format boundary (no Timestamp pattern)", () => {
  const validate = createSchemaValidator().compile({
    type: "string",
    format: "date-time",
  });

  it.each(boundaryCases.timestamp.accept)("accepts %s", (value) => {
    expect(validate(value)).toBe(true);
  });

  it.each(boundaryCases.timestamp.reject)("refuses %s", (value) => {
    expect(validate(value)).toBe(false);
  });
});

describe("Timestamp boundary through isStrictTimestamp", () => {
  it.each(boundaryCases.timestamp.accept)("accepts %s", (value) => {
    expect(isStrictTimestamp(value)).toBe(true);
  });

  it.each(boundaryCases.timestamp.reject)("refuses %s", (value) => {
    expect(isStrictTimestamp(value)).toBe(false);
  });
});

describe("revision integer boundary through the Relation schema", () => {
  const validate = compileSchemaValidator(createSchemaValidator(), RELATION_ID);
  const relation = (revision: number) => ({
    schema_version: 1,
    relation_id: "rel_boundary",
    relation_type: "related_to",
    from_id: "kb_from",
    to_id: "kb_to",
    revision,
    extensions: {},
  });

  it.each(boundaryCases.revision.accept)("accepts revision %s", (value) => {
    expect(validate(relation(value))).toBe(true);
  });

  it.each(boundaryCases.revision.reject)("refuses revision %s", (value) => {
    expect(validate(relation(value))).toBe(false);
  });
});

describe("revision integer boundary through isRevisionInteger", () => {
  it.each(boundaryCases.revision.accept)("accepts %s", (value) => {
    expect(isRevisionInteger(value)).toBe(true);
  });

  it.each(boundaryCases.revision.reject)("refuses %s", (value) => {
    expect(isRevisionInteger(value)).toBe(false);
  });
});
