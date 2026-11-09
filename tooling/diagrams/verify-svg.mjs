#!/usr/bin/env node
// Committed-diagram gate: every spec must have a light and a dark SVG, and each
// SVG must be a self-contained, script-free document whose accessible name
// resolves. Runs before `vitepress build` so a broken or missing diagram fails CI.

import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const SPECS = join(ROOT, "tooling/diagrams/specs");
const OUT = join(ROOT, "docs/public/diagrams");

const specs = readdirSync(SPECS)
  .filter((file) => file.endsWith(".json"))
  .map((file) => file.split(".")[0])
  .sort();

if (specs.length === 0) {
  console.error("diagram check FAIL: no specifications found");
  process.exit(1);
}

const failures = [];
for (const name of specs) {
  for (const theme of ["light", "dark"]) {
    const file = join(OUT, `${name}.${theme}.svg`);
    let svg;
    try {
      svg = readFileSync(file, "utf8");
    } catch (error) {
      const reason = error?.code === "ENOENT" ? "missing" : `unreadable (${error?.code || error?.message})`;
      failures.push(`${file}: ${reason}`);
      continue;
    }
    if (!/<svg\b/.test(svg)) failures.push(`${file}: no <svg> root`);
    if (!/viewBox="/.test(svg)) failures.push(`${file}: no viewBox`);
    if (/<script\b/i.test(svg)) failures.push(`${file}: contains <script>`);
    if (svg.length < 1000) failures.push(`${file}: suspiciously small`);

    const rootTag = svg.match(/<svg\b[^>]*>/)?.[0] ?? "";
    const labelledBy = rootTag.match(/aria-labelledby="([^"]+)"/)?.[1]?.trim().split(/\s+/).filter(Boolean) ?? [];
    if (labelledBy.length === 0) {
      failures.push(`${file}: root <svg> has no aria-labelledby`);
    } else {
      for (const id of labelledBy) {
        if (!new RegExp(`\\bid="${id}"`).test(svg)) {
          failures.push(`${file}: aria-labelledby references missing id "${id}"`);
        }
      }
    }
  }
}

if (failures.length > 0) {
  console.error(["diagram check FAIL:", ...failures].join("\n"));
  process.exit(1);
}

console.log(`diagram check: ${specs.length} specs, ${specs.length * 2} SVGs OK`);
