import assert from "node:assert/strict";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, it } from "node:test";
import { parseSemVer } from "./semver.mjs";
import {
  CARGO_LOCK_PATH,
  CARGO_WORKSPACE_PATH,
  parseCargoWorkspaceMembers,
  resolveCargoLockPackageNames,
} from "./lockstep-surfaces.mjs";
import {
  cleanupTempRepo,
  createTempRepo,
  findCargoMemberManifest,
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

  it("verifies and bumps aliased Cargo path pins", () => {
    const repoRoot = createTempRepo();
    tempDirs.push(repoRoot);
    initGitRepo(repoRoot);

    const current = readCanonicalVersion(repoRoot);
    const parsed = parseSemVer(current);
    assert.ok(parsed, "fixture version must be valid SemVer");
    const target = `${parsed.major}.${parsed.minor}.${parsed.patch + 1}`;
    const cratePath = findCargoMemberManifest(repoRoot, "spoke-connect");
    const crate = readFileSync(cratePath, "utf8");
    const aliased = crate.replace(
      /^spoke-schemas = \{ version = "([^"]+)", path = "\.\.\/spoke-schemas" \}$/m,
      `schemas = { package = "spoke-schemas", version = "$1", path = "../spoke-schemas" }`,
    );
    assert.notEqual(aliased, crate, "fixture must contain the direct pin");
    writeFileSync(cratePath, aliased, "utf8");

    const initiallyPassing = runReleaseScript(
      "assert-lockstep-version.mjs",
      [],
      repoRoot,
    );
    assert.equal(
      initiallyPassing.status,
      0,
      initiallyPassing.stderr || initiallyPassing.stdout,
    );

    writeFileSync(
      cratePath,
      aliased.replace(
        `version = "${current}"`,
        `version = "${current}-drift.test"`,
      ),
      "utf8",
    );
    const stalePin = runReleaseScript(
      "assert-lockstep-version.mjs",
      [],
      repoRoot,
    );
    assert.notEqual(stalePin.status, 0);
    assert.match(stalePin.stderr, /spoke-schemas dependency/);

    writeFileSync(cratePath, aliased, "utf8");
    const bumped = runReleaseScript(
      "bump-version.mjs",
      [target],
      repoRoot,
    );
    assert.equal(bumped.status, 0, bumped.stderr || bumped.stdout);
    const bumpedCrate = readFileSync(cratePath, "utf8");
    assert.match(
      bumpedCrate,
      new RegExp(
        `^schemas = \\{ package = "spoke-schemas", version = "${target}", path = "\\.\\./spoke-schemas" \\}`,
        "m",
      ),
    );
  });
  it("verifies and bumps literal-string aliased Cargo path pins", () => {
    const repoRoot = createTempRepo();
    tempDirs.push(repoRoot);
    initGitRepo(repoRoot);

    const current = readCanonicalVersion(repoRoot);
    const parsed = parseSemVer(current);
    assert.ok(parsed, "fixture version must be valid SemVer");
    const target = `${parsed.major}.${parsed.minor}.${parsed.patch + 1}`;
    const cratePath = findCargoMemberManifest(repoRoot, "spoke-connect");
    const crate = readFileSync(cratePath, "utf8");
    const aliased = crate.replace(
      /^spoke-schemas = \{ version = "([^"]+)", path = "\.\.\/spoke-schemas" \}$/m,
      `schemas = { package = 'spoke-schemas', version = '$1', path = '../spoke-schemas' }`,
    );
    assert.notEqual(aliased, crate, "fixture must contain the direct pin");
    writeFileSync(cratePath, aliased, "utf8");

    const initiallyPassing = runReleaseScript(
      "assert-lockstep-version.mjs",
      [],
      repoRoot,
    );
    assert.equal(
      initiallyPassing.status,
      0,
      initiallyPassing.stderr || initiallyPassing.stdout,
    );

    const stale = aliased.replace(
      `version = '${current}'`,
      `version = '${current}-drift.test'`,
    );
    writeFileSync(cratePath, stale, "utf8");
    const stalePin = runReleaseScript(
      "assert-lockstep-version.mjs",
      [],
      repoRoot,
    );
    assert.notEqual(stalePin.status, 0);
    assert.match(stalePin.stderr, /spoke-schemas dependency/);

    writeFileSync(cratePath, aliased, "utf8");
    const bumped = runReleaseScript(
      "bump-version.mjs",
      [target],
      repoRoot,
    );
    assert.equal(bumped.status, 0, bumped.stderr || bumped.stdout);
    const bumpedCrate = readFileSync(cratePath, "utf8");
    assert.match(
      bumpedCrate,
      new RegExp(
        `^schemas = \\{ package = 'spoke-schemas', version = '${target}', path = '\\.\\./spoke-schemas' \\}`,
        "m",
      ),
    );
  });

  it("refuses section-form workspace-member dependencies", () => {
    const repoRoot = createTempRepo();
    tempDirs.push(repoRoot);
    initGitRepo(repoRoot);

    const current = readCanonicalVersion(repoRoot);
    const parsed = parseSemVer(current);
    assert.ok(parsed, "fixture version must be valid SemVer");
    const target = `${parsed.major}.${parsed.minor}.${parsed.patch + 1}`;
    const cratePath = findCargoMemberManifest(repoRoot, "spoke-connect");
    const crate = readFileSync(cratePath, "utf8");
    const sectionForm = crate.replace(
      /^spoke-schemas = \{ version = "[^"]+", path = "\.\.\/spoke-schemas" \}\n/m,
      `[dependencies.spoke-schemas]\nversion = "${current}"\npath = "../spoke-schemas"\n`,
    );
    assert.notEqual(sectionForm, crate, "fixture must contain the direct pin");
    writeFileSync(cratePath, sectionForm, "utf8");

    const asserted = runReleaseScript(
      "assert-lockstep-version.mjs",
      [],
      repoRoot,
    );
    assert.notEqual(asserted.status, 0);
    assert.match(
      asserted.stderr,
      /crates\/spoke-connect\/Cargo\.toml: dependency section \[dependencies\.spoke-schemas\].*inline dependency tables are the supported shape/s,
    );

    const bumped = runReleaseScript(
      "bump-version.mjs",
      [target],
      repoRoot,
    );
    assert.notEqual(bumped.status, 0);
    assert.match(
      bumped.stderr,
      /crates\/spoke-connect\/Cargo\.toml: dependency section \[dependencies\.spoke-schemas\].*inline dependency tables are the supported shape/s,
    );
    assert.equal(readCanonicalVersion(repoRoot), current);
  });

  it("refuses dotted-key workspace-member dependencies", () => {
    const repoRoot = createTempRepo();
    tempDirs.push(repoRoot);
    initGitRepo(repoRoot);

    const current = readCanonicalVersion(repoRoot);
    const parsed = parseSemVer(current);
    assert.ok(parsed, "fixture version must be valid SemVer");
    const target = `${parsed.major}.${parsed.minor}.${parsed.patch + 1}`;
    const cratePath = findCargoMemberManifest(repoRoot, "spoke-connect");
    const crate = readFileSync(cratePath, "utf8");
    const dottedForm = crate.replace(
      /^spoke-schemas = \{ version = "[^"]+", path = "\.\.\/spoke-schemas" \}$/m,
      `spoke-schemas.version = "${current}"\nspoke-schemas.path = "../spoke-schemas"`,
    );
    assert.notEqual(dottedForm, crate, "fixture must contain the direct pin");
    writeFileSync(cratePath, dottedForm, "utf8");

    const asserted = runReleaseScript(
      "assert-lockstep-version.mjs",
      [],
      repoRoot,
    );
    assert.notEqual(asserted.status, 0);
    assert.match(
      asserted.stderr,
      /crates\/spoke-connect\/Cargo\.toml: dotted-key dependency spoke-schemas\.(?:version|path).*inline dependency tables are the supported shape/s,
    );

    const bumped = runReleaseScript(
      "bump-version.mjs",
      [target],
      repoRoot,
    );
    assert.notEqual(bumped.status, 0);
    assert.match(
      bumped.stderr,
      /crates\/spoke-connect\/Cargo\.toml: dotted-key dependency spoke-schemas\.(?:version|path).*inline dependency tables are the supported shape/s,
    );
    assert.equal(readCanonicalVersion(repoRoot), current);
  });


  it("refuses globbed Cargo workspace members with explicit-path guidance", () => {
    const repoRoot = createTempRepo();
    tempDirs.push(repoRoot);

    const workspace = '[workspace]\nmembers = ["crates/*"]\n';
    assert.throws(
      () =>
        resolveCargoLockPackageNames(workspace, (memberPath) =>
          readFileSync(join(repoRoot, memberPath, "Cargo.toml"), "utf8"),
        ),
      /Cargo\.toml: workspace member "crates\/\*" uses glob metacharacters; explicit member paths are required/,
    );
  });

  it("covers every extra Cargo workspace member in lockstep coverage", () => {
    const repoRoot = createTempRepo();
    tempDirs.push(repoRoot);

    const current = readCanonicalVersion(repoRoot);
    const extraMemberPath = "fixtures/extra-member/rust";
    const extraPackageName = "spoke-fixture-extra";
    const workspacePath = join(repoRoot, CARGO_WORKSPACE_PATH);
    const workspace = readFileSync(workspacePath, "utf8");
    const updatedWorkspace = workspace.replace(
      /(^members\s*=\s*\[[\s\S]*?)(^\])/m,
      `$1  "${extraMemberPath}",\n$2`,
    );
    assert.notEqual(updatedWorkspace, workspace);
    writeFileSync(workspacePath, updatedWorkspace);
    mkdirSync(join(repoRoot, extraMemberPath), { recursive: true });
    const extraManifestPath = join(repoRoot, extraMemberPath, "Cargo.toml");
    writeFileSync(
      extraManifestPath,
      `[package]\nname = "${extraPackageName}"\nversion.workspace = true\n`,
      "utf8",
    );

    const cargoLockPath = join(repoRoot, CARGO_LOCK_PATH);
    const cargoLock = readFileSync(cargoLockPath, "utf8");
    writeFileSync(
      cargoLockPath,
      `${cargoLock}\n[[package]]\nname = "${extraPackageName}"\nversion = "${current}"\n`,
      "utf8",
    );

    const workspaceMembers = parseCargoWorkspaceMembers(updatedWorkspace);
    const derivedNames = resolveCargoLockPackageNames(
      updatedWorkspace,
      (memberPath) =>
        readFileSync(join(repoRoot, memberPath, "Cargo.toml"), "utf8"),
    );
    assert.ok(workspaceMembers.includes(extraMemberPath));
    assert.ok(derivedNames.includes(extraPackageName));

    const passing = runReleaseScript(
      "assert-lockstep-version.mjs",
      [],
      repoRoot,
    );
    assert.equal(passing.status, 0, passing.stderr || passing.stdout);

    writeFileSync(
      cargoLockPath,
      cargoLock.replace(
        `name = "${extraPackageName}"\nversion = "${current}"`,
        `name = "${extraPackageName}"\nversion = "${current}-drift.test"`,
      ) +
        `\n[[package]]\nname = "${extraPackageName}"\nversion = "${current}-drift.test"\n`,
      "utf8",
    );
    const failing = runReleaseScript(
      "assert-lockstep-version.mjs",
      [],
      repoRoot,
    );
    assert.notEqual(failing.status, 0);
    assert.match(failing.stderr, new RegExp(extraPackageName));
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

  it("rejects when the spoke-connect crate drifts from lockstep", () => {
    const repoRoot = createTempRepo();
    tempDirs.push(repoRoot);

    const current = readCanonicalVersion(repoRoot);
    // Synthetic drift — must not equal the live lockstep SemVer.
    const drifted = `${current}-drift.test`;
    assert.notEqual(drifted, current);

    const cratePath = findCargoMemberManifest(repoRoot, "spoke-connect");
    const crate = readFileSync(cratePath, "utf8");
    const driftedCrate = crate.replace(
      /^spoke-schemas\s*=\s*\{[^}]*version\s*=\s*"[^"]+"/m,
      `spoke-schemas = { version = "${drifted}", path = "../spoke-schemas" }`,
    );
    assert.notEqual(driftedCrate, crate, "fixture must contain the dependency");
    writeFileSync(cratePath, driftedCrate, "utf8");

    const result = runReleaseScript(
      "assert-lockstep-version.mjs",
      [],
      repoRoot,
    );

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /Lockstep version mismatch/);
    assert.match(
      result.stderr,
      /crates\/spoke-connect\/Cargo\.toml/,
    );
  });

  it("rejects when the spoke-connect spoke-operations dependency drifts from lockstep", () => {
    const repoRoot = createTempRepo();
    tempDirs.push(repoRoot);

    const current = readCanonicalVersion(repoRoot);
    const drifted = `${current}-drift.test`;
    assert.notEqual(drifted, current);

    const cratePath = findCargoMemberManifest(repoRoot, "spoke-connect");
    const crate = readFileSync(cratePath, "utf8");
    const driftedCrate = crate.replace(
      /^spoke-operations\s*=\s*\{[^}]*version\s*=\s*"[^"]+"/m,
      `spoke-operations = { version = "${drifted}", path = "../spoke-operations", optional = true }`,
    );
    assert.notEqual(driftedCrate, crate, "fixture must contain the dependency");
    writeFileSync(cratePath, driftedCrate, "utf8");

    const result = runReleaseScript(
      "assert-lockstep-version.mjs",
      [],
      repoRoot,
    );

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /Lockstep version mismatch/);
    assert.match(result.stderr, /spoke-operations dependency/);
  });
});
