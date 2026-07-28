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

  it("accepts create with revision 0", () => {
    const result = validateRelateRequest(makeRelation({ revision: 0 }));

    expect(result.ok).toBe(true);
  });

  it("rejects create when revision is 1 or greater", () => {
    const result = validateRelateRequest(makeRelation({ revision: 1 }));

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });

  it("rejects update path without stored via explicit mode", () => {
    const result = validateRelateRequest(makeRelation(), { mode: "update" });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.RELATION_NOT_FOUND);
    }
  });

  it("rejects create path when stored is provided via explicit mode", () => {
    const stored = makeRelation({ relation_id: "rel_stored", revision: 0 });
    const result = validateRelateRequest(makeRelation(), {
      stored,
      mode: "create",
    });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.RELATION_ALREADY_EXISTS);
    }
  });

  describe("update path", () => {
    const stored: Relation = {
      ...makeRelation({ relation_id: "rel_stored", revision: 3 }),
    };

    it("accepts update when candidate revision matches stored", () => {
      const candidate = makeRelation({ relation_id: "rel_stored", revision: 3 });

      const result = validateRelateRequest(candidate, { stored });

      expect(result.ok).toBe(true);
    });

    it("rejects update when candidate revision is behind stored (stale)", () => {
      const candidate = makeRelation({ relation_id: "rel_stored", revision: 1 });

      const result = validateRelateRequest(candidate, { stored });

      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.code).toBe(SpokeRejectCode.STORED_REVISION_STALE);
      }
    });

    it("rejects update when candidate revision is ahead of stored (conflict)", () => {
      const candidate = makeRelation({ relation_id: "rel_stored", revision: 5 });

      const result = validateRelateRequest(candidate, { stored });

      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.code).toBe(SpokeRejectCode.REVISION_CONFLICT);
      }
    });

    it("rejects update when candidate omits revision", () => {
      const candidate = makeRelation({ relation_id: "rel_stored" });

      const result = validateRelateRequest(candidate, { stored });

      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.code).toBe(SpokeRejectCode.MISSING_REQUIRED_FIELD);
      }
    });

    it("rejects update when candidate relation_id differs from stored", () => {
      const candidate = makeRelation({
        relation_id: "rel_other",
        revision: 3,
      });

      const result = validateRelateRequest(candidate, { stored });

      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
      }
    });
  });
});
