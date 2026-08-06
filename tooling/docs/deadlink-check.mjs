#!/usr/bin/env node
// Internal dead-link gate for the built VitePress docs site.
//
// Crawls `docs/.vitepress/dist` (run `pnpm docs:build` first) and verifies
// that every internal `<a href>` resolves to a real page in the build
// output. Only same-site links are checked: root-relative hrefs under the
// site base and relative hrefs resolved against the referring page. External
// URLs (`https://…`, `mailto:`, `//…`) are skipped for page resolution.
//
// After the page-link crawl, a fragment audit scans `docs/**/*.md` for every
// in-page and cross-page `#fragment` markdown link and verifies the target
// id exists in the emitted `docs/.vitepress/dist/**/*.html`.
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
const DOCS_DIR = join(REPO_ROOT, "docs");

// Derive the site base from the VitePress config so the gate works with
// any base (e.g. `/spoke/` for bare project sites, `/` for custom domains).
// Falls back to `/` if the config line is not found.
function readBase() {
  try {
    const cfg = readFileSync(join(REPO_ROOT, "docs", ".vitepress", "config.mts"), "utf8");
    const m = cfg.match(/const\s+base\s*=\s*["']([^"']*)["']/);
    let b = m ? m[1] : "/";
    if (!b.startsWith("/")) b = "/" + b;
    if (!b.endsWith("/")) b += "/";
    return b;
  } catch {
    return "/";
  }
}
const BASE = readBase();

const ANCHOR_HREF = /<a\b[^>]*\bhref\s*=\s*["']([^"']*)["']/g;
const HTML_ID = /\bid="([^"]+)"/g;
const MARKDOWN_LINK = /\[([^\]]*)\]\(([^)]+)\)/g;

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
  const baseTrimmed = BASE.replace(/\/$/, ""); // "/spoke" or ""
  if (pathOnly.startsWith("/")) {
    // Root-relative: must live under the site base.
    if (pathOnly === BASE || pathOnly === baseTrimmed || pathOnly === baseTrimmed + "/") {
      rel = "";
    } else if (pathOnly.startsWith(BASE)) {
      rel = posix.normalize(pathOnly.slice(BASE.length));
    } else if (BASE === "/") {
      // base is root; any root-relative link is internal
      rel = posix.normalize(pathOnly.slice(1));
    } else {
      return { rel: pathOnly, note: `not under the site base ${BASE}` };
    }
  } else {
    rel = posix.normalize(posix.join(posix.dirname(referrerRel), pathOnly));
  }
  if (rel === ".." || rel.startsWith("../")) {
    return { rel, note: "escapes the build output" };
  }
  return { rel };
}

function distPagePath(distRel) {
  if (pageExists(distRel)) {
    if (distRel === "" || distRel === "index.html") return "index.html";
    if (isFile(distRel)) return distRel;
    if (isFile(`${distRel}.html`)) return `${distRel}.html`;
    if (isFile(`${distRel}/index.html`)) return `${distRel}/index.html`;
    if (distRel.endsWith("/") && isFile(`${distRel}index.html`)) return `${distRel}index.html`;
  }
  return null;
}

function markdownToDist(mdRel) {
  const withoutExt = mdRel.replace(/\.md$/, "");
  if (withoutExt === "index") return "index.html";
  return `${withoutExt}.html`;
}

function resolveMarkdownFragmentLink(href, sourceMdRel) {
  const hashIdx = href.indexOf("#");
  if (hashIdx === -1) return { skip: true };
  const fragment = decodeURIComponent(href.slice(hashIdx + 1));
  if (!fragment) return { skip: true };

  const pathPart = href.slice(0, hashIdx).split("?")[0];
  if (/^[a-z][a-z0-9+.-]*:/i.test(pathPart)) return { skip: true };
  if (pathPart.startsWith("//")) return { skip: true };

  let targetMdRel;
  if (pathPart === "") {
    targetMdRel = sourceMdRel;
  } else if (pathPart.startsWith("/")) {
    const baseTrimmed = BASE.replace(/\/$/, "");
    let sitePath = pathPart;
    if (BASE !== "/" && !sitePath.startsWith(BASE) && sitePath !== baseTrimmed) {
      return { skip: true, note: `not under the site base ${BASE}` };
    }
    if (BASE !== "/" && sitePath.startsWith(BASE)) {
      sitePath = sitePath.slice(BASE.length - 1);
    }
    sitePath = sitePath.replace(/^\//, "").replace(/\/$/, "");
    targetMdRel = sitePath === "" ? "index.md" : `${sitePath}.md`;
  } else {
    const sourceDir = posix.dirname(sourceMdRel);
    targetMdRel = posix.normalize(posix.join(sourceDir, pathPart));
    if (targetMdRel.endsWith("/")) targetMdRel += "index.md";
    else if (!targetMdRel.endsWith(".md")) targetMdRel += ".md";
    if (targetMdRel.startsWith("../")) {
      return { skip: true, note: "escapes docs/" };
    }
  }

  const distRel = markdownToDist(targetMdRel);
  return { fragment, distRel, targetMdRel };
}

function collectMarkdownFiles() {
  const out = [];
  (function walk(dir, skipDirs) {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        if (skipDirs.includes(entry.name)) continue;
        walk(full, skipDirs);
      } else if (entry.isFile() && entry.name.endsWith(".md")) {
        out.push(relative(DOCS_DIR, full).split(sep).join("/"));
      }
    }
  })(DOCS_DIR, [".vitepress"]);
  out.sort();
  return out;
}

function collectHtmlIds(htmlPath) {
  const html = readFileSync(htmlPath, "utf8");
  const ids = new Set();
  for (const match of html.matchAll(HTML_ID)) ids.add(match[1]);
  return ids;
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

let exitCode = 0;

if (deadLinks.length === 0) {
  console.log("deadlink: OK — all internal links resolve");
} else {
  console.error("deadlink: FAIL — internal links that resolve to nothing:");
  for (const { referrer, href, resolved, note } of deadLinks.sort((a, b) =>
    a.referrer.localeCompare(b.referrer),
  )) {
    console.error(`  referrer: ${referrer}`);
    console.error(`    href: ${href}  (resolved: ${resolved}${note ? `; ${note}` : ""})`);
  }
  exitCode = 1;
}

// Fragment audit: every `#fragment` in docs/**/*.md must exist on the target page.
const idCache = new Map();
function idsForDistPage(distRel) {
  const page = distPagePath(distRel);
  if (!page) return null;
  if (!idCache.has(page)) {
    idCache.set(page, collectHtmlIds(join(DIST_DIR, page)));
  }
  return idCache.get(page);
}

const brokenFragments = [];
let fragmentsChecked = 0;
for (const mdRel of collectMarkdownFiles()) {
  const content = readFileSync(join(DOCS_DIR, mdRel), "utf8");
  for (const match of content.matchAll(MARKDOWN_LINK)) {
    const href = match[2];
    const resolved = resolveMarkdownFragmentLink(href, mdRel);
    if (resolved.skip) continue;
    fragmentsChecked += 1;
    const page = distPagePath(resolved.distRel);
    if (!page) {
      brokenFragments.push({
        source: mdRel,
        href,
        target: resolved.targetMdRel,
        fragment: resolved.fragment,
        note: "target page missing from build output",
      });
      continue;
    }
    const ids = idsForDistPage(resolved.distRel);
    if (!ids?.has(resolved.fragment)) {
      brokenFragments.push({
        source: mdRel,
        href,
        target: resolved.targetMdRel,
        page,
        fragment: resolved.fragment,
      });
    }
  }
}

console.log(
  `fragment: ${fragmentsChecked} markdown fragment links checked, ${brokenFragments.length} broken`,
);

if (brokenFragments.length === 0) {
  console.log("fragment: OK — all markdown fragment links resolve");
} else {
  console.error("fragment: FAIL — markdown fragment links with no matching id:");
  for (const item of brokenFragments.sort((a, b) => a.source.localeCompare(b.source))) {
    console.error(`  source: ${item.source}`);
    console.error(`    href: ${item.href}`);
    if (item.page) console.error(`    page: ${item.page}  fragment: #${item.fragment}`);
    else console.error(`    target: ${item.target}${item.note ? ` (${item.note})` : ""}`);
  }
  exitCode = 1;
}

process.exit(exitCode);
