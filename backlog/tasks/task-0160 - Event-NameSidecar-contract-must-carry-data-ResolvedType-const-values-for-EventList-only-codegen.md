---
id: TASK-0160
title: >-
  Event/NameSidecar contract must carry data ResolvedType + const values for
  EventList-only codegen
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-18 16:43'
updated_date: '2026-05-18 23:04'
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
- [x] #1 NameSidecar (or Event) carries, per DataId, the ResolvedType (dims + ScalarType) sufficient to emit vec![zero; product(dims)] and the Rust element/slot type, deterministically
- [x] #2 Const name->value (and/or pre-resolved loop-bound) info reaches the contract so the backend renders bounds without AlgoIR consts
- [x] #3 Determinism + bit-identical e2e for 01/02/03/05/07 preserved
- [x] #4 Coupled with TASK-0159; together they unblock TASK-0124 AC#2 full switch
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

IMPLEMENTED (commit 4a79d6e). Separate NameSidecar carrier in compiler/src/sidecar.rs — NOT a field on ACFG (would churn block_transform/sync_inject/transfer_inject destructuring) and NOT a field on Event::Loop (kept lean per the prefer-sidecar steer; symbolic bound is per-program schedule metadata like name_data, not a per-event fact).

SIDECAR SHAPE: NameSidecar { data_types: BTreeMap<DataId,ResolvedType>, consts: BTreeMap<String,ConstValue{ty,value}>, loop_bounds: BTreeMap<IterVar,LoopBound{lo,hi:IrExpr}> }. build_sidecar(linked,acfg) pure/additive: inverts algo.data via acfg.name_data (canonical DataId the EventList uses), copies algo.consts, walks SOURCE IrStmt::For capturing UNEVALUATED lo/hi keyed by the same IterVar Event::Loop carries. alloc_len()/data_type() helpers.

KEY DECISION (no de-fold): eval_const fold + ACFGNode::Repeat.range + acfg_to_petri/Net/boundedness/deadlock ALL untouched. Symbolic bound captured IN PARALLEL at the build_acfg boundary, not by stopping the fold — the Net keeps concrete iteration counts, the sidecar carries source form. Decoupled by construction; this is exactly the high-regression refactor TASK-0142/0159 avoided, deliberately not done.

SERDE: ScalarType (ast.rs), ResolvedType + ResolvedConst (ir.rs) gained feature-gated serde derives (like event/contract types) so the sidecar is a committable/serialisable artifact. Trait impls only under serde feature; no behaviour change.

SUFFICIENCY PROOF (TASK-0156 style), tests/petri_to_events.rs: sidecar_alone_sizes_preinit_and_types_slots_for_all_e2e_examples (01/02/03/05/07: reconstructs exact vec![0;256] + i32 elem type from sidecar ALONE), sidecar_renders_stencil_symbolic_loop_bound_in_source_form (re-renders 05 `for y:1..H-1` as `(1_i64)..((16_i64 - 1_i64))` via loop_bounds+consts paired to Event::Loop iter_var — exactly pthreads-sync render_const_expr output; also inner W-1 loop), sidecar_const_table_matches_resolved_consts, sidecar_serde_roundtrip_is_byte_identical.

GATE (actual, this session): just test all groups 0 failed (1 pre-existing ignored); just e2e total 10 pass 8 fail 0 skip 2 required-fail 0 byte-identical (UNCHANGED — no sidecar codegen consumer yet, cannot regress by construction); determinism-check 8/0 byte-identical; determinism-check-negative correctly bites (7/1/2 on injected nondet); clippy --workspace -D warnings clean.

GOTCHAS: (1) flatten_all yields only non-Loop leaves, never Loop nodes — the inner-loop discovery in the bound test needed a dedicated recursive collect_loop_vars. (2) ScalarType/ResolvedType/ResolvedConst had NO serde derives (only IrExpr did); had to add them gated — additive, safe. (3) Same-named loop vars share ONE IterVar (PRD §6.2.3 one namespace) -> one loop_bounds entry; build_sidecar keeps first occurrence and PANICS loud if a later same-named loop has different bounds (a shared IterVar genuinely cannot represent both; no e2e example hits this — recorded as known limitation, not silently overwritten).

SCOPE BOUNDARY: TASK-0160 makes the contract SUFFICIENT and proves it. It does NOT switch pthreads-sync off the AlgoIR walk — that is TASK-0124 (the emit() signature change + AlgoIR removal).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added a NameSidecar codegen-contract carrier so an EventList-only backend (TASK-0124) can size pre-init allocations, type worker slots, and re-render rolled-loop bounds in SOURCE form WITHOUT the AlgoIR. 4 of 4 ACs met + verified. Commit 4a79d6e.

WHAT CHANGED
- nucleus/compiler/src/sidecar.rs (new): NameSidecar { data_types: BTreeMap<DataId,ResolvedType>, consts: BTreeMap<String,ConstValue>, loop_bounds: BTreeMap<IterVar,LoopBound{lo,hi:IrExpr}> } + build_sidecar(linked,acfg) (pure/additive) + alloc_len/data_type helpers.
- nucleus/compiler/src/lib.rs: pub mod sidecar; re-export NameSidecar/LoopBound/ConstValue/build_sidecar.
- nucleus/compiler/src/algo/ast.rs, ir.rs: feature-gated serde derives on ScalarType/ResolvedType/ResolvedConst (committable/serialisable contract; trait impls only under serde; no behaviour change).
- nucleus/compiler/tests/petri_to_events.rs: 4 sufficiency-proof + unit tests (TASK-0156 style).

WHY / DESIGN: separate sidecar (NOT a field on ACFG/Event::Loop) — it is whole-program schedule-pass metadata like the name_* tables, keeps ACFG/Event lean, avoids multi-site ACFG destructuring churn. eval_const fold / ACFGNode::Repeat.range / acfg_to_petri / Net / boundedness / deadlock ALL UNTOUCHED — the symbolic bound is captured in PARALLEL at the build_acfg boundary, not by de-folding (the high-regression refactor TASK-0142/0159 deliberately avoided). Analyses keep concrete iteration counts; the sidecar carries source form for codegen; decoupled by construction.

USER IMPACT: the codegen contract (EventList + name tables + NameSidecar) is now demonstrably SUFFICIENT for byte-identical EventList-only codegen. Together with TASK-0159 (Event::Loop structure) this unblocks TASK-0124 AC#2.

GATE (actual): just test all groups 0 failed; just e2e 10/8/0/2 byte-identical (UNCHANGED — no sidecar codegen consumer yet, cannot regress by construction); determinism-check 8/0 byte-identical; determinism-check-negative bites; clippy --workspace -D warnings clean.

SCOPE / HONEST LIMITATIONS
- TASK-0160 makes the contract sufficient and PROVES it (reconstruction tests). It does NOT switch pthreads-sync off the AlgoIR walk — that is TASK-0124 (the emit() signature change + AlgoIR removal).
- AC#4 is satisfied by the metadata being demonstrably sufficient (per task scope), NOT by TASK-0124 being done. The actual backend switch remains TASK-0124 work; both prerequisites (TASK-0159 structure + TASK-0160 types/consts/bound) now exist.
- Known limitation: two loops sharing a variable name share one IterVar (PRD §6.2.3) hence one loop_bounds entry; build_sidecar panics loud if such loops have DIFFERENT bounds (a shared IterVar cannot represent both — same constraint the EventList Event::Loop iter_var has). No e2e example exercises this.
<!-- SECTION:FINAL_SUMMARY:END -->
