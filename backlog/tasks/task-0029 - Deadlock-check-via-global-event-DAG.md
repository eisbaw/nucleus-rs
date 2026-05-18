---
id: TASK-0029
title: Deadlock check via global event DAG
status: To Do
assignee: []
created_date: '2026-05-17 23:05'
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
- [ ] #1 compiler exposes check_deadlock_free(per_worker_events) -> Result<(), DeadlockError>.
- [ ] #2 DeadlockError names the cycle: an ordered list of events forming the loop.
- [ ] #3 Test: a synthetic schedule with a circular push/wait dependency produces this error with the right cycle.
- [ ] #4 Implementation notes record design questions (e.g. should the error point at schedule directives or event entries; both?).
- [ ] #5 Implementation notes record honest limitations (only structural deadlocks are caught; runtime-livelock conditions are out of scope).
<!-- AC:END -->
