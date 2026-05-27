---
id: TASK-0350
title: >-
  Cycle-216 stale-narrative sweep: distributed-rows.sched.nuc:72 + sibling audit
  of schedule files for post-promotion staleness
status: To Do
assignee: []
created_date: '2026-05-27 21:12'
labels:
  - hygiene
  - doc-lie
  - cycle-216-follow-up
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
=== Filed as TASK-0341.01.01.01 cycle-216 architect P3 observation forward-carry ===

The cycle-216 architect read-only review found a stale narrative claim at:

  nuc-nucleus/examples/15-transpose/schedules/distributed-rows.sched.nuc:72

reading: 'Cross-backend promotion is the AC#3 follow-up; this cycle lands pthreads-sync only.'

This was present-tense + accurate at cycle 215 (when only pthreads-sync was [[required]]). Post-cycle-216 (all 7 tier-1 backends are now [[required]]), the claim is stale. Per TASK-0341.01 AC#4 + TASK-0341 AC#3 (cycle-178 doc-lie-promotion mitigation), //! module-level docstrings must be present-tense + cite the landing cycle. The line-72 claim now violates 'present-tense' (the follow-up has landed; the wording asserts it has not).

The cycle-216 architect explicitly classified this as 'NOT for cycle-216 fold-back — folding it would invite scope creep against an already-clean cycle'. Filed here per the phase3-backlog-ralph 'new tasks are the default output' discipline.

## Acceptance criteria

1. **Fix the immediate site**: rewrite distributed-rows.sched.nuc:72 to reflect the post-cycle-216 state. Suggested wording: 'Cross-backend promotion to the other 6 tier-1 backends landed cycle 216 (TASK-0341.01.01.01). All 7 tier-1 backends now [[required]].' Cite cycle 216 as the second landing cycle.

2. **Sibling audit**: grep across all schedules/*.sched.nuc files for similar 'this cycle lands X only' or 'only pthreads-sync is required' phrases that have become stale after subsequent cross-backend promotions. Specifically check:
   - 17-spmv/distributed.sched.nuc (cross-backend promoted cycle 214)
   - 16-jacobi/distributed.sched.nuc (cycle 207/208 — partition variant has different state)
   - 15-transpose/naive.sched.nuc (cycle 205)
   - 16-jacobi/naive.sched.nuc (cycle 207)
   - 17-spmv/naive.sched.nuc (cycle 211)
   For each: if a 'cycle N lands X only' claim exists and was stale by a later cycle, fix it + cite the later landing cycle.

3. **Unblock TASK-0341.01 AC#4 + TASK-0341 AC#3**: once the schedule comments are cleaned up, the cycle-178 doc-lie-promotion mitigation ACs on the parents become tickable. The closure cycle for the parents should explicitly cite this task as the prerequisite.

## Honest scope LIMIT

- This is a doc-only hygiene task. No Rust code changes, no e2e behaviour changes. The cycle that picks it up should run cheap structural gate + 1 e2e sample for sanity, but the test/e2e baseline will not change.
- The audit is bounded by the example schedules in 15-transpose / 16-jacobi / 17-spmv (the showcase example expansion epic). Older examples (01-14) have stable cross-backend matrix coverage filed pre-cycle-204 — their schedule comments are likely already drift-free, but a quick grep doesn't cost much.

## Why this is HIGHER value than it looks

Per memory feedback-comment-doc-lie-recurring + feedback-verbatim-copy-comment-doc-lie + cycle-178/181b/182b lessons: stale 'this cycle lands X' narratives in schedule files survive past their accurate-window because the schedule files are usually untouched after the AC#1 landing cycle. The cross-backend-promotion follow-up cycles edit e2e-matrix.toml (where the narrative IS updated each cycle) but leave the schedule file comment as cycle-N-frozen. This is the recurring 'time-bound claim becomes stale post-promotion' class.
<!-- SECTION:DESCRIPTION:END -->
