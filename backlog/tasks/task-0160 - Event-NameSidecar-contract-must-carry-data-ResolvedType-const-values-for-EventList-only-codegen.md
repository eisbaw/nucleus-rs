---
id: TASK-0160
title: >-
  Event/NameSidecar contract must carry data ResolvedType + const values for
  EventList-only codegen
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-18 16:43'
updated_date: '2026-05-18 22:57'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. New module compiler/src/sidecar.rs: NameSidecar codegen-contract carrier (BTreeMap-backed, deterministic, serde-gated like contract/Event types). Carries: (a) data_types: BTreeMap<DataId, ResolvedType> (dims+ScalarType -> vec! length, element/slot type, scalar casts); (b) consts: BTreeMap<String, i64> (+ ScalarType) (const name->value, re-render bounds w/o AlgoIR); (c) loop_bounds: BTreeMap<IterVar, LoopBound{lo:IrExpr,hi:IrExpr}> -- the UNEVALUATED For lo/hi AST captured additively at build_acfg, keyed by the SAME IterVar that Event::Loop carries.
2. build_sidecar(linked: &LinkedIR, acfg: &ACFG) -> NameSidecar: invert algo.data via acfg.name_data -> DataId->ResolvedType; copy algo.consts; walk linked.algo.stmts For nodes, map var via acfg.name_iter_vars -> IterVar, store (lo,hi) IrExpr clones. Pure, additive, reads LinkedIR+ACFG name maps only. eval_const fold UNTOUCHED; ACFGNode::Repeat.range stays concrete; acfg_to_petri/Net/boundedness/deadlock UNTOUCHED.
3. Export NameSidecar/LoopBound/build_sidecar from lib.rs. Helper to render a ScalarType -> Rust elem type + zero-literal, and a render of a LoopBound expr into Rust source form using consts.
4. SUFFICIENCY PROOF test (TASK-0156 style) in tests/petri_to_events.rs: for 01/02/03/05/07, from NameSidecar + EventList ALONE (no AlgoIR walk) reconstruct exact vec! length (product dims), Rust element/slot type, and for 05 the loop bound rendered in SOURCE form (16_i64 - 1_i64) via loop_bounds+consts paired with the Event::Loop iter_var. Plus sidecar unit tests (determinism, serde roundtrip, key pairing with Event::Loop).
5. Docs: module doc explaining sidecar vs Event vs Net separation; explicit scope boundary TASK-0160 ends / TASK-0124 begins (no backend switch here).
6. Gate before every commit: nix develop -c just test / e2e / determinism-check / determinism-check-negative / clippy -D warnings. e2e+determinism MUST be byte-identical (no codegen consumer). Commit per logical unit, no push, no AI credit.
7. Forward-carry coherent rolled-loop+bound story to TASK-0124 notes; note TASK-0159 AC#2 satisfiability (do not check it myself).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0159 (commit ee309ff): TASK-0159 carries loop STRUCTURE (Event::Loop) but its range is a CONCRETE Range<i64> — it explicitly does NOT solve the symbolic-bound half. Root cause pinpointed: build_acfg, IrStmt::For arm at acfg.rs ~688-707, calls eval_const(lo)/eval_const(hi) and PANICS if a bound is not a const i64, storing the folded i64 into ACFGNode::Repeat.range. So the unevaluated H-1 expression is destroyed at ACFG construction, well before petri_to_events. TASK-0159 AC#2 was left UNCHECKED and a dep task-0159->task-0160 added. For TASK-0160 to unblock TASK-0124 AC#2 byte-identical, it must make the loop-bound EXPRESSION (or a const name->value table sufficient to re-render `(16_i64 - 1_i64)`) reach the contract — i.e. stop eval_const folding at lowering OR sidecar the const table + pre-fold mapping. TASK-0160 AC#2 already names this. The natural seam: either add an optional symbolic bound onto Event::Loop, or a NameSidecar const table the backend uses to re-expand the concrete range back to source form. Coordinate so TASK-0124 gets ONE coherent rolled-loop+bound story.
<!-- SECTION:NOTES:END -->
