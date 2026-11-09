#!/usr/bin/env node
// Export a delivered Archify HTML diagram as standalone light/dark SVGs.
//
// The Archify CLI renders HTML only. This helper drives the packaged
// ChromeVisualBrowser over the DevTools pipe, clones the inline <svg>, and
// inlines the computed presentation properties so the result renders the same
// on GitHub and in VitePress without the viewer runtime.
//
// Usage (from the repo root):
//   ARCHIFY_SKILL=/path/to/archify ARCHIFY_CHROME=/path/to/chrome \
//     node tooling/diagrams/export-svg.mjs /tmp/spoke-overview.html docs/public/diagrams

import { mkdirSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const [htmlPath, outDirArg] = process.argv.slice(2);
if (!htmlPath) {
  console.error("usage: export-svg.mjs <diagram.html> [out-dir]");
  process.exit(2);
}

const skill = resolve(
  process.env.ARCHIFY_SKILL || join(process.env.HOME || "", ".agents/skills/archify"),
);
const { ChromeVisualBrowser } = await import(join(skill, "bin/visual-check.mjs"));
const chrome = process.env.ARCHIFY_CHROME;
if (!chrome) {
  console.error("ARCHIFY_CHROME must point at a Chrome/Chromium executable");
  process.exit(2);
}

const input = resolve(htmlPath);
const outDir = resolve(outDirArg || dirname(input));
const name = basename(input).replace(/\.html?$/i, "");
mkdirSync(outDir, { recursive: true });

const INLINE_PROPS = [
  "fill",
  "fill-opacity",
  "fill-rule",
  "stroke",
  "stroke-width",
  "stroke-opacity",
  "stroke-dasharray",
  "stroke-dashoffset",
  "stroke-linecap",
  "stroke-linejoin",
  "opacity",
  "font-size",
  "font-weight",
  "font-style",
  "letter-spacing",
  "text-anchor",
  "dominant-baseline",
  "paint-order",
  "marker-end",
  "marker-start",
  "marker-mid",
  "visibility",
  "shape-rendering",
];

const SNAPSHOT = `(function () {
  var source = document.querySelector('.diagram-container > svg')
    || document.querySelector('.diagram-container > .diagram-stage > svg')
    || document.querySelector('svg');
  if (!source) throw new Error('no svg found');
  var clone = source.cloneNode(true);
  var originals = [source].concat(Array.from(source.querySelectorAll('*')));
  var copies = [clone].concat(Array.from(clone.querySelectorAll('*')));
  var props = ${JSON.stringify(INLINE_PROPS)};
  for (var i = 0; i < originals.length; i++) {
    var computed = getComputedStyle(originals[i]);
    var target = copies[i];
    for (var p = 0; p < props.length; p++) {
      var value = computed.getPropertyValue(props[p]);
      var paint = props[p] === 'fill' || props[p] === 'stroke';
      if (value && (paint || (value !== 'none' && value !== 'normal' && value !== 'auto'))) {
        target.setAttribute(props[p], value);
      }
    }
    if (target.tagName && target.tagName.toLowerCase() === 'text') {
      target.setAttribute('font-family', 'ui-sans-serif, system-ui, -apple-system, Segoe UI, sans-serif');
    }
    target.removeAttribute('class');
  }
  Array.from(clone.querySelectorAll('[class]')).forEach(function (el) { el.removeAttribute('class'); });
  clone.setAttribute('xmlns', 'http://www.w3.org/2000/svg');
  clone.setAttribute('xmlns:xlink', 'http://www.w3.org/1999/xlink');
  var box = source.viewBox.baseVal;
  clone.setAttribute('width', String(box.width));
  clone.setAttribute('height', String(box.height));
  var bg = getComputedStyle(document.documentElement).getPropertyValue('--bg').trim();
  if (bg) {
    var backdrop = document.createElementNS('http://www.w3.org/2000/svg', 'rect');
    backdrop.setAttribute('width', '100%');
    backdrop.setAttribute('height', '100%');
    backdrop.setAttribute('fill', bg);
    var first = clone.firstElementChild;
    clone.insertBefore(backdrop, first && first.nextSibling ? first.nextSibling : clone.firstChild);
  }
  return '<?xml version="1.0" encoding="UTF-8"?>\\n' + new XMLSerializer().serializeToString(clone);
})()`;

const browser = new ChromeVisualBrowser(chrome);
try {
  const sessionId = await browser.sessionPromise;
  for (const theme of ["light", "dark"]) {
    const url = new URL(pathToFileURL(input).href);
    url.searchParams.set("theme", theme);
    url.searchParams.set("embed", "1");
    const loaded = browser.cdp.waitFor("Page.loadEventFired", sessionId);
    const nav = await browser.cdp.send("Page.navigate", { url: url.href }, sessionId);
    if (nav.errorText) throw new Error(`navigation failed: ${nav.errorText}`);
    await loaded;
    await browser.cdp.send(
      "Runtime.evaluate",
      {
        expression:
          "document.documentElement.setAttribute('data-motion','still');" +
          "(document.fonts && document.fonts.ready ? document.fonts.ready.catch(()=>{}) : Promise.resolve())" +
          ".then(()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r))))",
        awaitPromise: true,
      },
      sessionId,
    );
    const response = await browser.cdp.send(
      "Runtime.evaluate",
      { expression: SNAPSHOT, awaitPromise: false, returnByValue: true },
      sessionId,
    );
    if (response.exceptionDetails) {
      throw new Error(response.exceptionDetails.exception?.description || response.exceptionDetails.text);
    }
    const svg = response.result?.value;
    if (!svg || !svg.includes("<svg")) throw new Error(`empty ${theme} snapshot`);
    const file = join(outDir, `${name}.${theme}.svg`);
    writeFileSync(file, svg);
    console.log(`${theme}: ${file} (${svg.length} bytes)`);
  }
} finally {
  await browser.close();
}
