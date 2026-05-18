---
id: TASK-0025
title: Petri-net IR data structures
status: To Do
assignee: []
created_date: '2026-05-17 23:05'
labels:
  - M2
  - ir
  - compiler
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Implement the place/transition/arc/marking types per PRD §8. Place has capacity; arcs are weighted; net is acyclic for v2.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 petri-net crate (or module under compiler) exposes Place, Transition, Arc, Marking types.
- [ ] #2 Place has capacity (Option<NonZeroU32>; None = unbounded for analysis cases, but production v2 always has Some).
- [ ] #3 Net struct supports: add_place, add_transition, add_arc, initial_marking, fire(transition_id) -> Result<NewMarking, FireError>.
- [ ] #4 Test: classic Petri-net examples (producer/consumer, dining philosophers — small) execute as expected via the firing simulator.
- [ ] #5 Implementation notes record design questions (e.g. graph-library vs hand-rolled vec/index storage).
- [ ] #6 Implementation notes record honest limitations (no coloured nets, no hierarchical refinement, no timed nets; consistent with PRD §8.4 budget).
<!-- AC:END -->
