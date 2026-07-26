import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, it } from "node:test";
import {
  cleanupTempRepo,
  createTempRepo,
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

describe("assert-lockstep-version.mjs", () => {
  it("passes when all lockstep surfaces match", () => {
    const repoRoot = createTempRepo();
    tempDirs.push(repoRoot);

    const result = runReleaseScript(
      "assert-lockstep-version.mjs",
      [],
      repoRoot,
    );

    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(result.stdout, /Lockstep version OK/);
  });

  it("rejects when one manifest version drifts", () => {
    const repoRoot = createTempRepo();
    tempDirs.push(repoRoot);

    const current = readCanonicalVersion(repoRoot);
    // Synthetic drift — must not equal the live lockstep SemVer.
    const drifted = `${current}-drift.test`;
    assert.notEqual(drifted, current);

    const driftedPath = join(repoRoot, "packages/spoke-schemas/package.json");
    const pkg = JSON.parse(readFileSync(driftedPath, "utf8"));
    pkg.version = drifted;
    writeFileSync(driftedPath, `${JSON.stringify(pkg, null, 2)}\n`);

    const result = runReleaseScript(
      "assert-lockstep-version.mjs",
      [],
      repoRoot,
    );

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /Lockstep version mismatch/);
    assert.match(result.stderr, /packages\/spoke-schemas\/package\.json/);
  });
});
