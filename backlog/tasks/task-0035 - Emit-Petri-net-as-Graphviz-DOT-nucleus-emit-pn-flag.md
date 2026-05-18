---
id: TASK-0035
title: Emit Petri net as Graphviz DOT (nucleus --emit-pn flag)
status: To Do
assignee: []
created_date: '2026-05-17 23:06'
labels:
  - M2
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
CLI flag that dumps the global Petri net as a DOT file. Visualisation shows places, transitions, arcs, initial markings, capacities, with per-worker projection by colour. PRD §8.5.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 'nucleus build ... --emit-pn out.dot' writes a Graphviz DOT file alongside the regular build output.
- [ ] #2 Places labelled with name + capacity; transitions with name + worker; initial markings rendered as dots inside places.
- [ ] #3 Per-worker colouring: each transition node is filled with the worker's distinct colour.
- [ ] #4 Test: golden DOT files committed for each example × required schedule pair; CI diffs them.
- [ ] #5 Implementation notes record design questions (e.g. whether to render a separate per-worker view alongside the global net).
- [ ] #6 Implementation notes record honest limitations (very large nets become unreadable; v2 ships best-effort layout, not a custom layout engine).
<!-- AC:END -->
