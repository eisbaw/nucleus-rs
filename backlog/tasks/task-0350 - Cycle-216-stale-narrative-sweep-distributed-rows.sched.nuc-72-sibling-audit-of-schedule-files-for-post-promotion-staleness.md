---
id: TASK-0350
title: >-
  Cycle-216 stale-narrative sweep: distributed-rows.sched.nuc:72 + sibling audit
  of schedule files for post-promotion staleness
status: Done
assignee:
  - '@orchestrator'
created_date: '2026-05-27 21:12'
updated_date: '2026-05-27 21:40'
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

## Acceptance Criteria
<!-- AC:BEGIN -->
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

- [x] #1 Fix the immediate site: distributed-rows.sched.nuc:72 stale 'Cross-backend promotion is the AC#3 follow-up; this cycle lands pthreads-sync only' rewritten to cite cycle 215 + cycle 216 landings
- [x] #2 Sibling audit: grep across nuc-nucleus/examples/{15-transpose,16-jacobi,17-spmv}/schedules/*.sched.nuc for similar 'this cycle lands X only' staleness; no other sites found beyond the architect-flagged one
- [x] #3 Cheap structural gate + 1 e2e sanity sample clean post-edit: just check-textual-replace-on-codegen + check-include-str-coverage + check-narrative-doc-lie + check-mega-files OK; just e2e 280/246/0/34/0 unchanged (doc-only edits)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
=== Cycle 217b clarification addendum (architect P1.1 + P2.1 + P2.2 fold-back) ===

ARCHITECT P1.1 (silent-sibling defect within the same file): cycle-217 in-cycle scope expansion fixed README.md:11-44 + prog.algo.nuc but MISSED README.md:178-185 ('Required schedules' section bullet for distributed-rows still said 'cross-backend promotion is TASK-0341.01.01.01 follow-up' — stale post-cycle-216). Folded back in cycle 217b: rewrote the bullet to cycles 215/216 dual-citation matching the wording cycle 217 used elsewhere in the same file.

ARCHITECT P2.1 (AC#2 wording vs executed scope drift): AC#2 as ticked describes 'schedules/*.sched.nuc' scope — the as-planned narrow scope. The as-executed scope was wider (prog.algo.nuc + README.md within 15-transpose). NOT rewriting AC#2 in-place per feedback-ac-rewrite-on-done-task; this clarification addendum records the divergence.

ARCHITECT P2.2 (process gap — window-of-edit grep not whole-file): the cycle-217 in-cycle grep covered the SECTION I was editing (README.md:11-44) but missed the same idiom 140 lines further down (README.md:178). The discipline lesson: when applying a stale-narrative sweep with in-cycle scope expansion, the grep MUST be whole-file end-to-end, not window-of-edit. The cycle-217 forward-carried lesson on TASK-0351 already mentioned the sibling-sweep discipline; cycle 217b sharpens it to 'whole-file grep' explicitly.

CYCLE-217b EVIDENCE:
- README.md:178-185 (now :178-188): rewritten to dual-cycle citation post-cycle-216.
- Whole-file grep across 15-transpose/{prog.algo.nuc, kernels.rs, README.md, schedules/*.sched.nuc} after fold-back: 2 remaining hits, both legitimately not-stale (line 153 is the live TASK-0347 forward-carry; line 167 quotes the compiler diagnostic's literal text).
- Cheap structural gate + 1 e2e sample re-run post-fold-back: 280/246/0/34/0 unchanged.

PATTERN CLASSIFICATION:
- This is a fresh in-file variant of feedback-silent-sibling-defect: the sibling defect was in the SAME file (README.md), 140 lines below the edited region. Distinct from the cross-file silent-sibling pattern (where the sibling lives in another module/file).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 217 closure (orchestrator-direct, per memory feedback-spawned-agents-refuse-code-edits).

OUTCOME:
- distributed-rows.sched.nuc:72 stale narrative rewritten (architect-flagged cycle-216 P3 site).
- Sibling audit across 15-transpose / 16-jacobi / 17-spmv schedules/*.sched.nuc files: only the one architect-flagged site was stale; the other schedule files are clean.
- Cheap structural gate + 1 e2e sanity sample clean: 280/246/0/34/0 unchanged.

IN-CYCLE SCOPE EXPANSION (disclosed per honest-failure discipline):
- During the AC#2 audit grep, additional stale-narrative sites were found in sibling files of 15-transpose (prog.algo.nuc:22 + README.md table-row + 'NOT stress (yet)' section). Same defect class as the architect-flagged schedule-file site but in different carrier files. Per cycle-117 scope-creep-calibration discipline (same architectural pattern, doc-only, no separate tests, honest disclosure), fixed in-cycle for 15-transpose for consistency.
- Sibling 16-jacobi + 17-spmv have analogous stale-narrative sites in their prog.algo.nuc + README.md files. Per the same cycle-117 calibration discipline, the broader sweep is OUTSIDE the TASK-0350 filed scope and was filed as a precise follow-up: TASK-0351 (cycle-217 in-cycle out-of-scope follow-up).

CYCLE-215b DISCIPLINE APPLIED PRE-COMMIT:
- Pre-commit parent-AC grep: TASK-0341.01 AC#4 + TASK-0341 ACs were checked. AC#4 of TASK-0341.01 is partially closeable by cycle 217 (15-transpose subset) but full closure depends on TASK-0351 landing (16-jacobi + 17-spmv sibling subset). AC#3 of TASK-0341 has the same dependency. NOT pre-ticking those parent ACs; deferred to the cycle-217 review gate verification + TASK-0351 closure cycle.
- AC-backfill on To Do task (cycle-214b precedent): TASK-0350 had no structured ACs pre-cycle (filed cycle 216 with description-only); cycle 217 backfilled 3 ACs and ticks them in this same commit (NOT deferred to fold-back per cycle-214b P2.2 lesson).

CHANGES (cycle 217):
- nuc-nucleus/examples/15-transpose/schedules/distributed-rows.sched.nuc:72 — stale 'this cycle lands pthreads-sync only' → cycles 215/216 landing citation.
- nuc-nucleus/examples/15-transpose/prog.algo.nuc:20-32 — 'What this example does NOT demonstrate' section split into 'What this example also demonstrates (post-AC#1)' + 'What this example does NOT demonstrate' (removing the stale 'Multi-worker placement / partition=rows on the output. Deferred...' and 'Cross-backend differential beyond pthreads-sync. Filed as AC#3.' bullets which both landed cycle 215-216).
- nuc-nucleus/examples/15-transpose/README.md:11-44 — Backends-table row updated + 'What this example does NOT stress (yet)' split into 'What this example also demonstrates (post-AC#1)' + 'What this example does NOT stress' (dropping the 'yet' that was the recurring tell, removing landed bullets).
- TASK-0351 filed.
- TASK-0350 ticked + Done.

FORWARD-CARRIED LESSON (added to TASK-0351):
- The 'NOT stress (yet)' / 'NOT demonstrate' bullet sections in example READMEs / prog.algo.nuc are a recurring stale-narrative trap because the 'yet' / 'deferred to follow-up' modifiers are themselves time-bound claims. The hygiene discipline should be: when a follow-up task closes, sweep the example's example/* siblings for the 'yet' / 'follow-up' bullets that now reference closed tasks, and either rewrite (drop the 'yet'/'follow-up' framing) or move the bullet to a 'What this example also demonstrates (post-AC#X)' section. Future cross-backend-promotion cycles should fold this into their per-cycle checklist.
<!-- SECTION:FINAL_SUMMARY:END -->
