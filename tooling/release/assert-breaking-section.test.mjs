import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { describe, it } from "node:test";
import {
  findBreakingCommits,
  sectionHasBreakingHeading,
} from "./assert-breaking-section.mjs";

const SCRIPT = join(
  dirname(fileURLToPath(import.meta.url)),
  "assert-breaking-section.mjs",
);
const GIT_AVAILABLE = spawnSync("git", ["--version"], { stdio: "ignore" })
  .status === 0;

/**
 * @param {string} cwd
 * @param {...string} args
 * @returns {string}
 */
function git(cwd, ...args) {
  const result = spawnSync("git", args, { cwd, encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim();
}

describe("assert-breaking-section.mjs", () => {
  it("detects a breaking header when the section lacks the heading", () => {
    const breaking = findBreakingCommits([
      { subject: "feat(core)!: drop x" },
      { subject: "feat(core): add y" },
    ]);
    assert.deepEqual(
      breaking.map((commit) => commit.subject),
      ["feat(core)!: drop x"],
    );
    assert.equal(
      sectionHasBreakingHeading("### Added\n\n- **core:** Add y\n"),
      false,
    );
  });

  it("detects a footer-only breaking commit", () => {
    const breaking = findBreakingCommits([
      { subject: "fix: adjust wire fields", body: "Details.\n\nBREAKING CHANGE: field removed." },
      { subject: "fix: plain", body: "No footer." },
    ]);
    assert.deepEqual(
      breaking.map((commit) => commit.subject),
      ["fix: adjust wire fields"],
    );
  });

  it("passes when the section has the Breaking heading", () => {
    const breaking = findBreakingCommits([{ subject: "feat(core)!: drop x" }]);
    assert.equal(breaking.length, 1);
    assert.equal(
      sectionHasBreakingHeading(
        "### Breaking\n\n- **core:** Drop x\n\n### Added\n\n- Add y\n",
      ),
      true,
    );
    assert.equal(
      sectionHasBreakingHeading("#### Breaking\n\n- nested heading\n"),
      false,
    );
  });

  it("passes when no commit is breaking", () => {
    assert.deepEqual(
      findBreakingCommits([
        { subject: "feat(core): add y" },
        { subject: "chore: tidy" },
        { subject: "docs: explain", body: "No footer." },
      ]),
      [],
    );
  });

  it("matches scope variants", () => {
    const breaking = findBreakingCommits([
      { subject: "feat(connect)!: drop capability" },
      { subject: "fix!: drop helper" },
    ]);
    assert.equal(breaking.length, 2);
  });

  it(
    "CLI fails on missing or headingless sections and passes with the heading",
    { skip: !GIT_AVAILABLE && "git is unavailable" },
    () => {
      const repo = mkdtempSync(join(tmpdir(), "spoke-breaking-"));
      try {
        git(repo, "init", "-q");
        git(repo, "config", "user.email", "release-test@example.com");
        git(repo, "config", "user.name", "Release Test");
        git(repo, "commit", "--allow-empty", "-q", "-m", "chore: init");
        const base = git(repo, "rev-parse", "HEAD");
        git(repo, "commit", "--allow-empty", "-q", "-m", "feat(core)!: drop x");
        const run = () =>
          spawnSync(
            process.execPath,
            [SCRIPT, "v0.17.1", "--from", base],
            {
              cwd: repo,
              env: { ...process.env, SPOKE_REPO_ROOT: repo },
              encoding: "utf8",
            },
          );

        const missing = run();
        assert.equal(missing.status, 1, missing.stderr);
        assert.match(missing.stderr, /has no section for v0\.17\.1/);

        writeFileSync(
          join(repo, "CHANGELOG.md"),
          "# Changelog\n\n## [0.17.1]\n\n### Added\n\n- New.\n",
        );
        const headingless = run();
        assert.equal(headingless.status, 1, headingless.stderr);
        assert.match(headingless.stderr, /feat\(core\)!: drop x/);
        assert.match(headingless.stderr, /no "### Breaking"/);

        writeFileSync(
          join(repo, "CHANGELOG.md"),
          "# Changelog\n\n## [0.17.1]\n\n### Breaking\n\n- **core:** Drop x\n",
        );
        const passing = run();
        assert.equal(passing.status, 0, passing.stderr);
      } finally {
        rmSync(repo, { recursive: true, force: true });
      }
    },
  );
});
