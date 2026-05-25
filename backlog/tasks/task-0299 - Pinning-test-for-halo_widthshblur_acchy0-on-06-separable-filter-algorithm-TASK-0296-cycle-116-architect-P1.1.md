---
id: TASK-0299
title: >-
  Pinning test for halo_widths[hblur_acc][hy]=0 on 06-separable-filter algorithm
  (TASK-0296 cycle-116 architect P1.1)
status: To Do
assignee: []
created_date: '2026-05-25 01:18'
labels:
  - M5
  - compiler
  - test-coverage
  - halo_inference
  - 06-separable-filter
  - forward-carried-from-TASK-0296
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background
TASK-0296 cycle 116 added 06-separable-filter/distributed.sched.nuc with partition=rows on hy. The schedule header asserts (lines ~19-21): "halo inference produces `halo_widths[hblur_acc][hy] = 0` and transfer_inject does NOT extend per-tile transfer ranges." This is CORRECT today by inspection of the algorithm: `hblur_acc(tmp[hy][hx], in_arr[hy][hk], hx, hk)` — `hy` axis is accessed at offset 0 only.

## Risk
The claim is a comment-doc-lie waiting to happen: if a future kernel-surface change introduces a non-zero hy offset (e.g. `in_arr[hy-1][hk]` for a vertical-blur fold), the comment stays stale and the schedule silently mis-claims halo behaviour. The e2e cell would catch wrong output, but the *narrative* in the comment would lie.

## Acceptance criteria
1. Add a test (in nucleus-compiler/tests/halo_inference.rs or nearest sibling) that loads 06-separable-filter/prog.algo.nuc + a partition=rows schedule, runs halo inference, and asserts `halo_widths[hblur_acc][hy] == 0` (and equivalently for vblur_acc[vy]). Pin the claim by structural test.
2. If the algorithm ever changes such that this assertion no longer holds, the schedule comment must be updated in the same commit — the test forces the change to be intentional.

## Honest scope
- LOW priority — defends against a future class of bug, not a current one. The current cell is bit-identical correct.

## Cross-references
- `nuc-nucleus/examples/06-separable-filter/schedules/distributed.sched.nuc:19-21` — the load-bearing comment.
- `nuc-nucleus/examples/06-separable-filter/prog.algo.nuc:100` — the access patterns asserted.
- `nucleus/nucleus-compiler/src/passes/halo_inference.rs` — the inference pass.
<!-- SECTION:DESCRIPTION:END -->
