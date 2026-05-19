---
id: TASK-0173
title: >-
  Absolute-index rebinding for the non-divisible / trailing-partial-tile block=
  case
status: To Do
assignee: []
created_date: '2026-05-19 00:08'
updated_date: '2026-05-19 02:08'
labels:
  - M2
  - backend
  - contract
dependencies:
  - TASK-0124
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Discovered by TASK-0124. block_transform rewrites for VAR:LO..HI block=N into for VAR__tile:0..ceil((HI-LO)/N) { for VAR:0..N { body } } and REUSES VAR's IterVar on the inner loop; its own module docs (line ~83) say codegen must compute the absolute iteration value LO+tile*N+inner — block_transform deliberately defers that rebinding. The pre-TASK-0124 AlgoIR-walking backend never tiled (it walked source IrStmt), so it emitted untiled code that was accidentally correct. The EventList faithfully carries the tiled structure, so TASK-0124's backend MUST do the rebinding or an accumulator kernel (07-matmul madd) double-counts. TASK-0124 implements the rebinding for the EVENLY-DIVISIBLE single-nest case (07-matmul block=8, N=16: one nest, absolute = LO+tile*N+inner — clean, e2e PASS). The NON-DIVISIBLE case (05-stencil block=4, range length 14) decomposes into TWO sibling nests whose CORRECT absolute formulas differ: LO+tile*N+inner for the full-tile nest vs the CONSTANT base LO+num_full*N+inner for the trailing-partial-tile nest (its tile loop is 0..1, so tile*N=0 — wrong base). The EventList/ACFG does NOT carry num_full or a 'this is the partial tile' marker, so a correct general rebinding is a real contract extension, not a backend-local change. TASK-0124 leaves the non-divisible inner var on its SOURCE-form bound (current behaviour); 05-stencil/blocked stays runtime-correct only because blur3 is idempotent (re-writing img_out[y][x] with the same value). A non-divisible blocked schedule with an ACCUMULATOR kernel would be WRONG today — this task makes it correct.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 The EventList/ACFG carries enough (e.g. per-tile-nest base offset, or a partial-tile marker, or block N + num_full) for codegen to compute the correct absolute index for BOTH the full-tile and trailing-partial-tile nests
- [ ] #2 pthreads-sync rebinds the non-divisible inner-block loop correctly (full nest: LO+tile*N+inner; partial nest: LO+num_full*N+inner)
- [ ] #3 A synthetic non-divisible blocked schedule over an ACCUMULATOR kernel produces bit-identical output to its naive schedule
- [ ] #4 05-stencil/blocked + 07-matmul/blocked + determinism stay green
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
FORWARD-CARRY from TASK-0180 (per-occurrence strip-mine rebind tag, landed commit 3297066): TASK-0180 added BlockTag{block_n,num_full,is_partial} per-occurrence on Event::Loop (origin block_transform). This DELIVERS this task's AC#1 (the contract now carries N + num_full + a partial-tile marker) AND AC#2 (pthreads-sync rebinds the non-divisible inner-block loop: full nest LO+tile*N+inner, trailing-partial nest the CONSTANT LO+num_full*N+inner). 05-stencil/blocked (non-divisible, for y:1..15 block=4) now EXERCISES it and is STRUCTURALLY correct — codegen verified: full `1 + y__tile*4 + y`, partial `1 + 3*4 + y` — no longer correct-only-because-blur3-is-idempotent (pre-0180 the counts==2 guard excluded it and emitted the full source range 1..15 inside both tile loops). AC#4 (05-stencil/07-matmul/determinism green) holds (verified 3x non-flaky in 0180's gate). REMAINING: AC#3 — a DEDICATED synthetic non-divisible *accumulator* kernel differential test asserting bit-identical-to-naive. 05-stencil's blur3 is idempotent so it does not by itself prove the accumulator case, even though the arithmetic is now provably correct. Not self-checked here (forward-carry only); this task can likely close by adding that one synthetic test now that the rebinding is implemented.
<!-- SECTION:NOTES:END -->
