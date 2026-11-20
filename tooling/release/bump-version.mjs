#!/usr/bin/env node
/**
 * Bump lockstep SemVer across all SSOT surfaces, then run assert-lockstep-version.
 *
 * CLI: node tooling/release/bump-version.mjs <X.Y.Z>
 *
 * Normative: `.agents/specs/protocol/spoke-version-release.md`
 */

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  CANONICAL_PATH,
  CARGO_LOCK_PATH,
  CARGO_WORKSPACE_PATH,
  JSON_VERSION_PATHS,
  NUGET_CONNECT_CSPROJ_PATH,
  PYPI_CONNECT_PYPROJECT_PATH,
  MAVEN_CONNECT_GRADLE_PATH,
  parseCargoPathDependencyPins,
  parseCargoWorkspaceMembers,
  replaceCargoLockPackageVersions,
  replaceCargoPathDependencyPinVersions,
  replaceCsprojVersion,
  replaceGradleVersion,
  replacePyprojectVersion,
  resolveCargoLockPackageNames,
} from "./lockstep-surfaces.mjs";
import { runGitCliff, resolveGitCliffInvocation } from "./run-git-cliff.mjs";
import { SEMVER_PATTERN, isSemVerGreater } from "./semver.mjs";
import {
  checkBreakingSection,
  latestReleaseTag,
  sectionHasBreakingHeading,
} from "./assert-breaking-section.mjs";

const REPO_ROOT = process.env.SPOKE_REPO_ROOT
  ? join(process.env.SPOKE_REPO_ROOT)
  : join(dirname(fileURLToPath(import.meta.url)), "../..");
const CHANGELOG_PATH = "CHANGELOG.md";
const ASSERT_SCRIPT = join(
  dirname(fileURLToPath(import.meta.url)),
  "assert-lockstep-version.mjs",
);
const ASSERT_BREAKING_SCRIPT = join(
  dirname(fileURLToPath(import.meta.url)),
  "assert-breaking-section.mjs",
);

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
 * Read the Cargo workspace members and their package names from the workspace
 * manifests. The workspace manifest is the only member-set source of truth.
 *
 * @returns {{ paths: string[]; packageNames: string[] }}
 */
function readCargoWorkspaceMembers() {
  const workspaceContents = readRepoFile(CARGO_WORKSPACE_PATH);
  const paths = parseCargoWorkspaceMembers(workspaceContents);
  const packageNames = resolveCargoLockPackageNames(
    workspaceContents,
    (memberPath) => readRepoFile(join(memberPath, "Cargo.toml")),
  );
  for (const memberPath of paths) {
    const manifestPath = join(memberPath, "Cargo.toml");
    parseCargoPathDependencyPins(
      readRepoFile(manifestPath),
      packageNames,
      manifestPath,
    );
  }
  return { paths, packageNames };
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
 * @returns {{ targetVersion: string }}
 */
function parseArgs() {
  const argv = process.argv.slice(2).filter((arg) => arg !== "--");
  if (argv.length === 0 || argv[0] === "--help" || argv[0] === "-h") {
    console.log(`Usage: node tooling/release/bump-version.mjs <X.Y.Z>

Bump all lockstep version surfaces to X.Y.Z, run assert-lockstep-version, then exit.
X.Y.Z must be strictly greater than the current version.`);
    process.exit(argv.length === 0 ? 1 : 0);
  }

  const targetVersion = argv[0];
  if (!SEMVER_PATTERN.test(targetVersion)) {
    console.error(`Invalid SemVer: ${targetVersion}`);
    process.exit(1);
  }

  if (argv.length !== 1) {
    console.error(`Unexpected arguments: ${argv.slice(1).join(" ")}`);
    process.exit(1);
  }
  return { targetVersion };
}

/**
 * @returns {string | null} Tip commit that last modified CHANGELOG.md, or null.
 */
function lastChangelogCommit() {
  const result = spawnSync(
    "git",
    ["log", "-1", "--format=%H", "--", CHANGELOG_PATH],
    {
      cwd: REPO_ROOT,
      encoding: "utf8",
    },
  );
  if (result.status !== 0) {
    return null;
  }
  const sha = result.stdout.trim();
  return sha.length > 0 ? sha : null;
}

/**
 * Locate a Keep-a-Changelog version section (header + body).
 *
 * @param {string} changelog
 * @param {string} version
 * @returns {{ start: number; end: number; header: string; full: string } | null}
 */
function findChangelogSectionRange(changelog, version) {
  const normalized = version.replace(/^v/i, "");
  const headerRe = new RegExp(
    `^## \\[${normalized.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\](?:\\s+-\\s+\\d{4}-\\d{2}-\\d{2})?\\s*$`,
    "m",
  );
  const match = headerRe.exec(changelog);
  if (!match || match.index === undefined) {
    return null;
  }

  const header = match[0];
  const sectionStart = match.index;
  const afterHeader = match.index + header.length;
  const rest = changelog.slice(afterHeader);
  const nextHeader = rest.search(/^## \[/m);
  const end =
    nextHeader === -1 ? changelog.length : afterHeader + nextHeader;
  return {
    start: sectionStart,
    end,
    header,
    full: changelog.slice(sectionStart, end),
  };
}

/**
 * Move an existing version section to the top (newest-first). Prevents duplicate
 * headings when git-cliff would otherwise --prepend the same SemVer again.
 *
 * @param {string} version
 * @returns {boolean} true when a section existed and was ensured at top
 */
function promoteExistingChangelogSection(version) {
  if (!existsSync(repoPath(CHANGELOG_PATH))) {
    return false;
  }

  const changelog = readRepoFile(CHANGELOG_PATH);
  const found = findChangelogSectionRange(changelog, version);
  if (!found) {
    return false;
  }

  const firstHeading = changelog.search(/^## \[/m);
  if (firstHeading === found.start) {
    console.log(
      `${CHANGELOG_PATH} already has a top section for ${version}; skipping git-cliff.`,
    );
    return true;
  }

  const without =
    changelog.slice(0, found.start) + changelog.slice(found.end);
  const insertAt = without.search(/^## \[/m);
  const next = insertAt === -1 ? without : without.slice(0, insertAt);
  const rest = insertAt === -1 ? "" : without.slice(insertAt);
  const section = found.full.endsWith("\n") ? found.full : `${found.full}\n`;
  const promoted = `${next.replace(/\s*$/, "\n\n")}${section}\n${rest.replace(/^\s+/, "")}`;
  writeRepoFile(CHANGELOG_PATH, promoted);
  console.log(
    `Promoted existing ${CHANGELOG_PATH} section for ${version} to top; skipped git-cliff prepend.`,
  );
  return true;
}

/**
 * Rev range git-cliff includes for the next release, mirroring `updateChangelog`.
 *
 * @returns {string}
 */
function releaseCommitRange() {
  const tag = latestReleaseTag(REPO_ROOT);
  if (tag) {
    return `${tag}..HEAD`;
  }
  if (existsSync(repoPath(CHANGELOG_PATH))) {
    const since = lastChangelogCommit();
    if (since) {
      return `${since}..HEAD`;
    }
  }
  return "HEAD";
}

/**
 * Refuse before writing any surface when breaking commits would land in a
 * changelog section without `### Breaking`.
 *
 * @param {string} version
 */
function assertUpcomingBreakingSection(version) {
  const range = releaseCommitRange();
  let draft;
  try {
    draft = checkBreakingSection(version, {
      repoRoot: REPO_ROOT,
      range,
    });
  } catch (error) {
    console.error(`bump-version: ${error.message}`);
    process.exit(1);
  }

  if (draft.breaking.length === 0) {
    return;
  }

  const section =
    draft.section === null
      ? previewChangelogSection(version, range)
      : draft.section;
  if (section !== null && sectionHasBreakingHeading(section)) {
    return;
  }

  console.error(
    `bump-version: ${draft.breaking.length} breaking commit(s) would land in ${CHANGELOG_PATH} without "### Breaking"; nothing written:`,
  );
  for (const commit of draft.breaking) {
    console.error(`  - ${commit.subject}`);
  }
  console.error(
    "Fix cliff.toml's Breaking commit parser or the commit messages, then re-run.",
  );
  process.exit(1);
}

/**
 * Render the upcoming section with git-cliff without touching the worktree.
 *
 * @param {string} version
 * @param {string} range Commit range the generator will use; `HEAD` means full history.
 * @returns {string}
 */
function previewChangelogSection(version, range) {
  const { command, prefixArgs } = resolveGitCliffInvocation();
  const rangeArgs = range === "HEAD" ? ["--unreleased"] : [range];
  const dryRun = spawnSync(
    command,
    [...prefixArgs, ...rangeArgs, "--tag", `v${version}`, "-o", "-"],
    { cwd: REPO_ROOT, encoding: "utf8" },
  );
  if (dryRun.status !== 0) {
    console.error(
      `bump-version: git-cliff dry run failed: ${dryRun.stderr.trim()}`,
    );
    process.exit(dryRun.status ?? 1);
  }
  return dryRun.stdout;
}

/**
 * @param {string} version
 */
function updateChangelog(version) {
  if (promoteExistingChangelogSection(version)) {
    return;
  }

  const tag = `v${version}`;
  const changelogExists = existsSync(repoPath(CHANGELOG_PATH));
  // git-cliff requires -u/--unreleased or -l/--latest OR an explicit commit range
  // with --prepend / -o. Prefer:
  //   1) commits since latest v* tag (--unreleased)
  //   2) when no tags yet but CHANGELOG already exists: commits since last
  //      CHANGELOG.md update (avoids replaying history already under older sections)
  //   3) first CHANGELOG creation: full --unreleased history
  /** @type {string[]} */
  const cliffArgs = [];
  const releaseTag = latestReleaseTag(REPO_ROOT);
  if (releaseTag) {
    cliffArgs.push("--unreleased");
  } else if (changelogExists) {
    const since = lastChangelogCommit();
    if (since) {
      cliffArgs.push(`${since}..HEAD`);
    } else {
      cliffArgs.push("--unreleased");
    }
  } else {
    cliffArgs.push("--unreleased");
  }

  cliffArgs.push("--tag", tag);
  if (changelogExists) {
    cliffArgs.push("--prepend", CHANGELOG_PATH);
  } else {
    cliffArgs.push("-o", CHANGELOG_PATH);
  }

  const result = runGitCliff(cliffArgs, REPO_ROOT);
  if (result.status !== 0) {
    console.error(
      `bump-version: failed to update ${CHANGELOG_PATH} via git-cliff for ${tag}.`,
    );
    process.exit(result.status ?? 1);
  }

  // Guard: if git-cliff somehow left two headings for the same version, keep one.
  const after = readRepoFile(CHANGELOG_PATH);
  const headingRe = new RegExp(
    `^## \\[${version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\]`,
    "gm",
  );
  const matches = after.match(headingRe);
  if (matches && matches.length > 1) {
    console.error(
      `bump-version: ${CHANGELOG_PATH} has ${matches.length} sections for ${version} after git-cliff; refusing duplicate.`,
    );
    process.exit(1);
  }

  console.log(`Updated ${CHANGELOG_PATH} for ${tag}.`);
}

/**
 * @param {string} version
 */
function runBreakingAssert(version) {
  const result = spawnSync(process.execPath, [ASSERT_BREAKING_SCRIPT, version], {
    cwd: REPO_ROOT,
    env: { ...process.env, SPOKE_REPO_ROOT: REPO_ROOT },
    stdio: "inherit",
  });

  if (result.status !== 0) {
    console.error(
      `bump-version: ${version} has breaking commits but no "### Breaking" section in ${CHANGELOG_PATH}; fix cliff.toml or the commit messages and re-run.`,
    );
    process.exit(result.status ?? 1);
  }
}

/**
 * @param {string} version
 */
function runAssert(version) {
  const result = spawnSync(process.execPath, [ASSERT_SCRIPT], {
    cwd: REPO_ROOT,
    env: { ...process.env, SPOKE_REPO_ROOT: REPO_ROOT },
    stdio: "inherit",
  });

  if (result.status !== 0) {
    console.error(
      `bump-version: assert failed after writing ${version}; surfaces may be inconsistent.`,
    );
    process.exit(result.status ?? 1);
  }
}

const { targetVersion } = parseArgs();
const currentVersion = JSON.parse(readRepoFile(CANONICAL_PATH)).version;

if (typeof currentVersion !== "string" || currentVersion.length === 0) {
  console.error(`${CANONICAL_PATH}: missing or invalid "version" field`);
  process.exit(1);
}

if (!isSemVerGreater(targetVersion, currentVersion)) {
  console.error(
    `bump-version: target ${targetVersion} must be greater than current ${currentVersion}.`,
  );
  process.exit(1);
}
assertUpcomingBreakingSection(targetVersion);
const cargoWorkspaceMembers = readCargoWorkspaceMembers();

writeJsonVersion(CANONICAL_PATH, targetVersion);

for (const jsonPath of JSON_VERSION_PATHS) {
  writeJsonVersion(jsonPath, targetVersion);
}

const cargoContents = readRepoFile(CARGO_WORKSPACE_PATH);
writeRepoFile(
  CARGO_WORKSPACE_PATH,
  replaceWorkspacePackageVersion(cargoContents, targetVersion),
);

{
  const nugetContents = readRepoFile(NUGET_CONNECT_CSPROJ_PATH);
  writeRepoFile(
    NUGET_CONNECT_CSPROJ_PATH,
    replaceCsprojVersion(nugetContents, targetVersion, NUGET_CONNECT_CSPROJ_PATH),
  );
}

{
  const pyprojectContents = readRepoFile(PYPI_CONNECT_PYPROJECT_PATH);
  writeRepoFile(
    PYPI_CONNECT_PYPROJECT_PATH,
    replacePyprojectVersion(
      pyprojectContents,
      targetVersion,
      PYPI_CONNECT_PYPROJECT_PATH,
    ),
  );
}

{
  const gradleContents = readRepoFile(MAVEN_CONNECT_GRADLE_PATH);
  writeRepoFile(
    MAVEN_CONNECT_GRADLE_PATH,
    replaceGradleVersion(
      gradleContents,
      targetVersion,
      MAVEN_CONNECT_GRADLE_PATH,
    ),
  );
}
const {
  paths: cargoMemberPaths,
  packageNames: cargoLockPackageNames,
} = cargoWorkspaceMembers;
for (const memberPath of cargoMemberPaths) {
  const manifestPath = join(memberPath, "Cargo.toml");
  const manifestContents = readRepoFile(manifestPath);
  writeRepoFile(
    manifestPath,
    replaceCargoPathDependencyPinVersions(
      manifestContents,
      targetVersion,
      cargoLockPackageNames,
      manifestPath,
    ),
  );
}

const cargoLockContents = readRepoFile(CARGO_LOCK_PATH);
writeRepoFile(
  CARGO_LOCK_PATH,
  replaceCargoLockPackageVersions(
    cargoLockContents,
    targetVersion,
    cargoLockPackageNames,
  ),
);

updateChangelog(targetVersion);
runBreakingAssert(targetVersion);
runAssert(targetVersion);

console.log(`Bumped lockstep version ${currentVersion} → ${targetVersion}.`);
console.log("");

console.log("Next steps:");
console.log("  git add -A");
console.log(`  git commit -m "chore(release): bump version to ${targetVersion}"`);
console.log("  git push");
