---
id: TASK-0281
title: >-
  sync_inject: Sequence-boundary skip inside partition is unconditional — add
  participants-aware guard for M6+ cross-partition reducers
status: To Do
assignee:
  - '@mark'
created_date: '2026-05-24 14:16'
labels:
  - M6
  - compiler
  - sync_inject
  - tech-debt
  - forward-carried-from-TASK-0268
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
=== Filed as TASK-0268 cycle-102 architect P1 forward-carry ===

The cycle-102 fix to TASK-0268 introduced an unconditional skip of the
Sequence-boundary Sync rule inside a partitioned scope (sync_inject.rs
`inject_in_sequence`, the `if !inside_partition { ... }` guard). The
skip is sound for ALL shipped schedules today because:

1. PRD §6.2.1 single-assignment holds — no cross-iteration data
   dependency.
2. Every cross-worker dataflow edge crossing the partitioned-loop
   boundary is covered by the TASK-0117 fan-out Push/Wait pairs.
3. The shipped reduction (03-reduction/distributed) lives OUTSIDE
   the partitioned scope, not nested inside it.

The architect's cycle-102 P1 review flagged that a future M6+ schedule
could violate assumption (2): a cross-partition reducer (an inner
Sequence writing to a shared output region placed on a worker set
DIFFERENT from the partition's inner-body worker set, NOT covered by
the Push/Wait pair) would silently lose synchronisation.

## Acceptance criteria

1. **Discovery trigger**: when an M6+ schedule exercises a
   cross-partition reducer (inner Sequence writing to a shared
   region on different workers than the partition's inner body),
   make the skip conditional.
2. **Fix shape (option D from cycle-85 analysis)**: replace the
   unconditional `if !inside_partition { ... }` with a check that
   evaluates whether the Sequence boundary is ALREADY covered by a
   Push/Wait pair OR by the partitioned scope's once-per-iteration
   semantics. Equivalent to extending `push_wait_pair_covers` to
   "or partition-scope already provides equivalent synchronisation".
3. **Test**: add a fixture that lowers a synthetic cross-partition
   reducer schedule; assert sync_inject inserts the necessary
   Sync (or, equivalently, Petri net deadlock-checker passes).

## Dependencies

- Trigger: an M6 or later schedule that exercises a cross-partition
  reducer pattern. The 13-cnn-inference/batch_parallel cell is a
  candidate to inspect — if its reduction phase is partition-nested
  rather than placed OUTSIDE the partition.

## Honest scope

- This is a LATENT defect / envelope limit, NOT a current
  regression. The cycle-102 unconditional skip is sound for all
  currently-shipped schedules. The follow-up exists so a future
  silent-deadlock under M6+ schedules is caught by the existing
  architectural check, not by debugging.

- The code comment at `nucleus/nucleus-compiler/src/passes/
  sync_inject.rs:356-388` (post-cycle-102) cross-references this
  task as the trigger.

## Forward-carry context

- Memory: feedback-opacity-gate-rot (cycle-101 lesson — gates
  that predate newer machinery can quietly become wrong). This
  task is the converse precedent: a gate-removal that may need a
  partial restoration when newer schedules land.
<!-- SECTION:DESCRIPTION:END -->
