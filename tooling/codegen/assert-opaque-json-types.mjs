#!/usr/bin/env node
/**
 * Assert generated TypeScript keeps wire any-JSON fields opaque.
 *
 * `OpaqueJson` (`common.schema.json#/definitions/OpaqueJson`) is the wire any-JSON shape
 * (empty schema `{}`): scalars, arrays, objects and null are all valid. json-schema-to-typescript
 * renders `{ "$ref": <any-JSON schema>, "description": ... }` as an object index signature, which
 * narrows the field to an object map; `tooling/codegen/src/run.mjs` re-expresses those
 * annotation-bearing refs before generation so the emitted type stays `unknown`.
 *
 * This script fails when a field whose schema `$ref`s the canonical `OpaqueJson` definition is
 * emitted as an object map.
 *
 * Wire it from `pnpm run verify-codegen` alongside `assert-schema-count.mjs`.
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const SCHEMAS_ROOT = join(REPO_ROOT, "schemas");
const TS_OUT_ROOT = join(REPO_ROOT, "packages/spoke-schemas/src/generated");
const OPAQUE_JSON_FILE = "common/common.schema.json";

/** Keys that annotate a schema without constraining it. */
const ANNOTATION_KEYS = new Set([
  "title",
  "description",
  "$comment",
  "default",
  "examples",
  "deprecated",
  "readOnly",
  "writeOnly",
]);

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

function readSchema(relSchema) {
  return JSON.parse(readFileSync(join(SCHEMAS_ROOT, relSchema), "utf8"));
}

function isAnyJsonSchema(node) {
  return (
    node !== null &&
    typeof node === "object" &&
    !Array.isArray(node) &&
    Object.keys(node).every((key) => ANNOTATION_KEYS.has(key))
  );
}

function resolveDefinition(fragment) {
  let node = readSchema(OPAQUE_JSON_FILE);

  for (const segment of fragment.split("/").filter(Boolean)) {
    const key = decodeURIComponent(segment).replace(/~1/g, "/").replace(/~0/g, "~");
    node = node?.[key];
  }

  return node;
}

/** Fields whose schema node is an annotated `$ref` to the canonical any-JSON definition. */
function collectOpaqueFields(relSchema) {
  const fields = [];

  const visit = (node, propertyName) => {
    if (Array.isArray(node)) {
      node.forEach((child) => visit(child, propertyName));
      return;
    }
    if (node === null || typeof node !== "object") {
      return;
    }

    const siblings = Object.keys(node).filter((key) => key !== "$ref");
    if (
      propertyName !== undefined &&
      typeof node.$ref === "string" &&
      siblings.length > 0 &&
      node.$ref.split("#")[0].endsWith(OPAQUE_JSON_FILE) &&
      isAnyJsonSchema(resolveDefinition(node.$ref.split("#")[1] ?? ""))
    ) {
      fields.push(propertyName);
    }

    for (const [key, child] of Object.entries(node)) {
      visit(child, key);
    }
  };

  visit(readSchema(relSchema), undefined);
  return fields;
}

function escapeForRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

const problems = [];

for (const relSchema of collectSchemaFiles(SCHEMAS_ROOT).map((file) => relative(SCHEMAS_ROOT, file))) {
  const fields = collectOpaqueFields(relSchema);
  if (fields.length === 0) {
    continue;
  }

  const baseName = relSchema.replace(/\.schema\.json$/, "").split("/").pop();
  const generatedPath = join(TS_OUT_ROOT, dirname(relSchema), `${baseName}.ts`);

  let generated;
  try {
    generated = readFileSync(generatedPath, "utf8");
  } catch {
    problems.push(`${relSchema}: generated TypeScript missing at ${relative(REPO_ROOT, generatedPath)}`);
    continue;
  }

  for (const field of fields) {
    const name = escapeForRegExp(field);
    const opaque = new RegExp(`^\\s*${name}\\??\\s*:\\s*unknown\\s*;`, "m");
    const objectMap = new RegExp(`^\\s*${name}\\??\\s*:\\s*\\{[^}]*\\[k: string\\]`, "ms");

    if (objectMap.test(generated) || !opaque.test(generated)) {
      problems.push(
        `${relSchema}: generated \`${field}\` is not any-JSON (expected \`${field}?: unknown\`, got an object index signature)`,
      );
    }
  }
}

if (problems.length > 0) {
  console.error("Opaque any-JSON generated type regression:");
  for (const problem of problems) {
    console.error(`  - ${problem}`);
  }
  console.error(
    "See tooling/codegen/src/run.mjs normalizeOpaqueRefs and the T2 review finding on OpaqueJson fields.",
  );
  process.exit(1);
}

console.log("Opaque any-JSON generated types OK.");
