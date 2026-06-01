// bundle.mjs — turn a Marp-rendered deck into a single self-contained SPA.
//
// Marp's `--theme-set` already inlines the theme CSS and the bespoke
// navigation JS into the HTML, so the ONLY external dependency left is the
// 8 SVG diagrams referenced as `<img src="assets/*.svg">`. This script
// replaces every such reference with a base64 `data:image/svg+xml` URI,
// reading each SVG from the `assets/` dir beside the HTML.
//
// Why base64 data: URIs in <img> (and NOT raw inline <svg>):
//   - Each SVG stays an ISOLATED document, so its internal <style>
//     @keyframes / class selectors cannot collide with the page or with
//     the other 7 SVGs (raw-inlining would leak/merge those styles).
//   - SMIL <animate> and CSS @keyframes both still run inside an
//     <img>-referenced data: URI, so the animations are preserved.
//   - Marp's per-image width/style attributes on the <img> are untouched.
//
// Usage:  node bundle.mjs <index.html>   (rewrites the file in place)

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const htmlPath = process.argv[2];
if (!htmlPath) {
  console.error("usage: node bundle.mjs <index.html>");
  process.exit(2);
}

const baseDir = dirname(htmlPath);
let html = readFileSync(htmlPath, "utf8");

// Distinct `assets/<name>.svg` tokens referenced by the document.
const re = /assets\/([A-Za-z0-9._-]+\.svg)/g;
const names = [...new Set([...html.matchAll(re)].map((m) => m[1]))];

if (names.length === 0) {
  console.log("bundle.mjs: no assets/*.svg references found (already bundled?)");
  process.exit(0);
}

let inlined = 0;
for (const name of names) {
  const svgPath = join(baseDir, "assets", name);
  // readFileSync throws (loud) if a referenced asset is missing.
  const b64 = readFileSync(svgPath).toString("base64");
  const dataUri = `data:image/svg+xml;base64,${b64}`;
  // Literal global replace of the exact reference token.
  html = html.split(`assets/${name}`).join(dataUri);
  inlined++;
}

writeFileSync(htmlPath, html);

// A residual `assets/<name>.svg` cannot hide inside a base64 blob: the
// base64 alphabet has no '.', so the literal `.svg` can never match there.
const residual = [...new Set([...html.matchAll(re)].map((m) => m[0]))];
if (residual.length > 0) {
  console.error("bundle.mjs FAIL: residual external refs remain:", residual);
  process.exit(1);
}

console.log(
  `bundle.mjs: inlined ${inlined} SVG(s) as data: URIs; ` +
    `0 residual external refs; ${(Buffer.byteLength(html) / 1024).toFixed(0)} KiB self-contained`,
);
