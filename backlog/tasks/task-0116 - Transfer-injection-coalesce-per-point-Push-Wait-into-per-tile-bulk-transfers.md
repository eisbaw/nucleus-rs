---
id: TASK-0116
title: 'Transfer-injection: coalesce per-point Push/Wait into per-tile bulk transfers'
status: To Do
assignee: []
created_date: '2026-05-18 01:44'
labels:
  - M1
  - compiler
  - ir
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Currently transfer_inject.rs emits one Push/Wait pair per consumer iteration when the consumer sits inside a for-loop. A real backend bulk-sends a tile. Coalesce contiguous iterations into a single Push/Wait whose IterTile is the outer loop's full range, when the producer's tile and the consumer's tile match exactly. Requires access-pattern analysis on Operation.dataflow.edges (currently absent at M1).
<!-- SECTION:DESCRIPTION:END -->
