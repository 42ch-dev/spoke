#!/usr/bin/env node
/**
 * C++ smoke runner: compile, link and run the `bindings/cpp/Smoke` translation
 * units (`main.cpp` for the raw C ABI, `convenience.cpp` for the C++17
 * convenience layer) against a staged carrier native.
 *
 * The smoke is the executable boundary proof for the C ABI: it reads the shared
 * `golden-hello.json` vector, derives and verifies over the exported session
 * core, runs a ports round trip over a host-owned loopback, exercises the
 * rejection/ownership rules, and repeats the core groups through the
 * convenience layer. This script builds it in `target/cpp-smoke`, executes it,
 * and then requires the banner lines it printed to equal the configuration's
 * expected list exactly — a run that executes no assertions fails.
 *
 * Every RID is built twice: exceptions disabled (the consumer default) and
 * exceptions enabled, which is the build that exercises the convenience layer's
 * exception containment. Each configuration must report its own ordered banner
 * list, and the enabled one additionally proves the containment row.
 *
 * Usage:
 *   node tooling/connect/cpp-smoke.mjs --rid osx-arm64
 *   node tooling/connect/cpp-smoke.mjs --rid win-x64
 *
 * `--rid osx-arm64` compiles with Apple clang and links the staged dylib
 * (`-Wl,-rpath` to its directory). `--rid win-x64` compiles with `cl.exe`
 * against the staged import library and stages the DLL beside the executable
 * before running it; the MSVC developer environment must be on PATH. Every
 * subprocess receives an argv array — paths are never shell-concatenated.
 */

import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const CPP_DIR = join(REPO_ROOT, "crates", "spoke-connect", "bindings", "cpp");
const INCLUDE_DIR = join(CPP_DIR, "include");
const SMOKE_SOURCES = [
  join(CPP_DIR, "Smoke", "main.cpp"),
  join(CPP_DIR, "Smoke", "convenience.cpp"),
];
const FIXTURE = join(
  REPO_ROOT,
  "crates",
  "spoke-connect",
  "tests",
  "fixtures",
  "golden-hello.json",
);
const OUTPUT_DIR = join(REPO_ROOT, "target", "cpp-smoke");

/** The banners the smoke prints, each after a group of passing assertions. */
const BANNERS = [
  "golden peer-id: PASS",
  "golden hello signature: PASS",
  "protocol version 1: PASS",
  "loopback ports: PASS",
  "rejection/ownership: PASS",
  "C++ convenience values/core: PASS",
  "C++ convenience callbacks/session: PASS",
  "C++ convenience router: PASS",
];
/** The final banner every configuration prints last. */
const FINAL_BANNER = "C++ smoke: PASS";
/**
 * The exceptions-enabled configuration additionally injects a throwing host
 * callback, so only that build reports the containment row.
 */
const CONTAINMENT_BANNER = "C++ callback exception containment: PASS";

/**
 * The two configurations every RID is built in: the consumer default with
 * exceptions disabled, and the same translation units with exceptions enabled,
 * which is the only build that exercises the containment branch.
 */
const CONFIGURATIONS = [
  {
    id: "disabled",
    label: "exceptions disabled",
    banners: [...BANNERS, FINAL_BANNER],
    clang: ["-fno-exceptions", "-fno-rtti"],
    msvc: ["/EHs-c-", "/D_HAS_EXCEPTIONS=0"],
    executableSuffix: "",
  },
  {
    id: "enabled",
    label: "exceptions enabled",
    banners: [...BANNERS, CONTAINMENT_BANNER, FINAL_BANNER],
    clang: ["-fexceptions", "-fno-rtti"],
    msvc: ["/EHsc"],
    executableSuffix: "-exceptions",
  },
];

const RIDS = {
  "osx-arm64": {
    platform: "darwin",
    nativeDir: join(CPP_DIR, "native", "osx-arm64"),
    library: "libspoke_connect_capi.dylib",
    executable: join(OUTPUT_DIR, "cpp-smoke"),
  },
  "win-x64": {
    platform: "win32",
    nativeDir: join(CPP_DIR, "native", "win-x64"),
    library: "spoke_connect_capi.dll",
    importLibrary: "spoke_connect_capi.dll.lib",
    executable: join(OUTPUT_DIR, "cpp-smoke"),
  },
};

function fail(message) {
  console.error(`cpp-smoke: ${message}`);
  process.exit(1);
}

function display(path) {
  const relativePath = relative(REPO_ROOT, path);
  return relativePath.startsWith("..") ? path : relativePath;
}

function parseArgs(argv) {
  const args = { rid: null };
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    if (flag === "--rid") {
      args.rid = argv[index + 1];
      index += 1;
    } else {
      fail(`unknown argument '${flag}' (usage: --rid <${Object.keys(RIDS).join("|")}>)`);
    }
  }
  if (!args.rid) fail("--rid is required");
  if (!RIDS[args.rid]) {
    fail(`unsupported rid '${args.rid}' (supported: ${Object.keys(RIDS).join(", ")})`);
  }
  return args;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: REPO_ROOT, ...options });
  if (result.error) fail(`failed to run '${command}': ${result.error.message}`);
  if (result.status !== 0 && !options.allowFailure) {
    fail(`'${command}' exited ${result.status}`);
  }
  return result;
}

/** The compile/link argv for one RID and exception configuration. */
function compileArgs(spec, build) {
  const library = join(spec.nativeDir, spec.importLibrary ?? spec.library);
  const executable = executableFor(spec, build);
  if (spec.platform === "win32") {
    return [
      "/nologo",
      "/std:c++17",
      ...build.msvc,
      "/GR-",
      "/MD",
      "/W4",
      "/WX",
      `/I${INCLUDE_DIR}`,
      ...SMOKE_SOURCES,
      library,
      `/Fe:${executable}`,
    ];
  }
  return [
    "-std=c++17",
    ...build.clang,
    "-Wall",
    "-Wextra",
    "-Werror",
    `-I${INCLUDE_DIR}`,
    ...SMOKE_SOURCES,
    library,
    `-Wl,-rpath,${spec.nativeDir}`,
    "-o",
    executable,
  ];
}

/** The executable of one configuration; the two builds never share a path. */
function executableFor(spec, build) {
  return `${spec.executable}${build.executableSuffix}${spec.platform === "win32" ? ".exe" : ""}`;
}

/** A banner-shaped line: what `banner()` in the smoke prints per passed group. */
const BANNER_LINE = /^.+: PASS$/;

/** Every banner-shaped line one smoke run printed, in output order. The split
    is line-ending agnostic: the child's C runtime writes CRLF on Windows, so a
    "\n"-only split would leave a trailing "\r" on every line and no banner line
    would match. */
function bannerLines(output) {
  return output.split(/\r?\n/).filter((line) => BANNER_LINE.test(line));
}

/** Requires the run's banner lines to equal one configuration's expected list
    exactly — same count, same order, same text — so a missing, extra, repeated
    or reordered banner all fail naming the first position that differs; a run
    that asserts nothing prints no banner and cannot match a non-empty list. */
function verifyBanners(output, banners) {
  const observed = bannerLines(output);
  const mismatch = observed.findIndex((line, index) => line !== banners[index]);
  if (mismatch < 0 && observed.length === banners.length) return;
  const at = mismatch < 0 ? Math.min(observed.length, banners.length) : mismatch;
  fail(
    `the smoke run's banners differ at position ${at + 1}: expected ` +
      `${banners[at] ? `'${banners[at]}'` : "no banner"}, saw ` +
      `${observed[at] ? `'${observed[at]}'` : "no banner"} ` +
      `(${banners.length} expected, ${observed.length} printed)`,
  );
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const spec = RIDS[args.rid];

  for (const source of SMOKE_SOURCES) {
    if (!existsSync(source)) fail(`missing smoke source: ${display(source)}`);
  }
  if (!existsSync(INCLUDE_DIR)) fail(`missing include directory: ${display(INCLUDE_DIR)}`);
  if (!existsSync(FIXTURE)) fail(`missing golden vector: ${display(FIXTURE)}`);
  const library = join(spec.nativeDir, spec.library);
  if (!existsSync(library)) fail(`missing staged native: ${display(library)}`);
  const importLibrary = spec.importLibrary
    ? join(spec.nativeDir, spec.importLibrary)
    : null;
  if (importLibrary && !existsSync(importLibrary)) {
    fail(`missing staged import library: ${display(importLibrary)}`);
  }

  mkdirSync(OUTPUT_DIR, { recursive: true });

  const compiler = spec.platform === "win32" ? "cl.exe" : "clang++";
  for (const build of CONFIGURATIONS) {
    const executable = executableFor(spec, build);
    const argsForCompile = compileArgs(spec, build);
    console.log(`cpp-smoke: ${build.label}: ${compiler} ${argsForCompile.join(" ")}`);
    run(compiler, argsForCompile, { stdio: "inherit" });

    if (spec.importLibrary) {
      // The DLL is a runtime dependency of the executable, not a link input.
      copyFileSync(library, join(OUTPUT_DIR, spec.library));
    }

    console.log(`cpp-smoke: ${display(executable)} ${display(FIXTURE)}`);
    const executed = run(executable, [FIXTURE], { encoding: "utf8", allowFailure: true });
    const output = `${executed.stdout ?? ""}${executed.stderr ?? ""}`;
    process.stdout.write(output);
    if (!output.endsWith("\n")) process.stdout.write("\n");
    if (executed.status !== 0) {
      fail(`${display(executable)} exited ${executed.status} (rid ${args.rid}, ${build.id})`);
    }

    verifyBanners(output, build.banners);
    console.log(`C++ smoke (${build.label}): PASS`);
  }

  console.log(`C++ smoke runner: PASS (rid ${args.rid})`);
}

main();
