---
id: TASK-0024
title: 'M1 acceptance: examples 1-3 on pthreads-sync with naive'
status: To Do
assignee: []
created_date: '2026-05-17 23:05'
labels:
  - M1
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Milestone gate. All M1 examples (1, 2, 3) green under naive schedule + pthreads-sync backend, run via the e2e harness.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 'just e2e --milestone M1' exits 0.
- [ ] #2 Matrix report shows 3 examples × 1 schedule × 1 backend = 3 cells, all green.
- [ ] #3 CI workflow includes 'just e2e --milestone M1' as a required check.
- [ ] #4 Test: deliberately break one example's kernels.rs; CI catches it.
- [ ] #5 Implementation notes record any examples or features that almost made the milestone but were cut.
- [ ] #6 Implementation notes record honest limitations (only one backend, only one schedule shape; the matrix is 3 cells, not a real matrix yet).
<!-- AC:END -->
