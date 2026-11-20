import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  findBreakingCommits,
  sectionHasBreakingHeading,
} from "./assert-breaking-section.mjs";

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
});
