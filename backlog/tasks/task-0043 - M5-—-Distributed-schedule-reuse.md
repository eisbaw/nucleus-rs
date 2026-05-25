---
id: TASK-0043
title: M5 — Distributed schedule + reuse
status: In Progress
assignee:
  - '@mped-orchestrator'
created_date: '2026-05-17 23:08'
updated_date: '2026-05-25 00:50'
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
ORCHESTRATOR M5 AC#4 CLOSEOUT FILED (cycle 116, 2026-05-25, post-cycle-115).
Discovered gap during phase3-ralph cycle 116 task selection: M5 AC#4 says "examples 5-7 benefit measurably" but only example 5 ships distributed/distributed-2d/reuse schedules. Examples 06-separable-filter and 07-matmul ship only naive + blocked. Filed two M5-extension tasks:
- TASK-0296 (MEDIUM): 06-separable-filter/distributed schedule + e2e cells. Implementer to decide pass-1-only vs both-passes distributed based on halo inference behaviour on the rectangular-accumulator pattern.
- TASK-0297 (MEDIUM): 07-matmul/distributed schedule + e2e cells using partition=blocks2d. Inherits TASK-0294 cycle 115 2D slice-paste machinery on the output side.
TASK-0297 depends on TASK-0296 (validate broadcast-not-halo machinery on simpler 1D case first).
M5 AC#4 remains unchecked until both land.
<!-- SECTION:NOTES:END -->
