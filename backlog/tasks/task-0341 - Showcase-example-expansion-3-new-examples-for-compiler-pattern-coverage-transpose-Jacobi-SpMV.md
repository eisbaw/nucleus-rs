---
id: TASK-0341
title: >-
  Showcase example expansion: 3 new examples for compiler-pattern coverage
  (transpose, Jacobi, SpMV)
status: Done
assignee: []
created_date: '2026-05-26 11:49'
updated_date: '2026-05-27 22:55'
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
- [x] #1 Sub-tasks TASK-0341.01 (transpose), TASK-0341.02 (Jacobi), TASK-0341.03 (SpMV) all filed with concrete prog.algo.nuc + naive.sched.nuc + AC#1 language-sanity criteria.
- [x] #2 Each sub-task explicitly declares its example number (e.g. 14-transpose, 15-jacobi, 16-spmv) AND a tier-1 e2e cell that must pass for AC#1 closure.
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

=== Cycle 219 closure (orchestrator-direct; pre-commit fold-back of architect P1.1+P1.2+P2.1+P2.2+P3.1+P3.2 + qa-test-runner P2.1) — showcase example expansion epic closed ===

All 3 sub-tasks Done:
- TASK-0341.01 (15-transpose): Done cycle 219 (AC closure across cycles 204/205/215/216/218b).
- TASK-0341.02 (16-jacobi): Done cycle 219 (AC closure across cycles 206/207/208/218b; AC#2 honest-BLOCKED via TASK-0341.02.01).
- TASK-0341.03 (17-spmv): Done cycle 213 (AC closure across cycles 210/211/212/213; AC#2 honest-BLOCKED via TASK-0341.03.01; cycle 214 was the child TASK-0341.03.02.01 cross-backend × 6 distributed promotion, not the parent closure cycle).

AC tick map (parent):
- AC#1: ticked cycle 219 — sub-tasks filed with concrete prog.algo.nuc + naive.sched.nuc + AC#1 language-sanity criteria. All 3 examples (15-transpose, 16-jacobi, 17-spmv) have committed prog.algo.nuc + naive.sched.nuc + reference.bin + input.bin + reference/ + README.md.
- AC#2: ticked cycle 219 — sub-tasks declared 15/16/17 (not 14/15/16; 14 taken by hearing-aid TASK-0054 cycle 201) and have tier-1 e2e cells: 15-transpose | naive | pthreads-sync (cycle 204), 16-jacobi | naive | pthreads-sync (cycle 206), 17-spmv | naive | pthreads-sync (cycle 210).
- AC#3: ticked cycle 218b — cycle-178 doc-lie-promotion mitigation present-tense + cycle-citation across all 3 examples' module-level docstrings + READMEs + schedule files (cycles 217/217b/218/218b stale-narrative sweeps).

e2e baseline at closure: 280/246/0/34/0 (cycle 219 re-ran `just e2e` once at cycle-start + QA review-gate re-ran 2 more samples, all bit-identical; non-flake confirmed across 3 total samples; tracker-only no-Rust-changes cycle). [[required]] / [[skip]] counts (ground-truth from nuc-nucleus/e2e-matrix.toml, cycle-218b verification + cycle-219 QA re-verification):
- 15-transpose: 14 [[required]] (naive×7 + distributed-rows×7), 0 [[skip]].
- 16-jacobi: 7 [[required]] (naive×7), 7 [[skip]] (distributed×7) — honest-BLOCKED across all 7 tier-1 backends: 5 (pthreads-sync, pthreads-async, mp-tcp-event, openmp-rs, mp-uds-event) on TASK-0341.02.02.01.01 3D wait_slice gap; 2 (mp-tcp-bufsync, mp-tcp-poll) on TASK-0330 in-Loop w2w Push.
- 17-spmv: 14 [[required]] (naive×7 + distributed×7), 0 [[skip]].
- Total: 35 [[required]] + 7 [[skip]] new cells across the 3 examples.
- Unit tests: 1019 passed / 0 failed / 3 ignored (dev); 1018 passed / 0 failed / 3 ignored (release) — the 1-test delta is the known debug_assert!-gated #[should_panic] divergence per TASK-0291.

Status: Done.

Compiler-feature gaps surfaced by the showcase wave (filed follow-ups; deferred):
- TASK-0341.01.01 surfaced: nothing new (axis-swap output-driven partition lowers via cycle-118/121 axis-mapping filter).
- TASK-0341.02.01 (data-dependent loop termination grammar extension): convergence-check Jacobi requires while/break/runtime-dependent ForStmt bounds; same epic as TASK-0179/0044.05.01/0044.06.01 (project-grammar-deferred-cluster).
- TASK-0341.02.02.01.01 (extend wait_slice to N-D nested-loop dispatch): blocks 5 of 7 16-jacobi/distributed cells; structurally relevant to other multi-dim distributed shapes. Filed cycle 209.
- TASK-0341.03.01 (data-dependent indirect read grammar gap): SpMV's x[col_idx[i][k]] inexpressible — IndexExpr.Atom does not admit nested IndexSuffix; companion to TASK-0044.04 histogram. Stub description (filed cycle 210); backfill follow-up filed cycle 219 as TASK-0352.
- TASK-0347 (ACFG identity-copy dataflow; cycle-77 DEFERRED trigger fired by 15-transpose): reopens both ACFG-build and link-side lower for bare-LValue dataflow; would simplify 15-transpose by dropping the xpose identity passthrough. Note: TASK-0347 has prose-only ACs (no formal tickable AC list); cycle-219 formalization filed as TASK-0353.
- TASK-0348 (zero-init invariant behaviour-pin for 16-jacobi field/boundary cells): defensive, low priority.
- TASK-0349 (codegen unused_assignments warning on whole-array broadcast init): cosmetic, low priority.

Forward-carry lessons (for next phase3 cycle implementer briefs):
- Showcase example AC fold-out pattern (cycle 204b precedent): AC#1 lands in a focused cycle, AC#2/#3/#4 as separate follow-up sub-tasks. Worked across all 3 examples spanning cycles 204..219 (~16 major cycles incl. b-suffix fold-backs).
- Stale-narrative discipline: the cycles 215..218b stale-narrative sweep cluster (TASK-0350, 0351) revealed a recurring "cross-backend promotion AC follow-up filed... this cycle lands X only" pattern that becomes stale post-promotion. The whole-file grep discipline (cycle 217b) + ground-truth check against e2e-matrix.toml (cycle 218b) are the operational mitigations.
- Doc-lie verbatim-copy pattern fired AGAIN in cycle 219 closure draft itself, TWICE (architect P1.1 + qa-test-runner P2.1, both caught pre-commit): (a) the cycle-218 doc-lie ('5 of 7 [[required]], 2 honest-BLOCKED via TASK-0330') was re-introduced into TASK-0341.02 AC#3 closure tick line ONE block below the cycle-218b correction that fixed exactly this defect — 18th firing of feedback-silent-sibling-defect; (b) the parent narrative's "TASK-0341.03 Done cycle 214" was a sibling-copy from the cross-backend promotion cycle 214 when the actual parent closure was cycle 213 — 19th firing. Both caught pre-commit by the parallel read-only review-gate (architect + qa-test-runner) and folded in-thread before commit; no cycle-219b fold-back commit required. Ground-truth check against e2e-matrix.toml + `git log --oneline --grep=TASK-X` is the discipline that should be applied to EVERY narrative making cycle-citation or [[required]] / [[skip]] count claims, including parent-AC-closure summaries. TASK-0339 (just check-narrative-doc-lie structural recipe) was filed cycle 169 and is currently a static-text-only check; the cycle-219 firings suggest promoting it to also cross-check toml-cited counts AND git-log-cited cycles is worth a follow-up if the pattern fires again.
<!-- SECTION:NOTES:END -->
