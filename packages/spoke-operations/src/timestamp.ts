// why: strict RFC 3339 with mandatory offset — the one wire grammar
// (spoke-data-model §Shared wire scalars). Unlike `Date.parse`, it rejects
// date-only, slash-separated, RFC 2822, space-separated, naive, `UTC`-suffixed
// and whitespace-wrapped spellings.
const RFC3339_TIMESTAMP_RE =
  /^(\d{4})-(\d{2})-(\d{2})[Tt](\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?(?:[Zz]|([+-])(\d{2}):(\d{2}))$/;

/**
 * Strict RFC 3339 `date-time` gate with its mandatory offset (`Z` or `±hh:mm`).
 * Calendar and clock validity match the Rust parser (`DateTime::parse_from_rfc3339`
 * behind the shared ASCII grammar gate), including leap second `:60` and the
 * `±23:59` offset ceiling. The accept/reject contract is pinned by the shared
 * fixture `fixtures/timestamp-parity-cases.json`, asserted by both suites.
 * Exported for the cognition validators, which reuse the same Timestamp gate.
 */
export function isStrictTimestamp(value: string): boolean {
  // Grammar matches the raw wire string exactly as received — no trim;
  // whitespace-wrapped values refuse.
  const rfc3339 = RFC3339_TIMESTAMP_RE.exec(value);
  if (rfc3339 === null) {
    return false;
  }
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

/**
 * note: Calendar/clock validity on UTC parts. Seconds may be 60 (leap second), which
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
