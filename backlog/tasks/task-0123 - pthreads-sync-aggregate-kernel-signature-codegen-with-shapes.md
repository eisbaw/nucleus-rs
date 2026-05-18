---
id: TASK-0123
title: 'pthreads-sync: aggregate kernel signature codegen with shapes'
status: To Do
assignee: []
created_date: '2026-05-18 02:13'
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
