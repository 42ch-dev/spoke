#!/usr/bin/env node
// EN↔CN twin-parity gate for the VitePress docs site.
//
// Counts `*.md` pages under `docs/` (EN root locale) and `docs/zh/` (CN
// locale) and fails when a page exists in one locale without its twin in
// the other. For every EN/CN pair, the sequence of markdown heading levels
// (depth only, text-free) must match 1:1.
//
// Run from the repo root: `node tooling/docs/twin-parity.mjs`
// (the docs workflow runs this before `pnpm docs:build`).
//
// Locale-specific pages (legitimately present in only one locale) go into
// the `localeSpecific` list below, as docs-relative paths with `/`
// separators (e.g. "guide/roadmap.md"). A listed path is ignored in both
// directions and must match a real page in at least one locale, otherwise a
// warning is printed (a typo'd entry would silently skip nothing).

import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const DOCS_DIR = join(REPO_ROOT, "docs");

// Locale-specific pages, docs-relative with `/` separators. Default empty:
// EN and CN are 1:1 twins.
const localeSpecific = [];

function walk(base, dir, out, skipDirs) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (skipDirs.includes(entry.name)) continue;
      walk(base, full, out, skipDirs);
    } else if (entry.isFile() && entry.name.endsWith(".md")) {
      out.add(relative(base, full).split(sep).join("/"));
    }
  }
}

function collectLocale(dir, base, skipDirs) {
  const out = new Set();
  if (statSync(dir, { throwIfNoEntry: false })?.isDirectory()) {
    walk(base, dir, out, skipDirs);
  }
  return out;
}

function headingLevels(mdRel) {
  const content = readFileSync(join(DOCS_DIR, mdRel), "utf8");
  const levels = [];
  let inFence = false;
  for (const line of content.split("\n")) {
    const trimmed = line.trimStart();
    if (trimmed.startsWith("```")) {
      inFence = !inFence;
      continue;
    }
    if (inFence) continue;
    const m = line.match(/^(#{1,6})\s+/);
    if (m) levels.push(m[1].length);
  }
  return levels;
}

function formatLevels(levels) {
  return `[${levels.join(",")}]`;
}

// EN pages: everything under docs/ except the zh locale and VitePress dirs.
const en = collectLocale(DOCS_DIR, DOCS_DIR, ["zh", ".vitepress"]);
// CN pages: everything under docs/zh/, with the `zh/` prefix stripped so
// both sets share the same docs-relative path space.
const zhRaw = collectLocale(join(DOCS_DIR, "zh"), DOCS_DIR, []);
const zh = new Set([...zhRaw].map((p) => p.slice("zh/".length)));

for (const p of localeSpecific) {
  if (!en.has(p) && !zh.has(p)) {
    console.warn(`warn: localeSpecific "${p}" matches no page in either locale (typo?)`);
  }
  en.delete(p);
  zh.delete(p);
}

const missingInZh = [...en].filter((p) => !zh.has(p)).sort();
const missingInEn = [...zh].filter((p) => !en.has(p)).sort();

const headingMismatches = [];
const twinPaths = [...en].filter((p) => zh.has(p)).sort();
for (const p of twinPaths) {
  const enLevels = headingLevels(p);
  const zhLevels = headingLevels(`zh/${p}`);
  if (enLevels.length !== zhLevels.length || enLevels.some((lv, i) => lv !== zhLevels[i])) {
    headingMismatches.push({
      path: p,
      en: formatLevels(enLevels),
      zh: formatLevels(zhLevels),
    });
  }
}

console.log(
  `docs twin-parity: EN ${en.size} pages ↔ CN ${zh.size} pages (locale-specific: ${localeSpecific.length})`,
);

let exitCode = 0;

if (missingInZh.length === 0 && missingInEn.length === 0) {
  console.log("docs twin-parity: OK — every page has its twin");
} else {
  console.error("docs twin-parity: FAIL — pages missing a twin in the other locale:");
  if (missingInZh.length > 0) {
    console.error("  in CN (EN page without a CN twin):");
    for (const p of missingInZh) console.error(`    - ${p}`);
  }
  if (missingInEn.length > 0) {
    console.error("  in EN (CN page without an EN twin):");
    for (const p of missingInEn) console.error(`    - ${p}`);
  }
  exitCode = 1;
}

console.log(
  `docs twin-parity: ${twinPaths.length} twin pairs, ${headingMismatches.length} heading-structure mismatches`,
);

if (headingMismatches.length === 0) {
  console.log("docs twin-parity: OK — heading structure matches for every twin pair");
} else {
  console.error("docs twin-parity: FAIL — heading structure mismatch:");
  for (const { path, en: enFmt, zh: zhFmt } of headingMismatches) {
    console.error(`  ${path}`);
    console.error(`    EN: ${enFmt}`);
    console.error(`    zh: ${zhFmt}`);
  }
  exitCode = 1;
}

process.exit(exitCode);
