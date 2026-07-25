/**
 * Shared harness for release script unit tests (temp lockstep fixtures).
 */

import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  CANONICAL_PATH,
  CARGO_LOCK_PATH,
  CARGO_OPS_CRATE_PATH,
  CARGO_SCHEMA_CRATE_PATH,
  CARGO_WORKSPACE_PATH,
  JSON_VERSION_PATHS,
  README_BADGE_PATHS,
} from "./lockstep-surfaces.mjs";

const RELEASE_DIR = dirname(fileURLToPath(import.meta.url));

export const REPO_ROOT = join(RELEASE_DIR, "../..");

/** @type {readonly string[]} */
export const LOCKSTEP_FIXTURE_PATHS = [
  CANONICAL_PATH,
  ...JSON_VERSION_PATHS,
  CARGO_WORKSPACE_PATH,
  CARGO_SCHEMA_CRATE_PATH,
  CARGO_OPS_CRATE_PATH,
  CARGO_LOCK_PATH,
  ...README_BADGE_PATHS,
  "cliff.toml",
];

/**
 * @returns {string}
 */
export function createTempRepo() {
  const dir = mkdtempSync(join(tmpdir(), "spoke-lockstep-"));
  for (const rel of LOCKSTEP_FIXTURE_PATHS) {
    const dest = join(dir, rel);
    mkdirSync(dirname(dest), { recursive: true });
    cpSync(join(REPO_ROOT, rel), dest);
  }
  return dir;
}

/**
 * @param {string} dir
 */
export function initGitRepo(dir) {
  const run = (args) =>
    spawnSync("git", args, { cwd: dir, encoding: "utf8", stdio: "ignore" });

  run(["init"]);
  run(["config", "user.email", "release-test@example.com"]);
  run(["config", "user.name", "Release Test"]);
  run(["add", "-A"]);
  run(["commit", "-m", "init"]);
}

/**
 * @param {string} dir
 */
export function cleanupTempRepo(dir) {
  rmSync(dir, { recursive: true, force: true });
}

/**
 * @param {string} scriptName
 * @param {string[]} args
 * @param {string} repoRoot
 */
export function runReleaseScript(scriptName, args, repoRoot) {
  const scriptPath = join(RELEASE_DIR, scriptName);
  return spawnSync(process.execPath, [scriptPath, ...args], {
    cwd: repoRoot,
    env: { ...process.env, SPOKE_REPO_ROOT: repoRoot },
    encoding: "utf8",
  });
}

/**
 * @param {string} repoRoot
 * @returns {string}
 */
export function readCanonicalVersion(repoRoot) {
  const data = JSON.parse(
    readFileSync(join(repoRoot, CANONICAL_PATH), "utf8"),
  );
  if (typeof data.version !== "string") {
    throw new Error(`${CANONICAL_PATH}: missing version`);
  }
  return data.version;
}
