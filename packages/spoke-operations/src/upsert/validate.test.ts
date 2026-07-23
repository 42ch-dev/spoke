import type { Keyblock } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import { SpokeRejectCode } from "../result.js";
import { validateUpsertKeyblock } from "./validate.js";

function makeKeyblock(
  overrides: Partial<Keyblock> & Pick<Keyblock, "keyblock_id">,
): Keyblock {
  return {
    schema_version: 1,
    block_type: "character",
    canonical_name: "Mira Vale",
    status: "provisional",
    body: { summary: "Protagonist" },
    extensions: {},
    ...overrides,
  };
}

describe("validateUpsertKeyblock", () => {
  it("accepts valid create without revision", () => {
    const candidate = makeKeyblock({ keyblock_id: "kb_new" });
    expect(validateUpsertKeyblock(candidate).ok).toBe(true);
  });

  it("accepts valid create with revision 0", () => {
    const candidate = makeKeyblock({ keyblock_id: "kb_new", revision: 0 });
    expect(validateUpsertKeyblock(candidate).ok).toBe(true);
  });

  it("rejects create with revision >= 1", () => {
    const candidate = makeKeyblock({ keyblock_id: "kb_new", revision: 1 });
    const result = validateUpsertKeyblock(candidate);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });

  it("rejects create with empty canonical_name", () => {
    const candidate = makeKeyblock({ keyblock_id: "kb_new", canonical_name: "" });
    const result = validateUpsertKeyblock(candidate);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.EMPTY_CANONICAL_NAME);
    }
  });

  it("rejects create with whitespace-only canonical_name", () => {
    const candidate = makeKeyblock({ keyblock_id: "kb_new", canonical_name: "   " });
    const result = validateUpsertKeyblock(candidate);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.EMPTY_CANONICAL_NAME);
    }
  });

  it("accepts valid update with matching revision", () => {
    const stored = makeKeyblock({ keyblock_id: "kb_1", revision: 2, status: "confirmed" });
    const candidate = makeKeyblock({ keyblock_id: "kb_1", revision: 2, status: "confirmed" });

    expect(validateUpsertKeyblock(candidate, { stored }).ok).toBe(true);
  });

  it("rejects update without revision", () => {
    const stored = makeKeyblock({ keyblock_id: "kb_1", revision: 1 });
    const candidate = makeKeyblock({ keyblock_id: "kb_1" });

    const result = validateUpsertKeyblock(candidate, { stored });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.MISSING_REQUIRED_FIELD);
    }
  });

  it("rejects update when stored revision is stale", () => {
    const stored = makeKeyblock({ keyblock_id: "kb_1", revision: 3 });
    const candidate = makeKeyblock({ keyblock_id: "kb_1", revision: 2 });

    const result = validateUpsertKeyblock(candidate, { stored });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.STORED_REVISION_STALE);
    }
  });

  it("rejects update when stored has terminal status", () => {
    const stored = makeKeyblock({ keyblock_id: "kb_1", revision: 1, status: "merged" });
    const candidate = makeKeyblock({ keyblock_id: "kb_1", revision: 1, status: "merged" });

    const result = validateUpsertKeyblock(candidate, { stored });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.KEYBLOCK_TERMINAL_STATUS);
    }
  });

  it("rejects update path without stored via explicit mode", () => {
    const candidate = makeKeyblock({ keyblock_id: "kb_1", revision: 1 });
    const result = validateUpsertKeyblock(candidate, { mode: "update" });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.KEYBLOCK_NOT_FOUND);
    }
  });

  it("rejects create path when stored is provided via explicit mode", () => {
    const stored = makeKeyblock({ keyblock_id: "kb_1", revision: 0 });
    const candidate = makeKeyblock({ keyblock_id: "kb_1" });
    const result = validateUpsertKeyblock(candidate, { stored, mode: "create" });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.KEYBLOCK_ALREADY_EXISTS);
    }
  });

  it("rejects keyblock_id mismatch on update", () => {
    const stored = makeKeyblock({ keyblock_id: "kb_1", revision: 1 });
    const candidate = makeKeyblock({ keyblock_id: "kb_2", revision: 1 });

    const result = validateUpsertKeyblock(candidate, { stored });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });

  it("rejects update when candidate is missing required fields", () => {
    const stored = makeKeyblock({ keyblock_id: "kb_1", revision: 1 });
    const candidate = {
      keyblock_id: "kb_1",
      revision: 1,
    } as Keyblock;

    const result = validateUpsertKeyblock(candidate, { stored });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.MISSING_REQUIRED_FIELD);
    }
  });
});
