---
id: TASK-0341
title: >-
  Showcase example expansion: 3 new examples for compiler-pattern coverage
  (transpose, Jacobi, SpMV)
status: To Do
assignee: []
created_date: '2026-05-26 11:49'
updated_date: '2026-05-27 22:15'
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
- [x] #3 Cycle-178 doc-lie-promotion mitigation applies: any //! module-level docstring in the new example's prog.algo.nuc / kernels.rs / schedule must be present-tense + cite the landing cycle.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
=== Cycle 218 closure addendum (AC#3 cross-example doc-lie-promotion mitigation) ===

AC#3 closed across cycles 217b + 218.

Cycle-178 doc-lie-promotion mitigation surface complete across all 3 showcase examples:
- 15-transpose: cycles 217b + 218 — prog.algo.nuc + README.md + schedules/distributed-rows.sched.nuc all present-tense + cite landing cycles.
- 16-jacobi: cycle 218 — prog.algo.nuc + README.md reorganized; schedules unchanged (no stale narrative found).
- 17-spmv: cycle 218 — prog.algo.nuc + README.md reorganized; schedules unchanged.

The cycle-217b whole-file grep + broader regex discipline was applied to all 3 examples end-to-end. The post-edit grep across all 3 example directories returned only legitimately not-stale hits (live forward-carry references to TASK-0341.02.01 / TASK-0341.03.01 honest-BLOCKED follow-ups; diagnostic literal text quotes).

NOT in-place rewriting AC#3 text per memory feedback-ac-rewrite-on-done-task; this addendum records closure.

=== Cycle 218b architect P1 fold-back: AC#3 UN-TICKED ===

Architect cycle-218 review-gate caught P1: the cycle-218 16-jacobi narrative shipped a doc-lie ('5 of 7 [[required]]' for distributed; actual state is 0 of 7). AC#3 ('cycle-178 doc-lie-promotion mitigation; present-tense, no predictive claims') is therefore NOT closed for 16-jacobi.

Un-ticking AC#3 + addendum block below.

Cycle 218b corrected the 16-jacobi narrative to honest-BLOCKED state. Re-ticking AC#3 in the cycle-218b commit once all 3 examples' narratives are verified accurate against e2e-matrix.toml ground truth.

=== Cycle 218b re-tick: AC#3 corrected after architect P1 fold-back ===

Cycle 218b corrected the 16-jacobi narrative + the 17-spmv README 'Required schedules' section to the actual e2e-matrix.toml state.

Ground-truth verification (cycle 218b):
- 15-transpose: 14 [[required]] (naive×7 + distributed-rows×7), 0 [[skip]]. Narrative matches.
- 16-jacobi: 7 [[required]] (naive×7), 7 [[skip]] (distributed×7). Narrative now matches (cycle-218 doc-lie corrected).
- 17-spmv: 14 [[required]] (naive×7 + distributed×7), 0 [[skip]]. Narrative now matches (added distributed to README Required schedules per architect P3).

All 3 examples //! docstrings + READMEs are present-tense + cite landing cycles + match the e2e-matrix.toml ground truth.

AC#3 re-tickable. Cycle 218b applied the cycle-218 sharpened discipline (whole-file grep + per-claim ground-truth check against e2e-matrix.toml).
<!-- SECTION:NOTES:END -->
