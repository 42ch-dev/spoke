import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
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
