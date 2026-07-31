import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, it } from "node:test";
import {
  CARGO_CONNECT_CRATE_PATH,
  CANONICAL_PATH,
} from "./lockstep-surfaces.mjs";
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

/**
 * Strictly lower core SemVer than fixture (for refuse-path tests).
 * Prefer decrementing patch/minor/major over a hardcoded sentinel.
 *
 * @param {string} version
 * @returns {string}
 */
function strictlyLowerRelease(version) {
  const parsed = parseSemVer(version);
  if (!parsed) {
    throw new Error(`Invalid fixture SemVer: ${version}`);
  }
  if (parsed.patch > 0) {
    return `${parsed.major}.${parsed.minor}.${parsed.patch - 1}`;
  }
  if (parsed.minor > 0) {
    return `${parsed.major}.${parsed.minor - 1}.999`;
  }
  if (parsed.major > 0) {
    return `${parsed.major - 1}.999.999`;
  }
  // Fixture is 0.0.0* — any X.Y.Z with prerelease sorts lower than 0.0.0 release,
  // but bump refuses non-greater cores; use a sentinel that cannot equal live lockstep.
  return "0.0.0-test.0";
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

    // The private connect crate advances with the workspace: its
    // `spoke-schemas` path dependency and its Cargo.lock entry.
    const connectCrate = readFileSync(
      join(repoRoot, CARGO_CONNECT_CRATE_PATH),
      "utf8",
    );
    assert.match(
      connectCrate,
      new RegExp(
        `^spoke-schemas = \\{ version = "${target}", path = "../spoke-schemas" }`,
        "m",
      ),
    );
    const cargoLock = readFileSync(join(repoRoot, "Cargo.lock"), "utf8");
    assert.match(
      cargoLock,
      new RegExp(
        `\\[\\[package\\]\\]\\s*\\nname = "spoke-connect"\\s*\\nversion = "${target}"`,
      ),
      "Cargo.lock spoke-connect entry must be bumped",
    );

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
    const lower = strictlyLowerRelease(current);
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
