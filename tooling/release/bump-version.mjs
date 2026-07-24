#!/usr/bin/env node
/**
 * Bump lockstep SemVer across all SSOT surfaces, then run assert-lockstep-version.
 *
 * CLI: node tooling/release/bump-version.mjs <X.Y.Z> [--tag [message]]
 *
 * Normative: `.mstar/specs/spoke-version-release.md`
 */

import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  CANONICAL_PATH,
  CARGO_WORKSPACE_PATH,
  JSON_VERSION_PATHS,
  README_BADGE_PATHS,
} from "./lockstep-surfaces.mjs";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const ASSERT_SCRIPT = join(
  dirname(fileURLToPath(import.meta.url)),
  "assert-lockstep-version.mjs",
);

const SEMVER_PATTERN = /^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.]+)?$/;

/**
 * @param {string} relativePath
 * @returns {string}
 */
function repoPath(relativePath) {
  return join(REPO_ROOT, relativePath);
}

/**
 * @param {string} relativePath
 * @returns {string}
 */
function readRepoFile(relativePath) {
  return readFileSync(repoPath(relativePath), "utf8");
}

/**
 * @param {string} relativePath
 * @param {string} contents
 */
function writeRepoFile(relativePath, contents) {
  writeFileSync(repoPath(relativePath), contents, "utf8");
}

/**
 * @param {string} relativePath
 * @param {string} version
 */
function writeJsonVersion(relativePath, version) {
  const data = JSON.parse(readRepoFile(relativePath));
  data.version = version;
  writeRepoFile(relativePath, `${JSON.stringify(data, null, 2)}\n`);
}

/**
 * @param {string} contents
 * @param {string} version
 * @returns {string}
 */
function replaceWorkspacePackageVersion(contents, version) {
  const sectionMatch = contents.match(
    /(\[workspace\.package\][\s\S]*?)(?=\n\[|\s*$)/,
  );
  if (!sectionMatch) {
    throw new Error(
      `${CARGO_WORKSPACE_PATH}: missing [workspace.package] section`,
    );
  }

  const updatedSection = sectionMatch[1].replace(
    /^version\s*=\s*"[^"]*"/m,
    `version = "${version}"`,
  );

  if (updatedSection === sectionMatch[1]) {
    throw new Error(
      `${CARGO_WORKSPACE_PATH}: could not find version = "..." in [workspace.package]`,
    );
  }

  return contents.replace(sectionMatch[1], updatedSection);
}

/**
 * @param {string} contents
 * @param {string} oldVersion
 * @param {string} newVersion
 * @returns {string}
 */
function replaceReadmeBadgeVersion(contents, oldVersion, newVersion) {
  const escaped = oldVersion.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const badgePattern = new RegExp(
    `(img\\.shields\\.io\\/badge\\/version-)${escaped}(-)`,
    "g",
  );

  if (!badgePattern.test(contents)) {
    throw new Error(
      `README badge for version ${oldVersion} not found (expected shields.io version-${oldVersion}- segment)`,
    );
  }

  return contents.replace(
    badgePattern,
    `$1${newVersion}$2`,
  );
}

/**
 * @returns {{ targetVersion: string; tag: boolean; tagMessage: string | null }}
 */
function parseArgs() {
  const argv = process.argv.slice(2).filter((arg) => arg !== "--");
  if (argv.length === 0 || argv[0] === "--help" || argv[0] === "-h") {
    console.log(`Usage: node tooling/release/bump-version.mjs <X.Y.Z> [--tag [message]]

Bump all lockstep version surfaces to X.Y.Z, run assert-lockstep-version, then exit.
With --tag, create a local annotated tag vX.Y.Z (no push).`);
    process.exit(argv.length === 0 ? 1 : 0);
  }

  const targetVersion = argv[0];
  if (!SEMVER_PATTERN.test(targetVersion)) {
    console.error(`Invalid SemVer: ${targetVersion}`);
    process.exit(1);
  }

  const tagIndex = argv.indexOf("--tag");
  if (tagIndex === -1) {
    return { targetVersion, tag: false, tagMessage: null };
  }

  const messageParts = argv.slice(tagIndex + 1);
  const tagMessage =
    messageParts.length > 0
      ? messageParts.join(" ")
      : `Release v${targetVersion}`;

  return { targetVersion, tag: true, tagMessage };
}

/**
 * @param {string} version
 */
function runAssert(version) {
  const result = spawnSync(process.execPath, [ASSERT_SCRIPT], {
    cwd: REPO_ROOT,
    stdio: "inherit",
  });

  if (result.status !== 0) {
    console.error(
      `bump-version: assert failed after writing ${version}; surfaces may be inconsistent.`,
    );
    process.exit(result.status ?? 1);
  }
}

/**
 * @param {string} version
 * @param {string} message
 */
function createAnnotatedTag(version, message) {
  const tagName = `v${version}`;
  const result = spawnSync("git", ["tag", "-a", tagName, "-m", message], {
    cwd: REPO_ROOT,
    stdio: "inherit",
  });

  if (result.status !== 0) {
    console.error(`bump-version: failed to create annotated tag ${tagName}.`);
    process.exit(result.status ?? 1);
  }

  console.log(`Created annotated tag ${tagName} (local only; not pushed).`);
}

const { targetVersion, tag, tagMessage } = parseArgs();
const currentVersion = JSON.parse(readRepoFile(CANONICAL_PATH)).version;

if (typeof currentVersion !== "string" || currentVersion.length === 0) {
  console.error(`${CANONICAL_PATH}: missing or invalid "version" field`);
  process.exit(1);
}

if (currentVersion === targetVersion) {
  console.log(
    `Version already ${targetVersion}; re-running assert only.`,
  );
  runAssert(targetVersion);
  if (tag) {
    createAnnotatedTag(targetVersion, tagMessage ?? `Release v${targetVersion}`);
  }
  console.log("");
  console.log("Next steps:");
  console.log("  git add -A");
  console.log(`  git commit -m "chore(release): bump version to ${targetVersion}"`);
  console.log("  git push");
  if (tag) {
    console.log(`  git push origin v${targetVersion}`);
  } else {
    console.log(`  git tag -a v${targetVersion} -m "Release v${targetVersion}"`);
    console.log(`  git push origin v${targetVersion}`);
  }
  process.exit(0);
}

writeJsonVersion(CANONICAL_PATH, targetVersion);

for (const jsonPath of JSON_VERSION_PATHS) {
  writeJsonVersion(jsonPath, targetVersion);
}

const cargoContents = readRepoFile(CARGO_WORKSPACE_PATH);
writeRepoFile(
  CARGO_WORKSPACE_PATH,
  replaceWorkspacePackageVersion(cargoContents, targetVersion),
);

for (const readmePath of README_BADGE_PATHS) {
  const readmeContents = readRepoFile(readmePath);
  writeRepoFile(
    readmePath,
    replaceReadmeBadgeVersion(readmeContents, currentVersion, targetVersion),
  );
}

runAssert(targetVersion);

console.log(`Bumped lockstep version ${currentVersion} → ${targetVersion}.`);
console.log("");
console.log("Next steps:");
console.log("  git add -A");
console.log(`  git commit -m "chore(release): bump version to ${targetVersion}"`);
console.log("  git push");
if (tag) {
  createAnnotatedTag(targetVersion, tagMessage ?? `Release v${targetVersion}`);
  console.log(`  git push origin v${targetVersion}`);
} else {
  console.log(`  git tag -a v${targetVersion} -m "Release v${targetVersion}"`);
  console.log(`  git push origin v${targetVersion}`);
}
