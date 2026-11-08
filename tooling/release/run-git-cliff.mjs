#!/usr/bin/env node
/**
 * Resolve and run git-cliff for release changelog generation.
 *
 * Resolution order: workspace `node_modules/.bin/git-cliff`, `git-cliff` on PATH,
 * then pinned `pnpm dlx git-cliff@<version>`, then `npx git-cliff@<version>`.
 *
 * @module tooling/release/run-git-cliff
 */

import { existsSync, readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");

/**
 * @typedef {{ command: string; prefixArgs: string[] }} GitCliffInvocation
 */

/**
 * @param {string} command
 * @param {string[]} args
 * @returns {boolean}
 */
function commandExists(command, args) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    stdio: "ignore",
  });
  return result.status === 0;
}

/**
 * @returns {GitCliffInvocation | null}
 */
function resolveWorkspaceGitCliff() {
  const localBin = join(REPO_ROOT, "node_modules", ".bin", "git-cliff");
  if (!existsSync(localBin)) {
    return null;
  }

  if (commandExists(localBin, ["--version"])) {
    return { command: localBin, prefixArgs: [] };
  }

  return null;
}

/**
 * @returns {GitCliffInvocation}
 */
export function resolveGitCliffInvocation() {
  const workspace = resolveWorkspaceGitCliff();
  if (workspace) {
    return workspace;
  }

  if (commandExists("git-cliff", ["--version"])) {
    return { command: "git-cliff", prefixArgs: [] };
  }

  const version = JSON.parse(
    readFileSync(join(REPO_ROOT, "package.json"), "utf8"),
  ).devDependencies["git-cliff"];
  const pinnedPackage = `git-cliff@${version}`;

  if (commandExists("pnpm", ["--version"])) {
    return { command: "pnpm", prefixArgs: ["dlx", pinnedPackage] };
  }

  if (commandExists("npx", ["--version"])) {
    return { command: "npx", prefixArgs: [pinnedPackage] };
  }

  throw new Error(
    `git-cliff not found: install devDependency git-cliff@${version}, install git-cliff globally, or ensure pnpm/npx is available for dlx fallback.`,
  );
}

/**
 * @param {string[]} cliffArgs
 * @param {string} cwd
 * @returns {import("node:child_process").SpawnSyncReturns<string | Buffer>}
 */
export function runGitCliff(cliffArgs, cwd) {
  const { command, prefixArgs } = resolveGitCliffInvocation();
  return spawnSync(command, [...prefixArgs, ...cliffArgs], {
    cwd,
    stdio: "inherit",
  });
}

const isMain =
  process.argv[1] &&
  fileURLToPath(import.meta.url) === process.argv[1];

if (isMain) {
  const result = runGitCliff(process.argv.slice(2), process.cwd());
  process.exit(result.status ?? 1);
}
