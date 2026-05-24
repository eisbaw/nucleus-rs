---
id: TASK-0262
title: >-
  TASK-0258 follow-up: remainder policy for partition_rows + partition_workers
  (non-divisible ranges)
status: In Progress
assignee:
  - '@mped-architect-impl'
created_date: '2026-05-24 00:13'
updated_date: '2026-05-24 02:46'
labels:
  - compiler
  - partition
  - follow-up
  - M5
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0258 (cycle 79c) landed the partition_rows consumer pass for the OUTER loop of a 2D nest. The first-cut policy — shared with partition_workers (TASK-0212) — REFUSES TO COMPILE when the loop range is not evenly divisible by the worker count, reporting PartitionRowsError::NonDivisible / PartitionError::NonDivisible.

Surfaced in 05-stencil/distributed: the algo y-loop walks 1..H-1 = 1..15 (length 14). 14 / 4 = 3 remainder 2. The four-worker distributed schedule cannot carry `loop y : partition=rows;` today; the directive is held back in commented form in the schedule file.

## What's needed

A trailing-partial / last-worker-gets-remainder policy that mirrors block_transform's discipline (TASK-0142 / TASK-0218). Three workable shapes:

(a) Last-worker-gets-remainder: w0..w[N-2] each get `(length / N)` rows; w[N-1] gets `(length / N) + (length % N)`. Simple but unbalanced.
(b) Floor-with-spillover: first `(length % N)` workers get one extra row each. More balanced; mirrors numpy.array_split.
(c) Trailing-partial-tile sibling: introduce a synthetic 'remainder repeat' under the last worker. Matches block_transform's trailing-partial pattern (TASK-0142).

PRD §6.3.3 doesn't pin a policy; the schedule grammar carries no hint. Decision needs an explicit recording.

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Policy decision recorded (a / b / c, with rationale).
- [ ] #2 partition_workers and partition_rows BOTH adopt the chosen policy (consistency — both passes write into the same sidecar field and downstream consumers don't distinguish).
- [ ] #3 Existing tests in partition_workers.rs / partition_rows.rs that expect NonDivisible flip to assert the new shape.
- [ ] #4 05-stencil/distributed restores the `loop y : partition=rows;` directive (uncomment from the schedule file).
- [ ] #5 Sched-lower / parser / link tests pin the restored directive (loop count goes back to 2; y-loop carries Partition(Rows)).
- [ ] #6 Bit-identical preserved for any required e2e cell that exercises partition_workers (today: 13-cnn-inference/batch_parallel × 4 backends).

## Dependencies
- TASK-0258 (partition_rows consumer) — landed.
- TASK-0212 (partition_workers consumer) — landed.

## Out of scope
- Halo inference (TASK-0260 — sibling).
- partition_blocks2d (TASK-0259 — sibling).
<!-- SECTION:DESCRIPTION:END -->
<!-- AC:END -->
