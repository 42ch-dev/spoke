#!/usr/bin/env node
/**
 * C carrier build + native staging.
 *
 * Builds `spoke-connect-capi` for one of the two closed evidence targets,
 * stages only the carrier's own consumer artifacts into the committed
 * `bindings/cpp/native/<rid>/` directory, and refreshes that RID's entry in
 * `bindings/cpp/native/provenance.json` (source revision, target, `rustc -Vv`,
 * compiler version, build flags — the exact cargo argv plus the Rust flags this
 * recipe injects — header SHA-256, native SHA-256).
 *
 * Targets (closed set — the C++ channel ships osx-arm64 + win-x64 only):
 *
 *   aarch64-apple-darwin    → native/osx-arm64/libspoke_connect_capi.dylib
 *   x86_64-pc-windows-msvc  → native/win-x64/spoke_connect_capi.dll
 *                             native/win-x64/spoke_connect_capi.dll.lib
 *
 * macOS sets the `@rpath` install name with `install_name_tool -id` before
 * hashing, so the recorded SHA-256 describes the shipped file. Windows builds
 * against the dynamic Rust CRT (`-C target-feature=-crt-static`), matching the
 * C++ `/MD` release CRT of the consumers; that flag is appended to the
 * target-scoped `CARGO_TARGET_<TRIPLE>_RUSTFLAGS` variable instead of
 * `RUSTFLAGS`: `RUSTFLAGS` would replace the local toolchain's configured
 * rustflags for every target, while the target-scoped variable only touches
 * this target — and appending keeps the flags that variable already carries,
 * since it is cargo's `target.<triple>.rustflags` and outranks
 * `build.rustflags`.
 *
 * Build sources (the carrier crate, the linked implementation, the workspace
 * manifests) must be committed before staging: the recorded source revision
 * has to describe the binary being shipped.
 *
 * Usage:
 *   node tooling/connect/cpp-build.mjs --target aarch64-apple-darwin --toolchain nightly
 *   node tooling/connect/cpp-build.mjs --target x86_64-pc-windows-msvc --toolchain 1.96.0
 *   node tooling/connect/cpp-build.mjs --verify --target aarch64-apple-darwin
 *
 * `--toolchain <name>` is passed to cargo as `+<name>`; when omitted, the
 * caller's default toolchain is used.
 *
 * `--verify` is the read-only consumer check: it re-reads the committed
 * provenance and hashes the committed header and staged native for one RID,
 * failing when a recorded SHA-256 no longer describes those bytes. It builds
 * nothing and writes nothing, so it runs against any checkout — including one
 * whose header bytes drifted from the record (CRLF from a Windows
 * `core.autocrlf=true` checkout), which is the drift the build path hides by
 * rewriting the record it just hashed.
 */

import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { spawnSync } from "node:child_process";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const CARRIER_PACKAGE = "spoke-connect-capi";
const CPP_DIR = join(REPO_ROOT, "crates", "spoke-connect", "bindings", "cpp");
const HEADER = join(CPP_DIR, "include", "spoke_connect.h");
const PROVENANCE = join(CPP_DIR, "native", "provenance.json");
const TARGET_DIR_FLAG = "target/cpp";
const TARGET_DIR = join(REPO_ROOT, TARGET_DIR_FLAG);

/** Paths whose committed state defines the binary; they must be clean. */
const BUILD_SOURCE_PATHS = [
  "Cargo.toml",
  "Cargo.lock",
  "crates/spoke-connect-capi",
  "crates/spoke-connect/src",
  "crates/spoke-connect/Cargo.toml",
];

const TARGETS = {
  "aarch64-apple-darwin": {
    rid: "osx-arm64",
    // Built artifact → committed path relative to the native directory.
    artifacts: [
      { built: "libspoke_connect_capi.dylib", staged: "libspoke_connect_capi.dylib" },
    ],
    installName: "@rpath/libspoke_connect_capi.dylib",
    extraRustFlags: [],
    compilerName: "clang",
    compilerVersionArgs: ["--version"],
  },
  "x86_64-pc-windows-msvc": {
    rid: "win-x64",
    artifacts: [
      { built: "spoke_connect_capi.dll", staged: "spoke_connect_capi.dll" },
      { built: "spoke_connect_capi.dll.lib", staged: "spoke_connect_capi.dll.lib" },
    ],
    installName: null,
    // Dynamic Rust CRT: consumers link the C++ release CRT (`/MD`).
    extraRustFlags: ["-C", "target-feature=-crt-static"],
    compilerName: "cl.exe",
    compilerVersionArgs: [],
  },
};

function fail(message) {
  console.error(`cpp-build: ${message}`);
  process.exit(1);
}

function parseArgs(argv) {
  const args = { target: null, toolchain: null, verify: false };
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    if (flag === "--target") {
      args.target = argv[index + 1];
      index += 1;
    } else if (flag === "--toolchain") {
      args.toolchain = argv[index + 1];
      index += 1;
    } else if (flag === "--verify") {
      args.verify = true;
    } else {
      fail(`unknown argument '${flag}' (usage: --target <rust-target> [--toolchain <name>] [--verify])`);
    }
  }
  if (!args.target) fail("--target is required");
  if (!TARGETS[args.target]) {
    fail(`unsupported target '${args.target}' (supported: ${Object.keys(TARGETS).join(", ")})`);
  }
  if (args.verify && args.toolchain) {
    fail("--verify only reads the committed files; drop --toolchain rather than ignoring it");
  }
  return args;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: REPO_ROOT, ...options });
  if (result.error) fail(`failed to run '${command}': ${result.error.message}`);
  if (result.status !== 0 && !options.allowFailure) {
    fail(`'${command} ${args.join(" ")}' exited ${result.status}`);
  }
  return result;
}

function capture(command, args) {
  const result = run(command, args, { encoding: "utf8" });
  return (result.stdout ?? "").trim();
}

/**
 * The SHA-256 of the file's bytes, read once. There is no `existsSync` in
 * front of it: the bytes hashed are the bytes on disk at the hash, and a
 * missing file fails with the same message the presence check used to give.
 */
function sha256(path) {
  let bytes;
  try {
    bytes = readFileSync(path);
  } catch (error) {
    fail(`could not read ${relative(REPO_ROOT, path)}: ${error.message}`);
  }
  return createHash("sha256").update(bytes).digest("hex");
}

function assertBuildSourcesCommitted() {
  const status = capture("git", [
    "status",
    "--porcelain",
    "--",
    ...BUILD_SOURCE_PATHS,
  ]);
  if (status) {
    fail(
      `build sources are not committed; commit or stash them before staging so the ` +
        `provenance revision describes the binary:\n${status}`,
    );
  }
}

/**
 * The revision the binary was built from: the last commit that touched the
 * build sources. Using that commit rather than HEAD keeps the record stable
 * across staging refactors, and it names the code — not the artifact — that a
 * consumer resolves the native against.
 */
function buildSourceRevision() {
  const revision = capture("git", [
    "log",
    "-1",
    "--format=%H",
    "--",
    ...BUILD_SOURCE_PATHS,
  ]);
  if (!revision) {
    fail("could not resolve the revision of the build sources");
  }
  return revision;
}

function compilerVersion(target) {
  const spec = TARGETS[target];
  // `cl.exe` with no arguments prints its version banner and exits non-zero.
  const result = run(spec.compilerName, spec.compilerVersionArgs, {
    encoding: "utf8",
    allowFailure: true,
  });
  const output = `${result.stdout ?? ""}${result.stderr ?? ""}`;
  const line = output
    .split("\n")
    .map((entry) => entry.trim())
    .find((entry) => /Compiler Version/i.test(entry) || /clang version/i.test(entry));
  if (!line) {
    fail(
      `could not read a compiler version from '${spec.compilerName}' ` +
        `(is it on PATH?)`,
    );
  }
  return line;
}

/**
 * Adds the target's required Rust flags to `env`, appended to whatever
 * `CARGO_TARGET_<TRIPLE>_RUSTFLAGS` already carries so no pre-existing target
 * rust setting is dropped, and returns the flags this recipe injected (recorded
 * in provenance). Targets without extra flags leave `env` untouched.
 */
export function applyRustFlags(env, target, extraRustFlags) {
  if (extraRustFlags.length === 0) return [];
  const variable = `CARGO_TARGET_${target.toUpperCase().replace(/-/g, "_")}_RUSTFLAGS`;
  const added = extraRustFlags.join(" ");
  const configured = env[variable] ?? "";
  env[variable] = configured ? `${configured} ${added}` : added;
  return extraRustFlags;
}

/**
 * The existing provenance at `path`, or a fresh one when the file is not there.
 * The read *is* the presence check — an `existsSync` + `readFileSync` /
 * `writeFileSync` pair leaves a window in which another writer can create or
 * replace the file, and the merged result must be based on the bytes that were
 * actually read.
 */
export function readProvenance(path) {
  let text;
  try {
    text = readFileSync(path, "utf8");
  } catch (error) {
    if (error.code !== "ENOENT") {
      fail(`could not read ${relative(REPO_ROOT, path)}: ${error.message}`);
    }
    return { schemaVersion: 1, nativeArtifacts: {} };
  }
  let provenance;
  try {
    provenance = JSON.parse(text);
  } catch (error) {
    fail(`could not parse ${relative(REPO_ROOT, path)}: ${error.message}`);
  }
  if (!provenance.nativeArtifacts) provenance.nativeArtifacts = {};
  return provenance;
}

/**
 * The committed entry for `spec.rid` has to describe the bytes a consumer
 * receives: the contract header and every staged native the entry lists. The
 * read-only shape is the point — the build path records what it just hashed,
 * so only a reader can catch a checkout whose bytes no longer match the record.
 */
function verifyProvenance(spec) {
  const entry = readProvenance(PROVENANCE).nativeArtifacts[spec.rid];
  if (!entry) {
    fail(`provenance has no '${spec.rid}' entry, so nothing records what was shipped`);
  }

  const problems = [];
  const headerSha256 = sha256(HEADER);
  if (entry.headerSha256 !== headerSha256) {
    problems.push(
      `header: recorded ${entry.headerSha256}, committed bytes ${headerSha256} ` +
        `(${relative(REPO_ROOT, HEADER)})`,
    );
  }
  for (const artifact of spec.artifacts) {
    const recorded = (entry.artifacts ?? []).find((item) => item.path === artifact.staged);
    const actual = sha256(join(CPP_DIR, "native", spec.rid, artifact.staged));
    if (!recorded) {
      problems.push(`native/${spec.rid}/${artifact.staged}: missing from the entry`);
    } else if (recorded.sha256 !== actual) {
      problems.push(
        `native/${spec.rid}/${artifact.staged}: recorded ${recorded.sha256}, ` +
          `committed bytes ${actual}`,
      );
    }
  }

  if (problems.length > 0) {
    fail(
      `committed provenance does not describe the committed ${spec.rid} bytes:\n  ` +
        problems.join("\n  "),
    );
  }
  console.log(
    `cpp-build: verified ${spec.rid} provenance (header ${headerSha256}, ` +
      `natives ${spec.artifacts.length})`,
  );
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const spec = TARGETS[args.target];

  if (args.verify) {
    verifyProvenance(spec);
    return;
  }

  assertBuildSourcesCommitted();

  const cargoArgs = ["build", "--locked", "-p", CARRIER_PACKAGE, "--release", "--target", args.target, "--target-dir", TARGET_DIR_FLAG];
  const env = { ...process.env };
  const rustFlags = applyRustFlags(env, args.target, spec.extraRustFlags);

  const cargo = args.toolchain ? `cargo +${args.toolchain}` : "cargo";
  console.log(`cpp-build: ${cargo} ${cargoArgs.join(" ")}`);
  run("cargo", args.toolchain ? [`+${args.toolchain}`, ...cargoArgs] : cargoArgs, {
    stdio: "inherit",
    env,
  });

  const releaseDir = join(TARGET_DIR, args.target, "release");
  const nativeDir = join(CPP_DIR, "native", spec.rid);
  mkdirSync(nativeDir, { recursive: true });

  const artifacts = [];
  for (const artifact of spec.artifacts) {
    const built = join(releaseDir, artifact.built);
    if (!existsSync(built)) {
      fail(
        `expected build output missing: ${relative(REPO_ROOT, built)}; ` +
          `the carrier build produced no ${artifact.built}`,
      );
    }
    const staged = join(nativeDir, artifact.staged);
    copyFileSync(built, staged);
    if (spec.installName) {
      // Before hashing: the recorded SHA-256 must describe the shipped file.
      run("install_name_tool", ["-id", spec.installName, staged]);
    }
    artifacts.push({
      path: artifact.staged,
      sha256: sha256(staged),
    });
  }

  const entry = {
    target: args.target,
    sourceRevision: buildSourceRevision(),
    rustcVersion: capture("rustc", args.toolchain ? [`+${args.toolchain}`, "-Vv"] : ["-Vv"]),
    compilerVersion: compilerVersion(args.target),
    buildFlags: { cargoArgs, rustFlags },
    headerSha256: sha256(HEADER),
    artifacts,
  };

  const provenance = readProvenance(PROVENANCE);
  provenance.nativeArtifacts[spec.rid] = entry;
  // Stable key order keeps refresh diffs readable.
  provenance.nativeArtifacts = Object.fromEntries(
    Object.entries(provenance.nativeArtifacts).sort(([left], [right]) =>
      left.localeCompare(right),
    ),
  );
  writeFileSync(PROVENANCE, `${JSON.stringify(provenance, null, 2)}\n`);

  console.log(`cpp-build: staged ${spec.rid}`);
  for (const artifact of artifacts) {
    console.log(`  native/${spec.rid}/${artifact.path}  sha256=${artifact.sha256}`);
  }
  console.log(`  header sha256=${entry.headerSha256}`);
  console.log(`  source revision=${entry.sourceRevision}`);
  console.log(`  provenance: ${relative(REPO_ROOT, PROVENANCE)}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}
