#!/usr/bin/env node
/**
 * Assert all lockstep version surfaces match the canonical root package.json version.
 *
 * Local drift test:
 *   1. Temporarily change one manifest version (e.g. packages/spoke-schemas/package.json).
 *   2. Run `pnpm run verify:version` — expect non-zero exit and expected vs actual output.
 *   3. Revert the change; assert must pass again.
 *
 * Release tag test (optional SPOKE_RELEASE_TAG):
 *   Annotated-tag requirement is enforced in `.github/workflows/release.yml` verify-version job
 *   (lightweight tags fail before this script runs).
 *   SPOKE_RELEASE_TAG=v0.1.0 node tooling/release/assert-lockstep-version.mjs  # pass
 *   SPOKE_RELEASE_TAG=v0.1.0-rc.1 node tooling/release/assert-lockstep-version.mjs  # pass when manifest is 0.1.0
 *   SPOKE_RELEASE_TAG=0.1.0 node tooling/release/assert-lockstep-version.mjs  # fail (missing v)
 *   SPOKE_RELEASE_TAG=v9.9.9 node tooling/release/assert-lockstep-version.mjs  # fail (mismatch)
 *
 * Normative: `.mstar/specs/spoke-version-release.md`
 */

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  CANONICAL_PATH,
  CARGO_LOCK_PACKAGE_NAMES,
  CARGO_LOCK_PATH,
  CARGO_OPS_CRATE_PATH,
  CARGO_SCHEMA_CRATE_PATH,
  CARGO_WORKSPACE_PATH,
  JSON_VERSION_PATHS,
  README_BADGE_PATHS,
  README_RELEASE_BADGE_MARKER,
  hasReadmeReleaseBadge,
  parseCargoLockPackageVersion,
  parseOpsSpokeSchemasDependencyVersion,
} from "./lockstep-surfaces.mjs";

const REPO_ROOT = process.env.SPOKE_REPO_ROOT
  ? join(process.env.SPOKE_REPO_ROOT)
  : join(dirname(fileURLToPath(import.meta.url)), "../..");

const RELEASE_TAG_SEMVER_PATTERN = /^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.]+)?$/;
const RELEASE_TAG_RC_PATTERN = /^([0-9]+\.[0-9]+\.[0-9]+)-rc\.[0-9]+$/;

/**
 * @param {string} tagVersion SemVer segment after leading "v" (e.g. "0.1.0-rc.1").
 * @returns {string} Base X.Y.Z for RC tags; unchanged for stable tags.
 */
function releaseTagComparableVersion(tagVersion) {
  const rcMatch = tagVersion.match(RELEASE_TAG_RC_PATTERN);
  return rcMatch?.[1] ?? tagVersion;
}

/** @type {{ path: string; expected: string; actual: string; detail?: string }[]} */
const failures = [];

/**
 * @param {string} relativePath
 * @returns {string}
 */
function readRepoFile(relativePath) {
  return readFileSync(join(REPO_ROOT, relativePath), "utf8");
}

/**
 * @param {string} relativePath
 * @returns {string}
 */
function readJsonVersion(relativePath) {
  const data = JSON.parse(readRepoFile(relativePath));
  if (typeof data.version !== "string" || data.version.length === 0) {
    throw new Error(`${relativePath}: missing or invalid "version" field`);
  }
  return data.version;
}

/**
 * @param {string} contents
 * @returns {string | null}
 */
function parseWorkspacePackageVersion(contents) {
  const sectionMatch = contents.match(
    /\[workspace\.package\][\s\S]*?(?=\n\[|\s*$)/,
  );
  if (!sectionMatch) {
    return null;
  }

  const versionMatch = sectionMatch[0].match(
    /^version\s*=\s*"([^"]+)"/m,
  );
  return versionMatch?.[1] ?? null;
}

/**
 * @param {string} contents
 * @returns {boolean}
 */
function hasWorkspaceVersionDeclaration(contents) {
  return /^version\.workspace\s*=\s*true\s*$/m.test(contents);
}

/**
 * @param {string} relativePath
 * @param {string} expected
 * @param {string} actual
 * @param {string} [detail]
 */
function recordFailure(relativePath, expected, actual, detail) {
  failures.push({ path: relativePath, expected, actual, detail });
}

/**
 * @param {string} relativePath
 * @param {string} label
 * @param {string} expectedVersion
 * @param {string | null} workspaceVersion
 */
function assertWorkspaceCrateVersion(
  relativePath,
  label,
  expectedVersion,
  workspaceVersion,
) {
  const crateContents = readRepoFile(relativePath);
  if (!hasWorkspaceVersionDeclaration(crateContents)) {
    recordFailure(
      relativePath,
      "version.workspace = true",
      "(not declared)",
      `${label} must declare version.workspace = true`,
    );
  } else if (workspaceVersion !== null) {
    assertEqual(
      `${relativePath} (effective via workspace)`,
      expectedVersion,
      workspaceVersion,
    );
  }
}

/**
 * @param {string} relativePath
 * @param {string} expected
 * @param {string} actual
 */
function assertEqual(relativePath, expected, actual) {
  if (expected !== actual) {
    recordFailure(relativePath, expected, actual);
  }
}

const canonicalVersion = readJsonVersion(CANONICAL_PATH);

const releaseTag = process.env.SPOKE_RELEASE_TAG?.trim();
if (releaseTag) {
  if (!releaseTag.startsWith("v")) {
    recordFailure(
      "git tag (SPOKE_RELEASE_TAG)",
      `v${canonicalVersion}`,
      releaseTag,
      'Release tag MUST start with "v" (e.g. v0.2.0 or v0.2.0-rc.1).',
    );
  } else {
    const tagVersion = releaseTag.slice(1);
    if (!RELEASE_TAG_SEMVER_PATTERN.test(tagVersion)) {
      recordFailure(
        "git tag (SPOKE_RELEASE_TAG)",
        canonicalVersion,
        tagVersion,
        `Tag version segment must match SemVer (got "${tagVersion}").`,
      );
    } else {
      const comparableTagVersion = releaseTagComparableVersion(tagVersion);
      assertEqual(
        "git tag (SPOKE_RELEASE_TAG)",
        canonicalVersion,
        comparableTagVersion,
      );
    }
  }
}

for (const jsonPath of JSON_VERSION_PATHS) {
  const actual = readJsonVersion(jsonPath);
  assertEqual(jsonPath, canonicalVersion, actual);
}

const cargoWorkspaceContents = readRepoFile(CARGO_WORKSPACE_PATH);
const cargoWorkspaceVersion = parseWorkspacePackageVersion(cargoWorkspaceContents);
if (cargoWorkspaceVersion === null) {
  recordFailure(
    CARGO_WORKSPACE_PATH,
    canonicalVersion,
    "(missing [workspace.package].version)",
    "Could not parse workspace package version from Cargo.toml",
  );
} else {
  assertEqual(CARGO_WORKSPACE_PATH, canonicalVersion, cargoWorkspaceVersion);
}

assertWorkspaceCrateVersion(
  CARGO_SCHEMA_CRATE_PATH,
  "crates/spoke-schemas/Cargo.toml",
  canonicalVersion,
  cargoWorkspaceVersion,
);
assertWorkspaceCrateVersion(
  CARGO_OPS_CRATE_PATH,
  "crates/spoke-operations/Cargo.toml",
  canonicalVersion,
  cargoWorkspaceVersion,
);

const opsCrateContents = readRepoFile(CARGO_OPS_CRATE_PATH);
const opsSchemasDepVersion =
  parseOpsSpokeSchemasDependencyVersion(opsCrateContents);
if (opsSchemasDepVersion === null) {
  recordFailure(
    `${CARGO_OPS_CRATE_PATH} (spoke-schemas dependency)`,
    `version = "${canonicalVersion}" with path`,
    "(missing version in path dependency)",
    "spoke-operations must declare spoke-schemas with version + path for cargo publish",
  );
} else {
  assertEqual(
    `${CARGO_OPS_CRATE_PATH} (spoke-schemas dependency)`,
    canonicalVersion,
    opsSchemasDepVersion,
  );
}

const cargoLockContents = readRepoFile(CARGO_LOCK_PATH);
for (const packageName of CARGO_LOCK_PACKAGE_NAMES) {
  const lockVersion = parseCargoLockPackageVersion(
    cargoLockContents,
    packageName,
  );
  if (lockVersion === null) {
    recordFailure(
      `${CARGO_LOCK_PATH} (${packageName})`,
      canonicalVersion,
      "(missing [[package]] entry)",
      `Cargo.lock must list workspace member ${packageName}`,
    );
  } else {
    assertEqual(
      `${CARGO_LOCK_PATH} (${packageName})`,
      canonicalVersion,
      lockVersion,
    );
  }
}

for (const readmePath of README_BADGE_PATHS) {
  const contents = readRepoFile(readmePath);
  if (!hasReadmeReleaseBadge(contents)) {
    recordFailure(
      readmePath,
      README_RELEASE_BADGE_MARKER,
      "(badge not found)",
      `Expected dynamic GitHub Releases shields badge containing ${README_RELEASE_BADGE_MARKER}`,
    );
  }
}

if (failures.length > 0) {
  console.error(
    `Lockstep version mismatch: canonical ${CANONICAL_PATH} version is ${canonicalVersion}.`,
  );
  console.error("");
  for (const failure of failures) {
    console.error(`  ${failure.path}`);
    console.error(`    expected: ${failure.expected}`);
    console.error(`    actual:   ${failure.actual}`);
    if (failure.detail) {
      console.error(`    detail:   ${failure.detail}`);
    }
  }
  console.error("");
  console.error(
    "Sync all surfaces listed in tooling/release/lockstep-surfaces.mjs or run tooling/release/bump-version.mjs.",
  );
  process.exit(1);
}

console.log(
  `Lockstep version OK: all surfaces match ${canonicalVersion} (${CANONICAL_PATH}).`,
);
