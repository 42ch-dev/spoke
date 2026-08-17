#!/usr/bin/env node
/**
 * Pre-check / re-probe whether the full spoke-connect platform-wheel set is
 * already published on PyPI at the release-tag SemVer.
 *
 * Skip semantics (architect-locked): the pre-check is the PRIMARY gate.
 * `pypa/gh-action-pypi-publish`'s `skip-existing` input is only a per-file
 * duplicate guard during an actual publish attempt — it never decides green.
 *
 * Expected set = the three platform wheels locked by
 * `tooling/connect/verify-python-wheels.sh` (count contract == 3, no sdist):
 *   spoke_connect-<ver>-py3-none-manylinux_2_35_x86_64.whl
 *   spoke_connect-<ver>-py3-none-macosx_11_0_arm64.whl
 *   spoke_connect-<ver>-py3-none-win_amd64.whl
 *
 * Usage (from repo root):
 *   RELEASE_TAG=v0.10.0 node tooling/release/check-pypi-published.mjs
 *   RELEASE_TAG=v0.10.0 node tooling/release/check-pypi-published.mjs --verify
 *
 * Default mode emits GitHub Actions step outputs `publish_needed`
 * (`true|false`) and `missing_files` (comma-joined) to stdout and
 * `$GITHUB_OUTPUT` when set. Exit 0 for both "already published" (skip) and
 * "absent / partial" (publish needed); exit non-zero on any doubt (network
 * error, non-200/404 response, malformed JSON, unexpected payload shape) —
 * fail loud, never skip on doubt.
 *
 * `--verify` mode (the unconditional `Confirm published set` re-probe) exits 0
 * only when the FULL expected set is verified present; absent/partial or any
 * probe error exits non-zero.
 */

import { appendFileSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { PYPI_CONNECT_PYPROJECT_PATH } from "./lockstep-surfaces.mjs";
import { SEMVER_PATTERN } from "./semver.mjs";

const REPO_ROOT = process.env.SPOKE_REPO_ROOT
  ? join(process.env.SPOKE_REPO_ROOT)
  : join(dirname(fileURLToPath(import.meta.url)), "../..");

/** Overridable for tests; production always probes pypi.org. */
const PYPI_BASE_URL = process.env.PYPI_BASE_URL ?? "https://pypi.org";

/** Platform tags locked by tooling/connect/verify-python-wheels.sh (wheels only, no sdist). */
const WHEEL_PLATFORM_TAGS = [
  "manylinux_2_35_x86_64",
  "macosx_11_0_arm64",
  "win_amd64",
];

/**
 * PEP 503 / wheel-filename normalization: lowercase; runs of `-`, `_`, `.`
 * collapse to `_`. `spoke-connect` → `spoke_connect`.
 *
 * @param {string} name Distribution name from pyproject.toml.
 * @returns {string}
 */
export function normalizeDistributionName(name) {
  return name.toLowerCase().replace(/[-_.]+/g, "_");
}

/**
 * Expected PyPI wheel filenames for a lockstep SemVer (no sdist).
 *
 * @param {string} packageName Distribution name (source of truth: pyproject.toml).
 * @param {string} version SemVer without the leading "v".
 * @returns {string[]}
 */
export function expectedWheelFilenames(packageName, version) {
  const dist = normalizeDistributionName(packageName);
  return WHEEL_PLATFORM_TAGS.map(
    (tag) => `${dist}-${version}-py3-none-${tag}.whl`,
  );
}

/**
 * Read `name = "…"` from the `[project]` table in pyproject.toml.
 *
 * @param {string} contents
 * @returns {string | null}
 */
export function parsePyprojectName(contents) {
  const projectSection = contents.match(/\[project\][\s\S]*?(?=\n\[|$)/);
  if (!projectSection) {
    return null;
  }
  const match = projectSection[0].match(/^name\s*=\s*"([^"]+)"/m);
  return match?.[1]?.trim() ?? null;
}

/**
 * Probe the PyPI JSON API for the version.
 *
 * @param {object} opts
 * @param {string} opts.packageName Distribution name (e.g. "spoke-connect").
 * @param {string} opts.version SemVer without the leading "v".
 * @param {string} [opts.baseUrl] Override for tests; defaults to pypi.org.
 * @returns {Promise<{ outcome: "published" | "absent" | "partial"; missingFiles: string[] }>}
 * @throws {Error} Fail-loud conditions (network, non-200/404, malformed JSON,
 *   unexpected payload shape).
 */
async function probePyPI({ packageName, version, baseUrl = PYPI_BASE_URL }) {
  const url = `${baseUrl.replace(/\/+$/, "")}/pypi/${packageName}/${version}/json`;
  const expected = expectedWheelFilenames(packageName, version);

  let response;
  try {
    response = await fetch(url, { signal: AbortSignal.timeout(30_000) });
  } catch (err) {
    throw new Error(`cannot reach PyPI JSON API at ${url}: ${err.message}`);
  }
  if (response.status === 404) {
    return { outcome: "absent", missingFiles: expected };
  }
  if (!response.ok) {
    throw new Error(`PyPI JSON API returned HTTP ${response.status} for ${url}`);
  }

  let payload;
  try {
    payload = await response.json();
  } catch (err) {
    throw new Error(`PyPI JSON API returned malformed JSON for ${url}: ${err.message}`);
  }
  if (!Array.isArray(payload?.urls)) {
    throw new Error(`PyPI JSON API payload missing "urls" array for ${url}`);
  }

  const published = new Set(
    payload.urls.map((entry) => entry?.filename).filter(Boolean),
  );
  const missingFiles = expected.filter((filename) => !published.has(filename));
  if (missingFiles.length === 0) {
    return { outcome: "published", missingFiles: [] };
  }
  return { outcome: "partial", missingFiles };
}

async function main() {
  const verify = process.argv.includes("--verify");

  const releaseTag = process.env.RELEASE_TAG?.trim();
  if (!releaseTag) {
    console.error(
      "check-pypi-published: RELEASE_TAG env var is required (e.g. v0.10.0 or v0.10.0-rc.1)",
    );
    process.exit(1);
  }
  if (!releaseTag.startsWith("v")) {
    console.error(
      `check-pypi-published: RELEASE_TAG must start with "v" (got "${releaseTag}")`,
    );
    process.exit(1);
  }
  const version = releaseTag.slice(1);
  if (!SEMVER_PATTERN.test(version)) {
    console.error(
      `check-pypi-published: RELEASE_TAG SemVer segment invalid (got "${version}")`,
    );
    process.exit(1);
  }

  let packageName;
  try {
    const contents = readFileSync(
      join(REPO_ROOT, PYPI_CONNECT_PYPROJECT_PATH),
      "utf8",
    );
    packageName = parsePyprojectName(contents);
    if (!packageName) {
      throw new Error(`missing [project] name`);
    }
  } catch (err) {
    console.error(
      `check-pypi-published: cannot read package name from ${PYPI_CONNECT_PYPROJECT_PATH}: ${err.message}`,
    );
    process.exit(1);
  }

  let outcome;
  try {
    outcome = await probePyPI({ packageName, version });
  } catch (err) {
    console.error(`check-pypi-published: ${err.message}`);
    process.exit(1);
  }

  const expected = expectedWheelFilenames(packageName, version);
  const missing = outcome.missingFiles.join(",");

  if (outcome.outcome === "published") {
    console.log(
      `check-pypi-published: ${packageName} ${version} already published on PyPI (${expected.length}/${expected.length} expected files present) — skip publish`,
    );
  } else if (outcome.outcome === "absent") {
    console.log(
      `check-pypi-published: ${packageName} ${version} not found on PyPI (HTTP 404) — publish needed`,
    );
  } else {
    console.log(
      `check-pypi-published: ${packageName} ${version} partially published on PyPI — publish needed; missing: ${missing}`,
    );
  }

  if (verify) {
    if (outcome.outcome !== "published") {
      console.error(
        `check-pypi-published: verify failed — expected files missing from PyPI: ${missing}`,
      );
      process.exit(1);
    }
    console.log("check-pypi-published: confirmed full expected set on PyPI");
    process.exit(0);
  }

  const publishNeeded = outcome.outcome !== "published";
  console.log(`publish_needed=${publishNeeded}`);
  console.log(`missing_files=${missing}`);
  const githubOutput = process.env.GITHUB_OUTPUT;
  if (githubOutput) {
    appendFileSync(githubOutput, `publish_needed=${publishNeeded}\n`);
    appendFileSync(githubOutput, `missing_files=${missing}\n`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
