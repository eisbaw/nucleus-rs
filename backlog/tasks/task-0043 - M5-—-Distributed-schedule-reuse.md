---
id: TASK-0043
title: M5 — Distributed schedule + reuse
status: Done
assignee:
  - '@mped-orchestrator'
created_date: '2026-05-17 23:08'
updated_date: '2026-05-25 02:34'
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
- [x] #4 Test: M5 differential matrix is green on the new distributed schedules for examples 5, 6, 7.
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

ORCHESTRATOR M5 AC#4 CLOSEOUT VERIFIED (cycle 119, 2026-05-25).

Independent re-run of `just e2e` (NOT a transcribed implementer claim):
- total: 104, pass: 88, fail: 0, skipped: 16, required-fail: 0
- 06-separable-filter/distributed × {pthreads-sync, mp-tcp-bufsync, pthreads-async, mp-tcp-event}: ALL PASS (cycle 116 close, TASK-0296)
- 07-matmul/distributed × {pthreads-sync, mp-tcp-bufsync, pthreads-async, mp-tcp-event}: ALL PASS (cycle 118 close, TASK-0297 unblocked by TASK-0301)
- 05-stencil/distributed × {pthreads-async, mp-tcp-event} + distributed-2d × pthreads-async: PASS (required-fail=0 confirms)
- 05-stencil/reuse × {pthreads-sync, mp-tcp-bufsync, pthreads-async, mp-tcp-event}: ALL PASS (Stage 2 carry)

M5 AC#4 (examples 5-7 differential matrix green on distributed) is now genuinely closed across all 4 tier-1 backends. AC#1, AC#2, AC#3, AC#5, AC#6 already ticked in prior cycles.

HONEST LIMITS preserved as separate tracker tasks:
- TASK-0302: 2D iv->dim mapping for partition=blocks2d under matmul shape (axis-mapping latent extension; no shipped schedule constructs the shape today)
- TASK-0298: 06-separable-filter pass-2 distributed (tmp broadcast) — LOW priority icing
- TASK-0295: sibling-promotion audit for 2D row-loop slice-paste on pthreads-sync/mp-tcp-bufsync/mp-tcp-event under partition=blocks2d (LOW, blocked on capability unlock)

This was the final M5 AC. Milestone is feature-complete; the M5 task itself can move to Done after this AC checkpoint.

Backlog now entering the maturity / endgame phase per phase3-backlog-ralph: feature To-Do is essentially empty; remaining work is LOW-priority hardening + future-milestone (M6+) scaffolding. Cycle 119 also lands TASK-0299 (halo_widths pinning test for 06-separable-filter) as the first hardening item — defends against the feedback-comment-doc-lie-recurring pattern.
<!-- SECTION:NOTES:END -->
