#!/usr/bin/env node
/**
 * Assert all lockstep version surfaces match the canonical root package.json version.
 *
 * Local drift test:
 *   1. Temporarily change one manifest version (e.g. packages/spoke-schemas/package.json).
 *   2. Run `pnpm run verify:version` — expect non-zero exit and expected vs actual output.
 *   3. Revert the change; assert must pass again.
 *
 * Normative: `.mstar/specs/spoke-version-release.md`
 */

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  CANONICAL_PATH,
  CARGO_SCHEMA_CRATE_PATH,
  CARGO_WORKSPACE_PATH,
  JSON_VERSION_PATHS,
  README_BADGE_PATHS,
  README_BADGE_REGEX,
} from "./lockstep-surfaces.mjs";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");

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
 * @param {string} expected
 * @param {string} actual
 */
function assertEqual(relativePath, expected, actual) {
  if (expected !== actual) {
    recordFailure(relativePath, expected, actual);
  }
}

const canonicalVersion = readJsonVersion(CANONICAL_PATH);

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

const schemaCrateContents = readRepoFile(CARGO_SCHEMA_CRATE_PATH);
if (!hasWorkspaceVersionDeclaration(schemaCrateContents)) {
  recordFailure(
    CARGO_SCHEMA_CRATE_PATH,
    "version.workspace = true",
    "(not declared)",
    "crates/spoke-schemas/Cargo.toml must declare version.workspace = true",
  );
} else if (cargoWorkspaceVersion !== null) {
  assertEqual(
    `${CARGO_SCHEMA_CRATE_PATH} (effective via workspace)`,
    canonicalVersion,
    cargoWorkspaceVersion,
  );
}

for (const readmePath of README_BADGE_PATHS) {
  const contents = readRepoFile(readmePath);
  const match = contents.match(README_BADGE_REGEX);
  if (!match) {
    recordFailure(
      readmePath,
      canonicalVersion,
      "(badge not found)",
      `No shields.io version badge matching ${README_BADGE_REGEX}`,
    );
    continue;
  }

  assertEqual(readmePath, canonicalVersion, match[1]);
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
