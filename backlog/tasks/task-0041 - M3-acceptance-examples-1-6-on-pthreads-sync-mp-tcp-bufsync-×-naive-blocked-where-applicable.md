---
id: TASK-0041
title: >-
  M3 acceptance: examples 1-6 on (pthreads-sync, mp-tcp-bufsync) × (naive,
  blocked-where-applicable)
status: To Do
assignee: []
created_date: '2026-05-17 23:07'
updated_date: '2026-05-19 01:32'
labels:
  - M3
  - validation
dependencies:
  - TASK-0039
  - TASK-0040
  - TASK-0167
  - TASK-0178
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Milestone gate. Cross-backend differential test green. This is the moment the algorithm/schedule split AND the middle-end/presentation-layer split become falsifiable simultaneously.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 'just e2e --milestone M3' exits 0.
- [ ] #2 Matrix is examples {1..6} × schedules {naive, blocked-where-applicable} × backends {pthreads-sync, mp-tcp-bufsync}.
- [ ] #3 Every cell that should compile does compile; every cell that should not (capability mismatch) is correctly rejected at compile time, not at runtime.
- [ ] #4 CI runs the full M3 matrix on every commit.
- [ ] #5 Test: deliberately break one cell (e.g. flip a sign in mp-tcp-bufsync codegen); CI catches it.
- [ ] #6 Implementation notes record any cells skipped/excluded with reason.
- [ ] #7 Implementation notes record honest limitations (still sync only; async + buffered comes at M4; reuse and distributed come at M5).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
ORCHESTRATOR RE-TRIAGE (phase3-ralph, NOT fake-closed). The headline cross-backend differential IS green and independently reviewer-verified (just e2e 20/pass16/fail0/skip4 across 5 verbatim-identical runs; 02-split/split as two TCP processes SHA256==hand-written reference oracle; pthreads-sync + mp-tcp-bufsync). BUT TASK-0041 as specified is NOT genuinely met — precise gaps + encoded prerequisites: AC#1 (`just e2e --milestone M3` exits 0) — `--milestone` is accepted-but-IGNORED; needs TASK-0167 (genuine milestone parameterisation). AC#2 (examples {1..6}) — matrix has 01,02,03,05,07; examples 04 (prefix sum) + 06 (separable filter) DO NOT EXIST yet; needs TASK-0039 + TASK-0040. AC#3 substantially verified (capability mismatch = compile-time typed ContractGap fail-loud, reviewer-confirmed) but tied to the incomplete matrix. AC#4 (CI runs full M3 matrix every commit) — CI exists (just ci, TASK-0057) but single-job, not milestone-gated; needs TASK-0167. AC#5 (deliberately break a cell, CI catches) — NO cross-backend negative arm; filed as TASK-0178. AC#6/#7 (notes) — recordable once the matrix is complete. Dependencies added: task-0039, task-0040, task-0167, task-0178. TASK-0041 stays To Do (blocked on those); it is the M3 milestone capstone and closes only when all four land + AC#6/#7 recorded. Closing it now would be AC-gaming a milestone gate whose matrix is literally missing 2 of 6 examples — refused.

Forward-carried from TASK-0039/TASK-0040: examples 04-prefix-sum and 06-separable-filter now EXIST and are differentially green. AC#2 matrix is closer:
- 04-prefix-sum/naive: byte-identical vs independent reference.bin on BOTH pthreads-sync and mp-tcp-bufsync (required). 04/blocked is honestly SKIPPED (TASK-0180: reused loop-var name double-counts the accumulator) — NOT faked.
- 06-separable-filter/{naive,blocked}: byte-identical on BOTH backends (4 required cells). 06/blocked is the POSITIVE CONTROL confirming TASK-0180 (distinct per-pass loop-var names ⇒ rebinding applies ⇒ correct).
- e2e matrix now 28 cells, 22 pass, 0 required-fail, determinism byte-identical (3x non-flaky).
Open deps for the M3 capstone to track: TASK-0179 (in-array scan / acfg panic), TASK-0180 (blocked accumulator rebinding for reused loop-var names). TASK-0041 already depends on TASK-0039/0040.
<!-- SECTION:NOTES:END -->
