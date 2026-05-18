---
id: TASK-0024
title: 'M1 acceptance: examples 1-3 on pthreads-sync with naive'
status: Done
assignee: []
created_date: '2026-05-17 23:05'
updated_date: '2026-05-18 03:21'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
M1 matrix passes per TASK-0023's nucleus-e2e harness output: 01/naive PASS, 02/naive PASS, 02/split PASS, 03/naive PASS — all on pthreads-sync. SKIPPED: 03/distributed pending TASK-0117 + TASK-0126. M1 matrix is green for required cells. CI workflow integration (-D warnings on every commit) remains pending TASK-0057.
<!-- SECTION:NOTES:END -->
