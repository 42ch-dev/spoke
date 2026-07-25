#!/usr/bin/env node
/**
 * Assert target SemVer is strictly greater than the canonical package.json version.
 *
 * CLI: node tooling/release/assert-version-greater.mjs <X.Y.Z>
 *
 * Exit 0 when target > current; non-zero otherwise.
 *
 * @module tooling/release/assert-version-greater
 */

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { CANONICAL_PATH } from "./lockstep-surfaces.mjs";
import {
  SEMVER_PATTERN,
  isSemVerGreater,
  parseSemVer,
} from "./semver.mjs";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");

const target = process.argv[2]?.trim();
if (!target || target === "--help" || target === "-h") {
  console.log(
    "Usage: node tooling/release/assert-version-greater.mjs <X.Y.Z>",
  );
  process.exit(target ? 0 : 1);
}

if (!SEMVER_PATTERN.test(target) || !parseSemVer(target)) {
  console.error(`Invalid SemVer: ${target}`);
  process.exit(1);
}

const canonical = JSON.parse(
  readFileSync(join(REPO_ROOT, CANONICAL_PATH), "utf8"),
).version;

if (typeof canonical !== "string" || !parseSemVer(canonical)) {
  console.error(
    `${CANONICAL_PATH}: missing or invalid "version" field (${canonical})`,
  );
  process.exit(1);
}

if (target === canonical) {
  console.error(
    `Target version ${target} equals current ${CANONICAL_PATH} version; cut must advance.`,
  );
  process.exit(1);
}

if (!isSemVerGreater(target, canonical)) {
  console.error(
    `Target version ${target} must be greater than current ${canonical} (${CANONICAL_PATH}).`,
  );
  process.exit(1);
}

console.log(`Version OK: ${target} > ${canonical}`);
