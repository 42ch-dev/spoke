import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { MAX_SEQUENCE } from "../../src/core/sequence.js";
import { isRevisionInteger } from "../../src/remote/payload.js";

// Shared boundary contract (spoke-data-model §Shared wire scalars): the
// revision integer accepts the INCLUSIVE `2^53` ceiling; `sequence` stays one
// below it. `2^53 + 1` is not a JS number — it rounds to `2^53` — so the
// nearest representable out-of-domain integer is `2^53 + 2`; the exact
// `2^53 + 1` refusal is Rust-side (u64).
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
