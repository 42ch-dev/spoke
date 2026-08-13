import type { MindState } from "@42ch/spoke-schemas";

// Closed-envelope property table (mind-state.schema.json, additionalProperties: false).
const MIND_STATE_KEYS: Record<string, true> = {
  schema_version: true,
  mind_state_id: true,
  holder_entry_id: true,
  canonical_name: true,
  occurred_at: true,
  sort_key: true,
  snapshot: true,
  deltas: true,
  source_anchor: true,
  created_at: true,
  updated_at: true,
  extensions: true,
};

function isNonEmptyTrimmedString(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Wire-shape gate for MindState (optional capability l5-mind).
 *
 * Checks required fields, the closed envelope (no unknown properties), and
 * snapshot / deltas types. Pure wire-shape validation only — mental
 * transition and ToM inference stay product-owned.
 */
export function validateMindState(state: unknown): state is MindState {
  if (!isPlainObject(state)) {
    return false;
  }

  // Closed envelope: reject unknown properties.
  for (const key of Object.keys(state)) {
    if (MIND_STATE_KEYS[key] !== true) {
      return false;
    }
  }

  // Required fields.
  if (
    typeof state.schema_version !== "number" ||
    !Number.isInteger(state.schema_version) ||
    state.schema_version < 1
  ) {
    return false;
  }
  if (!isNonEmptyTrimmedString(state.mind_state_id)) {
    return false;
  }
  if (!isNonEmptyTrimmedString(state.holder_entry_id)) {
    return false;
  }
  if (!isPlainObject(state.extensions)) {
    return false;
  }

  // Optional typed fields when present.
  if (
    state.canonical_name !== undefined &&
    !isNonEmptyTrimmedString(state.canonical_name)
  ) {
    return false;
  }
  if (state.sort_key !== undefined && typeof state.sort_key !== "string") {
    return false;
  }
  for (const field of ["occurred_at", "created_at", "updated_at"] as const) {
    const value = state[field];
    if (value !== undefined) {
      if (typeof value !== "string" || Number.isNaN(Date.parse(value))) {
        return false;
      }
    }
  }
  if (state.snapshot !== undefined && !isPlainObject(state.snapshot)) {
    return false;
  }
  if (state.source_anchor !== undefined && !isPlainObject(state.source_anchor)) {
    return false;
  }
  if (state.deltas !== undefined) {
    if (
      !Array.isArray(state.deltas) ||
      !state.deltas.every((delta) => {
        if (!isPlainObject(delta)) {
          return false;
        }
        return isNonEmptyTrimmedString(delta.path);
      })
    ) {
      return false;
    }
  }

  return true;
}
