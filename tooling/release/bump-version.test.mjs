import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, it } from "node:test";
import { CANONICAL_PATH } from "./lockstep-surfaces.mjs";
import {
  cleanupTempRepo,
  createTempRepo,
  initGitRepo,
  readCanonicalVersion,
  runReleaseScript,
} from "./test-harness.mjs";

/** @type {string[]} */
const tempDirs = [];

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
    assert.equal(current, "0.1.0");

    const result = runReleaseScript(
      "bump-version.mjs",
      ["0.1.1"],
      repoRoot,
    );

    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(result.stdout, /Bumped lockstep version 0\.1\.0 → 0\.1\.1/);

    const bumped = JSON.parse(
      readFileSync(join(repoRoot, CANONICAL_PATH), "utf8"),
    );
    assert.equal(bumped.version, "0.1.1");

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

    const result = runReleaseScript(
      "bump-version.mjs",
      ["0.0.9"],
      repoRoot,
    );

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /must be greater than current/);

    assert.equal(readCanonicalVersion(repoRoot), "0.1.0");
  });
});
