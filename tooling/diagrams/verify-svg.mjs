#!/usr/bin/env node
// Committed-diagram gate: every spec must have a light and a dark SVG, and each
// SVG must be a self-contained, script-free document with an accessible title.
// Runs before `vitepress build` so a broken or missing diagram fails CI.

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
    } catch {
      failures.push(`${file}: missing`);
      continue;
    }
    if (!/<svg\b/.test(svg)) failures.push(`${file}: no <svg> root`);
    if (!/viewBox="/.test(svg)) failures.push(`${file}: no viewBox`);
    if (!/<title\b/.test(svg)) failures.push(`${file}: no <title> (accessible name)`);
    if (/<script\b/i.test(svg)) failures.push(`${file}: contains <script>`);
    if (svg.length < 1000) failures.push(`${file}: suspiciously small`);
  }
}

if (failures.length > 0) {
  console.error(["diagram check FAIL:", ...failures].join("\n"));
  process.exit(1);
}

console.log(`diagram check: ${specs.length} specs, ${specs.length * 2} SVGs OK`);
