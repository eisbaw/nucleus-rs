---
id: TASK-0027
title: Project Petri net to per-worker EventList
status: To Do
assignee: []
created_date: '2026-05-17 23:05'
labels:
  - M2
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Linearise the net's firing order and project transitions onto the worker that owns them, producing the per-worker EventList that backends consume.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 compiler exposes project(Net, SchedIR) -> Map<WorkerId, EventList>.
- [ ] #2 Linearisation is deterministic (same input = same output, byte-for-byte). Uses source order + dataflow constraints to break ties.
- [ ] #3 Each worker's EventList respects intra-worker data dependencies.
- [ ] #4 Inter-worker push/wait pairs have matching SeqTags.
- [ ] #5 Test: round-trip from example through to EventList, snapshot-tested.
- [ ] #6 Implementation notes record design questions (e.g. is greedy linearisation good enough for stencil-shaped schedules).
- [ ] #7 Implementation notes record honest limitations (no schedule optimisation; the linearisation is correct but not optimal).
<!-- AC:END -->
