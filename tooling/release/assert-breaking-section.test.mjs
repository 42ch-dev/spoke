import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { describe, it } from "node:test";
import {
  checkBreakingSection,
  sectionHasBreakingHeading,
} from "./assert-breaking-section.mjs";

const SCRIPT = join(
  dirname(fileURLToPath(import.meta.url)),
  "assert-breaking-section.mjs",
);

describe("assert-breaking-section.mjs", () => {
  it("fails when a marker lands outside the Breaking heading", () => {
    const { ok, markers } = checkBreakingSection("0.17.1", {
      section:
        "### Added\n\n- **BREAKING:** Drop x\n- **BREAKING:** Drop y\n",
    });
    assert.equal(ok, false);
    assert.equal(markers, 2);
  });

  it("passes when the Breaking heading exists", () => {
    const section =
      "### Breaking\n\n- **core:** Drop x\n\n### Added\n\n- Add y\n";
    assert.equal(sectionHasBreakingHeading(section), true);
    assert.deepEqual(checkBreakingSection("0.17.1", { section }), {
      ok: true,
      section,
      markers: 0,
    });
  });

  it("passes when no marker remains", () => {
    const section = "### Added\n\n- Add y\n";
    assert.equal(checkBreakingSection("0.17.1", { section }).ok, true);
  });

  it("passes an empty section", () => {
    assert.equal(checkBreakingSection("0.17.1", { section: "" }).ok, true);
    assert.equal(checkBreakingSection("0.17.1", { section: null }).ok, true);
  });

  it("CLI fails on a marker outside the heading and passes cleanly", () => {
    const repo = mkdtempSync(join(tmpdir(), "spoke-breaking-"));
    try {
      const run = () =>
        spawnSync(process.execPath, [SCRIPT, "0.17.1"], {
          cwd: repo,
          env: { ...process.env, SPOKE_REPO_ROOT: repo },
          encoding: "utf8",
        });

      writeFileSync(
        join(repo, "CHANGELOG.md"),
        "# Changelog\n\n## [0.17.1]\n\n### Added\n\n- **BREAKING:** Drop x\n",
      );
      const failing = run();
      assert.equal(failing.status, 1, failing.stderr);
      assert.match(
        failing.stderr,
        /marker\(s\) outside "### Breaking"/,
      );

      writeFileSync(
        join(repo, "CHANGELOG.md"),
        "# Changelog\n\n## [0.17.1]\n\n### Breaking\n\n- **core:** Drop x\n",
      );
      const passing = run();
      assert.equal(passing.status, 0, passing.stderr);
    } finally {
      rmSync(repo, { recursive: true, force: true });
    }
  });
});
