---
id: TASK-0043
title: M5 — Distributed schedule + reuse
status: To Do
assignee: []
created_date: '2026-05-17 23:08'
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
- [ ] #1 partition=rows works on a 1D outer loop; partition=blocks2d works on a 2D nest.
- [ ] #2 Halo regions are inferred from kernel access pattern at compile time; the schedule does not state halo size.
- [ ] #3 reuse loop option produces delay-line / circular-buffer code for affine-stride accesses.
- [ ] #4 Test: M5 differential matrix is green on the new distributed schedules for examples 5, 6, 7.
- [ ] #5 Implementation notes record design questions (e.g. when reuse pattern is too irregular to handle; should it error or fall back to no-reuse).
- [ ] #6 Implementation notes record honest limitations (only affine; data-dependent strides rejected).
<!-- AC:END -->
