---
id: TASK-0366
title: >-
  rewrite_cumulative_band_tiles: upgrade silent None->whole-array fallback to
  fail-loud EmitError for a partitioned cumulative symbol
status: Done
assignee:
  - '@claude'
created_date: '2026-05-30 09:53'
updated_date: '2026-05-30 17:52'
labels:
  - compiler
  - transfer_inject
  - M6
  - 16-jacobi
  - fail-loud
  - cycle-213-foldback
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-213 architect P3 fold-back. In transfer_inject.rs::rewrite_cumulative_band_tiles, when a CUMULATIVE data symbol (NameSidecar::cumulative_data) has a transfer for which cumulative_band_bounds() returns None, the tile is left unchanged (whole-array). For a cumulative array a whole-array transfer silently re-introduces the xN double-count the pass removes. Provably dead today (16-jacobi field always derives a write band; game-of-life ships no partitioned schedule) and the e2e bit-identity differential would catch it, so cycle 213 made it OBSERVABLE via nuc_trace! only. This task upgrades it to a fail-loud EmitError (the transfer_inject pass entry already returns Result, so the rewrite can be made fallible) so a future partitioned-cumulative shape that hits the None branch fails at compile time instead of emitting xN-wrong output. Reference: nuc_trace! site in rewrite_cumulative_band_tiles + the cumulative_band_bounds None branch.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 rewrite_cumulative_band_tiles is made fallible (or wrapped) so a None from cumulative_band_bounds on a symbol in cumulative_data raises EmitError::ContractGap instead of silently leaving a whole-array tile
- [x] #2 A negative unit test constructs a cumulative Xfer whose src has no band (None path) and asserts the typed error fires
- [x] #3 16-jacobi/distributed stays bit-identical (the dead branch is never hit by shipped schedules; e2e total unchanged)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
AC1: make rewrite_cumulative_band_tiles return Result<ACFGNode, TransferInjectError>; thread ? through Sequence (collect::<Result<Vec<_>,_>>()?) and Repeat (box recursion ?). Replace the nuc_trace! None branch with return Err(CumulativeWholeArrayFallback { data, src, message }) — new #[non_exhaustive] enum variant + Display arm; message names TASK-0366 + xN double-count risk. Add ? at entry call site ~783. Leaf arms (Operation/Sync) unchanged.
AC2: in-file mod tests negative test next to task034102_*: build a single cumulative Xfer node whose cumulative_band_bounds returns None. Force None by giving data_dim_iv_map an entry with NO partitioned iv covering any dim (saw_band==false) while data_dims has the symbol and the data is in cumulative_data. assert matches!(Err(CumulativeWholeArrayFallback{..})).
AC3: e2e totals must stay 322/265/0/57/0 (branch provably dead). If it changes, STOP — real defect, report.
Gate: just build && clippy && test && test-release && e2e inside nix develop.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED cycle-214 (commit b605490). KEY FINDING: the None branch was NOT dead, contra the task brief + cycle-213 narrative. 11-game-of-life/pipelined emits a cross-worker async double-buffer grid transfer (compute->host) and DOES reach the None branch (grid is cumulative + cross-worker, but the schedule has NO partition= so cumulative_band_bounds returns None). The first draft (unconditional return Err) regressed e2e to 322/262/3/57/3 — the 3 game-of-life/pipelined cells (pthreads-async, mp-tcp-event, mp-uds-event) failed at compile. Per AC#3 I stopped and investigated rather than papering over.

ROOT CAUSE of the bad assumption: the cycle-213 comment at the call site claimed game-of-life short-circuits to a structural no-op because partition_worker_ranges is empty. That was a comment lie (recurring pattern #1): the pass does NOT short-circuit; it walks the grid Xfer, gets None, and the whole-array tile it kept was simply CORRECT for the unpartitioned single-compute-worker case.

FIX: the None branch now distinguishes two cases. (A) partition_ranges NON-EMPTY: a partition is active but no iv covers this cumulative array -> array is replicated across partition workers -> whole-array transfer would xN-double-count -> fail-loud TransferInjectError::CumulativeWholeArrayFallback { data, src, message }. (B) partition_ranges EMPTY: no partition, whole-array tile is correct -> keep silently. Corrected the call-site comment lie + the enum variant doc.

SUBTLETIES for future cycles: (1) recursion threading — Sequence arm uses .collect::<Result<Vec<_>,_>>()?, Repeat boxes the recursive ?, leaf arms wrap in Ok(). Entry call site at ~834 uses ? (enclosing inject_transfers already returns Result). (2) forcing case A None in the unit test: give data_dim_iv_map the right dim count (avoid the per_dim.len()!=dims.len() early-None) but partition on a DECOY iv that does not index the data, so saw_band stays false. (3) case B test uses EMPTY partition_ranges and asserts Ok(Xfer) with the tile UNCHANGED — this pins the discriminator (without it a future edit could silently regress to rejecting game-of-life). (4) no clippy quirks; build/clippy clean first try after the A/B fix.

GATE (actual): just build OK; just clippy OK (-D warnings clean); just test 1141 passed/0 failed/3 ignored (dev); just test-release 1140 passed/0 failed/3 ignored (the -1 is the known dev-only debug_assert-gated should_panic divergence, TASK-0291); just e2e total: 322 pass: 265 fail: 0 skipped: 57 required-fail: 0 — reproduced 2x.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE cycle-214 (commit b605490). All 3 ACs met. AC#1: rewrite_cumulative_band_tiles made fallible; None-on-cumulative now raises TransferInjectError::CumulativeWholeArrayFallback { data, src, message } — but ONLY when partition_ranges is non-empty (the genuine xN-double-count shape); the unpartitioned game-of-life shape keeps the correct whole-array tile. AC#2: two in-file unit tests (case A error, case B no-error). AC#3: e2e 322/265/0/57/0 unchanged, reproduced 2x. NOTE: the brief premise (branch fully dead) was wrong — game-of-life/pipelined reaches the None branch via its unpartitioned async grid transfer; the fix discriminates partitioned-replicated (defect) from unpartitioned (correct) rather than rejecting unconditionally. Independent orchestrator review gate expected to re-verify.
<!-- SECTION:FINAL_SUMMARY:END -->
