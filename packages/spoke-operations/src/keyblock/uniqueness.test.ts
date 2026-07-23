import type { Keyblock } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import { SpokeRejectCode } from "../result.js";
import { assertUniqueActiveKeyblock } from "./uniqueness.js";

function makeKeyblock(
  overrides: Partial<Keyblock> & Pick<Keyblock, "keyblock_id">,
): Keyblock {
  return {
    schema_version: 1,
    block_type: "character",
    canonical_name: "Mira Vale",
    status: "confirmed",
    body: { summary: "Protagonist" },
    extensions: {},
    ...overrides,
  };
}

const baseInput = {
  scope_key: "world_1",
  block_type: "character",
  canonical_name: "Mira Vale",
};

describe("assertUniqueActiveKeyblock", () => {
  it("accepts when no conflicting active keyblock exists", () => {
    const candidate = makeKeyblock({ keyblock_id: "kb_new", status: "provisional" });
    const result = assertUniqueActiveKeyblock({
      ...baseInput,
      candidate,
      existing: [
        makeKeyblock({
          keyblock_id: "kb_other",
          block_type: "location",
          canonical_name: "Harbor",
        }),
      ],
    });

    expect(result.ok).toBe(true);
  });

  it("rejects duplicate active triple for a different keyblock_id", () => {
    const candidate = makeKeyblock({ keyblock_id: "kb_new", status: "provisional" });
    const result = assertUniqueActiveKeyblock({
      ...baseInput,
      candidate,
      existing: [makeKeyblock({ keyblock_id: "kb_existing", status: "confirmed" })],
    });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.DUPLICATE_ACTIVE_KEYBLOCK);
      expect(result.details).toEqual({
        scope_key: "world_1",
        block_type: "character",
        canonical_name: "Mira Vale",
        conflicting_keyblock_id: "kb_existing",
      });
    }
  });

  it("allows same keyblock_id update in place", () => {
    const candidate = makeKeyblock({ keyblock_id: "kb_1", status: "confirmed" });
    const result = assertUniqueActiveKeyblock({
      ...baseInput,
      candidate,
      existing: [makeKeyblock({ keyblock_id: "kb_1", status: "confirmed" })],
    });

    expect(result.ok).toBe(true);
  });

  it("ignores inactive existing keyblocks", () => {
    const candidate = makeKeyblock({ keyblock_id: "kb_new", status: "provisional" });
    const result = assertUniqueActiveKeyblock({
      ...baseInput,
      candidate,
      existing: [
        makeKeyblock({ keyblock_id: "kb_deprecated", status: "deprecated" }),
        makeKeyblock({ keyblock_id: "kb_merged", status: "merged" }),
        makeKeyblock({ keyblock_id: "kb_deleted", status: "deleted" }),
      ],
    });

    expect(result.ok).toBe(true);
  });

  it("passes when candidate is inactive", () => {
    const candidate = makeKeyblock({ keyblock_id: "kb_new", status: "deprecated" });
    const result = assertUniqueActiveKeyblock({
      ...baseInput,
      candidate,
      existing: [makeKeyblock({ keyblock_id: "kb_existing", status: "confirmed" })],
    });

    expect(result.ok).toBe(true);
  });

  it("rejects when block_type or canonical_name do not match candidate wire fields (R1)", () => {
    const candidate = makeKeyblock({ keyblock_id: "kb_new", status: "provisional" });

    const blockTypeMismatch = assertUniqueActiveKeyblock({
      ...baseInput,
      block_type: "location",
      candidate,
      existing: [],
    });
    expect(blockTypeMismatch.ok).toBe(false);
    if (!blockTypeMismatch.ok) {
      expect(blockTypeMismatch.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }

    const nameMismatch = assertUniqueActiveKeyblock({
      ...baseInput,
      canonical_name: "Other Name",
      candidate,
      existing: [],
    });
    expect(nameMismatch.ok).toBe(false);
    if (!nameMismatch.ok) {
      expect(nameMismatch.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });
});
