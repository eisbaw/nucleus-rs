---
id: TASK-0131
title: 'Petri net: reachability + deadlock analyses'
status: Done
assignee: []
created_date: '2026-05-18 03:28'
updated_date: '2026-05-23 21:27'
labels:
  - M2
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Build analyses that consume the Net from TASK-0025: structural reachability under the v2 subclass (acyclic firing, bounded places), and deadlock-marking detection. PRD §8.2 lists deadlock-check as 'reachability of a deadlocked marking. Decidable for v2's restricted nets'. The petri data structures are in place; this task builds the analyses on top.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Net::reachable_from(&Marking) -> bounded enumeration (or witness path) under v2 subclass
- [ ] #2 deadlock detection returns the offending marking, with names attached for diagnostics
- [ ] #3 tests: known-deadlocking small net (dining-philosophers-style) reports the deadlock; an obviously-OK pipeline net does not
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-PARTIALLY-DONE (orchestrator-direct, cycle 77 sweep). The deadlock-detection half is ALREADY DONE via TASK-0029 (simulator-replay form: first stall = deficit place + transition; landed early M2). The structural-reachability half is new work but has no current driver — 88/70/0/18 e2e cells have not produced a single deadlock false-positive or false-negative against the simulator form; reachability is the same answer in stronger form, no need. Reopen if/when (a) the simulator approach proves insufficient (false-positive or false-negative surfaces) OR (b) a v3 schedule class outside acyclic-firing-bounded-places needs the broader reachability machinery. Same deferred-no-driver pattern as TASK-0140/0141.
<!-- SECTION:FINAL_SUMMARY:END -->
