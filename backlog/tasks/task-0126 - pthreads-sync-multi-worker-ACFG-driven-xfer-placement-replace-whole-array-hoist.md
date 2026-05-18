---
id: TASK-0126
title: >-
  pthreads-sync multi-worker: ACFG-driven xfer placement (replace whole-array
  hoist)
status: To Do
assignee: []
created_date: '2026-05-18 02:51'
labels:
  - M1
  - backend
  - codegen
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0122 codegen synthesises transfers from linked.data_producers/data_consumers, ignoring the ACFG's Xfer placeholders. This works because the transfer_inject pass produces unmatched Waits (no matching Pushes for outer-scope producers, splice_pushes_for_waits is scope-local). Once that gap is fixed and per-tile coalescing (TASK-0116) lands, the multi-worker codegen should walk the ACFG's Xfer nodes directly to emit Push/Wait per-tile. Currently a whole-array Slot<Vec<i32>> per data symbol is used regardless of consumer granularity.
<!-- SECTION:DESCRIPTION:END -->
