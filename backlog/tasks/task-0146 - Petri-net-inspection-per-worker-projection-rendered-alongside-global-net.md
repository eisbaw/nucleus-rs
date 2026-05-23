---
id: TASK-0146
title: 'Petri net inspection: per-worker projection rendered alongside global net'
status: Done
assignee: []
created_date: '2026-05-18 05:18'
updated_date: '2026-05-23 21:07'
labels: []
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0035 implemented `--emit-pn` rendering the global Petri net with per-worker colouring via Graphviz subgraph clusters. PRD §8.5 also mentions "per-worker projection shown by colour". For large nets, a separate small graph per worker (alongside the global net) is more readable than a single cluster-coloured global. Add a sibling renderer that emits N+1 DOT files (one global + one per worker projection), or an `--emit-pn-projection DIR` flag. Decide on interface when a real example asks for it.
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED (orchestrator-direct, cycle 77 sweep). Description: 'Decide on interface when a real example asks for it.' No real example has requested per-worker projection — the existing --emit-pn cluster-coloured global net works for current example sizes. Reopen when a large net (e.g. 13-cnn-inference at higher batch size, or 14-hearing-aid M11 multi-MCU) makes the cluster-coloured form unreadable in practice. Same deferred-closure pattern as TASK-0147/0148 visualization tasks.
<!-- SECTION:FINAL_SUMMARY:END -->
