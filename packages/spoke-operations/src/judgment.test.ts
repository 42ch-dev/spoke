import { describe, expect, it } from "vitest";

import type { Judgment } from "@42ch/spoke-schemas";

import { judgmentsEqualCanonical, validateJudgment } from "./judgment.js";

function judgment(): Judgment {
  return {
    schema_version: 1,
    judgment_id: "jd_001",
    judgment_kind: "consistency",
    target: { kind: "knowledge_entry", id: "ke_mira_harbor" },
    evidence: [
      { kind: "knowledge_entry", id: "ke_mira_harbor", revision: 3 },
    ],
    verdict: { outcome: "consistent" },
    extensions: {},
  };
}

function expectField(result: unknown, field: string): void {
  expect(result).toMatchObject({
    ok: false,
    code: "INVALID_INPUT",
    details: { field },
  });
}

describe("validateJudgment", () => {
  it("accepts a minimal valid judgment with required fields only", () => {
    const j = judgment();
    j.evidence = [];
    expect(validateJudgment(j)).toEqual({ ok: true });
  });

  it("accepts a fully populated judgment", () => {
    const j = judgment();
    j.target.revision = 2;
    j.target.namespace = "toy_world";
    j.verdict.confidence = 0.75;
    j.verdict.detail = { note: "checked" };
    j.basis = "evidence review";
    j.created_at = "2026-11-22T20:36:17Z";
    j.extensions = { "my-product": { score: 1 } };
    expect(validateJudgment(j)).toEqual({ ok: true });
  });

  it("rejects a non-integer schema_version", () => {
    const j = judgment();
    (j as Record<string, unknown>).schema_version = 1.5;
    expectField(validateJudgment(j), "schema_version");
  });

  it("rejects an empty judgment_kind", () => {
    const j = judgment();
    j.judgment_kind = "";
    expectField(validateJudgment(j), "judgment_kind");
  });

  it("rejects an unknown envelope member", () => {
    const j = judgment() as unknown as Record<string, unknown>;
    j.extra = true;
    expectField(validateJudgment(j as Judgment), "judgment");
  });

  it("rejects a target with empty id", () => {
    const j = judgment();
    j.target.id = "  ";
    expectField(validateJudgment(j), "target.id");
  });

  it("rejects a non-positive target.revision", () => {
    const j = judgment();
    j.target.revision = 0;
    expectField(validateJudgment(j), "target.revision");
  });

  it("rejects a missing evidence member", () => {
    const j = judgment() as unknown as Record<string, unknown>;
    delete j.evidence;
    expectField(validateJudgment(j as Judgment), "evidence");
  });

  it("rejects an evidence item without revision", () => {
    const j = judgment();
    j.evidence = [{ kind: "knowledge_entry", id: "ke_mira_harbor" }] as Judgment["evidence"];
    expectField(validateJudgment(j), "evidence[0].revision");
  });

  it("rejects a non-positive evidence revision", () => {
    const j = judgment();
    j.evidence[0].revision = -1;
    expectField(validateJudgment(j), "evidence[0].revision");
  });

  it("rejects an empty verdict.outcome", () => {
    const j = judgment();
    j.verdict.outcome = "";
    expectField(validateJudgment(j), "verdict.outcome");
  });

  it("rejects verdict.confidence outside [0, 1]", () => {
    const j = judgment();
    j.verdict.confidence = 1.01;
    expectField(validateJudgment(j), "verdict.confidence");
  });

  it("rejects a reserved verdict.detail member", () => {
    const j = judgment();
    j.verdict.detail = { target: { kind: "knowledge_entry", id: "ke_x" } };
    expectField(validateJudgment(j), "verdict.detail.target");
  });

  it("rejects a missing extensions member", () => {
    const j = judgment() as unknown as Record<string, unknown>;
    delete j.extensions;
    expectField(validateJudgment(j as Judgment), "extensions");
  });

  it("rejects a non-string basis", () => {
    const j = judgment();
    (j as Record<string, unknown>).basis = 42;
    expectField(validateJudgment(j), "basis");
  });
});

describe("judgmentsEqualCanonical", () => {
  it("identical judgments compare equal", () => {
    expect(judgmentsEqualCanonical(judgment(), judgment())).toEqual({
      ok: true,
      value: true,
    });
  });

  it("member order does not change the canonical bytes", () => {
    const reordered = JSON.parse(
      JSON.stringify({
        extensions: {},
        verdict: { outcome: "consistent" },
        evidence: [
          { revision: 3, id: "ke_mira_harbor", kind: "knowledge_entry" },
        ],
        target: { id: "ke_mira_harbor", kind: "knowledge_entry" },
        judgment_kind: "consistency",
        judgment_id: "jd_001",
        schema_version: 1,
      }),
    ) as Judgment;
    expect(judgmentsEqualCanonical(judgment(), reordered)).toEqual({
      ok: true,
      value: true,
    });
  });

  it("verdict.detail is byte-significant in the comparison", () => {
    const withDetail = judgment();
    withDetail.verdict.detail = { note: "checked" };
    expect(judgmentsEqualCanonical(judgment(), withDetail)).toEqual({
      ok: true,
      value: false,
    });
  });
});
