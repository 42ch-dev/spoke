import type { MindState } from "@42ch/spoke-schemas";

import { validateCognitionProjection } from "../cognition/validate.js";
import { isParseableDateTime } from "../timestamp.js";

// note: Closed-envelope property table (mind-state.schema.json, additionalProperties: false).
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
  cognition: true,
  created_at: true,
  updated_at: true,
  extensions: true,
};

// note: Retained nine-field mental vocabulary (domain-profile-mental-state).
const MENTAL_FIELDS: Record<string, true> = {
  identity: true,
  beliefs: true,
  attention: true,
  goals: true,
  intentions: true,
  emotions: true,
  dispositions: true,
  norms: true,
  constraints: true,
};

/**
 * note: MindDelta path gate (common.schema.json MindDelta) — RFC 6901 JSON Pointer
 * with one of these exact roots: `/modules/mental/<field>...` for any of the
 * nine mental fields; `/modules/cognition/records/...` for derivative
 * standard cognition; `/modules/cognition/world_facts/...` for narrated-world
 * facts. No dot-path and no path targeting the MindState itself as authority.
 */
function isValidMindDeltaPath(value: unknown): boolean {
  if (typeof value !== "string" || value.length === 0) {
    return false;
  }
  const segments = value.split("/");
  if (segments[0] !== "" || /~(?![01])/.test(value)) {
    return false;
  }
  const [, root, section, ...rest] = segments;
  if (root !== "modules") {
    return false;
  }
  if (section === "mental") {
    return rest.length > 0 && MENTAL_FIELDS[rest[0]] === true;
  }
  // note: Schema `/(records|world_facts)/.+` counts the separating slash itself,
  // so only a literally empty post-root tail is rejected; an all-empty-token
  // tail such as `/records//` is valid (RFC 6901 permits empty tokens).
  return (
    section === "cognition" &&
    rest.length > 1 &&
    (rest.length > 2 || rest[1].length > 0) &&
    (rest[0] === "records" || rest[0] === "world_facts")
  );
}

function isNonEmptyTrimmedString(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Wire-shape gate for MindState (optional capability spoke-cognition).
 *
 * Checks required fields (`schema_version`, `mind_state_id`,
 * `holder_entry_id`, `extensions`), the closed envelope (no unknown
 * properties), `snapshot` / `deltas` types when present, RFC 6901 delta
 * paths rooted at `/modules/mental/<field>` or
 * `/modules/cognition/{records,world_facts}`, and the optional derivative
 * `cognition` CognitionProjection. Pure wire-shape validation only — mental
 * transition and ToM inference stay product-owned.
 */
export function validateMindState(state: unknown): state is MindState {
  if (!isPlainObject(state)) {
    return false;
  }

  for (const key of Object.keys(state)) {
    if (MIND_STATE_KEYS[key] !== true) {
      return false;
    }
  }

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
      if (typeof value !== "string" || !isParseableDateTime(value)) {
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
        return isValidMindDeltaPath(delta.path);
      })
    ) {
      return false;
    }
  }
  if (
    state.cognition !== undefined &&
    !validateCognitionProjection(state.cognition)
  ) {
    return false;
  }

  return true;
}
