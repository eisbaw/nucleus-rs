---
id: TASK-0376
title: >-
  08-histogram native data-dependent WRITE (scatter) histogram[bin]
  (TASK-0044.04 companion to gather)
status: To Do
assignee: []
created_date: '2026-05-30 22:46'
labels:
  - compiler
  - scatter
  - histogram
  - broaden
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
BROADEN: the data-dependent WRITE / scatter sibling of the gather (TASK-0341.03.01 landed the READ). 08-histogram fakes histogram[bin] <-- bin_inc(...) with a rectangular masked accumulator over (i,b) gated on value==bin. The native form histogram[bin] <-- inc(histogram[bin]) where bin is a loaded value needs a data-dependent index in WRITE (LHS) position. Much harder than the gather READ: single-assignment is keyed on the base data name (not the index), so a scatter has write-conflict / fan-in semantics (multiple iterations writing the same bin) that the gather read does not. Scope: (1) admit a data-dependent LHS index in lowering (lower_indices already lowers lhs.indices via lower_index_expr with allow_gather true after TASK-0341.03.01 — verify the LHS path); (2) codegen the scatter write histogram[(bin) as usize] = ...; (3) the accumulation semantics (read-modify-write to the same bin across iterations) must be sound single-worker; distributed scatter is a further step. Companion to TASK-0044.04.
<!-- SECTION:DESCRIPTION:END -->
