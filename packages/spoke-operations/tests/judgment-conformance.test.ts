/**
 * Judgment conformance corpus runner (spoke-judgment wire contract).
 *
 * Executes `fixtures/judgment-conformance/`: every `valid.json` envelope
 * must pass `validateJudgment`, every `refusals.json` case must reject
 * `INVALID_INPUT` with `details.field` equal to the case's exact logical
 * path, and every `canonical-parity.json` pair must canonicalize to the
 * case's expected RFC 8785 bytes. The Rust runner
 * (`crates/spoke-operations/tests/judgment_conformance.rs`) asserts the same
 * corpus, so shared `expected_canonical` strings make each vector a
 * cross-stack byte-parity check.
 */

import { readFileSync } from "node:fs";

import canonicalize from "canonicalize";
import { describe, expect, it } from "vitest";

import type { Judgment } from "@42ch/spoke-schemas";
import {
  judgmentsEqualCanonical,
  validateJudgment,
} from "@42ch/spoke-operations";

interface RefusalCase {
  id: string;
  judgment: unknown;
  expect: { field: string };
}

interface ValidCase {
  id: string;
  judgment: Judgment;
}

interface ParityCase {
  id: string;
  left: Judgment;
  right: Judgment;
  equal?: boolean;
  expected_canonical: string | { left: string; right: string };
}

function loadScenario<T>(filename: string): { cases: T[] } {
  return JSON.parse(
    readFileSync(
      new URL(
        `../../../fixtures/judgment-conformance/${filename}`,
        import.meta.url,
      ),
      "utf8",
    ),
  ) as { cases: T[] };
}

describe("judgment-conformance valid vectors", () => {
  const { cases } = loadScenario<ValidCase>("valid.json");
  for (const { id, judgment } of cases) {
    it(`${id} passes validateJudgment`, () => {
      expect(validateJudgment(judgment)).toEqual({ ok: true });
    });
  }
});

describe("judgment-conformance refusal vectors", () => {
  const { cases } = loadScenario<RefusalCase>("refusals.json");
  for (const { id, judgment, expect: expected } of cases) {
    it(`${id} rejects INVALID_INPUT with field ${expected.field}`, () => {
      expect(validateJudgment(judgment as Judgment)).toMatchObject({
        ok: false,
        code: "INVALID_INPUT",
        details: { field: expected.field },
      });
    });
  }
});

describe("judgment-conformance canonical-parity vectors", () => {
  const { cases } = loadScenario<ParityCase>("canonical-parity.json");
  for (const { id, left, right, equal, expected_canonical } of cases) {
    it(`${id} produces the expected canonical bytes`, () => {
      // Both producers must pass the wire-shape gate before byte claims.
      expect(validateJudgment(left)).toEqual({ ok: true });
      expect(validateJudgment(right)).toEqual({ ok: true });

      // `equal: false` cases carry per-side expected bytes; equal cases one
      // shared string.
      const leftExpected =
        typeof expected_canonical === "string"
          ? expected_canonical
          : expected_canonical.left;
      const rightExpected =
        typeof expected_canonical === "string"
          ? expected_canonical
          : expected_canonical.right;

      expect(judgmentsEqualCanonical(left, right)).toEqual({
        ok: true,
        value: equal !== false,
      });
      expect(canonicalize(left)).toBe(leftExpected);
      expect(canonicalize(right)).toBe(rightExpected);
    });
  }
});
