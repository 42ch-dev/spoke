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
 *   RELEASE_TAG=v0.10.0 node tooling/release/check-pypi-published.mjs --verify --verify-retry-seconds 120
 *
 * Default mode emits GitHub Actions step outputs `publish_needed`
 * (`true|false`) and `missing_files` (comma-joined) to stdout and
 * `$GITHUB_OUTPUT` when set. Exit 0 for both "already published" (skip) and
 * "absent / partial" (publish needed); exit non-zero on any doubt (network
 * error, non-200/404 response, malformed JSON, unexpected payload shape,
 * RELEASE_TAG/pyproject.toml version mismatch) — fail loud, never skip on
 * doubt. `urls[]` entries with `yanked: true` (PEP 592) count as absent.
 * Registry entries outside the expected set are logged to stderr as a
 * warning and never change the verdict (expected-set-subset semantics).
 *
 * `--verify` mode (the unconditional `Confirm published set` re-probe) exits 0
 * only when the FULL expected set is verified present; absent/partial or any
 * probe error exits non-zero. With `--verify-retry-seconds <n>` (default 0 =
 * single-shot), absent/partial probes are retried with backoff (2s, 4s, 8s…,
 * capped at 30s; base overridable via PYPI_VERIFY_RETRY_BASE_MS for tests)
 * until the deadline, tolerating PyPI JSON API propagation delay after an
 * upload; on deadline expiry it still fails loud with the missing-files
 * message. Probe errors are never retried, and the pre-check path stays
 * single-shot regardless of the flag.
 */

import { appendFileSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  parsePyprojectVersion,
  PYPI_CONNECT_PYPROJECT_PATH,
} from "./lockstep-surfaces.mjs";
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
 * PEP 440 normalize a version string for wheel-filename construction.
 *
 * Wheel filenames (PEP 427) embed the PEP 440-normalized version: prerelease
 * segments lose their separators and use the canonical letters (`alpha`/`a`
 * → `a`, `beta`/`b` → `b`, `pre`/`preview`/`c`/`rc` → `rc`), and post/dev
 * segments use `.postN` / `.devN`. Stable versions are already canonical and
 * pass through unchanged.
 *
 * Examples: `0.2.0-alpha.1` → `0.2.0a1`; `0.2.0-beta.1` → `0.2.0b1`;
 * `0.2.0-rc.1` → `0.2.0rc1`; `0.2.0-rc1` → `0.2.0rc1`; `0.2.0-dev.3` →
 * `0.2.0.dev3`.
 *
 * @param {string} version SemVer without the leading "v".
 * @returns {string}
 */
export function normalizePep440Version(version) {
  const hyphen = version.indexOf("-");
  if (hyphen === -1) {
    return version;
  }
  const release = version.slice(0, hyphen);
  const ids = version.slice(hyphen + 1).split(".");

  /** @type {Record<string, string>} PEP 440 prerelease canonical letters. */
  const PRE_LETTERS = {
    a: "a",
    alpha: "a",
    b: "b",
    beta: "b",
    c: "rc",
    pre: "rc",
    preview: "rc",
    rc: "rc",
  };

  let pre = "";
  let post = "";
  let dev = "";
  for (let i = 0; i < ids.length; i += 1) {
    const match = ids[i].match(/^([A-Za-z]+)([0-9]*)$/);
    if (!match) {
      continue; // bare numeric identifier — number for the preceding marker
    }
    const word = match[1].toLowerCase();
    let num = match[2];
    if (num === "" && i + 1 < ids.length && /^[0-9]+$/.test(ids[i + 1])) {
      num = ids[i + 1];
      i += 1;
    }
    if (PRE_LETTERS[word] !== undefined) {
      pre = `${PRE_LETTERS[word]}${num || "0"}`;
    } else if (word === "post" || word === "rev" || word === "r") {
      post = `.post${num || "0"}`;
    } else if (word === "dev") {
      dev = `.dev${num || "0"}`;
    }
  }
  return `${release}${pre}${post}${dev}`;
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
  const normalizedVersion = normalizePep440Version(version);
  return WHEEL_PLATFORM_TAGS.map(
    (tag) => `${dist}-${normalizedVersion}-py3-none-${tag}.whl`,
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
 * @returns {Promise<{ outcome: "published" | "absent" | "partial"; missingFiles: string[]; unexpectedEntries: string[] }>}
 *   `unexpectedEntries` lists `urls[]` filenames outside the expected set
 *   (observability only — subset semantics, the verdict never depends on it).
 *   `urls[]` entries with `yanked: true` (PEP 592) count as absent.
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
    return { outcome: "absent", missingFiles: expected, unexpectedEntries: [] };
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

  const expectedSet = new Set(expected);
  const unexpectedEntries = payload.urls
    .map((entry) => entry?.filename)
    .filter((filename) => typeof filename === "string" && !expectedSet.has(filename));
  const published = new Set(
    payload.urls
      .filter((entry) => entry && entry.yanked !== true)
      .map((entry) => entry?.filename)
      .filter(Boolean),
  );
  const missingFiles = expected.filter((filename) => !published.has(filename));
  if (missingFiles.length === 0) {
    return { outcome: "published", missingFiles: [], unexpectedEntries };
  }
  return { outcome: "partial", missingFiles, unexpectedEntries };
}

/** Backoff cap for --verify retries (2s, 4s, 8s… capped here). */
const VERIFY_RETRY_MAX_DELAY_MS = 30_000;

/**
 * Bounded retry for the `--verify` re-probe. PyPI's JSON API can lag behind
 * an upload by seconds, so a single-shot 404 immediately after publish is a
 * false red. Absent/partial probes are re-probed with exponential backoff
 * until the deadline; on deadline expiry the last outcome is returned and the
 * caller fails loud with the missing-files message. Probe errors (network,
 * non-404 HTTP, malformed JSON) propagate on the first attempt, and the
 * pre-check path is single-shot (retrySeconds = 0).
 *
 * @param {object} opts
 * @param {string} opts.packageName
 * @param {string} opts.version
 * @param {boolean} opts.verify
 * @param {number} opts.retrySeconds Retry deadline in seconds; 0 = single-shot.
 * @param {number} opts.retryBaseMs Backoff base; tests override via
 *   PYPI_VERIFY_RETRY_BASE_MS to keep sleeps short.
 * @returns {Promise<object>} Final probe outcome — published, or
 *   absent/partial once the deadline is exhausted.
 */
async function probeWithVerifyRetry({
  packageName,
  version,
  verify,
  retrySeconds,
  retryBaseMs,
}) {
  const deadlineMs =
    verify && retrySeconds > 0 ? Date.now() + retrySeconds * 1000 : 0;
  for (let attempt = 0; ; attempt += 1) {
    const outcome = await probePyPI({ packageName, version });
    if (outcome.outcome === "published" || deadlineMs === 0) {
      return outcome;
    }
    const delayMs = Math.min(
      VERIFY_RETRY_MAX_DELAY_MS,
      retryBaseMs * 2 ** attempt,
      Math.max(0, deadlineMs - Date.now()),
    );
    if (delayMs <= 0) {
      return outcome;
    }
    await new Promise((resolve) => setTimeout(resolve, delayMs));
  }
}

async function main() {
  const verify = process.argv.includes("--verify");
  let verifyRetrySeconds = 0;
  const retryFlagIdx = process.argv.indexOf("--verify-retry-seconds");
  if (retryFlagIdx !== -1) {
    const raw = process.argv[retryFlagIdx + 1];
    verifyRetrySeconds = Number(raw);
    if (!Number.isFinite(verifyRetrySeconds) || verifyRetrySeconds < 0) {
      console.error(
        `check-pypi-published: --verify-retry-seconds expects a non-negative number of seconds (got "${raw}")`,
      );
      process.exit(1);
    }
  }
  const retryBaseRaw = Number(process.env.PYPI_VERIFY_RETRY_BASE_MS);
  const retryBaseMs =
    Number.isFinite(retryBaseRaw) && retryBaseRaw > 0 ? retryBaseRaw : 2000;

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
  let manifestVersion;
  try {
    const contents = readFileSync(
      join(REPO_ROOT, PYPI_CONNECT_PYPROJECT_PATH),
      "utf8",
    );
    packageName = parsePyprojectName(contents);
    if (!packageName) {
      throw new Error(`missing [project] name`);
    }
    manifestVersion = parsePyprojectVersion(contents);
    if (!manifestVersion) {
      throw new Error(`missing [project] version`);
    }
  } catch (err) {
    console.error(
      `check-pypi-published: cannot read package name and version from ${PYPI_CONNECT_PYPROJECT_PATH}: ${err.message}`,
    );
    process.exit(1);
  }

  // The registry path is derived from the tag; cross-check it against the
  // lockstep version surface so a mismatched tag never probes a wrong version.
  if (manifestVersion !== version) {
    console.error(
      `check-pypi-published: ${PYPI_CONNECT_PYPROJECT_PATH} version "${manifestVersion}" does not match RELEASE_TAG version "${version}"`,
    );
    process.exit(1);
  }

  let outcome;
  try {
    outcome = await probeWithVerifyRetry({
      packageName,
      version,
      verify,
      retrySeconds: verifyRetrySeconds,
      retryBaseMs,
    });
  } catch (err) {
    console.error(`check-pypi-published: ${err.message}`);
    process.exit(1);
  }

  if (outcome.unexpectedEntries.length > 0) {
    console.error(
      `check-pypi-published: warning — registry lists files outside the expected set for ${packageName} ${version}: ${outcome.unexpectedEntries.join(", ")}`,
    );
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
