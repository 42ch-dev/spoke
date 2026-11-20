import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { describe, it } from "node:test";
import { checkBreakingSection } from "./assert-breaking-section.mjs";

const SCRIPT = join(
  dirname(fileURLToPath(import.meta.url)),
  "assert-breaking-section.mjs",
);

/**
 * @param {(repo: string) => void} body
 */
function withTempRepo(body) {
  const repo = mkdtempSync(join(tmpdir(), "spoke-breaking-"));
  try {
    body(repo);
  } finally {
    rmSync(repo, { recursive: true, force: true });
  }
}

/**
 * @param {string} repo
 * @returns {import("node:child_process").SpawnSyncReturns<string>}
 */
function runCli(repo) {
  return spawnSync(process.execPath, [SCRIPT, "0.17.1"], {
    cwd: repo,
    env: { ...process.env, SPOKE_REPO_ROOT: repo },
    encoding: "utf8",
  });
}

describe("assert-breaking-section.mjs", () => {
  it("fails when a marker remains beside a Breaking heading", () => {
    const { ok, markers } = checkBreakingSection("0.17.1", {
      section:
        "### Breaking\n\n- **core:** Drop x\n\n### Fixed\n\n- **BREAKING:** Fix y\n",
    });
    assert.equal(ok, false);
    assert.equal(markers, 1);
  });

  it("passes a heading-only Breaking section", () => {
    const section = "### Breaking\n\n- **core:** Drop x\n";
    assert.deepEqual(checkBreakingSection("0.17.1", { section }), {
      ok: true,
      section,
      markers: 0,
    });
  });

  it("fails when a marker lands outside the Breaking heading", () => {
    const { ok, markers } = checkBreakingSection("0.17.1", {
      section:
        "### Added\n\n- **BREAKING:** Drop x\n- **BREAKING:** Drop y\n",
    });
    assert.equal(ok, false);
    assert.equal(markers, 2);
  });

  it("passes when no marker remains", () => {
    const section = "### Added\n\n- Add y\n";
    assert.equal(checkBreakingSection("0.17.1", { section }).ok, true);
  });

  it("passes an empty section", () => {
    assert.equal(checkBreakingSection("0.17.1", { section: "" }).ok, true);
    assert.equal(checkBreakingSection("0.17.1", { section: null }).ok, true);
  });

  it("CLI fails on a marker beside a Breaking heading", () => {
    withTempRepo((repo) => {
      writeFileSync(
        join(repo, "CHANGELOG.md"),
        "# Changelog\n\n## [0.17.1]\n\n### Breaking\n\n- **core:** Drop x\n\n### Fixed\n\n- **BREAKING:** Fix y\n",
      );
      const result = runCli(repo);
      assert.equal(result.status, 1, result.stderr);
      assert.match(
        result.stderr,
        /marker\(s\) in the 0\.17\.1 section.*anywhere in the section fails/,
      );
    });
  });

  it("CLI skips a missing changelog or an absent version section", () => {
    withTempRepo((repo) => {
      const missing = runCli(repo);
      assert.equal(missing.status, 0, missing.stderr);
      assert.match(missing.stderr, /version section is absent/);

      writeFileSync(
        join(repo, "CHANGELOG.md"),
        "# Changelog\n\n## [0.1.0]\n\nOld.\n",
      );
      const absent = runCli(repo);
      assert.equal(absent.status, 0, absent.stderr);
      assert.match(absent.stderr, /version section is absent/);
    });
  });

  it("CLI fails when CHANGELOG.md cannot be read for a reason other than ENOENT", () => {
    withTempRepo((repo) => {
      mkdirSync(join(repo, "CHANGELOG.md"));
      const result = runCli(repo);
      assert.notEqual(result.status, 0);
      assert.match(result.stderr, /cannot read CHANGELOG\.md/);
      assert.doesNotMatch(result.stderr, /ENOENT/);
    });
  });
});
