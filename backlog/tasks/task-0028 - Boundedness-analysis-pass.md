---
id: TASK-0028
title: Boundedness analysis pass
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
Walk the firing order; track live tokens per place; verify no marking exceeds the place's declared capacity. PRD §8.2.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 compiler exposes check_bounded(Net, firing_order) -> Result<(), BoundednessError>.
- [ ] #2 BoundednessError names the offending place, the firing that overflows it, and the marking at the time of overflow.
- [ ] #3 Test: a schedule that demands buffer=N too small for the pipeline produces this error with the right place name.
- [ ] #4 Implementation notes record design questions (e.g. should we suggest minimum-N in the error message).
- [ ] #5 Implementation notes record honest limitations (the analysis is exact for v2's restricted nets; not symbolic, not general).
<!-- AC:END -->
