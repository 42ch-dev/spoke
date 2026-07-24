/**
 * Lockstep version surfaces — SSOT manifest for assert and bump scripts.
 *
 * Normative source: `.mstar/specs/spoke-version-release.md` rows 1–9.
 *
 * Excluded from lockstep (documented only; not asserted):
 * - tooling/codegen/rust-gen/Cargo.toml — standalone codegen bin crate; not a consumer pin surface.
 */

/** @type {string} Canonical version source (row 1). */
export const CANONICAL_PATH = "package.json";

/**
 * package.json files whose top-level `version` must match canonical (rows 2–5).
 * @type {readonly string[]}
 */
export const JSON_VERSION_PATHS = [
  "packages/spoke-schemas/package.json",
  "packages/spoke-operations/package.json",
  "fixtures/toy-world/package.json",
  "tooling/codegen/package.json",
];

/** @type {string} Cargo workspace version (row 6). */
export const CARGO_WORKSPACE_PATH = "Cargo.toml";

/** @type {string} Rust schema crate manifest (row 7). */
export const CARGO_SCHEMA_CRATE_PATH = "crates/spoke-schemas/Cargo.toml";

/**
 * README files with shields.io version badge (rows 8–9).
 * @type {readonly string[]}
 */
export const README_BADGE_PATHS = ["README.md", "README_CN.md"];

/** Shields.io version badge URL prefix (hostname avoided in regex for CodeQL). */
export const README_BADGE_PREFIX = "https://img.shields.io/badge/version-";

/**
 * Anchored SemVer matcher for the segment immediately after {@link README_BADGE_PREFIX}.
 * Captures the version before the trailing `-` (e.g. `0.1.0-` in `version-0.1.0-informational`).
 */
export const README_BADGE_VERSION_REGEX =
  /^([0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.]+)?)-/;

/**
 * @param {string} contents
 * @returns {string | null} Parsed SemVer from the version badge, or null when absent/invalid.
 */
export function parseReadmeBadgeVersion(contents) {
  const prefixIndex = contents.indexOf(README_BADGE_PREFIX);
  if (prefixIndex === -1) {
    return null;
  }

  const remainder = contents.slice(prefixIndex + README_BADGE_PREFIX.length);
  const match = remainder.match(README_BADGE_VERSION_REGEX);
  return match?.[1] ?? null;
}

/**
 * @param {string} contents
 * @param {string} oldVersion
 * @param {string} newVersion
 * @returns {string}
 */
export function replaceReadmeBadgeVersion(contents, oldVersion, newVersion) {
  const prefixIndex = contents.indexOf(README_BADGE_PREFIX);
  if (prefixIndex === -1) {
    throw new Error(
      `README badge prefix not found (expected ${README_BADGE_PREFIX}<version>- URL)`,
    );
  }

  const versionStart = prefixIndex + README_BADGE_PREFIX.length;
  const remainder = contents.slice(versionStart);
  const match = remainder.match(README_BADGE_VERSION_REGEX);
  if (!match) {
    throw new Error(
      `README badge version segment invalid after ${README_BADGE_PREFIX}`,
    );
  }

  if (match[1] !== oldVersion) {
    throw new Error(
      `README badge for version ${oldVersion} not found (found ${match[1]})`,
    );
  }

  return (
    contents.slice(0, versionStart) +
    newVersion +
    contents.slice(versionStart + match[1].length)
  );
}
