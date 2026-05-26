---
id: TASK-0341
title: >-
  Showcase example expansion: 3 new examples for compiler-pattern coverage
  (transpose, Jacobi, SpMV)
status: To Do
assignee: []
created_date: '2026-05-26 11:49'
labels:
  - examples
  - coverage
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

The current example matrix (01-13) covers elementwise, reduction, 1D/2D stencil with halo, matmul, pipelined producer/consumer, and CNN inference. M6 adds three more (08 histogram, 10 wavefront, 12 bitonic sort — TASK-0044.04/.05/.06).

This task tracks 3 ADDITIONAL examples chosen to (a) showcase what's currently possible, (b) exercise compiler paths existing examples don't, (c) increase end-to-end coverage across the now-7 tier-1 backends.

## Selection rationale

The 3 sub-tasks are picked to span the risk spectrum, one per category:

- **TASK-0341.01 matrix transpose** — likely-cleanly-expressible. Pure showcase, exercises 2D-tile permutation primitive. Low compiler risk. Useful as a baseline new-example to validate the per-example landing workflow under post-M6-skeleton state.
- **TASK-0341.02 Jacobi iteration** — likely surfaces 'loop with data-dependent termination' question. 5-point stencil but iterative-until-tolerance instead of fixed-N. HIGH leverage as a gap-finder for the Event::Loop shape.
- **TASK-0341.03 SpMV on small dense-stored sparse matrix** — data-dependent indexing (CSR-style row/col pointers). Companion to TASK-0044.04 (histogram) which has the same compiler-feature shape. Surfaces the data-dependent-write-address question if it isn't already covered by histogram.

## Discipline per sub-task

Each sub-task follows the cycle-161 honest-scoping pattern:

- AC#1 starts with a LANGUAGE-SANITY slice — minimal prog.algo.nuc + simplest naive.sched.nuc + pthreads-sync cell only. Verifies expressibility before authoring kernels/reference/binary data.
- If a sub-task hits a DSL expressiveness boundary, the HONEST outcome is the naive schedule landing + a grammar-extension follow-up task filed — NOT a milestone gate to block on.
- Parallel schedules (partition=rows, distributed, etc.) and multi-backend e2e cells defer to follow-up cycles per sub-task.

## Out of scope

- Iterative-convergence with floating-point: any example whose reference output is non-deterministic under different reduction orders. The cross-backend differential matrix requires bit-identical output across backends, so integer or fixed-rounding arithmetic only.
- Examples needing parallel-construct DSL features that don't yet exist. Each sub-task's AC#1 (language-sanity) is the empirical check; honest BLOCKED is acceptable.

## Forward-carries

This task does NOT yet decide whether the 3 example numbers should be 14/15/16 (sequential), or use a different scheme. Each sub-task should propose its number based on which compiler-pattern category it sits closest to in the existing examples directory.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Sub-tasks TASK-0341.01 (transpose), TASK-0341.02 (Jacobi), TASK-0341.03 (SpMV) all filed with concrete prog.algo.nuc + naive.sched.nuc + AC#1 language-sanity criteria.
- [ ] #2 Each sub-task explicitly declares its example number (e.g. 14-transpose, 15-jacobi, 16-spmv) AND a tier-1 e2e cell that must pass for AC#1 closure.
- [ ] #3 Cycle-178 doc-lie-promotion mitigation applies: any //! module-level docstring in the new example's prog.algo.nuc / kernels.rs / schedule must be present-tense + cite the landing cycle.
<!-- AC:END -->
