---
id: TASK-0380
title: >-
  PRD data-dependent-indexing out-of-class phrasing imprecise post single-worker
  gather (TASK-0375 P3.2)
status: Done
assignee:
  - Mark Ruvald Pedersen
created_date: '2026-05-31 01:30'
updated_date: '2026-05-31 01:45'
labels:
  - docs
  - doc-lie
  - prd
dependencies:
  - TASK-0341.03.01
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
DOC-HONESTY follow-up to TASK-0375 (architect P3.2, gather review gate). PRD.md:118 and PRD.md:1300 flatly list data-dependent indexing as out-of-class. After TASK-0341.03.01 a single-worker gather (17-spmv/gather, x[col[k]]) DOES compile and is 7-backend bit-identical, so the unqualified phrasing overshoots. The lines are framed around static decomposition / portability, and a DISTRIBUTED gather genuinely remains unsupported (halo_inference DataDependentStride fatal-under-partition; TASK-0373), so the statements are accurate about DISTRIBUTION but imprecise at the single-worker level. Tighten the PRD phrasing to data-dependent indexing does not DISTRIBUTE (single-worker gather supported; distributed broadcast is TASK-0373) so the PRD does not read as a doc-lie against the shipped 17-spmv/gather example. Recurring-defect-pattern #1.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation plan (cycle-219 doc-honesty): surgical PRD.md edits at two sites. (a) Section 3 Non-goals lines 117-121; (b) risks section lines 1299-1304. Add PRECISION without introducing a new lie. Verified sub-claims (traced before writing): 17-spmv/gather example EXISTS (prog.gather.algo.nuc); single-worker gather READ x[col[k]] compiles 7-backend bit-identical (TASK-0341.03.01); DISTRIBUTED gather UNSUPPORTED (halo_inference DataDependentStride fatal-under-partition, TASK-0373 To Do); scatter / data-dependent WRITE UNSUPPORTED (TASK-0376 To Do); convergence-check / data-dependent loop termination is a grammar epic UNSUPPORTED (TASK-0341.02.01 To Do); recursion still out. So the sparse-matrix-solver caveat (PRD:1303) stays TRUE — do NOT weaken it. Plan: qualify the blanket "data-dependent indexing ... out" to acknowledge a single-worker gather READ is expressible (cite 17-spmv/gather / TASK-0341.03.01) while keeping out: distribution of a gather, scatter/data-dependent writes, data-dependent control flow (loop termination), recursion, and therefore full sparse solvers. Keep affine-static-class spirit. Docs-only; e2e 329/272/0/57/0 invariant.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE cycle-219. Surgical PRD.md edits at the two flagged sites add PRECISION without introducing a new lie:
(a) Section 3 Non-goals (PRD.md:117-128): dropped the blanket "Data-dependent indexing ... out" sentence; replaced with the qualified statement that a single-worker gather READ x[col[k]] is now expressible and 7-backend bit-identical in single-worker form (cites 17-spmv/gather + TASK-0341.03.01), but does NOT distribute (TASK-0373), and that what stays out is distribution-of-gather, data-dependent WRITES (scatter), data-dependent control flow (loop termination), recursion, and therefore full sparse solvers.
(b) Risks section (PRD.md:1306-1317): "no data-dependent indexing" -> "largely no data-dependent indexing" + the single-worker gather-read carve-out; the sparse-matrix-solver caveat is KEPT (still true) and STRENGTHENED with the reason (the gather read is one ingredient; the solver also needs scatter + convergence-driven termination, both out).
SUB-CLAIM VERIFICATION (traced before writing, all confirmed): 17-spmv/gather example exists (prog.gather.algo.nuc + schedules/gather.sched.nuc); single-worker gather READ compiles 7-backend bit-identical (e2e run this cycle: all 7 "17-spmv gather" cells PASS); DISTRIBUTED gather UNSUPPORTED (TASK-0373 To Do, halo_inference DataDependentStride fatal-under-partition); scatter/data-dependent WRITE UNSUPPORTED (TASK-0376 To Do); convergence-check / data-dependent loop termination UNSUPPORTED grammar epic (TASK-0341.02.01 To Do); recursion out. No PRD sub-claim was found false. The §3 vs risks framing is kept consistent.
GATE: docs-only, no code change; full cheap gate green with the PRD edit in tree — build OK, clippy clean, test 1165/3, test-release 1164/3, e2e 329/272/0/57/0 (HARD invariant held).
<!-- SECTION:FINAL_SUMMARY:END -->
