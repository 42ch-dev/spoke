#!/usr/bin/env node
/**
 * Assert all `.schema.json` files under `schemas/` match EXPECTED_SCHEMA_COUNT.
 *
 * Bump procedure:
 * 1. Add or remove `.schema.json` files under `schemas/`.
 * 2. Update EXPECTED_SCHEMA_COUNT in this file.
 * 3. Run `pnpm run codegen`; commit schema + generated output + this constant in one commit.
 * 4. `pnpm run verify-codegen` must PASS.
 */

import { readdirSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const EXPECTED_SCHEMA_COUNT = 23;

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const SCHEMAS_ROOT = join(REPO_ROOT, "schemas");

function collectSchemaFiles(dir) {
  const files = [];

  for (const entry of readdirSync(dir)) {
    const fullPath = join(dir, entry);
    const stats = statSync(fullPath);

    if (stats.isDirectory()) {
      files.push(...collectSchemaFiles(fullPath));
      continue;
    }

    if (entry.endsWith(".schema.json")) {
      files.push(fullPath);
    }
  }

  return files;
}

const schemaFiles = collectSchemaFiles(SCHEMAS_ROOT);
const actualCount = schemaFiles.length;

if (actualCount !== EXPECTED_SCHEMA_COUNT) {
  console.error(
    `Schema count mismatch: expected ${EXPECTED_SCHEMA_COUNT}, found ${actualCount}.`,
  );
  console.error(
    "Paths counted under schemas/ (relative to repo root):",
  );
  for (const file of schemaFiles.sort()) {
    console.error(`  - ${relative(REPO_ROOT, file)}`);
  }
  console.error(
    "Bump procedure: see header comment in tooling/codegen/assert-schema-count.mjs",
  );
  process.exit(1);
}

console.log(`Schema count OK: ${actualCount} file(s) under schemas/`);
