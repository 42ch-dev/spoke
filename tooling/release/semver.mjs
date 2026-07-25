#!/usr/bin/env node
/**
 * SemVer helpers for SPOKE lockstep release cuts.
 *
 * Supports X.Y.Z and X.Y.Z-<prerelease> (dot-separated identifiers).
 *
 * @module tooling/release/semver
 */

/** @type {RegExp} */
export const SEMVER_PATTERN =
  /^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.]+)?$/;

/**
 * @typedef {{ major: number; minor: number; patch: number; prerelease: (string | number)[] | null }} ParsedSemVer
 */

/**
 * @param {string} version
 * @returns {ParsedSemVer | null}
 */
export function parseSemVer(version) {
  if (typeof version !== "string" || !SEMVER_PATTERN.test(version)) {
    return null;
  }

  const hyphen = version.indexOf("-");
  const core = hyphen === -1 ? version : version.slice(0, hyphen);
  const pre = hyphen === -1 ? null : version.slice(hyphen + 1);
  const [major, minor, patch] = core.split(".").map((part) => Number(part));

  /** @type {(string | number)[] | null} */
  let prerelease = null;
  if (pre !== null) {
    prerelease = pre.split(".").map((id) => {
      if (/^[0-9]+$/.test(id)) {
        return Number(id);
      }
      return id;
    });
  }

  return { major, minor, patch, prerelease };
}

/**
 * Compare SemVer strings. Returns negative if a < b, 0 if equal, positive if a > b.
 * Invalid versions sort as NaN behavior via throw.
 *
 * @param {string} a
 * @param {string} b
 * @returns {number}
 */
export function compareSemVer(a, b) {
  const left = parseSemVer(a);
  const right = parseSemVer(b);
  if (!left || !right) {
    throw new Error(`Invalid SemVer compare: "${a}" vs "${b}"`);
  }

  if (left.major !== right.major) {
    return left.major - right.major;
  }
  if (left.minor !== right.minor) {
    return left.minor - right.minor;
  }
  if (left.patch !== right.patch) {
    return left.patch - right.patch;
  }

  // SemVer: release without prerelease has higher precedence than with prerelease
  if (left.prerelease === null && right.prerelease === null) {
    return 0;
  }
  if (left.prerelease === null) {
    return 1;
  }
  if (right.prerelease === null) {
    return -1;
  }

  const len = Math.max(left.prerelease.length, right.prerelease.length);
  for (let i = 0; i < len; i += 1) {
    const l = left.prerelease[i];
    const r = right.prerelease[i];
    if (l === undefined) {
      return -1;
    }
    if (r === undefined) {
      return 1;
    }
    if (l === r) {
      continue;
    }
    const lNum = typeof l === "number";
    const rNum = typeof r === "number";
    if (lNum && rNum) {
      return l - r;
    }
    if (lNum) {
      return -1;
    }
    if (rNum) {
      return 1;
    }
    return String(l) < String(r) ? -1 : 1;
  }

  return 0;
}

/**
 * @param {string} next
 * @param {string} current
 * @returns {boolean}
 */
export function isSemVerGreater(next, current) {
  return compareSemVer(next, current) > 0;
}
