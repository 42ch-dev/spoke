import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, it } from "node:test";
import { CANONICAL_PATH } from "./lockstep-surfaces.mjs";
import { parseSemVer } from "./semver.mjs";
import {
  cleanupTempRepo,
  createTempRepo,
  initGitRepo,
  readCanonicalVersion,
  runReleaseScript,
} from "./test-harness.mjs";

/** @type {string[]} */
const tempDirs = [];

/**
 * Next patch release relative to a lockstep fixture version (drops prerelease).
 * Tests must not hardcode the repo's live SemVer — fixtures copy package.json.
 *
 * @param {string} version
 * @returns {string}
 */
function nextPatchRelease(version) {
  const parsed = parseSemVer(version);
  if (!parsed) {
    throw new Error(`Invalid fixture SemVer: ${version}`);
  }
  return `${parsed.major}.${parsed.minor}.${parsed.patch + 1}`;
}

afterEach(() => {
  while (tempDirs.length > 0) {
    const dir = tempDirs.pop();
    if (dir) {
      cleanupTempRepo(dir);
    }
  }
});

describe("bump-version.mjs", () => {
  it("bumps to a strictly greater SemVer across lockstep surfaces", () => {
    const repoRoot = createTempRepo();
    tempDirs.push(repoRoot);
    initGitRepo(repoRoot);

    const current = readCanonicalVersion(repoRoot);
    const target = nextPatchRelease(current);

    const result = runReleaseScript(
      "bump-version.mjs",
      [target],
      repoRoot,
    );

    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(
      result.stdout,
      new RegExp(
        `Bumped lockstep version ${escapeRegExp(current)} → ${escapeRegExp(target)}`,
      ),
    );

    const bumped = JSON.parse(
      readFileSync(join(repoRoot, CANONICAL_PATH), "utf8"),
    );
    assert.equal(bumped.version, target);

    const assertResult = runReleaseScript(
      "assert-lockstep-version.mjs",
      [],
      repoRoot,
    );
    assert.equal(assertResult.status, 0, assertResult.stderr || assertResult.stdout);
  });

  it("refuses a non-increasing target SemVer", () => {
    const repoRoot = createTempRepo();
    tempDirs.push(repoRoot);

    const current = readCanonicalVersion(repoRoot);
    // Equal target is an intentional idempotent path (changelog/assert only).
    // Strictly lower SemVer must refuse before any git-cliff work.
    const lower = "0.0.0";
    assert.notEqual(lower, current);

    const result = runReleaseScript(
      "bump-version.mjs",
      [lower],
      repoRoot,
    );

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /must be greater than current/);

    assert.equal(readCanonicalVersion(repoRoot), current);
  });
});

/**
 * @param {string} value
 * @returns {string}
 */
function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
