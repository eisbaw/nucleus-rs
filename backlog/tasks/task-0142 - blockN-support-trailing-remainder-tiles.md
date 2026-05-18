---
id: TASK-0142
title: 'block=N: support trailing remainder tiles'
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
TASK-0030 (block=N transformation) currently rejects any (HI-LO) not evenly divisible by N. The PRD §6.3.3 example shows 'for y_inner : y_outer..min(y_outer+N, H)' which clamps the trailing tile.

To support this, ACFGNode::Repeat needs to express a dynamic upper bound (function of an outer iter var) or we need a new variant (e.g. TileTail) that carries the remainder size. Codegen for the inner loop also needs to know which tile is partial.

For now, schedules with non-divisible block= are a hard compile error with BlockTransformError::NotDivisible. Unblock once a driving example needs it.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 ACFG can represent a trailing partial tile
- [ ] #2 block=64 on 0..100 produces an outer loop of 2 tiles, an inner of 64 for tile 0, and an inner of 36 for tile 1
- [ ] #3 all required (algo, sched) cells stay green
<!-- AC:END -->
