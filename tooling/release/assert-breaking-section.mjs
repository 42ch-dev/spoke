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
 * Exits 0 when the section has the heading, has no breaking commits, or the
 * version section is absent; exits 1 when breaking commits lack the heading.
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

/**
 * @param {{ subject: string; body?: string }[]} commits
 * @returns {{ subject: string; body?: string }[]}
 */
export function findBreakingCommits(commits) {
  return commits.filter(
    ({ subject, body = "" }) =>
      BREAKING_HEADER_PATTERN.test(subject) ||
      BREAKING_FOOTER_PATTERN.test(body),
  );
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
function parseCommitLog(raw) {
  return raw
    .split("\x1e")
    .filter((record) => record.trim().length > 0)
    .map((record) => {
      const [subject = "", body = ""] = record.split("\x1f");
      return { subject: subject.trim(), body };
    });
}

/**
 * @returns {string | null}
 */
function latestReleaseTag() {
  const result = spawnSync(
    "git",
    ["describe", "--tags", "--abbrev=0", "--match", "v[0-9]*", "HEAD"],
    { cwd: REPO_ROOT, encoding: "utf8" },
  );
  if (result.status !== 0) {
    return null;
  }
  const tag = result.stdout.trim();
  return tag.length > 0 ? tag : null;
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
    ["log", range, "--format=%s%x1f%b%x1e"],
    { cwd: REPO_ROOT, encoding: "utf8" },
  );
  if (log.status !== 0) {
    console.error(
      `assert-breaking-section: git log ${range} failed: ${log.stderr.trim()}`,
    );
    return 1;
  }

  let changelog;
  try {
    changelog = readFileSync(CHANGELOG_PATH, "utf8");
  } catch {
    console.error(
      `assert-breaking-section: ${CHANGELOG_PATH} is missing; skipping.`,
    );
    return 0;
  }

  const section = extractChangelogSection(changelog, version);
  if (section === null) {
    console.error(
      `assert-breaking-section: no CHANGELOG.md section for ${version}; skipping.`,
    );
    return 0;
  }

  const breaking = findBreakingCommits(parseCommitLog(log.stdout));
  if (breaking.length === 0) {
    console.log(
      `assert-breaking-section: no breaking commits since ${base ?? "repository start"}.`,
    );
    return 0;
  }

  if (sectionHasBreakingHeading(section)) {
    console.log(
      `assert-breaking-section: ${version} has "### Breaking" for ${breaking.length} breaking commit(s).`,
    );
    return 0;
  }

  console.error(
    `assert-breaking-section: ${breaking.length} breaking commit(s) since ${base ?? "repository start"}, but the ${version} section of CHANGELOG.md has no "### Breaking":`,
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
