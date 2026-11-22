import { describe, expect, it } from "vitest";

import { validateJudgment, judgmentsEqualCanonical } from "./judgment.js";
import type { Judgment } from "@42ch/spoke-schemas";

function baseJudgment(): Judgment {
  return {
    schema_version: 1,
    judgment_id: "j-1",
    judgment_kind: "assessment",
    target: { kind: "entry", id: "e-1" },
    evidence: [{ kind: "entry", id: "e-0", revision: 1 }],
    verdict: { outcome: "upheld" },
    extensions: {},
  } as unknown as Judgment;
}

describe("wire-member scope (own enumerable only)", () => {
  it("treats a non-enumerable own required member as absent", () => {
    const judgment = baseJudgment();
    Object.defineProperty(judgment, "judgment_kind", {
      value: "assessment",
      enumerable: false,
      writable: true,
      configurable: true,
    });
    const result = validateJudgment(judgment);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.details?.field).toBe("judgment_kind");
    }
  });

  it("ignores a non-enumerable own optional member with an invalid value", () => {
    const judgment = baseJudgment();
    Object.defineProperty(judgment, "basis", {
      value: 42,
      enumerable: false,
      configurable: true,
    });
    expect(validateJudgment(judgment).ok).toBe(true);
  });

  it("does not satisfy required presence from an inherited member", () => {
    const judgment = baseJudgment() as Record<string, unknown>;
    delete judgment.judgment_id;
    Object.setPrototypeOf(judgment, { judgment_id: "inherited" });
    const result = validateJudgment(judgment as unknown as Judgment);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.details?.field).toBe("judgment_id");
    }
  });

  it("ignores an inherited member inside a JudgmentRef", () => {
    const judgment = baseJudgment() as Record<string, unknown>;
    const ref = { id: "e-1" } as Record<string, unknown>;
    Object.setPrototypeOf(ref, { kind: "entry" });
    judgment.target = ref;
    const result = validateJudgment(judgment as unknown as Judgment);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.details?.field).toBe("target.kind");
    }
  });

  it("ignores non-enumerable members nested inside extensions values", () => {
    const judgment = baseJudgment() as Record<string, unknown>;
    const entry = { note: "ok" } as Record<string, unknown>;
    Object.defineProperty(entry, "toJSON", {
      value: () => ({}),
      enumerable: false,
    });
    judgment.extensions = { acme: entry };
    expect(validateJudgment(judgment as unknown as Judgment).ok).toBe(true);
  });

  it("excludes enumerable toJSON hooks from canonical bytes", () => {
    const left = baseJudgment() as Record<string, unknown>;
    const right = baseJudgment() as Record<string, unknown>;
    left.verdict = { outcome: "upheld", detail: { note: "x" } };
    const detail = { note: "x", toJSON: () => ({ note: "changed" }) };
    right.verdict = { outcome: "upheld", detail };
    expect(
      judgmentsEqualCanonical(
        left as unknown as Judgment,
        right as unknown as Judgment,
      ),
    ).toEqual({ ok: true, value: true });
  });

  it("excludes non-enumerable members from canonical bytes", () => {
    const left = baseJudgment() as Record<string, unknown>;
    const right = baseJudgment() as Record<string, unknown>;
    Object.defineProperty(right, "basis", {
      value: "hidden",
      enumerable: false,
    });
    expect(
      judgmentsEqualCanonical(
        left as unknown as Judgment,
        right as unknown as Judgment,
      ),
    ).toEqual({ ok: true, value: true });
  });
});
