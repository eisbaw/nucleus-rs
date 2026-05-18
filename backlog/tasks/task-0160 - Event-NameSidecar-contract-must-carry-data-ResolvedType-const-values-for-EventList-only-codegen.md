---
id: TASK-0160
title: >-
  Event/NameSidecar contract must carry data ResolvedType + const values for
  EventList-only codegen
status: To Do
assignee: []
created_date: '2026-05-18 16:43'
updated_date: '2026-05-18 22:47'
labels:
  - M2
  - compiler
  - backend
  - blocker
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Blocks TASK-0124 AC#2 byte-identical. The pthreads-sync backend emits pre-init allocations (let mut c = vec![0; 256]; let mut img_out = vec![0; 256]) and multi-worker Slot types (Arc<Slot<Vec<i32>>>) by reading AlgoIR/LinkedIR ResolvedType (dims product for the vec! length, ScalarType for the element zero literal and arg casts) and AlgoIR consts (const N=256). The per-worker EventList + the proposed NameSidecar (name_kernels/name_data/name_workers) carry NONE of this: DataSlice has only DataId+index IrExprs, no element type, no shape. A backend consuming ONLY the EventList+sidecar cannot size vec![0; N] nor type the slot nor cast scalar args -> cannot produce byte-identical (or even compilable) code without AlgoIR. Decide: extend NameSidecar with a per-DataId {ResolvedType} table and a const name->value table (committable, deterministic, BTreeMap), OR add the element type/shape onto DataSlice. Prefer the sidecar (keeps Event lean; types are schedule-pass-available metadata, like the name tables). Couple with TASK-0159 (loop structure) since TASK-0124's full AC#2 switch needs BOTH.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 NameSidecar (or Event) carries, per DataId, the ResolvedType (dims + ScalarType) sufficient to emit vec![zero; product(dims)] and the Rust element/slot type, deterministically
- [ ] #2 Const name->value (and/or pre-resolved loop-bound) info reaches the contract so the backend renders bounds without AlgoIR consts
- [ ] #3 Determinism + bit-identical e2e for 01/02/03/05/07 preserved
- [ ] #4 Coupled with TASK-0159; together they unblock TASK-0124 AC#2 full switch
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0159 (commit ee309ff): TASK-0159 carries loop STRUCTURE (Event::Loop) but its range is a CONCRETE Range<i64> — it explicitly does NOT solve the symbolic-bound half. Root cause pinpointed: build_acfg, IrStmt::For arm at acfg.rs ~688-707, calls eval_const(lo)/eval_const(hi) and PANICS if a bound is not a const i64, storing the folded i64 into ACFGNode::Repeat.range. So the unevaluated H-1 expression is destroyed at ACFG construction, well before petri_to_events. TASK-0159 AC#2 was left UNCHECKED and a dep task-0159->task-0160 added. For TASK-0160 to unblock TASK-0124 AC#2 byte-identical, it must make the loop-bound EXPRESSION (or a const name->value table sufficient to re-render `(16_i64 - 1_i64)`) reach the contract — i.e. stop eval_const folding at lowering OR sidecar the const table + pre-fold mapping. TASK-0160 AC#2 already names this. The natural seam: either add an optional symbolic bound onto Event::Loop, or a NameSidecar const table the backend uses to re-expand the concrete range back to source form. Coordinate so TASK-0124 gets ONE coherent rolled-loop+bound story.
<!-- SECTION:NOTES:END -->
