---
id: TASK-0138
title: 'petri_to_events: preserve per-iteration tile on Fire events'
status: Done
assignee: []
created_date: '2026-05-18 03:51'
updated_date: '2026-05-23 21:25'
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

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-no-current-driver (orchestrator-direct, cycle 77 sweep). Investigation: grep across all 4 tier-1 backends (pthreads-sync, mp-tcp-bufsync, pthreads-async, mp-tcp-event) + the shared backend-common walker shows every Event::Fire destructure pattern is 'Event::Fire { kernel, bindings, .. }' — the  field is universally ignored. FireBinding (TASK-0156, cycle ~37) is the per-firing value carrier the backends actually consume; per-iteration coordinates land in FireBinding.{inputs, output}, NOT in Fire.tile. The petri_to_events.rs:398 'tile: IterTile::empty()' is therefore documentation-grade-correct: empty IS the truthful tile for a Fire whose per-iter info lives in FireBinding. The PRD §8.3 framing pre-dates TASK-0156's FireBinding design. Reopen if/when a future backend genuinely needs Event::Fire.tile (e.g. a backend that consumes Fire events outside the FireBinding path, perhaps for vectorisation hints or a different IR layering). Until then, the Open-Q in the task description ('per-iter vs multi-iter tile') has no driver to ground the decision. Same deferred-no-driver pattern as the cycle-77 sweep.
<!-- SECTION:FINAL_SUMMARY:END -->
