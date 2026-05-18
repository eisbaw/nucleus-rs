---
id: TASK-0132
title: 'Petri net: structural isomorphism check'
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
PRD §8.2: 'schedule equivalence = net isomorphism. Two schedules are the same iff their nets are isomorphic up to worker renaming. Useful for caching, regression testing, and reasoning about refactors.' This consumes the Net from TASK-0025. Worker-rename equivalence (not graph-iso in full generality) is what the v2 use case wants; full structural graph iso is NP-hard and unnecessary.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 iso(a: &Net, b: &Net) -> Option<WorkerMap> returns the renaming witness when isomorphic, None otherwise
- [ ] #2 labels (Place/Transition names) participate in the match; the renaming is over WorkerId only
- [ ] #3 tests: two schedules differing only in worker labels are reported equivalent; structurally different nets are not
<!-- AC:END -->
