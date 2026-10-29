#!/usr/bin/env node
/**
 * Assert the formatted memory Rust files stay within MAX_BYTES.
 *
 * typify emits each file as one long line; `generateRust()` runs stable rustfmt on
 * these three files. This bound catches pathological schema growth (e.g. runaway
 * shared definitions) without flagging normal generation drift.
 *
 * Bump procedure: also record the measured post-format sizes in the task evidence.
 * If a legitimate schema change pushes a file over the bound, raise MAX_BYTES in the
 * same commit as the regenerated file.
 */

import { statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const MAX_BYTES = 2 * 1024 * 1024;

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const GENERATED_DIR = join(REPO_ROOT, "crates/spoke-schemas/src/generated/ops");
const FILES = [
  "memory_exports_get_response.rs",
  "memory_imports_stage_request.rs",
  "memory_receipts_get_response.rs",
];

let failed = false;

for (const file of FILES) {
  const bytes = statSync(join(GENERATED_DIR, file)).size;

  if (bytes > MAX_BYTES) {
    failed = true;
    console.error(
      `Generated Rust file too large: ${file} is ${bytes} bytes (limit ${MAX_BYTES}).`,
    );
  } else {
    console.log(`Generated Rust size OK: ${file} ${bytes} bytes`);
  }
}

if (failed) {
  console.error(
    "Bump procedure: see header comment in tooling/codegen/assert-generated-rust-size.mjs",
  );
  process.exit(1);
}
