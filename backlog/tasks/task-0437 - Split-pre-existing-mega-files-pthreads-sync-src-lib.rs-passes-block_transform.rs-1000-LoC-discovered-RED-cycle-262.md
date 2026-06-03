---
id: TASK-0437
title: >-
  Split pre-existing mega-files pthreads-sync/src/lib.rs +
  passes/block_transform.rs (>1000 LoC; discovered RED cycle-262)
status: To Do
assignee: []
created_date: '2026-06-03 17:58'
updated_date: '2026-06-03 17:59'
labels: []
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The just-ci check-mega-files gate was found RED at cycle-262 HEAD: nucleus/backends/pthreads-sync/src/lib.rs (1032 LoC at HEAD, grew to ~1097 after the cycle-262 break_loop.rs extraction) and nucleus/nucleus-compiler/src/passes/block_transform.rs (1043 LoC, untouched by cycle-262) BOTH exceed 1000 LoC and are NOT in the check-mega-files allow-list. This is the feedback-cheap-subset-blind-to-structural-fences recurrence: the cheap pre-commit subset (build/clippy/test/test-release/e2e) does not run check-mega-files, so a file silently crossing 1000 LoC sat RED. Cycle-262 (TASK-0341.02.01.06) extracted the new for..until break machinery into break_loop.rs (222 LoC) to AVOID worsening lib.rs further, and ALLOW-LISTED both files with a rationale to keep just ci green, but the proper fix is a SPLIT. lib.rs: the ~520-LoC render_event fn is the bulk; split along the Event:: arm seams (Fire / Loop / Sync-Push-Wait) named in the module docstring. block_transform.rs: split along its strip-mine tile/seq/inner construction seams. Preferred fix per the gate is option #1 (split into cohesive sub-modules), removing the allow-list entries afterward (direction-B stale-entry guard).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
THIRD pre-existing offender found: nucleus/nucleus-compiler/src/event.rs (1036 LoC at HEAD, untouched by cycle-262 — the EventList contract types Event/FireBinding/DataSlice/IterTile + serde). All THREE (event.rs, pthreads-sync/lib.rs, block_transform.rs) added to the check-mega-files allow-list cycle-262 to restore just-ci GREEN; proper split (option #1) + allow-list removal is this task. event.rs split seam: the Event enum variants vs the binding/slice value types vs the serde impls.
<!-- SECTION:NOTES:END -->
