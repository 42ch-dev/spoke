import assert from "node:assert/strict";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, it } from "node:test";
import { parseSemVer } from "./semver.mjs";
import {
  CARGO_LOCK_PATH,
  CARGO_WORKSPACE_PATH,
  parseCargoPathDependencyPins,
  parseCargoWorkspaceMembers,
  replaceCargoPathDependencyPinVersions,
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

  it("verifies and bumps quoted-key Cargo path pins", () => {
    for (const form of [
      {
        key: '"spoke-schemas"',
        replacement: `"spoke-schemas" = { version = '$1', path = "../spoke-schemas", optional = true }`,
        staleVersion: (version) => `'${version}-drift.test'`,
        expected: (target) =>
          new RegExp(
            `^"spoke-schemas" = \\{ version = '${target}', path = "\\.\\./spoke-schemas", optional = true \\}`,
            "m",
          ),
      },
      {
        key: "'schemas'",
        replacement: `'schemas' = { package = "spoke-schemas", version = '$1', path = "../spoke-schemas", optional = true }`,
        staleVersion: (version) => `'${version}-drift.test'`,
        expected: (target) =>
          new RegExp(
            `^'schemas' = \\{ package = "spoke-schemas", version = '${target}', path = "\\.\\./spoke-schemas", optional = true \\}`,
            "m",
          ),
      },
    ]) {
      const repoRoot = createTempRepo();
      tempDirs.push(repoRoot);
      initGitRepo(repoRoot);

      const current = readCanonicalVersion(repoRoot);
      const parsed = parseSemVer(current);
      assert.ok(parsed, "fixture version must be valid SemVer");
      const target = `${parsed.major}.${parsed.minor}.${parsed.patch + 1}`;
      const cratePath = findCargoMemberManifest(repoRoot, "spoke-connect");
      const crate = readFileSync(cratePath, "utf8");
      const quoted = crate.replace(
        /^spoke-schemas = \{ version = "([^"]+)", path = "\.\.\/spoke-schemas" \}$/m,
        form.replacement,
      );
      assert.notEqual(quoted, crate, `${form.key}: fixture must contain the direct pin`);
      writeFileSync(cratePath, quoted, "utf8");

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

      const stale = quoted.replace(
        `version = '${current}'`,
        `version = ${form.staleVersion(current)}`,
      );
      writeFileSync(cratePath, stale, "utf8");
      const stalePin = runReleaseScript(
        "assert-lockstep-version.mjs",
        [],
        repoRoot,
      );
      assert.notEqual(stalePin.status, 0);
      assert.match(stalePin.stderr, /spoke-schemas dependency/);

      writeFileSync(cratePath, quoted, "utf8");
      const bumped = runReleaseScript(
        "bump-version.mjs",
        [target],
        repoRoot,
      );
      assert.equal(bumped.status, 0, bumped.stderr || bumped.stdout);
      assert.match(readFileSync(cratePath, "utf8"), form.expected(target));
    }
  });

  it("verifies and bumps an escaped basic-quoted Cargo path pin", () => {
    const repoRoot = createTempRepo();
    tempDirs.push(repoRoot);
    initGitRepo(repoRoot);

    const current = readCanonicalVersion(repoRoot);
    const parsed = parseSemVer(current);
    assert.ok(parsed, "fixture version must be valid SemVer");
    const target = `${parsed.major}.${parsed.minor}.${parsed.patch + 1}`;
    const cratePath = findCargoMemberManifest(repoRoot, "spoke-connect");
    const crate = readFileSync(cratePath, "utf8");
    const escaped = crate.replace(
      /^spoke-schemas = \{ version = "([^"]+)", path = "\.\.\/spoke-schemas" \}$/m,
      '"spoke\\u002Dschemas" = { version = "$1", path = "../spoke-schemas" }',
    );
    assert.notEqual(escaped, crate, "fixture must contain the direct pin");
    writeFileSync(cratePath, escaped, "utf8");

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

    const stale = escaped.replace(
      `version = "${current}"`,
      `version = "${current}-drift.test"`,
    );
    writeFileSync(cratePath, stale, "utf8");
    const stalePin = runReleaseScript(
      "assert-lockstep-version.mjs",
      [],
      repoRoot,
    );
    assert.notEqual(stalePin.status, 0);
    assert.match(stalePin.stderr, /spoke-schemas dependency/);

    writeFileSync(cratePath, escaped, "utf8");
    const bumped = runReleaseScript(
      "bump-version.mjs",
      [target],
      repoRoot,
    );
    assert.equal(bumped.status, 0, bumped.stderr || bumped.stdout);
    assert.match(
      readFileSync(cratePath, "utf8"),
      new RegExp(
        `^"spoke\\\\u002Dschemas" = \\{ version = "${target}", path = "\\.\\./spoke-schemas" \\}$`,
        "m",
      ),
    );
  });

  it("resolves escaped basic package attributes as workspace members", () => {
    const repoRoot = createTempRepo();
    tempDirs.push(repoRoot);
    initGitRepo(repoRoot);

    const current = readCanonicalVersion(repoRoot);
    const parsed = parseSemVer(current);
    assert.ok(parsed, "fixture version must be valid SemVer");
    const target = `${parsed.major}.${parsed.minor}.${parsed.patch + 1}`;
    const cratePath = findCargoMemberManifest(repoRoot, "spoke-connect");
    const crate = readFileSync(cratePath, "utf8");
    const escaped = crate.replace(
      /^spoke-schemas = \{ version = "([^"]+)", path = "\.\.\/spoke-schemas" \}$/m,
      'schemas = { package = "spoke\\u002Dschemas", version = "$1", path = "../spoke-schemas" }',
    );
    assert.notEqual(escaped, crate, "fixture must contain the direct pin");
    writeFileSync(cratePath, escaped, "utf8");

    const stale = escaped.replace(
      `version = "${current}"`,
      `version = "${current}-drift.test"`,
    );
    writeFileSync(cratePath, stale, "utf8");
    const stalePin = runReleaseScript(
      "assert-lockstep-version.mjs",
      [],
      repoRoot,
    );
    assert.notEqual(stalePin.status, 0);
    assert.match(stalePin.stderr, /spoke-schemas dependency/);

    writeFileSync(cratePath, escaped, "utf8");
    const bumped = runReleaseScript(
      "bump-version.mjs",
      [target],
      repoRoot,
    );
    assert.equal(bumped.status, 0, bumped.stderr || bumped.stdout);
    assert.match(
      readFileSync(cratePath, "utf8"),
      new RegExp(
        `^schemas = \\{ package = "spoke\\\\u002Dschemas", version = "${target}", path = "\\.\\./spoke-schemas" \\}$`,
        "m",
      ),
    );
  });

  it("refuses unsupported and malformed basic-string escapes", () => {
    for (const escapedKey of ['"spoke\\x2Dschemas"', '"spoke\\u12schemas"']) {
      const repoRoot = createTempRepo();
      tempDirs.push(repoRoot);
      initGitRepo(repoRoot);

      const cratePath = findCargoMemberManifest(repoRoot, "spoke-connect");
      const crate = readFileSync(cratePath, "utf8");
      const malformed = crate.replace(
        /^spoke-schemas = \{ version = "([^"]+)", path = "\.\.\/spoke-schemas" \}$/m,
        `${escapedKey} = { version = "$1", path = "../spoke-schemas" }`,
      );
      assert.notEqual(malformed, crate, "fixture must contain the direct pin");
      writeFileSync(cratePath, malformed, "utf8");

      const result = runReleaseScript(
        "assert-lockstep-version.mjs",
        [],
        repoRoot,
      );
      assert.notEqual(result.status, 0);
      assert.match(
        result.stderr,
        new RegExp(
          `crates/spoke-connect/Cargo\\.toml: (?:unsupported|malformed) TOML basic-string escape in token ${escapedKey.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`,
        ),
      );
    }
  });

  it("uses literal strings verbatim when resolving Cargo pin names", () => {
    const [pin] = parseCargoPathDependencyPins(
      "'spoke\\u002Dschemas' = { version = '0.12.0', path = '../spoke-schemas' }",
      ["spoke-schemas"],
      "fixture/Cargo.toml",
    );
    assert.equal(pin.name, "spoke\\u002Dschemas");
    assert.equal(pin.version, "0.12.0");
    assert.equal(pin.path, "../spoke-schemas");
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

  it("refuses quoted-key section-form workspace-member dependencies", () => {
    for (const key of ['"spoke-schemas"', "'spoke-schemas'"]) {
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
        `[dependencies.${key}]\nversion = "${current}"\npath = "../spoke-schemas"\n`,
      );
      assert.notEqual(sectionForm, crate, `${key}: fixture must contain the direct pin`);
      writeFileSync(cratePath, sectionForm, "utf8");

      const asserted = runReleaseScript(
        "assert-lockstep-version.mjs",
        [],
        repoRoot,
      );
      assert.notEqual(asserted.status, 0);
      assert.match(
        asserted.stderr,
        /crates\/spoke-connect\/Cargo\.toml: dependency section \[dependencies\.(?:"spoke-schemas"|'spoke-schemas')\].*workspace member "spoke-schemas".*inline dependency tables are the supported shape/s,
      );

      const bumped = runReleaseScript(
        "bump-version.mjs",
        [target],
        repoRoot,
      );
      assert.notEqual(bumped.status, 0);
      assert.match(
        bumped.stderr,
        /crates\/spoke-connect\/Cargo\.toml: dependency section \[dependencies\.(?:"spoke-schemas"|'spoke-schemas')\].*workspace member "spoke-schemas".*inline dependency tables are the supported shape/s,
      );
      assert.equal(readCanonicalVersion(repoRoot), current);
    }
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

  it("refuses quoted-key dotted workspace-member dependencies", () => {
    for (const key of ['"spoke-schemas"', "'spoke-schemas'"]) {
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
        `${key}.version = "${current}"\n${key}.path = "../spoke-schemas"`,
      );
      assert.notEqual(dottedForm, crate, `${key}: fixture must contain the direct pin`);
      writeFileSync(cratePath, dottedForm, "utf8");

      const asserted = runReleaseScript(
        "assert-lockstep-version.mjs",
        [],
        repoRoot,
      );
      assert.notEqual(asserted.status, 0);
      assert.match(
        asserted.stderr,
        /crates\/spoke-connect\/Cargo\.toml: dotted-key dependency (?:"spoke-schemas"|'spoke-schemas')\.(?:version|path).*workspace member "spoke-schemas".*inline dependency tables are the supported shape/s,
      );

      const bumped = runReleaseScript(
        "bump-version.mjs",
        [target],
        repoRoot,
      );
      assert.notEqual(bumped.status, 0);
      assert.match(
        bumped.stderr,
        /crates\/spoke-connect\/Cargo\.toml: dotted-key dependency (?:"spoke-schemas"|'spoke-schemas')\.(?:version|path).*workspace member "spoke-schemas".*inline dependency tables are the supported shape/s,
      );
      assert.equal(readCanonicalVersion(repoRoot), current);
    }
  });
  it("refuses target-qualified and nested section/dotted workspace-member declarations", () => {
    const cases = [
      {
        name: "target-qualified dependency section",
        replacement:
          `[target.'cfg(windows)'.dependencies.spoke-schemas]\nversion = "{version}"\npath = "../spoke-schemas"`,
        error:
          /dependency section \[target\.'cfg\(windows\)'\.dependencies\.spoke-schemas\].*workspace member "spoke-schemas".*inline dependency tables are the supported shape/s,
      },
      {
        name: "patch registry dependency section",
        replacement:
          `[patch.crates-io.spoke-schemas]\nversion = "{version}"\npath = "../spoke-schemas"`,
        error:
          /dependency section \[patch\.crates-io\.spoke-schemas\].*workspace member "spoke-schemas".*inline dependency tables are the supported shape/s,
      },
      {
        name: "nested dotted dependency key",
        replacement:
          `dependencies.spoke-schemas.version = "{version}"\ndependencies.spoke-schemas.path = "../spoke-schemas"`,
        error:
          /dotted-key dependency dependencies\.spoke-schemas\.(?:version|path).*workspace member "spoke-schemas".*inline dependency tables are the supported shape/s,
      },
      {
        name: "quoted target-qualified dependency section",
        replacement:
          `[target.'cfg(windows)'.dependencies.'spoke-schemas']\nversion = "{version}"\npath = "../spoke-schemas"`,
        error:
          /dependency section \[target\.'cfg\(windows\)'\.dependencies\.'spoke-schemas'\].*workspace member "spoke-schemas".*inline dependency tables are the supported shape/s,
      },
    ];

    for (const testCase of cases) {
      const repoRoot = createTempRepo();
      tempDirs.push(repoRoot);
      initGitRepo(repoRoot);

      const current = readCanonicalVersion(repoRoot);
      const parsed = parseSemVer(current);
      assert.ok(parsed, "fixture version must be valid SemVer");
      const target = `${parsed.major}.${parsed.minor}.${parsed.patch + 1}`;
      const cratePath = findCargoMemberManifest(repoRoot, "spoke-connect");
      const crate = readFileSync(cratePath, "utf8");
      const replacement = testCase.replacement.replace("{version}", current);
      const declaration = crate.replace(
        /^spoke-schemas = \{ version = "[^"]+", path = "\.\.\/spoke-schemas" \}\n/m,
        `${replacement}\n`,
      );
      assert.notEqual(declaration, crate, `${testCase.name}: fixture must contain the direct pin`);
      writeFileSync(cratePath, declaration, "utf8");

      const asserted = runReleaseScript(
        "assert-lockstep-version.mjs",
        [],
        repoRoot,
      );
      assert.notEqual(asserted.status, 0, `${testCase.name}: assert must refuse`);
      assert.match(asserted.stderr, testCase.error);

      const bumped = runReleaseScript(
        "bump-version.mjs",
        [target],
        repoRoot,
      );
      assert.notEqual(bumped.status, 0, `${testCase.name}: bump must refuse`);
      assert.match(bumped.stderr, testCase.error);
      assert.equal(readCanonicalVersion(repoRoot), current);
    }
  });

  it("keeps legitimate Cargo table shapes and target inline pins in lockstep", () => {
    const contents = `[package]
name = "fixture"
version = "0.12.0"

[lib]
name = "fixture"

[[bin]]
name = "fixture-bin"

[dependencies]
serde = "1"

[dev-dependencies]
serde_json = "1"

[build-dependencies]
cc = "1"

[package.metadata.release]
tag = true

[target.'cfg(unix)'.dependencies]
spoke-schemas = { version = "0.11.0", path = "../spoke-schemas" }

[workspace.dependencies]
serde = "1"
`;
    const pins = parseCargoPathDependencyPins(
      contents,
      ["spoke-schemas"],
      "fixture/Cargo.toml",
    );
    assert.deepEqual(pins, [
      {
        name: "spoke-schemas",
        version: "0.11.0",
        path: "../spoke-schemas",
      },
    ]);
    assert.equal(
      replaceCargoPathDependencyPinVersions(
        contents,
        "0.12.0",
        ["spoke-schemas"],
        "fixture/Cargo.toml",
      ),
      contents.replace('version = "0.11.0"', 'version = "0.12.0"'),
    );

    const repoRoot = createTempRepo();
    tempDirs.push(repoRoot);
    initGitRepo(repoRoot);
    const current = readCanonicalVersion(repoRoot);
    const parsed = parseSemVer(current);
    assert.ok(parsed, "fixture version must be valid SemVer");
    const target = `${parsed.major}.${parsed.minor}.${parsed.patch + 1}`;
    const cratePath = findCargoMemberManifest(repoRoot, "spoke-connect");
    const crate = readFileSync(cratePath, "utf8");
    const targetInline = crate.replace(
      /^spoke-schemas = \{ version = "[^"]+", path = "\.\.\/spoke-schemas" \}\n/m,
      `[target.'cfg(windows)'.dependencies]\nspoke-schemas = { version = "0.11.0", path = "../spoke-schemas" }\n`,
    );
    assert.notEqual(targetInline, crate, "fixture must contain the direct pin");
    writeFileSync(cratePath, targetInline, "utf8");

    const stale = runReleaseScript(
      "assert-lockstep-version.mjs",
      [],
      repoRoot,
    );
    assert.notEqual(stale.status, 0);
    assert.match(
      stale.stderr,
      new RegExp(
        String.raw`spoke-schemas dependency\).*expected: ${current}.*actual:   0\.11\.0`,
        "s",
      ),
    );

    const bumped = runReleaseScript("bump-version.mjs", [target], repoRoot);
    assert.equal(bumped.status, 0, bumped.stderr);
    assert.match(
      readFileSync(cratePath, "utf8"),
      /\[target\.'cfg\(windows\)'\.dependencies\]\nspoke-schemas = \{ version = "[^"]+", path = "\.\.\/spoke-schemas" \}/,
    );
    assert.match(
      readFileSync(cratePath, "utf8"),
      new RegExp(
        String.raw`\[target\.'cfg\(windows\)'\.dependencies\]\nspoke-schemas = \{ version = "${target}", path = "\.\.\/spoke-schemas" \}`,
      ),
    );
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
