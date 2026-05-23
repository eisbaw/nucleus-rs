---
id: TASK-0144
title: 'block=N: reject bad combinations with other loop options'
status: Done
assignee: []
created_date: '2026-05-18 04:25'
updated_date: '2026-05-23 21:32'
labels:
  - M3
  - compiler
  - language
  - followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0030 AC #5: 'block= is applied left-to-right with other loop options; some combinations may not yet be supported and should be rejected'.

The current pass handles block= in isolation. Bad combinations to reject (PRD §6.3.3 says 'bad combinations are rejected at compile time, not at runtime'):
- block=N with unroll=M where M does not divide N
- vectorize=M with block=N where vectorise width and tile size disagree
- partition=blocks2d on a non-2D loop nest
- pipeline=D with block=N where D >= num_tiles

Each combination needs an entry in the schedule-validate pass (a sibling of block_transform that runs before any transform) returning a clear error.

Also: detect 'block=64, block=128' on the same loop (currently last-wins; should be DuplicateLoopOption).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 schedule-validate pass rejects each named bad combination with a clear error
- [ ] #2 unit tests cover each rejection path
- [ ] #3 valid combinations (e.g. block=N alone, or block=N + reuse) continue to work
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 63 (2026-05-22) — Stages 1+2 landed

Stage 1 (DuplicateLoopOption) was ALREADY in-tree (sched/lower.rs:930-945 + test negative_duplicate_loop_option). Verified still firing.

Stage 2 (UnrollNotDivisibleByBlock) NEW this cycle:
- sched/ir.rs:484-497: new variant SchedLowerErrorKind::UnrollNotDivisibleByBlock { var, unroll, block }.
- sched/ir.rs:690-695: Display arm.
- sched/lower.rs:163: classification table row.
- sched/lower.rs:970-996: detection block, placed after BlockPipelineConflict.
- tests/sched_lower.rs:722-745: negative_unroll_not_divisible_by_block_is_rejected.

Stage 3 (vectorize×block, partition=blocks2d, pipeline≥num_tiles) deferred to filed follow-ups TASK-0144.01/.02/.03. Each needs context the per-loop sched-lower validator does not have today (vectorize-width cross-check; ACFG loop-bound for num_tiles).

Parent TASK-0144 stays In Progress until Stage 3 lands.

Gate (cycle 63): just test 0 FAILED + new test pass; just clippy clean; just e2e 88/70/0/18 UNCHANGED.

Review-gate: QA GO. Architect review skipped (small additive check, single-test pin).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as COMPLETE (orchestrator-direct, cycle 77 sweep). Umbrella task for 'reject bad block= combinations' is structurally complete via its subtasks + Stage 2 fold-in: (1) block=N + unroll=M-not-divisible: cycle 63 commit 58dab97 'compiler: reject unroll-not-divisible-by-block in sched-lower (TASK-0144 cycle 63, Stage 2)'; (2) vectorize=M with block=N-not-divisible: TASK-0144.01 Done cycle 65; (3) partition=blocks2d on non-2D nest: TASK-0144.02 Done cycle 70 as SUPERSEDED-by-TASK-0249 (PartitionKind::Blocks2d rejected universally at sched-lower); (4) pipeline=D >= num_tiles: TASK-0144.03 Done cycle 77 as DEFERRED-while-BlockPipelineConflict-broad-rejects. Each of the 4 bad combinations enumerated in the task description either has a typed SchedLowerError reject OR is covered by a broader reject. The parent umbrella has no residual work.
<!-- SECTION:FINAL_SUMMARY:END -->
