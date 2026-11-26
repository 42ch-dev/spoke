import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { checkFreshness } from "./check-artifact-freshness.mjs";

const SCRIPT_PATH = fileURLToPath(
  new URL("./check-artifact-freshness.mjs", import.meta.url),
);

const SWIFT_PROVENANCE =
  "crates/spoke-connect/bindings/swift/xcframework/provenance.json";
const CPP_PROVENANCE =
  "crates/spoke-connect/bindings/cpp/native/provenance.json";

const IDENTITY = {
  GIT_AUTHOR_NAME: "Freshness Test",
  GIT_AUTHOR_EMAIL: "freshness-test@example.com",
  GIT_COMMITTER_NAME: "Freshness Test",
  GIT_COMMITTER_EMAIL: "freshness-test@example.com",
  GIT_AUTHOR_DATE: "2026-01-01T00:00:00Z",
  GIT_COMMITTER_DATE: "2026-01-01T00:00:00Z",
};

function git(repoRoot, args) {
  const result = spawnSync("git", args, {
    cwd: repoRoot,
    encoding: "utf8",
    env: { ...process.env, ...IDENTITY },
  });
  assert.equal(
    result.status,
    0,
    `git ${args.join(" ")} failed: ${result.stderr}`,
  );
  return result.stdout.trim();
}

function write(repoRoot, relPath, content) {
  const target = join(repoRoot, relPath);
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, content);
}

function createRepo(t) {
  const repoRoot = mkdtempSync(join(tmpdir(), "spoke-freshness-"));
  t.after(() => rmSync(repoRoot, { recursive: true, force: true }));
  git(repoRoot, ["init", "--quiet"]);
  return repoRoot;
}

function commit(repoRoot, message) {
  git(repoRoot, ["add", "-A"]);
  git(repoRoot, ["-c", "commit.gpgsign=false", "commit", "--quiet", "-m", message]);
  return git(repoRoot, ["rev-parse", "HEAD"]);
}

function runCli(repoRoot) {
  return spawnSync(process.execPath, [SCRIPT_PATH, "--repo", repoRoot], {
    encoding: "utf8",
    env: { ...process.env, ...IDENTITY },
  });
}

function lane(report, name) {
  return report.lanes.find((entry) => entry.lane === name);
}

function writeSwiftProvenance(repoRoot, { sourceRevision, recipeRevision }) {
  write(
    repoRoot,
    SWIFT_PROVENANCE,
    `${JSON.stringify(
      {
        schemaVersion: 1,
        xcframework: { sourceRevision, recipeRevision },
      },
      null,
      2,
    )}\n`,
  );
}

function writeCppProvenance(repoRoot, nativeArtifacts) {
  write(
    repoRoot,
    CPP_PROVENANCE,
    `${JSON.stringify({ schemaVersion: 1, nativeArtifacts }, null, 2)}\n`,
  );
}

/** Writes the compiled sources and both artifacts, then both sidecars. */
function seedRepo(t) {
  const repoRoot = createRepo(t);
  write(repoRoot, "Cargo.toml", "[workspace]\n");
  write(repoRoot, "Cargo.lock", "# lockfile\n");
  write(
    repoRoot,
    "crates/spoke-connect/Cargo.toml",
    '[package]\nname = "spoke-connect"\n',
  );
  write(repoRoot, "crates/spoke-connect/src/lib.rs", "// source v1\n");
  write(
    repoRoot,
    "crates/spoke-connect-capi/Cargo.toml",
    '[package]\nname = "spoke-connect-capi"\n',
  );
  write(repoRoot, "crates/spoke-connect-capi/src/lib.rs", "// capi v1\n");
  write(
    repoRoot,
    "crates/spoke-connect/bindings/cpp/native/osx-arm64/libspoke_connect_capi.dylib",
    "binary\n",
  );
  write(
    repoRoot,
    "crates/spoke-connect/bindings/swift/xcframework/spoke_connectFFI.xcframework/Info.plist",
    "plist\n",
  );
  const baseline = commit(repoRoot, "build(connect): initial sources");

  writeSwiftProvenance(repoRoot, {
    sourceRevision: baseline,
    recipeRevision: baseline,
  });
  writeCppProvenance(repoRoot, {
    "osx-arm64": { sourceRevision: baseline, recipeRevision: baseline },
  });
  commit(repoRoot, "build(connect): record provenance");

  return { repoRoot, baseline };
}

test("unrelated docs commit after the writeback is fresh", (t) => {
  const { repoRoot } = seedRepo(t);

  write(repoRoot, "docs/notes.md", "# notes\n");
  commit(repoRoot, "docs: unrelated note");

  const report = checkFreshness({ repoRoot });
  for (const name of ["swift", "cpp"]) {
    assert.equal(lane(report, name).status, "fresh", name);
  }
  assert.equal(runCli(repoRoot).status, 0);
});

test("test-only change after the writeback is fresh", (t) => {
  const { repoRoot } = seedRepo(t);

  write(repoRoot, "crates/spoke-connect/tests/freshness.rs", "// test\n");
  commit(repoRoot, "test(connect): cover freshness");

  const report = checkFreshness({ repoRoot });
  for (const name of ["swift", "cpp"]) {
    assert.equal(lane(report, name).status, "fresh", name);
  }
});

test("compiled source commit after the writeback is stale", (t) => {
  const { repoRoot, baseline } = seedRepo(t);

  write(repoRoot, "crates/spoke-connect/src/lib.rs", "// source v2\n");
  const offending = commit(repoRoot, "feat(connect): change source");

  const report = checkFreshness({ repoRoot });
  for (const name of ["swift", "cpp"]) {
    const entry = lane(report, name);
    const label =
      name === "swift"
        ? "xcframework sourceRevision"
        : "nativeArtifacts.osx-arm64 sourceRevision";
    assert.equal(entry.status, "stale", name);
    assert.deepEqual(
      entry.stale,
      [{ base: baseline, label, commit: offending }],
      name,
    );
    assert.deepEqual(entry.errors, []);
  }
  assert.equal(runCli(repoRoot).status, 1);
});

test("a refreshed C++ rid does not clear another rid's staleness", (t) => {
  const repoRoot = createRepo(t);
  write(repoRoot, "Cargo.toml", "[workspace]\n");
  write(repoRoot, "crates/spoke-connect/src/lib.rs", "// source v1\n");
  write(
    repoRoot,
    "crates/spoke-connect/bindings/swift/xcframework/spoke_connectFFI.xcframework/Info.plist",
    "plist\n",
  );
  write(
    repoRoot,
    "crates/spoke-connect/bindings/cpp/native/osx-arm64/libspoke_connect_capi.dylib",
    "binary\n",
  );
  write(
    repoRoot,
    "crates/spoke-connect/bindings/cpp/native/win-x64/spoke_connect_capi.dll",
    "binary\n",
  );
  const oldBaseline = commit(repoRoot, "build(connect): initial sources");

  write(repoRoot, "crates/spoke-connect/src/lib.rs", "// source v2\n");
  const sourceChange = commit(repoRoot, "feat(connect): change source");

  writeSwiftProvenance(repoRoot, {
    sourceRevision: sourceChange,
    recipeRevision: oldBaseline,
  });
  writeCppProvenance(repoRoot, {
    "osx-arm64": {
      sourceRevision: oldBaseline,
      recipeRevision: oldBaseline,
    },
    "win-x64": {
      sourceRevision: sourceChange,
      recipeRevision: oldBaseline,
    },
  });
  commit(repoRoot, "build(connect): refresh win-x64");

  const report = checkFreshness({ repoRoot });
  assert.equal(lane(report, "swift").status, "fresh");
  const cpp = lane(report, "cpp");
  assert.equal(cpp.status, "stale");
  assert.deepEqual(cpp.stale, [
    {
      base: oldBaseline,
      label: "nativeArtifacts.osx-arm64 sourceRevision",
      commit: sourceChange,
    },
  ]);
});

test("two stale C++ rids keep distinct attribution", (t) => {
  const repoRoot = createRepo(t);
  write(repoRoot, "Cargo.toml", "[workspace]\n");
  write(repoRoot, "crates/spoke-connect/src/lib.rs", "// source v1\n");
  write(
    repoRoot,
    "crates/spoke-connect/bindings/cpp/native/osx-arm64/libspoke_connect_capi.dylib",
    "binary\n",
  );
  write(
    repoRoot,
    "crates/spoke-connect/bindings/cpp/native/win-x64/spoke_connect_capi.dll",
    "binary\n",
  );
  const baseline = commit(repoRoot, "build(connect): initial sources");

  write(repoRoot, "crates/spoke-connect/src/lib.rs", "// source v2\n");
  const sourceChange = commit(repoRoot, "feat(connect): change source");

  writeSwiftProvenance(repoRoot, {
    sourceRevision: sourceChange,
    recipeRevision: baseline,
  });
  writeCppProvenance(repoRoot, {
    "osx-arm64": { sourceRevision: baseline, recipeRevision: baseline },
    "win-x64": { sourceRevision: baseline, recipeRevision: baseline },
  });
  commit(repoRoot, "build(connect): record provenance");

  const cpp = lane(checkFreshness({ repoRoot }), "cpp");
  assert.equal(cpp.status, "stale");
  assert.deepEqual(
    cpp.stale.map((entry) => entry.label),
    [
      "nativeArtifacts.osx-arm64 sourceRevision",
      "nativeArtifacts.win-x64 sourceRevision",
    ],
  );
  assert.deepEqual(
    cpp.stale.map((entry) => entry.base),
    [baseline, baseline],
  );
});

test("a capi-only change is stale for C++ and fresh for Swift", (t) => {
  const { repoRoot } = seedRepo(t);

  write(repoRoot, "crates/spoke-connect-capi/src/lib.rs", "// capi v2\n");
  commit(repoRoot, "feat(capi): change carrier source");

  const report = checkFreshness({ repoRoot });
  assert.equal(lane(report, "swift").status, "fresh");
  assert.equal(lane(report, "cpp").status, "stale");
});

test("missing sidecar is invalid, not fresh", (t) => {
  const repoRoot = createRepo(t);
  write(repoRoot, "Cargo.toml", "[workspace]\n");
  write(repoRoot, "crates/spoke-connect/src/lib.rs", "// source\n");
  commit(repoRoot, "build(connect): sources without provenance");

  const report = checkFreshness({ repoRoot });
  for (const name of ["swift", "cpp"]) {
    const entry = lane(report, name);
    assert.equal(entry.status, "invalid", name);
    assert.deepEqual(entry.stale, [], name);
    assert.ok(entry.errors.length > 0, name);
  }
  assert.equal(runCli(repoRoot).status, 1);
});

test("malformed sidecar is invalid, not fresh", (t) => {
  const { repoRoot } = seedRepo(t);

  write(repoRoot, SWIFT_PROVENANCE, "[]\n");
  write(repoRoot, CPP_PROVENANCE, "{}\n");
  commit(repoRoot, "build(connect): record unusable provenance");

  const report = checkFreshness({ repoRoot });
  for (const name of ["swift", "cpp"]) {
    const entry = lane(report, name);
    assert.equal(entry.status, "invalid", name);
    assert.deepEqual(entry.stale, [], name);
    assert.ok(entry.errors.length > 0, name);
  }
  assert.equal(runCli(repoRoot).status, 1);
});

test("sourceRevision HEAD is invalid, not fresh", (t) => {
  const { repoRoot, baseline } = seedRepo(t);

  writeSwiftProvenance(repoRoot, {
    sourceRevision: "HEAD",
    recipeRevision: baseline,
  });
  commit(repoRoot, "build(connect): record malformed revision");

  const report = checkFreshness({ repoRoot });
  const swift = lane(report, "swift");
  assert.equal(swift.status, "invalid");
  assert.deepEqual(swift.stale, []);
  assert.ok(swift.errors.length > 0);
  assert.equal(runCli(repoRoot).status, 1);
});

test("unresolvable recorded revision is invalid", (t) => {
  const { repoRoot, baseline } = seedRepo(t);

  writeCppProvenance(repoRoot, {
    "osx-arm64": { sourceRevision: "f".repeat(40), recipeRevision: baseline },
  });
  commit(repoRoot, "build(connect): record unresolvable revision");

  const report = checkFreshness({ repoRoot });
  const cpp = lane(report, "cpp");
  assert.equal(cpp.status, "invalid");
  assert.deepEqual(cpp.stale, []);
  assert.ok(cpp.errors.length > 0);
});

test("a revision outside HEAD history is invalid", (t) => {
  const repoRoot = createRepo(t);
  write(repoRoot, "Cargo.toml", "[workspace]\n");
  write(repoRoot, "crates/spoke-connect/src/lib.rs", "// source v1\n");
  const baseline = commit(repoRoot, "build(connect): initial sources");

  const defaultBranch = git(repoRoot, ["branch", "--show-current"]);
  git(repoRoot, ["checkout", "--quiet", "-b", "side"]);
  write(repoRoot, "docs/side.md", "# side\n");
  const sideRevision = commit(repoRoot, "docs: side branch commit");
  git(repoRoot, ["checkout", "--quiet", defaultBranch]);

  writeSwiftProvenance(repoRoot, {
    sourceRevision: sideRevision,
    recipeRevision: baseline,
  });
  writeCppProvenance(repoRoot, {
    "osx-arm64": { sourceRevision: baseline, recipeRevision: baseline },
  });
  commit(repoRoot, "build(connect): record foreign revision");

  const report = checkFreshness({ repoRoot });
  const swift = lane(report, "swift");
  assert.equal(swift.status, "invalid");
  assert.deepEqual(swift.stale, []);
  assert.match(swift.errors.join("\n"), /not an ancestor of HEAD/);
  assert.equal(runCli(repoRoot).status, 1);
});

test("CLI exits 2 when the check cannot run", (t) => {
  const notARepo = mkdtempSync(join(tmpdir(), "spoke-freshness-norepo-"));
  t.after(() => rmSync(notARepo, { recursive: true, force: true }));

  const result = runCli(notARepo);
  assert.equal(result.status, 2, result.stderr);
  assert.ok(JSON.parse(result.stdout).error);
});
