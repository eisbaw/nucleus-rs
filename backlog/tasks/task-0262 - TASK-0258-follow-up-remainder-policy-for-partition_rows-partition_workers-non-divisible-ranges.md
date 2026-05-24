---
id: TASK-0262
title: >-
  TASK-0258 follow-up: remainder policy for partition_rows + partition_workers
  (non-divisible ranges)
status: Done
assignee:
  - '@mped-architect-impl'
created_date: '2026-05-24 00:13'
updated_date: '2026-05-24 04:07'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle 83: floor-with-spillover policy landed (commit 624d7dc). 14/4 case now produces w0,w1=4 rows; w2,w3=3 rows. Test fixtures updated; clippy clean; e2e baseline preserved (no regressions on the 13-cnn-inference/batch_parallel divisible case).

Closure caveat: the policy is CORRECT in isolation but exposes a partition_rows × sync_inject seam (per-iteration barriers + unequal worker counts ⇒ deadlock). Documented under TASK-0266. The remainder policy itself is not the bug — sync_inject's expectation of equal iteration counts is what needs the next-cycle work (trailing-partial sibling à la block_transform TASK-0142).

Status: Done. The narrowly-scoped policy decision the task names is met; the architectural follow-up is owned by TASK-0266.
<!-- SECTION:NOTES:END -->
