import type { Relation } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import { SpokeRejectCode } from "../result.js";
import { validateRelateRequest } from "./validate.js";

function makeRelation(overrides: Partial<Relation> = {}): Relation {
  return {
    schema_version: 1,
    relation_id: "rel_1",
    relation_type: "related_to",
    from_id: "kb_1",
    to_id: "kb_2",
    extensions: {},
    ...overrides,
  };
}

describe("validateRelateRequest", () => {
  it("accepts a valid relation", () => {
    expect(validateRelateRequest(makeRelation()).ok).toBe(true);
  });

  it("rejects self-edge", () => {
    const result = validateRelateRequest(
      makeRelation({ from_id: "kb_1", to_id: "kb_1" }),
    );

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.RELATION_SELF_EDGE);
    }
  });

  it("rejects self-edge when ids differ only by surrounding whitespace", () => {
    const result = validateRelateRequest(
      makeRelation({ from_id: "kb_1", to_id: "kb_1 " }),
    );

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.RELATION_SELF_EDGE);
    }
  });

  it("rejects missing from_id", () => {
    const result = validateRelateRequest(makeRelation({ from_id: "   " }));

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.RELATION_MISSING_ENDPOINT);
    }
  });

  it("rejects missing to_id", () => {
    const result = validateRelateRequest(makeRelation({ to_id: "" }));

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.RELATION_MISSING_ENDPOINT);
    }
  });
});
