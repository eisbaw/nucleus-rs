# Nucleus v2 — thesis presentation

A [Marp / Marpit](https://marp.app/) deck on the Nucleus v2 algorithm/schedule
compiler, with animated SVG diagrams.

## View

Open the pre-built deck in a browser:

```
xdg-open docs/presentation/index.html
```

The SVGs animate when viewed as a normal web page (SMIL + CSS keyframes,
no JavaScript). Use the browser; a static PDF/PNG export freezes the
animation on its first frame.

## Build / re-render

`marp` + `node` are provided by the `.#docs` Nix dev shell:

```
just slides
```

This runs two steps in the `.#docs` shell:

1. **`marp`** renders `slides.md` → `index.html`, inlining the theme CSS
   and the navigation JS (`--theme-set ... --html`).
2. **`node bundle.mjs index.html`** inlines the 8 SVG diagrams as base64
   `data:` URIs.

The result, `index.html`, is a **single self-contained SPA** — one file
with no sibling dependencies (theme, nav JS, and all animated SVGs are
embedded). You can copy it anywhere and open it offline; the SVGs still
animate (they stay isolated `<img>` documents, so their internal
`@keyframes`/SMIL don't collide). Verified by rendering it from a
directory containing nothing but the HTML.

Marp can also serve a live-reloading preview (`marp -s docs/presentation`,
referencing the un-bundled `assets/`) or export PDF/PPTX
(`marp slides.md --pdf`, needs Chromium).

## Layout

| Path | Role |
|---|---|
| `slides.md` | the deck source (Markdown + Marp front-matter) |
| `themes/nucleus.css` | custom dark-slate Marp theme (`@theme nucleus`) |
| `assets/*.svg` | 8 self-contained animated SVG diagrams (build inputs) |
| `bundle.mjs` | post-build step: inlines the SVGs as base64 `data:` URIs |
| `index.html` | the single self-contained SPA (regenerate with `just slides`) |

## Narrative arc

Title → what it is → philosophy (algorithm/schedule split) → motivation →
origin & influences → built clean-room by an AI agent loop → the two `.nuc`
languages + a concrete example → compiler pipeline → ACFG & injection passes →
the Petri-net IR & soundness gate → EventList contract & the 10-backend matrix →
matmul deep dive → tier-3 multi-MCU Renode → state today → future work → close.
