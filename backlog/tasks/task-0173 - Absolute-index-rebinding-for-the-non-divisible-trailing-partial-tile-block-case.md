---
id: TASK-0173
title: >-
  Absolute-index rebinding for the non-divisible / trailing-partial-tile block=
  case
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 00:08'
updated_date: '2026-05-19 03:27'
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
- [x] #1 The EventList/ACFG carries enough (e.g. per-tile-nest base offset, or a partial-tile marker, or block N + num_full) for codegen to compute the correct absolute index for BOTH the full-tile and trailing-partial-tile nests
- [x] #2 pthreads-sync rebinds the non-divisible inner-block loop correctly (full nest: LO+tile*N+inner; partial nest: LO+num_full*N+inner)
- [x] #3 A synthetic non-divisible blocked schedule over an ACCUMULATOR kernel produces bit-identical output to its naive schedule
- [x] #4 05-stencil/blocked + 07-matmul/blocked + determinism stay green
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Baseline gate captured: e2e 28 total / 24 pass / 4 skip / 0 fail; 04-prefix-sum/blocked already required+green on both backends (TASK-0180). 30 [[required]] entries.
2. AC#3 host = 04-prefix-sum. Add schedules/blocked-nondiv.sched.nuc: loop j : block=6. j is Pass-3 within-block scan inner var: SINGLE occurrence, range 0..BS=64 (L=64), 64%6=4 != 0 -> num_full=10 (>=2), remainder=4. j IS the genuine accumulation axis of out[b][i] <-- block_scan(...) (read-then-write fold, NON-idempotent, unlike 05-stencil blur3). Strongest accumulator proof: tiling the actual reduction axis non-divisibly. Mirror 06-separable-filter comment discipline, INVERTED rationale (DELIBERATELY NON-DIVISIBLE = TASK-0173 AC#3 accumulator proof).
3. Add [[required]] cells blocked-nondiv x {pthreads-sync, mp-tcp-bufsync}, milestone M3 (matches 04-prefix-sum/blocked). runnable_examples already lists 04-prefix-sum.
4. Optional: block_transform unit test asserting emitted partial-nest abs index for L=64 block=6 (full: 0 + j__tile*6 + j ; partial: 0 + 10*6 + j).
5. Full gate: e2e 28->30 / 24->26, the 2 new cells bit-identical to reference.bin on BOTH backends; determinism-check + negatives; just test; clippy -D warnings; just ci. Run e2e >=3x non-flaky.
6. Honest discipline: if new cell NOT bit-identical -> real bug, file blocker dep TASK-0173/0180, leave AC#3 unchecked. No fake, no silent skip.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
FORWARD-CARRY from TASK-0180 (per-occurrence strip-mine rebind tag, landed commit 3297066): TASK-0180 added BlockTag{block_n,num_full,is_partial} per-occurrence on Event::Loop (origin block_transform). This DELIVERS this task's AC#1 (the contract now carries N + num_full + a partial-tile marker) AND AC#2 (pthreads-sync rebinds the non-divisible inner-block loop: full nest LO+tile*N+inner, trailing-partial nest the CONSTANT LO+num_full*N+inner). 05-stencil/blocked (non-divisible, for y:1..15 block=4) now EXERCISES it and is STRUCTURALLY correct — codegen verified: full `1 + y__tile*4 + y`, partial `1 + 3*4 + y` — no longer correct-only-because-blur3-is-idempotent (pre-0180 the counts==2 guard excluded it and emitted the full source range 1..15 inside both tile loops). AC#4 (05-stencil/07-matmul/determinism green) holds (verified 3x non-flaky in 0180's gate). REMAINING: AC#3 — a DEDICATED synthetic non-divisible *accumulator* kernel differential test asserting bit-identical-to-naive. 05-stencil's blur3 is idempotent so it does not by itself prove the accumulator case, even though the arithmetic is now provably correct. Not self-checked here (forward-carry only); this task can likely close by adding that one synthetic test now that the rebinding is implemented.

AC#3 DELIVERED (self-verified, not forward-carried). Host: 04-prefix-sum. New schedule schedules/blocked-nondiv.sched.nuc: loop j : block=6 over Pass-3 within-block-scan axis for j : 0..BS (L=64). NON-divisible: 64 = 6*10 + 4 -> num_full=10 (>=2), rem=4 (non-zero). j is a GENUINE non-idempotent accumulator: out[b][i] <-- block_scan(out[b][i], in_arr[b][j], block_off[b], i, j) reads+writes out[b][i] (additive fold over j); contrast 05-stencil blur3 which is idempotent. j occurs EXACTLY ONCE (Pass 3 only) -> isolates the non-divisible accumulator path, no reused-name confound.

Emitted code VERIFIED (nucleus build, pthreads-sync): full-tile nest `for j__tile in 0..10` -> abs j = (0_i64 + (j__tile * 6_i64) + j) = LO+tile*N+inner ; trailing partial tile `for j__tile in 0..1` -> abs j = (0_i64 + (10_i64 * 6_i64) + j) = LO+num_full*N+inner = 60+j (the CONSTANT num_full*N base; tile*6 would be 0 = wrong base 0..4 instead of 60..64). The abs j feeds BOTH in_arr index and block_scan's j arg (the j<=i / j==0 predicates) so a wrong base would diverge.

Gate (ACTUAL): e2e 28->30 total / 24->26 pass / 0 fail / 4 skip (unchanged); 04-prefix-sum/blocked-nondiv PASS bit-identical to reference.bin on pthreads-sync AND mp-tcp-bufsync, 3x non-flaky. determinism-check 30/26/0/4 byte-identical (incl. new cells). determinism-check-negative + xbackend-check-negative still bite. just test 0 failed (+ new unit test rewrite_node_prefix_sum_nondiv_j_block6). clippy --workspace -D warnings clean. just ci green. Pre-existing 28-cell matrix UNCHANGED.

AC#1/#2/#4 re-verified (not just forward-carried): 05-stencil/blocked emitted code re-inspected -> full `1_i64 + (y__tile * 4_i64) + y`; 05/07-blocked + determinism green in the 3x gate. Commit 237fd53.

ORCHESTRATOR REVIEW GATE (phase3-ralph): qa-test-runner GO + mped-architect GO ("Done is honest"), both read-only. Numbers RE-RUN by reviewers: just test 372/0/1 (new rewrite_node_prefix_sum_nondiv_j_block6 passes); just e2e 30/26/0/skip4/required-fail0 x3 verbatim non-flaky (pre-existing 28 unchanged; +2 new 04-prefix-sum/blocked-nondiv cells PASS both backends); SHA256 d51709d0… IDENTICAL across reference.bin AND blocked-nondiv output.bin on BOTH pthreads-sync + mp-tcp-bufsync; emitted abs-index VERIFIED full `0 + j__tile*6 + j`, trailing-partial CONSTANT `0 + 10*6 + j`(=60+j, not j__tile*6); determinism-check 30/26/0/4 byte-identical + determinism-check-negative + xbackend-check-negative all bite; clippy clean; just ci exit 0; commit 237fd53 NO Co-Authored-By/AI trailer (the implementer self-disclosed an initial trailer and amended it out — verified clean; correct honesty behaviour). Architect: GENUINE falsifiable accumulator proof — block_scan is a true read-then-write non-idempotent accumulator (vs 05-stencil blur3 idempotent escape); rebound abs j feeds BOTH in_arr index AND order-sensitive predicates (j==0 boff-once, j<=i mask) so a wrong trailing-partial base WOULD diverge a byte; L=64,N=6 genuinely non-divisible (num_full=10>=2, rem=4); j is SINGLE-occurrence (no TASK-0180 reused-name confound — isolates the partial-tile rebinding cleanly); reference/ std-only structurally-distinct straight-line oracle, harness diffs per-EXAMPLE reference.bin (apples-to-apples, not per-schedule); AC#1/#2/#4 genuinely RE-VERIFIED (re-inspected 05-stencil emitted code + re-ran gate, not blind forward-carry); additive-only (+256/-0, no backend/contract/transform-logic change); no AC-gaming/silent-change/stale-comment. Single-host limitation correctly scoped+disclosed; the multi-process-cross-worker-partial-tile frontier filed as a tracked follow-up (NOT a 0173 defect — do not hold 0173 open). TASK-0173 Done is HONEST: all 4 ACs genuinely met + independently verified + both reviews GO.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closes TASK-0173: absolute-index rebinding for the non-divisible / trailing-partial-tile block= case is now COMPLETE and proven over a non-idempotent accumulator.

AC#1/AC#2 were delivered by TASK-0180 (commit 3297066: per-occurrence Event::Loop.block_tag {block_n,num_full,is_partial} emitted by block_transform; pthreads-sync rebinds full nest LO+tile*N+inner, trailing partial LO+num_full*N+inner). Re-verified here (not merely forward-carried): 05-stencil/blocked emitted code re-inspected -> full `1 + y__tile*4 + y`; 05/07-matmul-blocked + determinism re-run green 3x.

AC#3 (this task's remaining work): added a synthetic NON-divisible blocked schedule over a genuine ACCUMULATOR and proved it bit-identical to the naive schedule.

Changes:
- nuc-nucleus/examples/04-prefix-sum/schedules/blocked-nondiv.sched.nuc: `loop j : block=6` tiles Pass-3's within-block-scan ACCUMULATION axis `for j : 0..BS` (L=64). 64 = 6*10 + 4 -> 10 full tiles of 6 + a trailing partial tile of 4 (num_full>=2, non-zero remainder). j is a non-idempotent accumulator (out[b][i] read+written each step) and occurs exactly once (Pass 3), isolating the non-divisible accumulator path with no reused-name confound — the precise gap 05-stencil's idempotent blur3 could not close. Comment discipline mirrors 06-separable-filter/blocked with INVERTED rationale.
- nuc-nucleus/e2e-matrix.toml: [[required]] blocked-nondiv x {pthreads-sync, mp-tcp-bufsync}, M3.
- nucleus/compiler/src/passes/block_transform.rs: unit test rewrite_node_prefix_sum_nondiv_j_block6 pinning emitted full/partial BlockTags for the L=64 block=6 shape.
No change to block_transform/backend/BlockTag contract (TASK-0180 owns those).

Verified emitted code: full nest abs j = 0 + j__tile*6 + j; trailing partial tile abs j = 0 + 10*6 + j (constant num_full*N=60 base, not tile*6=0). Bit-identical to reference.bin on BOTH backends.

Gate (actual): e2e 28->30 total / 24->26 pass / 0 fail / 4 skip (pre-existing matrix unchanged), 3x non-flaky. determinism-check byte-identical incl. new cells; determinism + xbackend negatives still bite. just test 0 failed; clippy -D warnings clean; just ci green.

Honest limitation: 04-prefix-sum is single-host so under mp-tcp the cell is single-process — it proves accumulator ARITHMETIC correctness under a non-divisible partial tile, not multi-process cross-worker transfer of the partial tile (inherent to single-host, out of AC#3 scope, already documented in the matrix). No new bug surfaced.

Commit: 237fd53.
<!-- SECTION:FINAL_SUMMARY:END -->
