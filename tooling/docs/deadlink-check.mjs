#!/usr/bin/env node
// Internal dead-link gate for the built VitePress docs site.
//
// Crawls `docs/.vitepress/dist` (run `pnpm docs:build` first) and verifies
// that every internal `<a href>` resolves to a real page in the build
// output. Only same-site links are checked: root-relative hrefs under the
// site base `/spoke/` and relative hrefs resolved against the referring
// page. External URLs (`https://…`, `mailto:`, `//…`) and in-page
// `#fragments` are ignored. The gate exits non-zero listing
// `{referrer, href, resolved}` for every miss.
//
// simplify: HTML is parsed with a regex over the `<a href>` attribute
// instead of a full HTML parser. VitePress emits deterministic markup, so
// the regex is exact for this build; if the site ever ships hand-authored
// HTML with unusual anchor markup, re-evaluate against a real parser.

import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, posix, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "../..");
const DIST_DIR = join(REPO_ROOT, "docs", ".vitepress", "dist");
const BASE = "/spoke/";

const ANCHOR_HREF = /<a\b[^>]*\bhref\s*=\s*["']([^"']*)["']/g;

function isFile(distRel) {
  const st = statSync(join(DIST_DIR, distRel), { throwIfNoEntry: false });
  return st?.isFile() ?? false;
}

// True when a dist-relative path resolves to an emitted page. Handles the
// shapes VitePress actually emits: `foo.html`, bare `foo`, and `foo/`
// (directory-style → `foo/index.html`).
function pageExists(distRel) {
  if (distRel === "") return isFile("index.html");
  if (distRel.endsWith("/")) return isFile(`${distRel}index.html`);
  return isFile(distRel) || isFile(`${distRel}.html`) || isFile(`${distRel}/index.html`);
}

// Map a raw href to a dist-relative path, or skip when it is not an
// internal page link (external scheme, protocol-relative, in-page anchor).
// `note` marks a link that can never resolve on the live site (outside the
// base, or escaping the build output) — reported as dead, not skipped.
function resolveHref(href, referrerRel) {
  const pathOnly = href.split("#")[0].split("?")[0];
  if (pathOnly === "") return { skip: true }; // bare in-page anchor
  if (/^[a-z][a-z0-9+.-]*:/i.test(pathOnly)) return { skip: true }; // http:, mailto:, …
  if (pathOnly.startsWith("//")) return { skip: true }; // protocol-relative

  let rel;
  if (pathOnly.startsWith("/")) {
    // Root-relative: must live under the site base `/spoke/`.
    if (pathOnly === BASE || pathOnly === "/spoke") {
      rel = "";
    } else if (pathOnly.startsWith(BASE)) {
      rel = posix.normalize(pathOnly.slice(BASE.length));
    } else {
      return { rel: pathOnly, note: "not under the site base /spoke/" };
    }
  } else {
    rel = posix.normalize(posix.join(posix.dirname(referrerRel), pathOnly));
  }
  if (rel === ".." || rel.startsWith("../")) {
    return { rel, note: "escapes the build output" };
  }
  return { rel };
}

if (!statSync(DIST_DIR, { throwIfNoEntry: false })?.isDirectory()) {
  console.error(`deadlink: build output not found at ${DIST_DIR}`);
  console.error("deadlink: run `pnpm docs:build` first");
  process.exit(1);
}

const htmlFiles = [];
(function walk(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) walk(full);
    else if (entry.isFile() && entry.name.endsWith(".html")) htmlFiles.push(full);
  }
})(DIST_DIR);
htmlFiles.sort();

const deadLinks = [];
let linksChecked = 0;
for (const file of htmlFiles) {
  const referrerRel = relative(DIST_DIR, file).split(sep).join("/");
  const html = readFileSync(file, "utf8");
  for (const match of html.matchAll(ANCHOR_HREF)) {
    linksChecked += 1;
    const resolved = resolveHref(match[1], referrerRel);
    if (resolved.skip) continue;
    if (resolved.note || !pageExists(resolved.rel)) {
      deadLinks.push({
        referrer: referrerRel,
        href: match[1],
        resolved: resolved.rel,
        note: resolved.note,
      });
    }
  }
}

console.log(
  `deadlink: ${htmlFiles.length} pages, ${linksChecked} anchor links checked, ${deadLinks.length} dead`,
);

if (deadLinks.length === 0) {
  console.log("deadlink: OK — all internal links resolve");
  process.exit(0);
}

console.error("deadlink: FAIL — internal links that resolve to nothing:");
for (const { referrer, href, resolved, note } of deadLinks.sort((a, b) => a.referrer.localeCompare(b.referrer))) {
  console.error(`  referrer: ${referrer}`);
  console.error(`    href: ${href}  (resolved: ${resolved}${note ? `; ${note}` : ""})`);
}
process.exit(1);
