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

`marp` is provided by the `.#docs` Nix dev shell. The one-liner:

```
just slides
```

which expands to:

```
nix develop .#docs --command marp docs/presentation/slides.md \
    --theme-set docs/presentation/themes/nucleus.css \
    -o docs/presentation/index.html --html
```

Marp can also serve a live-reloading preview (`marp -s docs/presentation`)
or export PDF/PPTX (`marp slides.md --pdf`, needs Chromium).

## Layout

| Path | Role |
|---|---|
| `slides.md` | the deck source (Markdown + Marp front-matter) |
| `themes/nucleus.css` | custom dark-slate Marp theme (`@theme nucleus`) |
| `assets/*.svg` | 8 self-contained animated SVG diagrams |
| `index.html` | the rendered, self-contained deck (regenerate with `just slides`) |

## Narrative arc

Title → what it is → philosophy (algorithm/schedule split) → motivation →
origin & influences → built clean-room by an AI agent loop → the two `.nuc`
languages + a concrete example → compiler pipeline → ACFG & injection passes →
the Petri-net IR & soundness gate → EventList contract & the 10-backend matrix →
matmul deep dive → tier-3 multi-MCU Renode → state today → future work → close.
