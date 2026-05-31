---
id: TASK-0385
title: >-
  DSL grammar extension: computed local bin (scalar-producing statement inside a
  loop) for the textbook scatter
status: To Do
assignee: []
created_date: '2026-05-31 05:03'
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
