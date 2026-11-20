#!/usr/bin/env node
/**
 * Fail when generated release notes leave breaking commits outside `### Breaking`.
 *
 * git-cliff renders `**BREAKING:**` inline for a breaking commit unless it is
 * grouped under `### Breaking`; a marker in any other group means the changelog
 * hid the breaking change. The check reads the generated section (or a caller's
 * preview) instead of replaying git history.
 *
 * CLI: node tooling/release/assert-breaking-section.mjs <vX.Y.Z|X.Y.Z>
 *
 * Exits 1 when the section has `**BREAKING:**` markers but no `### Breaking`;
 * exits 0 otherwise, and when the version section is absent.
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
const BREAKING_HEADING_PATTERN = /^### Breaking\s*$/m;

/**
 * @param {string | null} section
 * @returns {boolean}
 */
export function sectionHasBreakingHeading(section) {
  return typeof section === "string" && BREAKING_HEADING_PATTERN.test(section);
}

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
    let changelog = null;
    try {
      changelog = readFileSync(join(repoRoot, "CHANGELOG.md"), "utf8");
    } catch {
      // Absent changelog means no generated section to inspect.
    }
    section =
      changelog === null ? null : extractChangelogSection(changelog, version);
  }

  if (section === null || sectionHasBreakingHeading(section)) {
    return { ok: true, section, markers: 0 };
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

  const { ok, section, markers } = checkBreakingSection(version);
  if (section === null) {
    console.error(
      `assert-breaking-section: no CHANGELOG.md section for ${version}; skipping.`,
    );
    return 0;
  }
  if (ok) {
    console.log(
      `assert-breaking-section: ${version} has no breaking changes outside "### Breaking".`,
    );
    return 0;
  }

  console.error(
    `assert-breaking-section: ${markers} "**BREAKING:**" marker(s) outside "### Breaking" in the ${version} section of CHANGELOG.md.`,
  );
  console.error(
    'Add cliff.toml\'s Breaking commit parser and regenerate, or move those entries under "### Breaking".',
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
