---
id: TASK-0041
title: >-
  M3 acceptance: examples 1-6 on (pthreads-sync, mp-tcp-bufsync) × (naive,
  blocked-where-applicable)
status: In Progress
assignee: []
created_date: '2026-05-17 23:07'
updated_date: '2026-05-19 03:07'
labels:
  - M3
  - validation
dependencies:
  - TASK-0166
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Milestone gate. Cross-backend differential test green. This is the moment the algorithm/schedule split AND the middle-end/presentation-layer split become falsifiable simultaneously.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 'just e2e --milestone M3' exits 0.
- [x] #2 Matrix is examples {1..6} × schedules {naive, blocked-where-applicable} × backends {pthreads-sync, mp-tcp-bufsync}.
- [x] #3 Every cell that should compile does compile; every cell that should not (capability mismatch) is correctly rejected at compile time, not at runtime.
- [ ] #4 CI runs the full M3 matrix on every commit.
- [x] #5 Test: deliberately break one cell (e.g. flip a sign in mp-tcp-bufsync codegen); CI catches it.
- [x] #6 Implementation notes record any cells skipped/excluded with reason.
- [x] #7 Implementation notes record honest limitations (still sync only; async + buffered comes at M4; reuse and distributed come at M5).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
ORCHESTRATOR RE-TRIAGE (phase3-ralph, NOT fake-closed). The headline cross-backend differential IS green and independently reviewer-verified (just e2e 20/pass16/fail0/skip4 across 5 verbatim-identical runs; 02-split/split as two TCP processes SHA256==hand-written reference oracle; pthreads-sync + mp-tcp-bufsync). BUT TASK-0041 as specified is NOT genuinely met — precise gaps + encoded prerequisites: AC#1 (`just e2e --milestone M3` exits 0) — `--milestone` is accepted-but-IGNORED; needs TASK-0167 (genuine milestone parameterisation). AC#2 (examples {1..6}) — matrix has 01,02,03,05,07; examples 04 (prefix sum) + 06 (separable filter) DO NOT EXIST yet; needs TASK-0039 + TASK-0040. AC#3 substantially verified (capability mismatch = compile-time typed ContractGap fail-loud, reviewer-confirmed) but tied to the incomplete matrix. AC#4 (CI runs full M3 matrix every commit) — CI exists (just ci, TASK-0057) but single-job, not milestone-gated; needs TASK-0167. AC#5 (deliberately break a cell, CI catches) — NO cross-backend negative arm; filed as TASK-0178. AC#6/#7 (notes) — recordable once the matrix is complete. Dependencies added: task-0039, task-0040, task-0167, task-0178. TASK-0041 stays To Do (blocked on those); it is the M3 milestone capstone and closes only when all four land + AC#6/#7 recorded. Closing it now would be AC-gaming a milestone gate whose matrix is literally missing 2 of 6 examples — refused.

Forward-carried from TASK-0039/TASK-0040: examples 04-prefix-sum and 06-separable-filter now EXIST and are differentially green. AC#2 matrix is closer:
- 04-prefix-sum/naive: byte-identical vs independent reference.bin on BOTH pthreads-sync and mp-tcp-bufsync (required). 04/blocked is honestly SKIPPED (TASK-0180: reused loop-var name double-counts the accumulator) — NOT faked.
- 06-separable-filter/{naive,blocked}: byte-identical on BOTH backends (4 required cells). 06/blocked is the POSITIVE CONTROL confirming TASK-0180 (distinct per-pass loop-var names ⇒ rebinding applies ⇒ correct).
- e2e matrix now 28 cells, 22 pass, 0 required-fail, determinism byte-identical (3x non-flaky).
Open deps for the M3 capstone to track: TASK-0179 (in-array scan / acfg panic), TASK-0180 (blocked accumulator rebinding for reused loop-var names). TASK-0041 already depends on TASK-0039/0040.

FORWARD-CARRY from TASK-0180 (landed, commit 3297066): per-occurrence strip-mine rebinding closed the last known blocked-schedule correctness gap. e2e matrix is now 28 total / 24 pass / 0 fail / 4 skip / 0 required-fail, 3x non-flaky, determinism byte-identical. 04-prefix-sum/blocked is now a [[required]] byte-identical differential on BOTH pthreads-sync and mp-tcp-bufsync (was an honest [[skip]] for the accumulator double-count); 05/06/07-blocked stay green and 05-stencil/blocked is now structurally correct (no longer idempotence-dependent). All examples 04-07 naive+blocked are required-green on both backends. Remaining matrix skips are distributed-placement cells (TASK-0117/0126/0172), not blocked-schedule correctness. Informational forward-carry; not self-checking this task's ACs.

ORCHESTRATOR M3-CAPSTONE ASSESSMENT (phase3-ralph; supersedes the earlier re-triage — all four encoded prerequisites are now Done: TASK-0039 ✅, TASK-0040 ✅, TASK-0167 ✅, TASK-0178 ✅). The M3 cross-backend differential is SUBSTANTIVELY ACHIEVED and independently reviewer-verified across the dependency cycles. AC status: AC#1 MET (just e2e-milestone M3 / --milestone M3 exits 0, 28/24/0/skip4/required-fail0 — qa-verified in the TASK-0167 gate; --milestone is now genuinely honoured). AC#2 MET (examples 1..6 all exist: 01/02/03 + 04-prefix-sum [TASK-0039] + 05 + 06-separable-filter [TASK-0040], 07 bonus; naive + blocked-where-applicable; BOTH backends pthreads-sync + mp-tcp-bufsync [TASK-0036]; 28-cell matrix, 24 required green vs the hand-written backend-independent reference.bin oracle). AC#3 MET (every should-compile cell compiles; capability mismatch = compile-time typed EmitError::ContractGap fail-loud, reviewer-verified; distributed cells [[skip]] rejected upstream not at runtime). AC#5 MET (TASK-0178 xbackend-check-negative — deliberately corrupts an mp-tcp cell, CI/just-ci catches it with required-fail>0, three-way-asymmetry-proven non-flaky x3 + the determinism-check-negative + TASK-0163 coverage guards). AC#6 RECORDED: skipped/excluded cells = the 4 distributed cells (03-reduction/distributed, 05-stencil/distributed) under BOTH backends, [[skip]] with reasons in e2e-matrix.toml (TASK-0117 distributed placement + TASK-0126 per-tile transfer codegen + TASK-0172 non-uniform barrier + halo synthesis — not transport-specific; not [[required]] at any milestone). AC#7 HONEST LIMITATIONS: still sync-only (async + buffered transfers = M4); reuse + distributed placement = M5; the 28-cell matrix is single-host + the one host↔w0 multi-process split (worker↔worker mesh = TASK-0175); inherited fail-loud caveats TASK-0172/0173/0181 (no required cell hits them). AC#4 NOT checked — "CI runs the full M3 matrix on every commit" literally requires a live CI runner; this repo has NO git remote/runner (ci.yml + the genuine M1/M2/M3 milestone matrix from TASK-0167 are gate-logic-complete and locally reproducible verbatim, but UNOBSERVED on a real runner). This is the SAME honest standing limitation already tracked on TASK-0057 (AC#2/#4/#5) and TASK-0167 (AC#4), dep TASK-0166 (branch protection / runner — a maintainer/GitHub-settings action, impossible from the repo). Per phase3-ralph honest-failure discipline and consistent with the TASK-0036/0057 precedent, TASK-0041 is NOT marked Done while AC#4 is real-runner-pending — it stays In Progress. SUBSTANTIVE CONCLUSION: the user-goal part "verify examples run across different backends and produce the same correct results as their naive reference rust counterparts" is SUBSTANTIATED and falsifiable-and-holding for the tier-1 M3 set; the only residual is environmental (no CI runner), tracked under TASK-0166.
<!-- SECTION:NOTES:END -->
