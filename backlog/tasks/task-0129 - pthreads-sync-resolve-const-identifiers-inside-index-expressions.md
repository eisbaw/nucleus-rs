---
id: TASK-0129
title: 'pthreads-sync: resolve const identifiers inside index expressions'
status: To Do
assignee: []
created_date: '2026-05-18 03:09'
labels:
  - M1
  - backend
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
render_int_expr (called from render_flat_index) does not resolve Nuc const identifiers to their literal values — only render_const_expr (used for loop bounds) does. Result: writing 'a[w * PARTITION_SIZE + i]' in an algorithm leaks the bare 'PARTITION_SIZE' identifier into the generated Rust, breaking the build. Example 03-reduction (TASK-0022) works around this by giving 'a' a 2D shape 'i32[NUM_WORKERS][PARTITION_SIZE]' so the row-major stride is resolved from the data shape instead of from a const reference. Fix: make render_int_expr consult AlgoIR::consts (the same lookup render_const_expr does) and substitute the literal value. When fixed, the 03-reduction example can be simplified back to a 1D 'a : i32[N]' (or not — the 2D shape is also valid and documents the partition axis).
<!-- SECTION:DESCRIPTION:END -->
