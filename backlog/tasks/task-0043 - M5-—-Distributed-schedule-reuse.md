---
id: TASK-0043
title: M5 — Distributed schedule + reuse
status: To Do
assignee: []
created_date: '2026-05-17 23:08'
updated_date: '2026-05-23 23:54'
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
<!-- SECTION:NOTES:END -->
