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

  it("excludes non-enumerable envelope members from canonical bytes", () => {
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

describe("wire projection details", () => {
  it("round-trips an own enumerable __proto__ data key in opaque content", () => {
    const detail = JSON.parse('{"__proto__":{"x":1},"note":"n"}') as Record<
      string,
      unknown
    >;
    const judgment = baseJudgment() as Record<string, unknown>;
    judgment.verdict = { outcome: "upheld", detail };
    expect(validateJudgment(judgment as unknown as Judgment).ok).toBe(true);

    const explicit = baseJudgment() as Record<string, unknown>;
    explicit.verdict = {
      outcome: "upheld",
      detail: Object.fromEntries([
        ["note", "n"],
        ["__proto__", { x: 1 }],
      ]),
    };
    expect(
      judgmentsEqualCanonical(
        judgment as unknown as Judgment,
        explicit as unknown as Judgment,
      ),
    ).toEqual({ ok: true, value: true });
    // The projected detail still carries __proto__ as an own data property,
    // not a swallowed prototype write.
    const projected = judgmentsEqualCanonical(
      judgment as unknown as Judgment,
      (() => {
        const missing = baseJudgment() as Record<string, unknown>;
        missing.verdict = { outcome: "upheld", detail: { note: "n" } };
        return missing as unknown as Judgment;
      })(),
    );
    expect(projected).toEqual({ ok: true, value: false });
  });

  it("compacts a hidden array index instead of reading it", () => {
    const validRef = { kind: "entry", id: "e-0", revision: 1 };
    const judgment = baseJudgment() as Record<string, unknown>;
    const evidence = [
      { kind: "entry", id: "hidden-bad" },
      validRef,
    ] as Record<string, unknown>[];
    // Index 0 is hidden: it is not a wire member, must not be read at all.
    Object.defineProperty(evidence, "0", { enumerable: false });
    judgment.evidence = evidence;
    expect(validateJudgment(judgment as unknown as Judgment).ok).toBe(true);

    const explicit = baseJudgment() as Record<string, unknown>;
    explicit.evidence = [validRef];
    expect(
      judgmentsEqualCanonical(
        judgment as unknown as Judgment,
        explicit as unknown as Judgment,
      ),
    ).toEqual({ ok: true, value: true });
  });

  it("keeps extension values verbatim: odd and non-enumerable members untouched", () => {
    const judgment = baseJudgment() as Record<string, unknown>;
    const acme = { note: "x" } as Record<string, unknown>;
    Object.defineProperty(acme, "hidden", {
      value: "off-wire",
      enumerable: false,
    });
    Object.setPrototypeOf(acme, { inherited: "off-wire" });
    judgment.extensions = { acme };
    expect(validateJudgment(judgment as unknown as Judgment).ok).toBe(true);

    const explicit = baseJudgment() as Record<string, unknown>;
    explicit.extensions = { acme: { note: "x" } };
    expect(
      judgmentsEqualCanonical(
        judgment as unknown as Judgment,
        explicit as unknown as Judgment,
      ),
    ).toEqual({ ok: true, value: true });
  });

  it("honors toJSON inside opaque detail exactly as wire serialization does", () => {
    const left = baseJudgment() as Record<string, unknown>;
    const right = baseJudgment() as Record<string, unknown>;
    left.verdict = { outcome: "upheld", detail: { note: "x" } };
    // detail is opaque content copied verbatim; its toJSON hook fires during
    // canonicalization just as it would during JSON.stringify emission.
    right.verdict = {
      outcome: "upheld",
      detail: { note: "x", toJSON: () => ({ note: "changed" }) },
    };
    expect(
      judgmentsEqualCanonical(
        left as unknown as Judgment,
        right as unknown as Judgment,
      ),
    ).toEqual({ ok: true, value: false });

    const explicit = baseJudgment() as Record<string, unknown>;
    explicit.verdict = { outcome: "upheld", detail: { note: "changed" } };
    expect(
      judgmentsEqualCanonical(
        right as unknown as Judgment,
        explicit as unknown as Judgment,
      ),
    ).toEqual({ ok: true, value: true });
  });
});
