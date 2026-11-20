#!/usr/bin/env node
/**
 * Fail when generated release notes render a breaking marker outside `### Breaking`.
 *
 * git-cliff renders `**BREAKING:**` inline for a breaking commit unless it is
 * grouped under `### Breaking`; a marker in any group means the changelog
 * attributes a breaking change to an ordinary section. The check reads the
 * generated section text (or a caller's preview) instead of replaying git history.
 *
 * CLI: node tooling/release/assert-breaking-section.mjs <vX.Y.Z|X.Y.Z>
 *
 * Exits 1 when the section renders `**BREAKING:**` anywhere; exits 0 when the
 * section is absent, unreadable only through ENOENT, or marker-free.
 *
 * @module tooling/release/assert-breaking-section
 */

import { readFileSync, realpathSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { extractChangelogSection } from "./extract-changelog-notes.mjs";

const REPO_ROOT = process.env.SPOKE_REPO_ROOT
  ? join(process.env.SPOKE_REPO_ROOT)
  : join(dirname(fileURLToPath(import.meta.url)), "../..");

const BREAKING_MARKER_PATTERN = /\*\*BREAKING:\*\*/g;

/**
 * Evaluate a release section's generated text.
 *
 * @param {string} version
 * @param {{ repoRoot?: string; section?: string | null }} [options]
 * @returns {{ ok: boolean; section: string | null; markers: number }}
 */
export function checkBreakingSection(version, options = {}) {
  const repoRoot = options.repoRoot ?? REPO_ROOT;
  let section;
  if (options.section !== undefined) {
    section = options.section;
  } else {
    try {
      const changelog = readFileSync(join(repoRoot, "CHANGELOG.md"), "utf8");
      section = extractChangelogSection(changelog, version);
    } catch (error) {
      if (error.code !== "ENOENT") {
        throw error;
      }
      section = null;
    }
  }

  if (section === null) {
    return { ok: true, section: null, markers: 0 };
  }
  const markers = section.match(BREAKING_MARKER_PATTERN)?.length ?? 0;
  return { ok: markers === 0, section, markers };
}

/**
 * @returns {number} process exit code
 */
function main() {
  const version = process.argv[2] ?? "";
  if (version === "" || version === "--help" || version === "-h") {
    console.error(
      "Usage: node tooling/release/assert-breaking-section.mjs <vX.Y.Z|X.Y.Z>",
    );
    return version === "" ? 1 : 0;
  }

  let result;
  try {
    result = checkBreakingSection(version);
  } catch (error) {
    console.error(
      `assert-breaking-section: cannot read CHANGELOG.md: ${error.message}`,
    );
    return 1;
  }
  const { ok, section, markers } = result;
  if (section === null) {
    console.error(
      `assert-breaking-section: the ${version} version section is absent from CHANGELOG.md; skipping.`,
    );
    return 0;
  }
  if (ok) {
    console.log(
      `assert-breaking-section: ${version} renders no breaking markers.`,
    );
    return 0;
  }

  console.error(
    `assert-breaking-section: ${markers} "**BREAKING:**" marker(s) in the ${version} section of CHANGELOG.md; a rendered marker anywhere in the section fails.`,
  );
  console.error(
    'Regenerate with cliff.toml\'s Breaking commit parsers, or move those entries under "### Breaking".',
  );
  return 1;
}

const isMain =
  process.argv[1] &&
  realpathSync(process.argv[1]) ===
    realpathSync(fileURLToPath(import.meta.url));

if (isMain) {
  process.exit(main());
}
