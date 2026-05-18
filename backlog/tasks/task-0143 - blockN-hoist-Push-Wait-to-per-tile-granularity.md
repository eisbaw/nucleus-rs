---
id: TASK-0143
title: 'block=N: hoist Push/Wait to per-tile granularity'
status: To Do
assignee: []
created_date: '2026-05-18 04:24'
labels:
  - M3
  - compiler
  - language
  - followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §6.3.3 says: 'block=N -- Tile iteration into chunks of N; transfers happen per tile'. The structural transform (TASK-0030) creates the two-level nest (outer tile, inner intra-tile), but the existing transfer_inject pass still injects Push/Wait inside the inner loop -- one transfer per intra-tile iteration, not per tile.

To get true per-tile transfers:
- transfer_inject should detect when a Push/Wait would be loop-invariant w.r.t. the inner loop (data shape, access pattern, producer worker all constant across intra-tile iterations) and hoist it to the outer (tile) body.
- IterTile on the hoisted Xfer needs to project onto the tile coordinate, not the intra-tile coordinate.

For example 05 (stencil) with block=64 on y, hoisting Push for img_in to the y_outer loop would yield 'each Push covers a 64-row band' (TASK-0030 AC #3).

This is the per-tile transfer optimisation that TASK-0030 deferred.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 transfer_inject detects loop-invariant Xfers in an inner block-tile loop and hoists them to the outer tile loop
- [ ] #2 IterTile bounds on hoisted Xfers cover the full tile, not a single intra-tile iteration
- [ ] #3 example 05 + block=64 produces an EventList where each Push covers a 64-row band (TASK-0030 AC #3)
- [ ] #4 examples without block= unchanged
<!-- AC:END -->
