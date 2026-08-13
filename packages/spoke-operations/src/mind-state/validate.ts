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

// Strict RFC 3339 with mandatory offset — mirrors the first two arms of the
// Rust `is_parseable_date_time` chain (`DateTime::parse_from_rfc3339` /
// `DateTime<Utc>` FromStr). Unlike `Date.parse`, it rejects date-only,
// slash-separated, RFC 2822, and space-separated strings.
const RFC3339_TIMESTAMP_RE =
  /^(\d{4})-(\d{2})-(\d{2})[Tt](\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?(?:[Zz]|([+-])(\d{2}):(\d{2}))$/;
// NaiveDateTime fallback: "YYYY-MM-DDTHH:MM:SS[.fraction]" without timezone.
const NAIVE_TIMESTAMP_RE =
  /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?$/;

/**
 * Timestamp gate mirroring Rust `is_parseable_date_time`
 * (crates/spoke-operations/src/mind_state.rs): RFC 3339 first, then the
 * NaiveDateTime fallback. The accept/reject contract is pinned by the shared
 * fixture `fixtures/timestamp-parity-cases.json`, asserted by both suites.
 */
function isParseableDateTime(value: string): boolean {
  const trimmed = value.trim();
  if (trimmed.length === 0) {
    return false;
  }
  const rfc3339 = RFC3339_TIMESTAMP_RE.exec(trimmed);
  if (rfc3339 !== null) {
    // Groups: 1 year, 2 month, 3 day, 4 hour, 5 minute, 6 second,
    // 7 fraction, 8 offset sign, 9 offset hour, 10 offset minute.
    // A `Z`/`z` offset (sign group undefined) is always in range.
    return (
      isCalendarDateTimeValid(
        rfc3339[1],
        rfc3339[2],
        rfc3339[3],
        rfc3339[4],
        rfc3339[5],
        rfc3339[6],
      ) &&
      (rfc3339[8] === undefined ||
        (Number(rfc3339[9]) <= 23 && Number(rfc3339[10]) <= 59))
    );
  }
  const naive = NAIVE_TIMESTAMP_RE.exec(trimmed);
  if (naive !== null) {
    return isCalendarDateTimeValid(
      naive[1],
      naive[2],
      naive[3],
      naive[4],
      naive[5],
      naive[6],
    );
  }
  return false;
}

/**
 * Calendar/clock validity on UTC parts. Seconds may be 60 (leap second), which
 * chrono accepts; JS Date cannot represent it, so it is checked separately.
 * Round-trip through Date catches rollovers (Feb 30, month 13, hour 24, ...).
 */
function isCalendarDateTimeValid(
  year: string,
  month: string,
  day: string,
  hour: string,
  minute: string,
  second: string,
): boolean {
  const y = Number(year);
  const mo = Number(month);
  const d = Number(day);
  const h = Number(hour);
  const mi = Number(minute);
  const s = Number(second);
  if (mo < 1 || mo > 12 || d < 1 || d > 31) {
    return false;
  }
  if (h > 23 || mi > 59 || s > 60) {
    return false;
  }

  const dt = new Date(0);
  dt.setUTCHours(h, mi, s === 60 ? 59 : s, 0);
  dt.setUTCFullYear(y, mo - 1, d);
  return (
    dt.getUTCFullYear() === y &&
    dt.getUTCMonth() === mo - 1 &&
    dt.getUTCDate() === d &&
    dt.getUTCHours() === h &&
    dt.getUTCMinutes() === mi &&
    (s === 60 || dt.getUTCSeconds() === s)
  );
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
        return isNonEmptyTrimmedString(delta.path);
      })
    ) {
      return false;
    }
  }

  return true;
}
