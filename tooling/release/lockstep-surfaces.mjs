/**
 * Lockstep version surfaces — SSOT manifest for assert and bump scripts.
 *
 * Normative source: `.mstar/specs/spoke-version-release.md` rows 1–11.
 *
 * Excluded from lockstep (documented only; not asserted):
 * - tooling/codegen/rust-gen/Cargo.toml — standalone codegen bin crate; not a consumer pin surface.
 * - pnpm-lock.yaml — workspace `link:` entries do not embed package SemVer.
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

/** @type {string} Rust operations crate manifest (row 8). */
export const CARGO_OPS_CRATE_PATH = "crates/spoke-operations/Cargo.toml";

/** @type {string} Cargo lockfile — workspace member package versions (row 11). */
export const CARGO_LOCK_PATH = "Cargo.lock";

/**
 * Workspace member crate names whose `[[package]]` version in Cargo.lock must
 * match the lockstep SemVer.
 * @type {readonly string[]}
 */
export const CARGO_LOCK_PACKAGE_NAMES = ["spoke-schemas", "spoke-operations"];

/**
 * @param {string} version
 * @returns {string}
 */
export function formatOpsSpokeSchemasDependency(version) {
  return `spoke-schemas = { version = "${version}", path = "../spoke-schemas" }`;
}

/**
 * @param {string} contents
 * @returns {string | null}
 */
export function parseOpsSpokeSchemasDependencyVersion(contents) {
  const match = contents.match(
    /^spoke-schemas\s*=\s*\{[^}]*version\s*=\s*"([^"]+)"/m,
  );
  return match?.[1] ?? null;
}

/**
 * @param {string} contents
 * @param {string} version
 * @returns {string}
 */
export function replaceOpsSpokeSchemasDependencyVersion(contents, version) {
  const updated = contents.replace(
    /^spoke-schemas\s*=\s*\{[^}]*\}/m,
    formatOpsSpokeSchemasDependency(version),
  );
  if (updated === contents) {
    throw new Error(
      `${CARGO_OPS_CRATE_PATH}: could not update spoke-schemas path dependency`,
    );
  }
  return updated;
}

/**
 * Read the version for a top-level `[[package]]` entry in Cargo.lock.
 *
 * @param {string} contents
 * @param {string} packageName
 * @returns {string | null}
 */
export function parseCargoLockPackageVersion(contents, packageName) {
  const escaped = packageName.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = contents.match(
    new RegExp(
      `\\[\\[package\\]\\]\\s*\\nname\\s*=\\s*"${escaped}"\\s*\\nversion\\s*=\\s*"([^"]+)"`,
    ),
  );
  return match?.[1] ?? null;
}

/**
 * Rewrite workspace member package versions in Cargo.lock to match lockstep.
 * Does not invoke `cargo` (New release CI is Node-only). Idempotent when the
 * lockfile already lists the target version.
 *
 * @param {string} contents
 * @param {string} version
 * @param {readonly string[]} [packageNames]
 * @returns {string}
 */
export function replaceCargoLockPackageVersions(
  contents,
  version,
  packageNames = CARGO_LOCK_PACKAGE_NAMES,
) {
  let updated = contents;
  for (const packageName of packageNames) {
    const current = parseCargoLockPackageVersion(updated, packageName);
    if (current === null) {
      throw new Error(
        `${CARGO_LOCK_PATH}: missing [[package]] entry for ${packageName}`,
      );
    }
    if (current === version) {
      continue;
    }
    const escaped = packageName.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const pattern = new RegExp(
      `(\\[\\[package\\]\\]\\s*\\nname\\s*=\\s*"${escaped}"\\s*\\nversion\\s*=\\s*")[^"]+(")`,
    );
    const next = updated.replace(pattern, `$1${version}$2`);
    if (next === updated) {
      throw new Error(
        `${CARGO_LOCK_PATH}: could not update [[package]] version for ${packageName}`,
      );
    }
    updated = next;
  }
  return updated;
}

/**
 * README files that must carry the dynamic GitHub Releases version badge.
 * @type {readonly string[]}
 */
export const README_BADGE_PATHS = ["README.md", "README_CN.md"];

/**
 * Dynamic shields.io GitHub Releases badge (includes prereleases, SemVer sort).
 * Not rewritten on bump — tracks the latest GitHub Release for the repo.
 */
export const README_RELEASE_BADGE_MARKER =
  "https://img.shields.io/github/v/release/42ch-dev/spoke";

/**
 * @param {string} contents
 * @returns {boolean}
 */
export function hasReadmeReleaseBadge(contents) {
  return contents.includes(README_RELEASE_BADGE_MARKER);
}
