---
id: TASK-0351
title: >-
  Cycle-217 stale-narrative broader sweep: 16-jacobi + 17-spmv prog.algo.nuc /
  README.md / kernels.rs follow-up (TASK-0350 in-cycle out-of-scope finding)
status: To Do
assignee: []
created_date: '2026-05-27 21:22'
updated_date: '2026-05-27 21:40'
labels:
  - hygiene
  - doc-lie
  - cycle-217-follow-up
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
=== Filed as TASK-0350 cycle-217 in-cycle scope-expansion follow-up ===

TASK-0350 cycle 217 closed its filed scope (distributed-rows.sched.nuc:72 + sibling audit across schedules/*.sched.nuc). During the audit, additional stale-narrative-class defects were found OUTSIDE the filed scope in sibling files of the showcase example expansion epic:

## Stale-narrative sites (post-cycle-216 state)

### 16-jacobi
- prog.algo.nuc:37-38 — 'Multi-worker distributed schedules. Filed as the AC#3 follow-up; same precedent as 15-transpose's AC#2 (TASK-0341.01.01).' TASK-0341.02.02 landed cycle 208 (with 2 honest-BLOCKED cells: mp-tcp-bufsync + mp-tcp-poll per TASK-0330); 5 of 7 tier-1 backends [[required]]. The claim 'Filed as the AC#3 follow-up' is stale.
- README.md:37-50 — 'What this example does NOT stress (yet)' section. Subsection #46 'Multi-worker distributed schedules. Filed as a follow-up' is stale (landed cycle 208).

### 17-spmv
- prog.algo.nuc:57-62 — 'Multi-worker distributed schedules. Filed as the AC#3 follow-up.' TASK-0341.03.02 landed cycle 212 + TASK-0341.03.02.01 cross-backend × 6 siblings landed cycle 214. All 7 tier-1 backends now [[required]]. Stale.
- README.md:69-84 — 'What this example does NOT stress (yet)' section. Subsection #78 'Multi-worker distributed schedules. Filed as a follow-up' is stale (landed cycle 212/214).

## Acceptance criteria

1. **16-jacobi cleanup**: rewrite prog.algo.nuc:37-38 and README.md multi-worker bullet to reflect cycle-208 landing (5 of 7 tier-1 backends [[required]]; 2 honest-BLOCKED via TASK-0330 [[skip]]). Cite TASK-0341.02.02 + TASK-0341.02.03 landings.

2. **17-spmv cleanup**: rewrite prog.algo.nuc:57-62 and README.md multi-worker bullet to reflect cycles 212/214 landings (all 7 tier-1 backends [[required]]). Cite TASK-0341.03.02 + TASK-0341.03.02.01 landings.

3. **Bonus audit**: kernels.rs files in 16-jacobi + 17-spmv — verify no stale predictive claims about deferred ACs. (15-transpose/kernels.rs was clean per cycle-217 audit.)

4. **Unblocking parent ACs**: once 16-jacobi + 17-spmv staleness is fixed, TASK-0341.02 AC#5 + TASK-0341 AC#3 become fully tickable (cycle 217 ticked the 15-transpose-class half of these obligations).

## Honest scope LIMIT

- Doc-only hygiene; no Rust code changes; no e2e impact.
- Specifically does NOT cover the 'Backends' table column staleness in any of the showcase example READMEs — that's a separate cosmetic class (the table headers vs the body content) and not the predictive-claim class.

## Defect class

Per memory feedback-comment-doc-lie-recurring: 'What this example does NOT stress (yet)' sections are particularly stale-prone because (a) they live in the example's own README/prog.algo.nuc (not in e2e-matrix.toml which gets updated each promotion cycle), and (b) the 'yet' modifier IS the time-bound claim that gets stale. The 'yet' is the recurring tell.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
=== Cycle 217b architect P2.2 forward-carried lesson sharpening ===

When applying a stale-narrative sweep with in-cycle scope expansion (cycle-117 calibration), the in-cycle grep MUST be WHOLE-FILE end-to-end, not window-of-edit.

Cycle 217 failed this discipline: the in-cycle grep covered the SECTION the orchestrator was editing (README.md:11-44) but missed a structurally-identical stale-narrative site 140 lines further down in the SAME file (README.md:178-185). The architect P1.1 caught it via independent whole-file grep with a broader regex.

DISCIPLINE FOR TASK-0351 IMPLEMENTER:
- Before any commit on the 16-jacobi + 17-spmv sweep, run a whole-file grep across each touched file with the architect's broader regex: grep -nE '(follow-up|deferred|yet|will be|future|next cycle)' on each file end-to-end.
- For each hit, verify against current state: is the referenced follow-up still LIVE (in tracker, To Do or In Progress), or has it landed (closed Done)? Only LIVE references survive; landed references must be rewritten to past-tense cycle-citation.
- The 'yet' modifier is the recurring time-bound tell (per cycle-217's original lesson). The 'follow-up' phrase is a SECOND recurring tell (per cycle-217b sharpening). Both must be in the grep regex.

The TASK-0351 implementer should also apply the cycle-215b parent-AC-tick discipline: before commit, grep TASK-0341.02 + TASK-0341 parent ACs and tick any directly closed by the cycle's work.
<!-- SECTION:NOTES:END -->
