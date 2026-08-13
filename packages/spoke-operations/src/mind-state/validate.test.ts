import type { MindState } from "@42ch/spoke-schemas";
import { describe, expect, it } from "vitest";

import { validateMindState } from "./validate.js";

const validState: MindState = {
  schema_version: 1,
  mind_state_id: "mind_tw_bo_pre_transfer",
  holder_entry_id: "kb_tw_bo",
  canonical_name: "Bo before the hidden transfer",
  occurred_at: "2026-07-23T09:00:00Z",
  sort_key: "fb-001",
  snapshot: {
    beliefs: { ref: "kb_tw_bo", count: 2 },
    attention: { target: "kb_tw_room", modality: "visual" },
    emotions: [{ emotion: "calm", intensity: 0.4 }],
  },
  deltas: [],
  created_at: "2026-07-23T09:00:00Z",
  updated_at: "2026-07-23T09:00:00Z",
  extensions: {
    toy: {
      story: "false-belief-box-basket",
      phase: "pre-transfer",
    },
  },
};

describe("validateMindState", () => {
  it("accepts a valid MindState record", () => {
    expect(validateMindState(validState)).toBe(true);
  });

  it("accepts a minimal required-only record", () => {
    expect(
      validateMindState({
        schema_version: 1,
        mind_state_id: "mind_tw_min",
        holder_entry_id: "kb_tw_bo",
        extensions: {},
      }),
    ).toBe(true);
  });

  it("rejects missing holder_entry_id", () => {
    const { holder_entry_id: _removed, ...rest } = validState;

    expect(validateMindState(rest)).toBe(false);
  });

  it("rejects missing mind_state_id", () => {
    const { mind_state_id: _removed, ...rest } = validState;

    expect(validateMindState(rest)).toBe(false);
  });

  it("rejects empty holder_entry_id", () => {
    expect(validateMindState({ ...validState, holder_entry_id: "  " })).toBe(
      false,
    );
  });

  it("rejects extra properties (closed envelope)", () => {
    expect(validateMindState({ ...validState, mind_engine: {} })).toBe(false);
    expect(validateMindState({ ...validState, constructor: {} })).toBe(false);
  });

  it("rejects a MindDelta without path", () => {
    expect(
      validateMindState({
        ...validState,
        deltas: [{ previous: 1, next: 2 }],
      }),
    ).toBe(false);
  });

  it("accepts a MindDelta with path only", () => {
    expect(
      validateMindState({ ...validState, deltas: [{ path: "beliefs" }] }),
    ).toBe(true);
  });

  it("rejects a non-array deltas value", () => {
    expect(
      validateMindState({ ...validState, deltas: { path: "beliefs" } }),
    ).toBe(false);
  });

  it("rejects a non-object snapshot", () => {
    expect(validateMindState({ ...validState, snapshot: [] })).toBe(false);
  });

  it("rejects a non-object extensions value", () => {
    expect(validateMindState({ ...validState, extensions: [] })).toBe(false);
  });

  it("rejects an invalid occurred_at", () => {
    expect(
      validateMindState({ ...validState, occurred_at: "not-a-timestamp" }),
    ).toBe(false);
  });

  it("rejects a non-integer schema_version", () => {
    expect(validateMindState({ ...validState, schema_version: 0 })).toBe(false);
  });

  it("rejects null and non-object state", () => {
    expect(validateMindState(null)).toBe(false);
    expect(validateMindState([])).toBe(false);
    expect(validateMindState("mind")).toBe(false);
  });
});
