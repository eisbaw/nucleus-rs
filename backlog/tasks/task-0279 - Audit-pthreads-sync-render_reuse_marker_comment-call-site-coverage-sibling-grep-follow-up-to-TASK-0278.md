---
id: TASK-0279
title: >-
  Audit pthreads-sync render_reuse_marker_comment call-site coverage
  (sibling-grep follow-up to TASK-0278)
status: To Do
assignee: []
created_date: '2026-05-24 12:28'
labels:
  - M5
  - test-gap
  - reuse
  - sibling-grep-audit
  - forward-carried-from-TASK-0278
dependencies:
  - TASK-0278
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

Forward-carried from TASK-0278 cycle-99 architect review. The cycle-93/95/97/98 silent-sibling defect family teaches: always grep ALL peer call sites and pin each. TASK-0273 + TASK-0278 closed both walker arms in `backend-common/multi_worker_walker.rs` (lines 404 strip-mine + 478 non-strip-mine), but `nucleus/backends/pthreads-sync/src/lib.rs` has TWO call sites to `render_reuse_marker_comment` (lines 653 + 675) that were NOT re-audited this cycle.

The existing single-worker grep test `nucleus/nucleus-compiler/tests/e2e_example_05.rs::reuse_marker_present_on_reuse_schedule_absent_on_naive` asserts `>=1` marker occurrence. That is satisfied by EITHER call site emitting — a regression dropping the marker from ONLY one of the two call sites would silently pass.

## Acceptance

1. Investigate: which call site (line 653 vs line 675) emits the marker for the shipped `05-stencil/reuse.sched.nuc`? `NUC_TRACE=1` or `println!` instrumentation may help — or static reading suffices.
2. If both arms can fire on different schedule shapes (e.g. one for strip-mine, one for non-strip-mine, analogous to multi_worker_walker), add a second e2e test or synthetic fixture pinning the under-covered arm. If one is structurally dead, document it + consider removing.
3. Run `just e2e` + `just test` post-fix — no regressions.

## Honest scope

This is a 30-minute investigation followed by either a small test addition or a documentation update. Most likely the two call sites differentiate between the strip-mine and non-strip-mine arms (mirroring multi_worker_walker.rs's structure) — in which case the existing single-worker e2e test exercises ONE arm (whichever the shipped reuse.sched.nuc routes through) and the other arm needs a synthetic pin.

## Dependencies

- Forward-carried from: TASK-0278 cycle-99 architect review (silent-sibling family closure).
- Related: TASK-0273 (the original walker arm), TASK-0278 (strip-mine arm closure).
- Pattern: same as the cycle-95 `UnknownIterVarInScope` rename sweep — grep ALL peer sites + verify each is covered.
<!-- SECTION:DESCRIPTION:END -->
