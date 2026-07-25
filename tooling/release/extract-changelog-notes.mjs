#!/usr/bin/env node
/**
 * Print the CHANGELOG.md section body for a release version (GitHub Release notes).
 *
 * CLI: node tooling/release/extract-changelog-notes.mjs <vX.Y.Z|X.Y.Z>
 *
 * Exits 0 with section body on stdout when found; exits 1 with empty stdout when missing.
 *
 * @module tooling/release/extract-changelog-notes
 */

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const CHANGELOG_PATH = join(REPO_ROOT, "CHANGELOG.md");

const SEMVER_PATTERN =
  /^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.]+)?$/;

/**
 * @param {string} value
 * @returns {string}
 */
function escapeRegex(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * @param {string} input
 * @returns {string}
 */
function normalizeVersion(input) {
  return input.trim().replace(/^v/i, "");
}

/**
 * @param {string} changelog
 * @param {string} version
 * @returns {string | null}
 */
export function extractChangelogSection(changelog, version) {
  const normalized = normalizeVersion(version);
  if (!SEMVER_PATTERN.test(normalized)) {
    return null;
  }

  const headerRe = new RegExp(
    `^## \\[${escapeRegex(normalized)}\\](?:\\s+-\\s+\\d{4}-\\d{2}-\\d{2})?\\s*$`,
    "m",
  );
  const match = headerRe.exec(changelog);
  if (!match) {
    return null;
  }

  const sectionStart = match.index + match[0].length;
  const rest = changelog.slice(sectionStart);
  const nextHeader = rest.search(/^## \[/m);
  const body = nextHeader === -1 ? rest : rest.slice(0, nextHeader);

  const trimmed = body.trim();
  return trimmed.length > 0 ? `${trimmed}\n` : null;
}

/**
 * @param {string[]} argv
 */
function main(argv) {
  if (argv.length === 0 || argv[0] === "--help" || argv[0] === "-h") {
    console.error(
      "Usage: node tooling/release/extract-changelog-notes.mjs <vX.Y.Z|X.Y.Z>",
    );
    process.exit(argv.length === 0 ? 1 : 0);
  }

  let changelog;
  try {
    changelog = readFileSync(CHANGELOG_PATH, "utf8");
  } catch {
    process.exit(1);
  }

  const section = extractChangelogSection(changelog, argv[0]);
  if (!section) {
    process.exit(1);
  }

  process.stdout.write(section);
}

if (
  process.argv[1] &&
  fileURLToPath(import.meta.url) === process.argv[1]
) {
  main(process.argv.slice(2));
}
