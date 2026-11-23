import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import { compileSchemaValidator, createSchemaValidator } from "./schema-validator.js";

// Shared boundary contract (spoke-data-model §Shared wire scalars):
// `SchemaVersion` is `[1, 2^53-1]`; `2^53` is one past the ceiling.
const boundaryCases = JSON.parse(
  readFileSync(new URL("../../wire-boundary-cases.json", import.meta.url), "utf8"),
) as { schema_version: { accept: number[]; reject: number[] } };

const SCHEMA_VERSION_ID =
  "https://spoke42.invalid/schemas/common/common.schema.json#/definitions/SchemaVersion";

describe("common.SchemaVersion boundary", () => {
  const validate = compileSchemaValidator(createSchemaValidator(), SCHEMA_VERSION_ID);

  it.each(boundaryCases.schema_version.accept)("accepts %s", (value) => {
    expect(validate(value)).toBe(true);
  });

  it.each(boundaryCases.schema_version.reject)("refuses %s", (value) => {
    expect(validate(value)).toBe(false);
  });
});
