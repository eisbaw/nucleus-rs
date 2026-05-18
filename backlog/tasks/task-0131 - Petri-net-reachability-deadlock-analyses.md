---
id: TASK-0131
title: 'Petri net: reachability + deadlock analyses'
status: To Do
assignee: []
created_date: '2026-05-18 03:28'
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
