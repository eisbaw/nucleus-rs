---
id: TASK-0385
title: >-
  DSL grammar extension: computed local bin (scalar-producing statement inside a
  loop) for the textbook scatter
status: To Do
assignee: []
created_date: '2026-05-31 05:03'
updated_date: '2026-06-02 23:12'
labels:
  - compiler
  - grammar
  - scatter
  - deferred
dependencies:
  - TASK-0376
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
DEEPER follow-up to TASK-0376. TASK-0376 landed the bounded native scatter `histogram[input[i]] <-- inc(histogram[input[i]])` where `input` is a TOP-LEVEL data symbol already pre-clipped to [0, BINS). That works because the bin index is a direct data read.

The TEXTBOOK histogram over UNCONSTRAINED input needs a value->bin BUCKETING step then a scatter: `bin = bucket(input[i]); histogram[bin] <-- inc(histogram[bin])`. The v2 algorithm DSL has NO syntax for a computed local / scalar-producing statement inside a loop body (PRD §6.2.4: no conditionals, no local bindings) — the only loop-body statement is a single `D[idx] <-- kernel(...)` dataflow. So the bucketing today MUST live inside a kernel (the masked-accumulator `bin_inc` shape, or a future `histogram[bucket_kernel(input[i])]`-style index — but a KERNEL CALL in index position is also rejected: lower_index_expr Expr::Call arm + render_int_expr IrExpr::Call arm both fail-loud "kernel call inside an integer index expression").

This is the SAME grammar bottleneck as the [[project-grammar-deferred-cluster]] (TASK-0179 1D prefix scan / TASK-0044.05.01 2D wavefront / TASK-0044.06.01 bitonic stage-parallel). Treat as part of that grammar-extension epic: a DSL that admits scalar-producing statements / local bindings inside loop bodies (and possibly a kernel call in index position) would unlock the textbook scatter, the computed-bin histogram, and the deferred scan/wavefront/bitonic forms in one move. Decision needed: local-binding syntax + lowering (likely a new IrStmt::Let or an inline-expand of pure-kernel index calls) + single-assignment/scope rules for the local.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
CORRECTION (cycle-246 design-slice, orchestrator-verified against code): this task body claims a DSL admitting scalar-producing local bindings would unlock the textbook scatter AND the scan/wavefront/bitonic forms IN ONE MOVE. That claim is REFUTED. The textbook-scatter need (bucket(input[i]) -> histogram[bin]) is unlocked by X1 = admit a PURE kernel call in subscript index position (filed as TASK-0430), which reuses the existing gather machinery and needs NO local-binding construct. Wavefront (TASK-0044.05.01) and bitonic (TASK-0044.06.01) are blocked by ORTHOGONAL deeper constraints — variable (iter-var-dependent) loop bounds [(c), acfg eval_const NonConstLoopBound] + single-assignment-per-symbol relaxation [(b)] + (for wavefront) conditionals [(a)] — NONE of which a local binding addresses. So local-bindings/index-calls are IRRELEVANT to 10/12. This task is effectively SUPERSEDED for the scatter use-case by TASK-0430 (the inline-pure-call route is preferred over a new IrStmt::Let on blast-radius grounds; a Let, if ever wanted, can be sugar that desugars to TASK-0430). Recommend: treat TASK-0430 as the live work; keep this task only if a first-class local-binding LANGUAGE FEATURE is wanted for its own sake (a human/PRD call, not forced by any example).
<!-- SECTION:NOTES:END -->
