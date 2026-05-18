---
id: TASK-0146
title: 'Petri net inspection: per-worker projection rendered alongside global net'
status: To Do
assignee: []
created_date: '2026-05-18 05:18'
labels: []
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0035 implemented `--emit-pn` rendering the global Petri net with per-worker colouring via Graphviz subgraph clusters. PRD §8.5 also mentions "per-worker projection shown by colour". For large nets, a separate small graph per worker (alongside the global net) is more readable than a single cluster-coloured global. Add a sibling renderer that emits N+1 DOT files (one global + one per worker projection), or an `--emit-pn-projection DIR` flag. Decide on interface when a real example asks for it.
<!-- SECTION:DESCRIPTION:END -->
