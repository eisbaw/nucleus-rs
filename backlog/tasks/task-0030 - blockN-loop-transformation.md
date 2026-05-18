---
id: TASK-0030
title: block=N loop transformation
status: To Do
assignee: []
created_date: '2026-05-17 23:06'
labels:
  - M2
  - compiler
  - language
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Implement block=N in the schedule's loop transforms. Outer loop iterates over tiles of N; inner loop iterates within a tile. Transfer events happen at tile granularity.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Schedule directive 'loop var : block=N' rewrites the iteration tree to a (tile-loop, intra-tile-loop) nest.
- [ ] #2 Transfer event sizes (IterTile bounds) align with the tile, not per-point.
- [ ] #3 Test: example 5 (stencil) compiled with block=64 produces an EventList where each Push covers a 64-row band.
- [ ] #4 Implementation notes record design questions (e.g. handling of trailing remainder when iteration size is not a multiple of N).
- [ ] #5 Implementation notes record honest limitations (block= is applied left-to-right with other loop options; some combinations may not yet be supported and should be rejected).
<!-- AC:END -->
