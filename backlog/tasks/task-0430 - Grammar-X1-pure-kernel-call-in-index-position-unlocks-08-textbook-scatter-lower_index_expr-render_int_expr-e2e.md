---
id: TASK-0430
title: >-
  Grammar X1: pure-kernel-call in index position unlocks 08 textbook scatter
  (lower_index_expr + render_int_expr + e2e)
status: To Do
assignee: []
created_date: '2026-06-02 23:12'
labels:
  - compiler
  - grammar
  - scatter
  - grammar-extension-epic
dependencies:
  - TASK-0385
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-246 design-slice outcome (read-only Plan-agent investigation, orchestrator-verified against code). The cleanest resolution of TASK-0385 textbook-scatter need is NOT a new IrStmt::Let local binding but X1: admit a PURE kernel call in SUBSCRIPT index position (histogram[bucket(input[i])] <-- inc(...)), reusing the existing gather machinery (TASK-0341.03.01). Far smaller blast radius than a Let (no new AST/IR node; one lowering arm + one shared render arm; single-assignment/scope untouched). A Let, if ever wanted, can be later defined as sugar that desugars to this. VERIFIED code sites: lower_index_expr Expr::Call rejection at nucleus/nucleus-compiler/src/algo/lower.rs:1179 (NonIntegerShapeExpr kernels-not-allowed-here); render_int_expr IrExpr::Call rejection at nucleus/backend-common/src/render/expr.rs:72 (EmitError::UnsupportedFeature). The adjacent gather paths (lower.rs:1191-1202 allow_gather DataRef + render_gather_index_load render/expr.rs:71) are the machinery to mirror. Risk: LOW (additive, gated on pure-kernel callee, subscript-only; loop-bound position keeps rejecting -> const-bound rule (c) intact).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 lower_index_expr admits Expr::Call iff allow_gather AND callee is a declared PURE kernel; lowers to IrExpr::Call; effectful kernel rejected; loop-bound position (allow_gather=false) still rejects. Unit tests: positive + 2 negatives.
- [ ] #2 render_int_expr IrExpr::Call arm emits callee(<rendered args>) as the integer index (recurse render_int_expr for scalar args, render_gather_index_load for data-ref args); silent-sibling sweep confirms no per-backend independent IrExpr::Call index rejection beyond the shared backend-common arm.
- [ ] #3 08-histogram textbook variant (prog.textbook.algo.nuc with a pure bucket kernel over UNCONSTRAINED input + a single-worker schedule + reference oracle) emits bit-identical across the 7 tier-1 backends; new e2e cell added; e2e total baseline bumped + recorded in commit msg (cumulative/accumulate classification re-verified for the bucket(input[i]) self-read form).
<!-- AC:END -->
