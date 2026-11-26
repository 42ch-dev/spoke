#!/usr/bin/env node
/**
 * Recorded-provenance artifact freshness check.
 *
 * Each lane compares the build-input revisions recorded in its committed
 * provenance sidecar to HEAD over the lane's compiled-source and recipe path
 * sets. A lane is stale only when a recorded revision's range `base..HEAD`
 * touched one of those inputs, so a commit that does not change the build
 * inputs never reports the lane stale. A refresh that rewrites one platform's
 * entry clears only that entry's staleness.
 *
 *   swift → `.xcframework.sourceRevision` / `.xcframework.recipeRevision`
 *   cpp   → each `nativeArtifacts[rid].sourceRevision` / `.recipeRevision`
 *
 * Every recorded revision must be a full 40/64-hex commit that resolves in
 * this checkout and is an ancestor of HEAD. A missing sidecar, a malformed
 * sidecar, or a missing, malformed, unresolvable or non-ancestor revision is
 * an invalid record — the lane reports `invalid`, never `fresh`.
 *
 * Usage:
 *   node tooling/release/check-artifact-freshness.mjs [--repo <path>]
 *
 * Prints a deterministic `{ "lanes": [ ... ] }` JSON report.
 * Exit 0 when every lane is fresh, 1 when a lane is stale or invalid,
 * 2 when the check itself could not run.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { pathToFileURL } from "node:url";

const CONNECT_SOURCE = [
  "crates/spoke-connect/src",
  "crates/spoke-connect/Cargo.toml",
  "crates/spoke-connect/build.rs",
];

const SCHEMAS_SOURCE = [
  "crates/spoke-schemas/src",
  "crates/spoke-schemas/Cargo.toml",
];

const OPERATIONS_SOURCE = [
  "crates/spoke-operations/src",
  "crates/spoke-operations/Cargo.toml",
];

/** Shared inputs of both native builds: manifest, lockfile, toolchain, Rust sources. */
const COMMON_SOURCE = [
  "Cargo.toml",
  "Cargo.lock",
  ".cargo",
  "rust-toolchain",
  "rust-toolchain.toml",
  ...CONNECT_SOURCE,
  ...SCHEMAS_SOURCE,
  ...OPERATIONS_SOURCE,
];

/** The C++ carrier additionally compiles the capi crate. */
const CPP_SOURCE = [
  "Cargo.toml",
  "Cargo.lock",
  ".cargo",
  "rust-toolchain",
  "rust-toolchain.toml",
  "crates/spoke-connect-capi/src",
  "crates/spoke-connect-capi/Cargo.toml",
  ...CONNECT_SOURCE,
  ...SCHEMAS_SOURCE,
  ...OPERATIONS_SOURCE,
];

/** The Swift bundle selects `spoke-connect` only, never the capi crate. */
const SWIFT_SOURCE = COMMON_SOURCE;

/** Recipe files that define how the C++ carriers are built. */
const CPP_RECIPE = [
  "tooling/connect/cpp-build.mjs",
  ".github/workflows/cpp-connect.yml",
];

/** Recipe files that define how the Swift bundle is built. */
const SWIFT_RECIPE = [
  "tooling/connect/build-swift-xcframework.sh",
  "tooling/connect/xcframework-manifest.sh",
  "tooling/connect/normalize-cargo-version.mjs",
  "tooling/release/lockstep-surfaces.mjs",
  ".github/workflows/xcframework.yml",
];

/** Full git object names recorded in provenance: SHA-1 or SHA-256. */
const COMMIT_PATTERN = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/i;

const LANES = [
  {
    lane: "swift",
    provenance:
      "crates/spoke-connect/bindings/swift/xcframework/provenance.json",
    source: SWIFT_SOURCE,
    recipe: SWIFT_RECIPE,
    /**
     * @param {Record<string, unknown>} data
     * @param {string[]} errors
     */
    entries(data, errors) {
      if (!isRecord(data.xcframework)) {
        errors.push("xcframework is missing or not an object");
        return [];
      }
      return [{ label: "xcframework", record: data.xcframework }];
    },
  },
  {
    lane: "cpp",
    provenance: "crates/spoke-connect/bindings/cpp/native/provenance.json",
    source: CPP_SOURCE,
    recipe: CPP_RECIPE,
    /**
     * @param {Record<string, unknown>} data
     * @param {string[]} errors
     */
    entries(data, errors) {
      if (!isRecord(data.nativeArtifacts)) {
        errors.push("nativeArtifacts is missing or not an object");
        return [];
      }
      const rids = Object.keys(data.nativeArtifacts).sort();
      if (rids.length === 0) {
        errors.push("nativeArtifacts records no entries");
        return [];
      }
      const entries = [];
      for (const rid of rids) {
        const entry = data.nativeArtifacts[rid];
        if (!isRecord(entry)) {
          errors.push(`nativeArtifacts.${rid} is not an object`);
          continue;
        }
        entries.push({ label: `nativeArtifacts.${rid}`, record: entry });
      }
      return entries;
    },
  },
];

/**
 * @param {string} repoRoot
 * @param {string[]} args
 */
function git(repoRoot, args) {
  const result = spawnSync("git", args, {
    cwd: repoRoot,
    encoding: "utf8",
  });
  if (result.error) {
    throw new Error(
      `git ${args.join(" ")} could not run: ${result.error.message}`,
    );
  }
  return {
    ok: result.status === 0,
    stdout: (result.stdout ?? "").trim(),
    stderr: (result.stderr ?? "").trim(),
  };
}

/**
 * Git call the check cannot proceed without.
 * @param {string} repoRoot
 * @param {string[]} args
 */
function expectGit(repoRoot, args) {
  const result = git(repoRoot, args);
  if (!result.ok) {
    throw new Error(
      `git ${args.join(" ")} failed: ${result.stderr || "unknown error"}`,
    );
  }
  return result.stdout;
}

/**
 * @param {string} repoRoot
 * @param {string} relPath
 */
function readJson(repoRoot, relPath) {
  try {
    return {
      data: JSON.parse(readFileSync(join(repoRoot, relPath), "utf8")),
    };
  } catch (error) {
    if (error.code === "ENOENT") return { error: "is missing" };
    return { error: `is not valid JSON (${error.message})` };
  }
}

/**
 * @param {unknown} value
 * @returns {value is Record<string, unknown>}
 */
function isRecord(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * A recorded revision is usable only as a full object name that resolves to a
 * commit in this checkout.
 * @param {string} repoRoot
 * @param {unknown} value
 * @returns {string | null}
 */
function resolveCommit(repoRoot, value) {
  if (typeof value !== "string") return null;
  const candidate = value.trim();
  if (!COMMIT_PATTERN.test(candidate)) return null;
  return git(repoRoot, ["cat-file", "-e", `${candidate}^{commit}`]).ok
    ? candidate
    : null;
}

/**
 * @param {string} repoRoot
 * @param {(typeof LANES)[number]} spec
 */
function checkLane(repoRoot, spec) {
  /** @type {string[]} */
  const errors = [];
  const sidecar = readJson(repoRoot, spec.provenance);

  /** @type {{label: string, revision: string, inputs: string[]}[]} */
  const bases = [];
  if (sidecar.error) {
    errors.push(`${spec.provenance} ${sidecar.error}`);
  } else if (!isRecord(sidecar.data)) {
    errors.push(`${spec.provenance} is not a JSON object`);
  } else {
    for (const { label, record } of spec.entries(sidecar.data, errors)) {
      for (const [field, inputs] of [
        ["sourceRevision", spec.source],
        ["recipeRevision", spec.recipe],
      ]) {
        if (!Object.hasOwn(record, field)) {
          errors.push(`${spec.provenance} ${label} is missing ${field}`);
          continue;
        }
        const value = record[field];
        if (
          typeof value !== "string" ||
          !COMMIT_PATTERN.test(value.trim())
        ) {
          errors.push(
            `${spec.provenance} ${label} ${field} ${JSON.stringify(value)} is not a commit hash`,
          );
          continue;
        }
        bases.push({
          label: `${label} ${field}`,
          revision: value.trim(),
          inputs,
        });
      }
    }
  }

  /** @type {{label: string, revision: string, inputs: string[]}[]} */
  const resolved = [];
  for (const base of bases) {
    const commit = resolveCommit(repoRoot, base.revision);
    if (!commit) {
      errors.push(
        `${spec.provenance} ${base.label} ${base.revision} does not resolve in this checkout`,
      );
      continue;
    }
    if (!git(repoRoot, ["merge-base", "--is-ancestor", commit, "HEAD"]).ok) {
      errors.push(
        `${spec.provenance} ${base.label} ${commit} is not an ancestor of HEAD`,
      );
      continue;
    }
    resolved.push({ ...base, revision: commit });
  }

  if (errors.length > 0) {
    return { lane: spec.lane, status: "invalid", stale: [], errors };
  }

  /** @type {{base: string, label: string, commit: string}[]} */
  const stale = [];
  for (const base of resolved) {
    const commit = expectGit(repoRoot, [
      "log",
      "-1",
      "--format=%H",
      `${base.revision}..HEAD`,
      "--",
      ...base.inputs,
    ]);
    if (commit) stale.push({ base: base.revision, label: base.label, commit });
  }

  return {
    lane: spec.lane,
    status: stale.length > 0 ? "stale" : "fresh",
    stale,
    errors: [],
  };
}

/**
 * @param {{repoRoot: string}} options
 * @returns {{lanes: {lane: string, status: "fresh" | "stale" | "invalid", stale: {base: string, label: string, commit: string}[], errors: string[]}[]}}
 */
export function checkFreshness({ repoRoot }) {
  expectGit(repoRoot, ["rev-parse", "--is-inside-work-tree"]);
  return { lanes: LANES.map((spec) => checkLane(repoRoot, spec)) };
}

const USAGE =
  "usage: node tooling/release/check-artifact-freshness.mjs [--repo <path>]";

/**
 * @param {string[]} argv
 */
function parseArgs(argv) {
  /** @type {{repoRoot: string} | null} */
  const options = { repoRoot: process.cwd() };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--help" || arg === "-h") return null;
    if (arg !== "--repo") {
      throw new Error(`unknown argument: ${arg}`);
    }
    const value = argv[index + 1];
    if (value === undefined || value.startsWith("--")) {
      throw new Error(`missing value for ${arg}`);
    }
    index += 1;
    options.repoRoot = value;
  }
  return options;
}

/**
 * @param {string[]} argv
 */
function main(argv) {
  let options;
  try {
    options = parseArgs(argv);
  } catch (error) {
    console.error(`${error.message}\n${USAGE}`);
    process.exitCode = 2;
    return;
  }
  if (!options) {
    console.log(USAGE);
    return;
  }

  let report;
  try {
    report = checkFreshness({ repoRoot: options.repoRoot });
  } catch (error) {
    process.stdout.write(
      `${JSON.stringify({ error: error.message }, null, 2)}\n`,
    );
    process.exitCode = 2;
    return;
  }

  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
  process.exitCode = report.lanes.every((lane) => lane.status === "fresh")
    ? 0
    : 1;
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  main(process.argv.slice(2));
}
