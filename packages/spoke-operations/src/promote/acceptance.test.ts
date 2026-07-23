import type { Keyblock, PromoteRequest } from "@42ch/spoke-schema";
import { describe, expect, it } from "vitest";

import { SpokeRejectCode } from "../result.js";
import {
  applyPromoteAcceptance,
  validatePromoteRequest,
} from "./acceptance.js";

function makeCandidate(overrides: Partial<Keyblock> = {}): Keyblock {
  return {
    schema_version: 1,
    keyblock_id: "kb_1",
    block_type: "character",
    canonical_name: "Mira Vale",
    status: "provisional",
    body: { summary: "Protagonist" },
    extensions: {},
    ...overrides,
  };
}

function makeRequest(overrides: Partial<PromoteRequest> = {}): PromoteRequest {
  return {
    candidate: makeCandidate(),
    ...overrides,
  };
}

describe("validatePromoteRequest", () => {
  it("accepts a valid provisional candidate", () => {
    expect(validatePromoteRequest(makeRequest()).ok).toBe(true);
  });

  it("rejects deleted candidate", () => {
    const result = validatePromoteRequest(
      makeRequest({ candidate: makeCandidate({ status: "deleted" }) }),
    );

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.CANDIDATE_TERMINAL_STATUS);
    }
  });

  it("rejects merged candidate", () => {
    const result = validatePromoteRequest(
      makeRequest({ candidate: makeCandidate({ status: "merged" }) }),
    );

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.CANDIDATE_TERMINAL_STATUS);
    }
  });

  it("rejects empty canonical_name", () => {
    const result = validatePromoteRequest(
      makeRequest({ candidate: makeCandidate({ canonical_name: "   " }) }),
    );

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.EMPTY_CANONICAL_NAME);
    }
  });

  it("rejects merge target equal to candidate id", () => {
    const result = validatePromoteRequest(
      makeRequest({ target_keyblock_id: "kb_1" }),
    );

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.MERGE_TARGET_SELF);
    }
  });

  it("rejects non-provisional candidate", () => {
    const result = validatePromoteRequest(
      makeRequest({ candidate: makeCandidate({ status: "confirmed" }) }),
    );

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.CANDIDATE_NOT_PROVISIONAL);
    }
  });
});

describe("applyPromoteAcceptance", () => {
  it("promotes provisional candidate to confirmed", () => {
    const result = applyPromoteAcceptance(makeRequest());

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.status).toBe("confirmed");
      expect(result.value.canonical_name).toBe("Mira Vale");
    }
  });

  it("bumps revision from undefined to 1", () => {
    const result = applyPromoteAcceptance(
      makeRequest({ candidate: makeCandidate({ revision: undefined }) }),
    );

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.revision).toBe(1);
    }
  });

  it("bumps revision from 2 to 3", () => {
    const result = applyPromoteAcceptance(
      makeRequest({ candidate: makeCandidate({ revision: 2 }) }),
    );

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.revision).toBe(3);
    }
  });

  it("does not set updated_at", () => {
    const result = applyPromoteAcceptance(makeRequest());

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.updated_at).toBeUndefined();
    }
  });
});
