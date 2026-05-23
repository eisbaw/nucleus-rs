---
id: TASK-0126
title: >-
  pthreads-sync multi-worker: ACFG-driven xfer placement (replace whole-array
  hoist)
status: Done
assignee: []
created_date: '2026-05-18 02:51'
updated_date: '2026-05-23 21:33'
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

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-transitively (orchestrator-direct, cycle 77 sweep). The task says 'Once [TASK-0149's gap is fixed] and per-tile coalescing (TASK-0116) lands, the multi-worker codegen should walk the ACFG's Xfer nodes directly.' BOTH dependencies closed cycle 77 as deferred-no-driver (TASK-0149 and TASK-0116). The current whole-array Slot<Vec<i32>> approach has been correct + sufficient for every tier-1 multi-worker e2e cell. The 'walk ACFG's Xfer nodes directly' optimization is a precision upgrade with no perf or correctness driver. Reopen when TASK-0149 + TASK-0116 reopen on real drivers (a real perf measurement or a SKIPPED-blocked-multi-worker cell unskipping). Same transitively-deferred pattern as TASK-0114 (blocked on TASK-0110).
<!-- SECTION:FINAL_SUMMARY:END -->
