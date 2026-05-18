---
id: TASK-0137
title: 'petri_to_events: emit Alloc/Free events and resolve Region'
status: To Do
assignee: []
created_date: '2026-05-18 03:50'
labels:
  - compiler
  - M3
  - ir
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0027 elided Event::Alloc / Event::Free for M2: the pthreads-sync backend uses on-stack allocation and has no need. PRD §8.3 specifies these events and the Region tag; the schedule sublanguage (PRD §6.3.1) has `place_data D in MEMORY_REGION`, but that surface is not yet threaded through the link/ACFG passes either.

This follow-up does:
1. Plumb `place_data` directives through SchedIR -> LinkedIR -> ACFG (and ultimately into the per-data Region assignment).
2. Augment `petri_to_events` to scan each worker's EventList for first-use/last-use of each data symbol and inject Alloc/Free at those positions.

Open design Q: do Alloc/Free live in the ACFG (so multiple passes can see them) or do they get synthesised purely in the projection step? The PRD's §5 pipeline diagram suggests the former; the projection-only form is simpler for M2.5.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Each data symbol has an Alloc emitted on first use per worker, and a Free emitted after last use.
- [ ] #2 Region is derived from schedule directives (place_data D in MEMORY_REGION); when absent, the backend's default region is used.
- [ ] #3 Tests cover the Alloc/Free positions in a multi-worker schedule with explicit place_data.
<!-- AC:END -->
