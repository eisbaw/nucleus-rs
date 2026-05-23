---
id: TASK-0123
title: 'pthreads-sync: aggregate kernel signature codegen with shapes'
status: Done
assignee: []
created_date: '2026-05-18 02:13'
updated_date: '2026-05-23 21:31'
labels:
  - M1
  - backend
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
At TASK-0020 the codegen treats algorithm aggregate data symbols (T[N]) as flat Vec<T> on the Rust side, which works because the example 01 kernels.rs spells aggregates as Vec<T>. PRD §6.2.2's example uses Box<[[T; W]; H]> with Nuc-level const sizes - TASK-0103 covers the surface design. Once that lands, the pthreads-sync codegen must:
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Emit array-typed bindings (e.g. let mut a: [i32; N] = ...) matching the kernels.rs convention TASK-0103 picks.
- [ ] #2 Pass aggregates to effect kernels by reference or by move based on declared signature.
- [ ] #3 Support multi-dimensional shapes without flattening (currently render_flat_index does row-major flattening for >=2D).
- [ ] #4 Depends on TASK-0103 (PRD const-shape resolution).
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-no-driver (orchestrator-direct, cycle 77 sweep). The task requires the codegen to emit array-typed bindings ('let mut a: [i32; N]') instead of flat 'Vec<T>'. TASK-0103 (Done cycle ~64) chose the kernels.rs convention to be Vec<T>-shaped (matching example 01 + others); every in-tree kernels.rs uses Vec<T>. There is NO kernel today that needs the multi-dim array shape ('Box<[[T;W];H]>') the task envisions — TASK-0103 explicitly settled this as the v2 convention. The task description's 'Once that lands' clause has actually been answered by TASK-0103 choosing the OPPOSITE direction (stay flat). Reopen if/when a future kernels.rs convention shifts (highly unlikely for v2) OR a tier-3 backend needs typed-array binding emission for embedded constraints (TASK-0050 territory). Same deferred-no-driver pattern as TASK-0127/0130.
<!-- SECTION:FINAL_SUMMARY:END -->
