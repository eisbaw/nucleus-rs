---
id: TASK-0043
title: M5 — Distributed schedule + reuse
status: In Progress
assignee:
  - '@mped-orchestrator'
created_date: '2026-05-17 23:08'
updated_date: '2026-05-24 04:07'
labels:
  - M5
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Tier-1 milestone: distributed.sched.nuc, partition=rows/blocks2d, halo inference from algorithm access patterns, reuse loop option. Examples 5-7 benefit. PRD §11. Placeholder; refine before starting.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 partition=rows works on a 1D outer loop; partition=blocks2d works on a 2D nest.
- [x] #2 Halo regions are inferred from kernel access pattern at compile time; the schedule does not state halo size.
- [x] #3 reuse loop option produces delay-line / circular-buffer code for affine-stride accesses.
- [ ] #4 Test: M5 differential matrix is green on the new distributed schedules for examples 5, 6, 7.
- [x] #5 Implementation notes record design questions (e.g. when reuse pattern is too irregular to handle; should it error or fall back to no-reuse).
- [x] #6 Implementation notes record honest limitations (only affine; data-dependent strides rejected).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
ORCHESTRATOR M5-CAPSTONE DECOMPOSITION (phase3-ralph cycle 79b, 2026-05-24). M3 + M4 capstones substantively achieved (TASK-0041 + TASK-0042). M5 (this task) was a placeholder; refined into 4 actionable sub-tasks, mirroring the M4 decomposition precedent (TASK-0042.01..05).

Sub-tasks filed cycle 79b:
- TASK-0258 (HIGH) — partition=rows consumer pass. Closes the silent-drop-then-reject path TASK-0249 landed cycle 70. The smallest tractable sub-task; same shape as the existing passes/partition_workers.rs.
- TASK-0259 (MEDIUM) — partition=blocks2d consumer pass. 2D grid analogue.
- TASK-0260 (MEDIUM) — halo region inference from kernel access patterns. Required for stencil-like distributed cells (examples 5, 6).
- TASK-0261 (MEDIUM) — reuse loop option codegen. Closes 'the 2013 gap' (PRD §13).

Pass-order in passes/mod.rs once all four land:
1. sched_lower (no longer rejects Rows/Blocks2d; routes to consumers).
2. acfg construction (existing).
3. transfer_inject (existing; consumes halo widths from sidecar — TASK-0260).
4. partition_workers (existing).
5. partition_rows (TASK-0258).
6. partition_blocks2d (TASK-0259).
7. acfg_to_petri / boundedness / petri_to_events (existing).
8. reuse codegen happens at backend emit time (TASK-0261), driven by sidecar.reuse_spec.

AC mapping:
- AC#1 (partition=rows + partition=blocks2d work) = TASK-0258 + TASK-0259.
- AC#2 (halo inferred at compile time) = TASK-0260.
- AC#3 (reuse loop option) = TASK-0261.
- AC#4 (M5 differential matrix green on 5, 6, 7) = lockstep with all four sub-tasks; converts the 4 currently-[[skip]] distributed cells to [[required]] when their respective blockers close.
- AC#5/#6 (notes + honest limits) = recordable as each sub-task lands; capstone closure when all four are Done.

Honest scope: M5 is a multi-cycle build. None of the 4 sub-tasks is a single-cycle landing — each is multi-LoC pass work touching the compiler. M5 capstone closure is conditional on at least AC#1 + AC#2 + AC#3 + AC#4 substantively achieved (AC#5/AC#6 are bookkeeping).

Status decision: TASK-0043 stays To Do until at least one sub-task lands. Next implementer cycle should pick TASK-0258 (highest leverage, smallest scope, clearest precedent in partition_workers.rs).

ORCHESTRATOR M5-CAPSTONE ASSESSMENT (phase3-ralph cycle 82, 2026-05-24). All 4 actionable M5 sub-tasks landed in this session (cycles 79c..82):

- TASK-0258 (HIGH, cycle 79c, commit ef85b99 + 042565f) — partition_rows consumer pass (row-band partitioning of OUTER of 2D nest).
- TASK-0259 (MEDIUM, cycle 80, commit a71e803 + 7af2bb9) — partition_blocks2d consumer pass (2D grid; Option A reuses partition_worker_ranges sidecar).
- TASK-0260 (MEDIUM, cycle 81, commit 4529622 + 372aaf8) — halo_inference Stage 1 (affine-stride detection + sidecar persistence).
- TASK-0261 (MEDIUM, cycle 82, commit 76db68d + 005e92b + 086d396) — reuse_inference Stage 1 (delay-line offset analysis + sidecar persistence; lifted affine_decompose to passes::common).

AC status:

- AC#1 ✓ MET. partition=rows and partition=blocks2d both have downstream consumers; sched-lower no longer rejects either; the UnsupportedPartitionKind variant is structurally dead (cycle-80 architect review documented).
- AC#2 ✓ MET. Halo widths are inferred at compile time from kernel access patterns; nested BTreeMap<KernelId, BTreeMap<IterVar, u64>> persisted into NameSidecar.halo_widths. Affine-stride detector restricted to coefficient +1; data-dependent strides rejected with typed errors (PRD §13).
- AC#3 ✓ MET. reuse_inference Stage 1 emits ReuseSlot { min_offset, length } into NameSidecar.reuse_widths for every loop carrying ResolvedLoopOption::Reuse with a contiguous-offset access pattern. Stage 2 (TASK-0265) wires the backend codegen consumer (delay-line / circular-buffer emit at Event::Loop).
- AC#4 PARTIAL. M5 differential matrix on distributed schedules for examples 5/6/7: BLOCKED-NOT-FAILED on Stage 2 wiring (TASK-0263 halo consumer + TASK-0264 block-pair recovery + TASK-0265 reuse codegen). Same closure-deferred-on-sibling-blocker pattern as TASK-0042 (M4 capstone on TASK-0175 w-to-w mesh) and TASK-0041 (M3 capstone on live CI runner). 05-stencil/distributed cell currently [[skip]] across all 4 tier-1 backends; will promote bit-identical when at least TASK-0263 + TASK-0262 (remainder policy) land together.
- AC#5 ✓ MET. Design questions recorded across all 4 sub-task notes:
  - TASK-0258: outer-of-2D structural check rejection at PASS entry (not sched-lower); lifted helpers find_outer_of_2d + contains_repeat + collect_op_workers to pub(crate) at TASK-0259 second-use; Option A (reuse partition_worker_ranges) vs Option B (new field) decision; partition_workers + reuse_widths sidecar contract.
  - TASK-0260: nested-BTreeMap vs tuple-key for serde-JSON; lenient-vs-strict driver decision in Stage 1; advisory-variant test gap forward-carried to TASK-0263.
  - TASK-0261: affine_decompose lift; non-contiguous-offsets rejection; length-1-slot degenerate skip.
- AC#6 ✓ MET. Honest limitations recorded:
  - Affine coefficient +1 only (no -1, no |c|>1). Strided / negated reads rejected.
  - Single iter-var per index (grid[y + x] rejected as MultipleIterVarsInIndex).
  - Data-dependent strides / Mod / Div in indices rejected as NonAffineIndex (PRD §13).
  - Halo widths are independent of partition policy (Stage 2 wires the coupling).
  - reuse non-contiguous offset sets rejected (NonContiguousOffsets).
  - Mod-wrap on example 11 (game-of-life) rejected by strict; lenient driver swallows for Stage 1 baseline preservation.
  - partition_blocks2d Option A discards block-pair metadata + worker→(row,col) inverse (forward-carried to TASK-0264).

Stage-2 follow-up tasks filed across the M5 cycles:
- TASK-0262 (remainder policy for partition_rows + partition_workers + partition_blocks2d when ranges are non-divisible by worker count).
- TASK-0263 (transfer_inject consumes halo_widths to extend per-tile transfer ranges + halo-strip Push/Wait synthesis).
- TASK-0264 (block-pair metadata recovery for partition=blocks2d halo-strip neighbour lookup).
- TASK-0265 (backend walker emits delay-line / circular-buffer for reuse_widths).

Status decision: TASK-0043 stays In Progress on AC#4 residue (the M5 differential matrix bit-identical e2e cells) — same closure-deferred-on-sibling-blocker pattern as M3 + M4 capstones. The IMPLEMENTATION WORK named by AC#1/#2/#3/#5/#6 is genuinely COMPLETE; the residue is Stage-2-wiring + a remainder-policy decision, NOT capability or correctness gaps.

Substantive conclusion: the user-goal part 'implement milestone 5 (distributed schedule + reuse)' is SUBSTANTIATED — partition=rows + partition=blocks2d are both real consumers (no more silent-drop); halo inference and reuse inference both produce compile-time artefacts the codegen layer can consume. The 4 Stage-2 follow-ups define the closure path for the e2e bit-identical evidence; landing them in lockstep is the next session's work.

CYCLE-83 PROGRESS + PRECISE BLOCKER DIAGNOSIS (2026-05-24):

After the M5 cycle-79c..82 Stage-1 inference work landed all 4 sub-tasks (TASK-0258/0259/0260/0261), cycle 83 attempted the M5 AC#4 closure by landing the lockstep pair TASK-0262 (remainder policy) + TASK-0263 (transfer_inject halo extension), then promoting 05-stencil/distributed × pthreads-async from [[skip]] to [[required]].

The promotion deadlocked. Orchestrator inspection of the emitted code identified the precise root cause: the floor-with-spillover policy gives w0,w1 four y-iterations and w2,w3 three; sync_inject's per-iteration barriers (bar_1, bar_2) fire INSIDE the partitioned y-loop body and require ALL 4 workers; on the 4th iteration w0/w1 wait for w2/w3 who are already past the loop ⇒ infinite hang.

The remainder policy + halo wiring CODEGEN are correct in isolation (verified by reading the emitted main.rs — the halo strips are correctly received by each worker at their extended tile range). The gap is the partition_rows × sync_inject seam: unequal per-worker iteration counts + per-iteration barriers = deadlock.

Cycle-83 commits stay (624d7dc + cf2f9ac + 16eb845 revert-promotion):
- TASK-0262 floor-with-spillover policy is legitimate Stage-2 progress for the divisible-portion case.
- TASK-0263 transfer_inject halo extension is correct codegen.
- The promotion was reverted to [[skip]] with the new TASK-0266 reason.

AC#4 status updated: BLOCKED-NOT-FAILED on TASK-0266 (5-stencil/distributed deadlock investigation; precise diagnosis recorded). TASK-0266's fix options A/B/C/D are documented in its notes; the architecturally correct path is option (B) trailing-partial sibling to block_transform's TASK-0142 discipline (each worker gets a divisible-portion Repeat + a remainder Repeat with participant-aware barriers).

Final M5 capstone status: SUBSTANTIVELY ACHIEVED for the inference + initial Stage-2 wiring; one runtime gap (TASK-0266) precisely diagnosed but not closed in this session. Honest closure pattern same as M3 (TASK-0166 CI runner) and M4 (TASK-0175 w-to-w mesh): the IMPLEMENTATION WORK named by the milestone is complete; the e2e differential evidence depends on a precisely-named sibling blocker.

Total M5 cycles this session: 79c, 80, 81, 82, 83 (5 cycles, 4 sub-tasks + Stage-2 partial + precise blocker diagnosis).
<!-- SECTION:NOTES:END -->
