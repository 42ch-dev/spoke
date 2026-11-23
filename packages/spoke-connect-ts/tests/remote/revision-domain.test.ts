import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { MAX_SEQUENCE } from "../../src/core/sequence.js";
import { isRevisionInteger } from "../../src/remote/payload.js";

// Shared boundary contract (spoke-data-model §Shared wire scalars): the
// revision integer accepts the INCLUSIVE `2^53` ceiling; `sequence` stays one
// below it. The bound is asserted on the parsed integer value, so a JS host
// refuses the first representable value above it: `2^53 + 2`. The literal
// `2^53 + 1` is unrepresentable in IEEE-754 and rounds to `2^53` during a JS
// JSON parse, so its exact refusal is a parser-level property (Rust u64) —
// this test does not claim to prove refusal of that literal. See
// `.agents/specs/protocol/spoke-data-model.md:525`.
const boundaryCases = JSON.parse(
  readFileSync(
    join(
      dirname(fileURLToPath(import.meta.url)),
      "../../../../fixtures/wire-boundary-cases.json",
    ),
    "utf8",
  ),
) as { revision: { accept: number[]; reject: number[] } };

describe("revision integer domain", () => {
  it.each(boundaryCases.revision.accept)("accepts revision %s", (value) => {
    expect(isRevisionInteger(value)).toBe(true);
  });

  it.each(boundaryCases.revision.reject)("refuses revision %s", (value) => {
    expect(isRevisionInteger(value)).toBe(false);
  });

  it("keeps sequence one below the revision ceiling", () => {
    expect(MAX_SEQUENCE).toBe(2 ** 53 - 1);
  });
});
