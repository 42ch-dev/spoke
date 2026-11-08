import assert from "node:assert/strict";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { execFileSync, spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, it } from "node:test";
import { applyRustFlags, readProvenance } from "./cpp-build.mjs";

/**
 * Windows CRT flag (`TARGETS["x86_64-pc-windows-msvc"].extraRustFlags`).
 * The Windows branch cannot run on a macOS host (no MSVC toolchain), so the
 * composition is covered here instead of by an executed carrier build.
 */
const WINDOWS_RUST_FLAGS = ["-C", "target-feature=-crt-static"];

const WINDOWS_VARIABLE = "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS";

describe("applyRustFlags", () => {
  it("injects the target flag when the environment carries none", () => {
    const env = {};

    const injected = applyRustFlags(env, "x86_64-pc-windows-msvc", WINDOWS_RUST_FLAGS);

    assert.equal(env[WINDOWS_VARIABLE], "-C target-feature=-crt-static");
    assert.deepEqual(injected, WINDOWS_RUST_FLAGS);
  });

  it("appends the target flag without dropping existing target rust flags", () => {
    const env = { [WINDOWS_VARIABLE]: "-C target-cpu=native" };

    const injected = applyRustFlags(env, "x86_64-pc-windows-msvc", WINDOWS_RUST_FLAGS);

    assert.equal(
      env[WINDOWS_VARIABLE],
      "-C target-cpu=native -C target-feature=-crt-static",
    );
    assert.deepEqual(injected, WINDOWS_RUST_FLAGS);
  });

  it("leaves the environment untouched for a target without extra flags", () => {
    const env = { RUSTFLAGS: "" };

    const injected = applyRustFlags(env, "aarch64-apple-darwin", []);

    assert.deepEqual(env, { RUSTFLAGS: "" });
    assert.deepEqual(injected, []);
  });
});

/**
 * The staging step merges the new entry onto whatever the file held, so the
 * read *is* the presence check: a missing file starts a fresh record, and an
 * existing one is parsed from the bytes that were read (an `existsSync` in
 * front of the read would leave a window in which the file changes).
 */
describe("readProvenance", () => {
  it("starts a fresh record when the file is not there", () => {
    const dir = mkdtempSync(join(tmpdir(), "cpp-build-provenance-"));
    try {
      assert.deepEqual(readProvenance(join(dir, "provenance.json")), {
        schemaVersion: 1,
        nativeArtifacts: {},
      });
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it("returns the entries it read", () => {
    const dir = mkdtempSync(join(tmpdir(), "cpp-build-provenance-"));
    try {
      const path = join(dir, "provenance.json");
      writeFileSync(
        path,
        JSON.stringify({
          schemaVersion: 1,
          nativeArtifacts: { "win-x64": { target: "x86_64-pc-windows-msvc" } },
        }),
      );

      const provenance = readProvenance(path);

      assert.deepEqual(provenance.nativeArtifacts, {
        "win-x64": { target: "x86_64-pc-windows-msvc" },
      });
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe("dependency source identity", () => {
  for (const crate of ["spoke-operations", "spoke-schemas"]) {
    it(`records committed ${crate} changes and refuses dirty changes`, () => {
      const root = realpathSync(mkdtempSync(join(tmpdir(), "cpp-build-source-")));
      const bin = join(root, "bin");
      const script = join(root, "tooling/connect/cpp-build.mjs");
      const dependency = `crates/${crate}/src/lib.rs`;
      const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();
      const put = (path, content, options) => {
        const absolute = join(root, path);
        mkdirSync(dirname(absolute), { recursive: true });
        writeFileSync(absolute, content, options);
      };
      const build = () => spawnSync(process.execPath, [script, "--target", "x86_64-pc-windows-msvc"], {
        cwd: root,
        encoding: "utf8",
        env: { ...process.env, PATH: `${bin}:${process.env.PATH}` },
      });
      const revision = () => JSON.parse(readFileSync(join(root, "crates/spoke-connect/bindings/cpp/native/provenance.json")))
        .nativeArtifacts["win-x64"].sourceRevision;
      try {
        mkdirSync(dirname(script), { recursive: true });
        copyFileSync(fileURLToPath(new URL("./cpp-build.mjs", import.meta.url)), script);
        git("init", "-q");
        git("config", "user.name", "Fixture");
        git("config", "user.email", "fixture@example.invalid");
        put("Cargo.toml", "workspace\n");
        put(dependency, "initial\n");
        put("crates/spoke-connect/bindings/cpp/include/spoke_connect.h", "header\n");
        put("crates/spoke-connect/bindings/cpp/include/spoke_connect.hpp", "header\n");
        put("unrelated.txt", "initial\n");
        git("add", ".");
        git("commit", "-qm", "initial");
        put("bin/cargo", "#!/bin/sh\nmkdir -p target/cpp/x86_64-pc-windows-msvc/release\nprintf native > target/cpp/x86_64-pc-windows-msvc/release/spoke_connect_capi.dll\nprintf import > target/cpp/x86_64-pc-windows-msvc/release/spoke_connect_capi.dll.lib\n", { mode: 0o755 });
        put("bin/rustc", "#!/bin/sh\nprintf 'rustc fixture\\n'\n", { mode: 0o755 });
        put("bin/cl.exe", "#!/bin/sh\nprintf 'Compiler Version fixture\\n'\n", { mode: 0o755 });

        const initial = build();
        assert.equal(initial.status, 0, initial.stderr);
        put(dependency, "committed change\n");
        git("add", dependency);
        git("commit", "-qm", "dependency change");
        const dependencyRevision = git("rev-parse", "HEAD");
        const committed = build();
        assert.equal(committed.status, 0, committed.stderr);
        assert.equal(revision(), dependencyRevision);

        put("unrelated.txt", "unrelated change\n");
        git("add", "unrelated.txt");
        git("commit", "-qm", "unrelated change");
        const unrelated = build();
        assert.equal(unrelated.status, 0, unrelated.stderr);
        assert.equal(revision(), dependencyRevision);

        put(dependency, "dirty change\n");
        const dirty = build();
        assert.notEqual(dirty.status, 0);
        assert.equal(revision(), dependencyRevision);
      } finally {
        rmSync(root, { recursive: true, force: true });
      }
    });
  }
});

/**
 * `--verify` is the read-only consumer gate wired into CI: it must accept the
 * bytes the build path just recorded and fail closed on any later drift,
 * including a committed entry that lost the `.hpp` hash altogether.
 */
describe("verify committed provenance", () => {
  const PROVENANCE_PATH = "crates/spoke-connect/bindings/cpp/native/provenance.json";
  const setupVerifyFixture = () => {
    const root = realpathSync(mkdtempSync(join(tmpdir(), "cpp-build-verify-")));
    const bin = join(root, "bin");
    const script = join(root, "tooling/connect/cpp-build.mjs");
    const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();
    const put = (path, content, options) => {
      const absolute = join(root, path);
      mkdirSync(dirname(absolute), { recursive: true });
      writeFileSync(absolute, content, options);
    };
    const run = (...args) => spawnSync(process.execPath, [script, ...args], {
      cwd: root,
      encoding: "utf8",
      env: { ...process.env, PATH: `${bin}:${process.env.PATH}` },
    });
    mkdirSync(dirname(script), { recursive: true });
    copyFileSync(fileURLToPath(new URL("./cpp-build.mjs", import.meta.url)), script);
    git("init", "-q");
    git("config", "user.name", "Fixture");
    git("config", "user.email", "fixture@example.invalid");
    put("Cargo.toml", "workspace\n");
    put("crates/spoke-connect/src/lib.rs", "initial\n");
    put("crates/spoke-connect/Cargo.toml", "package\n");
    put("crates/spoke-connect/bindings/cpp/include/spoke_connect.h", "header\n");
    put("crates/spoke-connect/bindings/cpp/include/spoke_connect.hpp", "header\n");
    git("add", ".");
    git("commit", "-qm", "initial");
    put("bin/cargo", "#!/bin/sh\nmkdir -p target/cpp/x86_64-pc-windows-msvc/release\nprintf native > target/cpp/x86_64-pc-windows-msvc/release/spoke_connect_capi.dll\nprintf import > target/cpp/x86_64-pc-windows-msvc/release/spoke_connect_capi.dll.lib\n", { mode: 0o755 });
    put("bin/rustc", "#!/bin/sh\nprintf 'rustc fixture\\n'\n", { mode: 0o755 });
    put("bin/cl.exe", "#!/bin/sh\nprintf 'Compiler Version fixture\\n'\n", { mode: 0o755 });
    const build = run("--target", "x86_64-pc-windows-msvc");
    assert.equal(build.status, 0, build.stderr);
    const verify = () => run("--verify", "--target", "x86_64-pc-windows-msvc");
    return { root, verify, put, PROVENANCE_PATH };
  };

  it("passes on the bytes the build path just recorded", () => {
    const { root, verify } = setupVerifyFixture();
    try {
      const result = verify();
      assert.equal(result.status, 0, result.stderr);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("fails closed when the committed entry has no .hpp hash", () => {
    const { root, verify, put, PROVENANCE_PATH } = setupVerifyFixture();
    try {
      const provenance = JSON.parse(readFileSync(join(root, PROVENANCE_PATH), "utf8"));
      delete provenance.nativeArtifacts["win-x64"].hppSha256;
      put(PROVENANCE_PATH, `${JSON.stringify(provenance, null, 2)}\n`);
      const result = verify();
      assert.notEqual(result.status, 0);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("fails closed when the committed .hpp bytes drift from the record", () => {
    const { root, verify, put } = setupVerifyFixture();
    try {
      put("crates/spoke-connect/bindings/cpp/include/spoke_connect.hpp", "drifted header\n");
      const result = verify();
      assert.notEqual(result.status, 0);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});
