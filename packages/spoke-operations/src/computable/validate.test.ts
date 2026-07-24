import type {
  ComputableLogEntry,
  ComputeRequest,
  ProjectRequest,
} from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import { SpokeRejectCode } from "../result.js";
import {
  validateComputableFieldMap,
  validateComputableLogEntry,
  validateComputeRequest,
  validateProjectRequest,
} from "./validate.js";

describe("validateComputableFieldMap", () => {
  it("accepts an empty plain object", () => {
    expect(validateComputableFieldMap({}).ok).toBe(true);
  });

  it("accepts a map with domain values", () => {
    expect(
      validateComputableFieldMap({ tide_level: 2.4, cargo_tons: 38 }).ok,
    ).toBe(true);
  });

  it("rejects null", () => {
    const result = validateComputableFieldMap(null);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });

  it("rejects arrays", () => {
    const result = validateComputableFieldMap([]);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });
});

describe("validateComputableLogEntry", () => {
  const validEntry: ComputableLogEntry = {
    logged_at: "2026-07-23T05:50:00Z",
    entry_id: "kb_tw_harbor",
    changes: [{ path: "tide_level", previous: 2.1, next: 2.4 }],
    session_id: "sess_tw_dawn_arrival",
    message: "Tide updated.",
  };

  it("accepts a valid log entry", () => {
    expect(validateComputableLogEntry(validEntry).ok).toBe(true);
  });

  it("rejects invalid logged_at", () => {
    const result = validateComputableLogEntry({
      ...validEntry,
      logged_at: "not-a-timestamp",
    });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });

  it("rejects empty entry_id", () => {
    const result = validateComputableLogEntry({
      ...validEntry,
      entry_id: "  ",
    });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.MISSING_REQUIRED_FIELD);
    }
  });

  it("rejects change without path", () => {
    const result = validateComputableLogEntry({
      ...validEntry,
      changes: [{ path: "" }],
    });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.MISSING_REQUIRED_FIELD);
    }
  });

  it("rejects null change without throwing", () => {
    const result = validateComputableLogEntry({
      ...validEntry,
      changes: [null as unknown as ComputableLogEntry["changes"][number]],
    });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });

  it("rejects undefined change without throwing", () => {
    const result = validateComputableLogEntry({
      ...validEntry,
      changes: [
        undefined as unknown as ComputableLogEntry["changes"][number],
      ],
    });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });
});

describe("validateProjectRequest", () => {
  const validRequest: ProjectRequest = {
    session_id: "sess_tw_dawn_arrival",
    entry_id: "kb_tw_harbor",
    state: { tide_level: 2.1, cargo_tons: 40 },
  };

  it("accepts a valid project request", () => {
    expect(validateProjectRequest(validRequest).ok).toBe(true);
  });

  it("rejects missing session_id", () => {
    const result = validateProjectRequest({
      ...validRequest,
      session_id: "",
    });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.MISSING_REQUIRED_FIELD);
    }
  });

  it("rejects invalid state", () => {
    const result = validateProjectRequest({
      ...validRequest,
      state: null as unknown as ProjectRequest["state"],
    });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });
});

describe("validateComputeRequest", () => {
  const validRequest: ComputeRequest = {
    session_id: "sess_tw_dawn_arrival",
    entry_id: "kb_tw_harbor",
    computable: { tide_level: 2.5, cargo_tons: 37 },
  };

  it("accepts a valid compute request", () => {
    expect(validateComputeRequest(validRequest).ok).toBe(true);
  });

  it("accepts settle true", () => {
    expect(
      validateComputeRequest({ ...validRequest, settle: true }).ok,
    ).toBe(true);
  });

  it("rejects non-boolean settle", () => {
    const result = validateComputeRequest({
      ...validRequest,
      settle: "true" as unknown as boolean,
    });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });

  it("rejects invalid computable map", () => {
    const result = validateComputeRequest({
      ...validRequest,
      computable: [] as unknown as ComputeRequest["computable"],
    });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.code).toBe(SpokeRejectCode.INVALID_INPUT);
    }
  });
});
