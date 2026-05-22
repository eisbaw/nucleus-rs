---
id: TASK-0144
title: 'block=N: reject bad combinations with other loop options'
status: To Do
assignee: []
created_date: '2026-05-18 04:25'
updated_date: '2026-05-22 21:35'
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
