#!/usr/bin/env node
/**
 * Fail when a release changelog section hides breaking changes (no `### Breaking`).
 *
 * Breaking commits are Conventional Commits with `!` after type/scope, or a
 * `BREAKING CHANGE:` footer. git-cliff groups them under `### Breaking`; this
 * guard catches config/hand-edit regressions before a version is published.
 *
 * CLI: node tooling/release/assert-breaking-section.mjs <vX.Y.Z|X.Y.Z> [--from <ref>]
 *
 * Exits 0 when there are no breaking commits or the section has the heading;
 * exits 1 when breaking commits lack the heading or the section is absent.
 *
 * @module tooling/release/assert-breaking-section
 */

import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { extractChangelogSection } from "./extract-changelog-notes.mjs";

const REPO_ROOT = process.env.SPOKE_REPO_ROOT
  ? join(process.env.SPOKE_REPO_ROOT)
  : join(dirname(fileURLToPath(import.meta.url)), "../..");
const CHANGELOG_PATH = join(REPO_ROOT, "CHANGELOG.md");

const BREAKING_HEADER_PATTERN = /^[a-z]+(?:\([^)]*\))?!:/;
const BREAKING_FOOTER_PATTERN = /^BREAKING CHANGE:/m;
const BREAKING_HEADING_PATTERN = /^### Breaking\s*$/m;

// Subjects/messages cliff.toml drops before grouping; a `!` there never reaches the changelog.
const CLIFF_SKIP_MESSAGE_PATTERNS = [
  /v0-iter/i,
  /iteration\/v0-iter/i,
  /merge\(plan\)/i,
];
const CLIFF_SKIP_SUBJECT_PATTERNS = [
  /^chore\(release\)/,
  /^chore\(plan\)/,
  /^chore\(protocol\)/,
  /^chore\(deps/,
  /^chore: add Greptile/,
  /^merge:/,
  /^[Mm]erge /,
];

/**
 * @param {{ subject: string; body?: string }[]} commits
 * @returns {{ subject: string; body?: string }[]}
 */
export function findBreakingCommits(commits) {
  return commits.filter(({ subject, body = "" }) => {
    const message = `${subject}\n${body}`;
    if (CLIFF_SKIP_MESSAGE_PATTERNS.some((pattern) => pattern.test(message))) {
      return false;
    }
    if (CLIFF_SKIP_SUBJECT_PATTERNS.some((pattern) => pattern.test(subject))) {
      return false;
    }
    return (
      BREAKING_HEADER_PATTERN.test(subject) ||
      BREAKING_FOOTER_PATTERN.test(body)
    );
  });
}

/**
 * @param {string | null} section
 * @returns {boolean}
 */
export function sectionHasBreakingHeading(section) {
  return typeof section === "string" && BREAKING_HEADING_PATTERN.test(section);
}

/**
 * @param {string} raw `git log --format=%s%x1f%b%x1e` output
 * @returns {{ subject: string; body: string }[]}
 */
export function parseCommitLog(raw) {
  return raw
    .split("\x1e")
    .filter((record) => record.trim().length > 0)
    .map((record) => {
      const [subject = "", body = ""] = record.split("\x1f");
      return { subject: subject.trim(), body };
    });
}

/**
 * @param {string} [repoRoot]
 * @returns {string | null}
 */
export function latestReleaseTag(repoRoot = REPO_ROOT) {
  const result = spawnSync(
    "git",
    ["describe", "--tags", "--abbrev=0", "--match", "v[0-9]*", "HEAD"],
    { cwd: repoRoot, encoding: "utf8" },
  );
  if (result.status !== 0) {
    return null;
  }
  const tag = result.stdout.trim();
  return tag.length > 0 ? tag : null;
}

/**
 * Read commits for `range` and evaluate the upcoming `version` section.
 *
 * @param {string} version
 * @param {{ repoRoot?: string; range?: string; section?: string | null }} [options]
 * @returns {{ breaking: { subject: string; body?: string }[]; section: string | null; ok: boolean }}
 */
export function checkBreakingSection(version, options = {}) {
  const repoRoot = options.repoRoot ?? REPO_ROOT;
  const base = latestReleaseTag(repoRoot);
  const range = options.range ?? (base ? `${base}..HEAD` : "HEAD");
  const log = spawnSync(
    "git",
    ["log", range, "--no-merges", "--format=%s%x1f%b%x1e"],
    { cwd: repoRoot, encoding: "utf8" },
  );
  if (log.status !== 0) {
    throw new Error(`git log ${range} failed: ${log.stderr.trim()}`);
  }

  const breaking = findBreakingCommits(parseCommitLog(log.stdout));
  let section;
  if (options.section !== undefined) {
    section = options.section;
  } else {
    let changelog = null;
    try {
      changelog = readFileSync(join(repoRoot, "CHANGELOG.md"), "utf8");
    } catch {
      // Missing changelog with breaking commits is reported by `ok`.
    }
    section =
      changelog === null ? null : extractChangelogSection(changelog, version);
  }

  const ok =
    breaking.length === 0 ||
    (section !== null && sectionHasBreakingHeading(section));
  return { breaking, section, ok };
}

/**
 * @param {string[]} argv
 * @returns {{ version: string; from: string | null }}
 */
function parseArgs(argv) {
  let version = "";
  let from = null;
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--from") {
      from = argv[index + 1] ?? "";
      index += 1;
      if (from.length === 0) {
        throw new Error("--from requires a git ref");
      }
    } else if (version.length === 0) {
      version = arg;
    } else {
      throw new Error(`Unexpected argument: ${arg}`);
    }
  }
  return { version, from };
}

/**
 * @returns {number} process exit code
 */
function main() {
  let version;
  let from;
  try {
    ({ version, from } = parseArgs(process.argv.slice(2)));
  } catch (error) {
    console.error(`assert-breaking-section: ${error.message}`);
    console.error(
      "Usage: node tooling/release/assert-breaking-section.mjs <vX.Y.Z|X.Y.Z> [--from <ref>]",
    );
    return 1;
  }

  if (version.length === 0) {
    console.error(
      "Usage: node tooling/release/assert-breaking-section.mjs <vX.Y.Z|X.Y.Z> [--from <ref>]",
    );
    return 1;
  }

  const base = from ?? latestReleaseTag();
  const range = base ? `${base}..HEAD` : "HEAD";
  const log = spawnSync(
    "git",
    ["log", range, "--no-merges", "--format=%s%x1f%b%x1e"],
    { cwd: REPO_ROOT, encoding: "utf8" },
  );
  if (log.status !== 0) {
    console.error(
      `assert-breaking-section: git log ${range} failed: ${log.stderr.trim()}`,
    );
    return 1;
  }

  const breaking = findBreakingCommits(parseCommitLog(log.stdout));
  const since = base ?? "repository start";

  if (breaking.length === 0) {
    console.log(`assert-breaking-section: no breaking commits since ${since}.`);
    return 0;
  }

  let changelog = null;
  try {
    changelog = readFileSync(CHANGELOG_PATH, "utf8");
  } catch {
    // Missing changelog with breaking commits is a failure, reported below.
  }

  const section =
    changelog === null ? null : extractChangelogSection(changelog, version);
  if (section === null) {
    console.error(
      `assert-breaking-section: ${breaking.length} breaking commit(s) since ${since}, but CHANGELOG.md has no section for ${version}:`,
    );
    for (const commit of breaking) {
      console.error(`  - ${commit.subject}`);
    }
    console.error(
      "Generate the version section with cliff.toml's Breaking commit parser before publishing.",
    );
    return 1;
  }

  if (sectionHasBreakingHeading(section)) {
    console.log(
      `assert-breaking-section: ${version} has "### Breaking" for ${breaking.length} breaking commit(s).`,
    );
    return 0;
  }

  console.error(
    `assert-breaking-section: ${breaking.length} breaking commit(s) since ${since}, but the ${version} section of CHANGELOG.md has no "### Breaking":`,
  );
  for (const commit of breaking) {
    console.error(`  - ${commit.subject}`);
  }
  console.error(
    'Regenerate the section with cliff.toml\'s Breaking commit parser, or mark the commits with `!` and a `BREAKING CHANGE:` footer.',
  );
  return 1;
}

const isMain =
  process.argv[1] &&
  fileURLToPath(import.meta.url) === process.argv[1];

if (isMain) {
  process.exit(main());
}
