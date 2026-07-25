#!/usr/bin/env node
/**
 * Resolve and run git-cliff for release changelog generation.
 *
 * Resolution order: `git-cliff` on PATH, then `pnpm dlx git-cliff`, then `npx git-cliff`.
 *
 * @module tooling/release/run-git-cliff
 */

import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

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
 * @returns {GitCliffInvocation}
 */
export function resolveGitCliffInvocation() {
  if (commandExists("git-cliff", ["--version"])) {
    return { command: "git-cliff", prefixArgs: [] };
  }

  if (commandExists("pnpm", ["--version"])) {
    return { command: "pnpm", prefixArgs: ["dlx", "git-cliff"] };
  }

  if (commandExists("npx", ["--version"])) {
    return { command: "npx", prefixArgs: ["git-cliff"] };
  }

  throw new Error(
    "git-cliff not found: install git-cliff, or ensure pnpm/npx is available for dlx fallback.",
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
