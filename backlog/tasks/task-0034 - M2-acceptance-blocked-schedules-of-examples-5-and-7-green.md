---
id: TASK-0034
title: 'M2 acceptance: blocked schedules of examples 5 and 7 green'
status: To Do
assignee: []
created_date: '2026-05-17 23:06'
labels:
  - M2
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Milestone gate. Examples 5 and 7 under both naive and blocked schedules, on pthreads-sync, all bit-identical. Determinism CI green.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 'just e2e --milestone M2' exits 0.
- [ ] #2 Matrix is examples {1,2,3,5,7} × schedules {naive, blocked-where-applicable} × backends {pthreads-sync}. All cells green.
- [ ] #3 --emit-pn produces a DOT file for at least one example that renders meaningfully.
- [ ] #4 Boundedness and deadlock checks pass for all M2 examples.
- [ ] #5 Implementation notes record any features that almost made M2 but were cut (e.g. reuse, double-buffering, pipeline).
- [ ] #6 Implementation notes record honest limitations (still one backend; cross-backend differential test arrives at M3).
<!-- AC:END -->
