---
id: TASK-0129
title: 'pthreads-sync: resolve const identifiers inside index expressions'
status: Done
assignee: []
created_date: '2026-05-18 03:09'
updated_date: '2026-05-22 21:06'
labels:
  - M1
  - backend
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
render_int_expr (called from render_flat_index) does not resolve Nuc const identifiers to their literal values — only render_const_expr (used for loop bounds) does. Result: writing 'a[w * PARTITION_SIZE + i]' in an algorithm leaks the bare 'PARTITION_SIZE' identifier into the generated Rust, breaking the build. Example 03-reduction (TASK-0022) works around this by giving 'a' a 2D shape 'i32[NUM_WORKERS][PARTITION_SIZE]' so the row-major stride is resolved from the data shape instead of from a const reference. Fix: make render_int_expr consult AlgoIR::consts (the same lookup render_const_expr does) and substitute the literal value. When fixed, the 03-reduction example can be simplified back to a 1D 'a : i32[N]' (or not — the 2D shape is also valid and documents the partition axis).
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 60c tracker hygiene (2026-05-22). FIXED in cycle 35 (TASK-0042.04 commit 894f63f). render_int_expr signature changed from (e: &IrExpr, subst: &BTreeMap<String,String>) to (e: &IrExpr, ctx: &RenderCtx<'_>); the new Ident arm consults ctx.sidecar.consts with precedence abs_subst > consts > bare-ident (matching render_const_expr). The fix migrated to backend_common::render in cycle 37 (TASK-0244 de-dup); all 3 backends inherit it via the shared render_flat_index_pub helper.

Regression pin: cycle-36 TASK-0245 added 3 sister tests across the 3 backends pinning that const-in-IndexExpr resolves to a literal value (not bare ident). The 2D-shape workaround in 03-reduction is no longer NECESSARY but is also valid + documents the partition axis — left in place. No source changes; closing as stale-tracker.
<!-- SECTION:FINAL_SUMMARY:END -->
