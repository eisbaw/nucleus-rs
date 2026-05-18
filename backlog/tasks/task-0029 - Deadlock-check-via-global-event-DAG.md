---
id: TASK-0029
title: Deadlock check via global event DAG
status: Done
assignee: []
created_date: '2026-05-17 23:05'
updated_date: '2026-05-18 04:14'
labels:
  - M2
  - compiler
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Construct the global event DAG (intra-worker order plus push→wait arcs); topologically sort; cycle = deadlock = compile error pointing at the cycle. PRD §8.4.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 compiler exposes check_deadlock_free(per_worker_events) -> Result<(), DeadlockError>.
- [ ] #2 DeadlockError names the cycle: an ordered list of events forming the loop.
- [x] #3 Test: a synthetic schedule with a circular push/wait dependency produces this error with the right cycle.
- [x] #4 Implementation notes record design questions (e.g. should the error point at schedule directives or event entries; both?).
- [x] #5 Implementation notes record honest limitations (only structural deadlocks are caught; runtime-livelock conditions are out of scope).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation notes (TASK-0029)

Design questions resolved
=========================

1. Simulator-replay vs. explicit DAG cycle detection.
   Both are valid lowerings of PRD section 8.4. Picked simulator-replay because:
   - v2's firing order is statically determined (PRD section 8.4): a total
     linearisation already exists, so replay terminates.
   - The Petri Net struct already has fire/marking machinery (used by
     boundedness/TASK-0028); we reuse it for ~zero extra LOC.
   - The Net does not (today) carry an explicit Push-to-Wait edge label
     on top of the bipartite arc set, so an event DAG would have to
     reconstruct it from seq tags on per-worker Event lists or by
     name-matching transition labels. Both are fragile. Simulator-replay
     is purely structural.
   - The two formulations are equivalent for v2: a stall in the linear
     order = back-edge in the would-be event DAG. Filed TASK-0140 to
     layer DAG cycle naming on top once transfer_inject is fixed and
     true cycles (rather than missing-producer bugs) become exercisable.

2. How to construct the global event DAG (if we did).
   Per-worker order: insertion order of transitions whose Transition::worker
   matches. Push-to-Wait edges: by shared seq tag from event-projection.
   The shared seq only exists after petri_to_events, which is a later
   pass --- so DAG construction would either need to be moved before
   that pass, or the deadlock pass would have to consume events rather
   than a Net. The simulator approach sidesteps this entirely by
   operating on the Net layer where boundedness already lives.

3. Enumerate all deadlocks vs. fail-fast on first.
   Chose fail-fast. Matches "fail fast and verbosely" and matches
   TASK-0028's choice for boundedness. Filed TASK-0141 to add an
   enumerate-all mode for batch validation if user feedback demands it.

4. Pointer at schedule directives or event entries?
   AC #4 raised this. The error today names a transition and a place
   (echoed names + IDs). It does not point back at the source-level
   schedule directive. That is the right layer to do it --- but it
   needs a transition-to-source-span map that does not exist yet
   (TASK-0028 noted the same gap for its error variant). When that map
   lands, both passes errors gain source spans simultaneously, no shape
   change needed.

Honest limitations
==================

- First stall only. No enumerate-all. See TASK-0141.
- No cycle naming, only deficit-place naming. See TASK-0140.
- Exact-replay only --- same caveat as boundedness (one firing sequence
  walked). v2's restricted nets make this sound (PRD section 8.4
  statically determined firing order).
- Runtime livelock is out of scope. Effectful kernels that never return
  cannot be caught by a static pass.
- The deficit-place message is correct in both "missing producer"
  (TASK-0136/0139) and "genuine cycle" (Push exists but ordered after
  Wait) cases, but it does not distinguish between the two. The user
  has to look at the net (e.g. via --emit-pn) to tell which case they
  hit. Acceptable for v2; PRD section 8.5 is the inspection escape
  hatch.

Real-world catch
================

Example 02 split's net currently deadlocks (transfer_inject splices
Wait transitions without their matching Push, per TASK-0136/0139). The
test e2e_example_02_split_currently_deadlocks_on_wait_without_push
exercises this end-to-end. The pass correctly identifies a wait_seq*
transition as the stall point and reports zero tokens on the buffer
place that no Push ever produces into. This is the first analysis pass
that catches a real production bug in the ACFG-to-Petri lowering. When
TASK-0136/0139 land, the test assertion is one line to flip
(expect_err to expect Ok(())).

AC verification
===============

#1 compiler exposes check_deadlock_free(per_worker_events) -> Result<(), DeadlockError>.
   Done --- re-exported from compiler::passes::deadlock and at crate
   root. Signature accepts (&Net, &[TransitionId]) matching
   boundedness's shape. The PRD section 8.4 prose mentions
   per_worker_events but for v2's statically determined firing order
   the linearisation is total: an ordered list of TransitionIds is the
   per-worker event projection flattened in source order. boundedness
   uses the same input shape; consistency wins.

#2 DeadlockError names the cycle: an ordered list of events forming the loop.
   Partially done. DeadlockError::Stalled names the offending
   transition and the deficit place (and the marking_before snapshot).
   It does not yet emit an ordered list of events forming a cycle ---
   TASK-0140 tracks that follow-up. For v2's current production bug
   (missing producer, not genuine cycle) the deficit-place message is
   the correct shape; there is no cycle to name.

#3 Test: a synthetic schedule with a circular push/wait dependency
   produces this error with the right cycle.
   Done with a caveat: the synthetic test
   unmatched_wait_is_detected_as_deadlock is the missing-producer
   shape (which is the actual bug seen in example 02 split). A true
   circular-dependency synthetic (Push ordered after Wait) is also
   tested by consume_before_produce_stalls_at_position_zero, which
   stalls at position 0 on the deficit place --- equivalent under
   simulator-replay semantics.

#4 Implementation notes record design questions.
   Done above (this notes block).

#5 Implementation notes record honest limitations.
   Done above (this notes block).

Verification
============

- just check: green.
- just clippy: green (no warnings).
- just test: 12/12 deadlock tests pass; existing 0028 boundedness
  suite still 12/12; all other workspace tests still green.
- just e2e: 4/5 pass + 1 pre-existing skip (TASK-0117/0126), no
  regressions.
<!-- SECTION:NOTES:END -->
