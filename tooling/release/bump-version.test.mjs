import assert from "node:assert/strict";
import { cpSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";
import { afterEach, describe, it } from "node:test";
import {
  CANONICAL_PATH,
  CARGO_WORKSPACE_PATH,
  parseCargoPathDependencyPins,
  parseCargoWorkspaceMembers,
  resolveCargoLockPackageNames,
} from "./lockstep-surfaces.mjs";
import { extractChangelogSection } from "./extract-changelog-notes.mjs";
import { parseSemVer } from "./semver.mjs";
import {
  cleanupTempRepo,
  createTempRepo,
  findCargoMemberManifest,
  REPO_ROOT,
  initGitRepo,
  readCanonicalVersion,
  runReleaseScript,
} from "./test-harness.mjs";
/** @type {string[]} */
const tempDirs = [];

function git(repoRoot, ...args) {
  const result = spawnSync("git", args, { cwd: repoRoot, encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout;
}

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

    const pathOnlyCratePath = findCargoMemberManifest(
      repoRoot,
      "spoke-connect-capi",
    );
    const pathOnlyCrateBefore = readFileSync(pathOnlyCratePath, "utf8");

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
      findCargoMemberManifest(repoRoot, "spoke-connect"),
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
    assert.equal(
      readFileSync(pathOnlyCratePath, "utf8"),
      pathOnlyCrateBefore,
      "path-only dependency manifest must remain untouched",
    );
  });

  it("refuses a non-increasing target SemVer", () => {
    const repoRoot = createTempRepo();
    tempDirs.push(repoRoot);
    const current = readCanonicalVersion(repoRoot);
    writeFileSync(join(repoRoot, "CHANGELOG.md"), `# Changelog\n\n## [${current}]\n\nExisting notes.\n`);
    initGitRepo(repoRoot);

    const paths = git(repoRoot, "ls-files", "-z").split("\0").filter(Boolean);
    const snapshot = () => ({
      files: paths.map((path) => {
        const full = join(repoRoot, path);
        return [path, readFileSync(full, "utf8"), statSync(full, { bigint: true }).mtimeNs];
      }),
      status: git(repoRoot, "status", "--porcelain"),
      refs: git(repoRoot, "show-ref", "--head"),
    });
    const before = snapshot();
    const lower = strictlyLowerRelease(current);
    for (const args of [
      [current],
      [lower],
      [current, "--tag"],
      [nextPatchRelease(current), "--tag", "Release"],
    ]) {
      const result = runReleaseScript("bump-version.mjs", args, repoRoot);
      assert.notEqual(result.status, 0, args.join(" "));
      assert.match(result.stderr, args.length === 1 ? /must be greater than current/ : /Unexpected arguments/);
      assert.deepEqual(snapshot(), before, `${args.join(" ")} must not mutate the fixture`);
    }
  });

  it("promotes existing notes and bumps all fixture pins including optional aliases", () => {
    const repoRoot = createTempRepo();
    tempDirs.push(repoRoot);
    const current = readCanonicalVersion(repoRoot);
    const parsed = parseSemVer(current);
    assert.ok(parsed);
    const target = `${parsed.major}.${parsed.minor + 1}.0`;
    const connectPath = findCargoMemberManifest(repoRoot, "spoke-connect");
    const original = readFileSync(connectPath, "utf8");
    const withOptional = original
      .replace(`spoke-schemas = { version = "${current}", path = "../spoke-schemas" }`,
        `schemas = { package = "spoke-schemas", version = "${current}", path = "../spoke-schemas", optional = true }`)
      .replace(`spoke-operations = { version = "${current}", path = "../spoke-operations" }`,
        `spoke-operations = { version = "${current}", path = "../spoke-operations", optional = true }`);
    assert.notEqual(withOptional, original);
    assert.equal((withOptional.match(/path = "\.\.\/spoke-[^"]+", optional = true/g) ?? []).length, 2);
    writeFileSync(connectPath, withOptional);
    const capiPath = findCargoMemberManifest(repoRoot, "spoke-connect-capi");
    const capiBefore = readFileSync(capiPath, "utf8");
    const excluded = ["tooling/codegen/rust-gen/Cargo.toml", "pnpm-lock.yaml"];
    for (const path of excluded) {
      mkdirSync(dirname(join(repoRoot, path)), { recursive: true });
      cpSync(join(REPO_ROOT, path), join(repoRoot, path));
    }
    const excludedBefore = excluded.map((path) => readFileSync(join(repoRoot, path), "utf8"));
    const changelog = `# Changelog\n\n## [${current}]\n\nOld notes.\n\n## [${target}]\n\n### Fixed\n\n- Correct release notes.\n`;
    writeFileSync(join(repoRoot, "CHANGELOG.md"), changelog);
    const workspace = readFileSync(join(repoRoot, CARGO_WORKSPACE_PATH), "utf8");
    const members = parseCargoWorkspaceMembers(workspace);
    const names = resolveCargoLockPackageNames(workspace, (member) =>
      readFileSync(join(repoRoot, member, "Cargo.toml"), "utf8"));
    const pinsBefore = members.map((member) => {
      const path = join(repoRoot, member, "Cargo.toml");
      return parseCargoPathDependencyPins(readFileSync(path, "utf8"), names, path);
    });
    initGitRepo(repoRoot);
    const result = runReleaseScript("bump-version.mjs", [target], repoRoot);
    assert.equal(result.status, 0, result.stderr || result.stdout);
    const pinsAfter = members.map((member) => {
      const path = join(repoRoot, member, "Cargo.toml");
      return parseCargoPathDependencyPins(readFileSync(path, "utf8"), names, path);
    });
    assert.deepEqual(pinsAfter, pinsBefore.map((pins) => pins.map((pin) => ({ ...pin, version: target }))));
    const connectAfter = readFileSync(connectPath, "utf8");
    assert.ok(connectAfter.includes(`schemas = { package = "spoke-schemas", version = "${target}", path = "../spoke-schemas", optional = true }`));
    assert.ok(connectAfter.includes(`spoke-operations = { version = "${target}", path = "../spoke-operations", optional = true }`));
    assert.equal(readFileSync(capiPath, "utf8"), capiBefore);
    assert.deepEqual(excluded.map((path) => readFileSync(join(repoRoot, path), "utf8")), excludedBefore);
    const updated = readFileSync(join(repoRoot, "CHANGELOG.md"), "utf8");
    assert.equal((updated.match(new RegExp(`^## \\[${escapeRegExp(target)}\\]`, "gm")) ?? []).length, 1);
    assert.equal(updated.match(/^## \[([^\]]+)\]/m)?.[1], target);
    assert.equal(extractChangelogSection(updated, target), "### Fixed\n\n- Correct release notes.\n");
    const check = runReleaseScript("assert-lockstep-version.mjs", [], repoRoot);
    assert.equal(check.status, 0, check.stderr || check.stdout);
  });

  it("generates a missing changelog section from fixture conventional commits", () => {
    const repoRoot = createTempRepo();
    tempDirs.push(repoRoot);
    const current = readCanonicalVersion(repoRoot);
    const target = nextPatchRelease(current);
    writeFileSync(join(repoRoot, "CHANGELOG.md"), `# Changelog\n\n## [${current}]\n\nOld notes.\n`);
    initGitRepo(repoRoot);
    writeFileSync(join(repoRoot, "fixture.txt"), "fixture\n");
    git(repoRoot, "add", "fixture.txt");
    git(repoRoot, "commit", "-m", "fix: preserve fixture release notes");
    const result = runReleaseScript("bump-version.mjs", [target], repoRoot);
    assert.equal(result.status, 0, result.stderr || result.stdout);
    const notes = extractChangelogSection(readFileSync(join(repoRoot, "CHANGELOG.md"), "utf8"), target);
    assert.match(notes ?? "", /preserve fixture release notes/i);
  });
});

/**
 * @param {string} value
 * @returns {string}
 */
function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
