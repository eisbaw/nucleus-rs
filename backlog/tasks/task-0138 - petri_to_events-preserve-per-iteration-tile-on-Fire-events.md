---
id: TASK-0138
title: 'petri_to_events: preserve per-iteration tile on Fire events'
status: To Do
assignee: []
created_date: '2026-05-18 03:51'
labels:
  - compiler
  - M3
  - ir
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0027 (and acfg_to_petri before it) unrolls Repeat bodies but drops the per-iteration coordinate. Every Fire ends up with tile=IterTile::empty(). PRD §8.3 specifies that Fire carries the iteration coordinates the firing covers; we are not currently honouring that.

Fix in two steps:
1. `acfg_to_petri` retains the current iteration coordinate when unrolling (most ergonomic: thread a Vec<(IterVar, i64)> through the walker and tag each emitted Transition with a per-iter tile).
2. `petri_to_events` reads that tile through onto every emitted Fire.

Open Q: do we keep one Fire per iteration (current direction) or fold into one Fire with a multi-iter tile (`bounds=[(i, 0..N)]`)? The per-iter form is more honest to "one event = one firing" semantics; the multi-iter form is what a real backend wants to vectorise. Pick at design time, justify in the task notes.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A Repeat over range a..b emits Fire events whose tile names the iteration coordinate i = a, a+1, ..., b-1 (rather than IterTile::empty()).
- [ ] #2 acfg_to_petri preserves the iter-var/range info on each unrolled transition (e.g. via Transition.label or a sidecar map).
- [ ] #3 Tests assert tile = [(i_var, k..k+1)] for the k-th Fire in a unrolled loop.
<!-- AC:END -->
