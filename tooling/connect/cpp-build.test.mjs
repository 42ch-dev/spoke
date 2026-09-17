import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { applyRustFlags } from "./cpp-build.mjs";

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
