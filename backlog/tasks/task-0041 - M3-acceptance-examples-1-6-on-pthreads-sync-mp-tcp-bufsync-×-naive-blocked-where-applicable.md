---
id: TASK-0041
title: >-
  M3 acceptance: examples 1-6 on (pthreads-sync, mp-tcp-bufsync) × (naive,
  blocked-where-applicable)
status: To Do
assignee: []
created_date: '2026-05-17 23:07'
labels:
  - M3
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Milestone gate. Cross-backend differential test green. This is the moment the algorithm/schedule split AND the middle-end/presentation-layer split become falsifiable simultaneously.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 'just e2e --milestone M3' exits 0.
- [ ] #2 Matrix is examples {1..6} × schedules {naive, blocked-where-applicable} × backends {pthreads-sync, mp-tcp-bufsync}.
- [ ] #3 Every cell that should compile does compile; every cell that should not (capability mismatch) is correctly rejected at compile time, not at runtime.
- [ ] #4 CI runs the full M3 matrix on every commit.
- [ ] #5 Test: deliberately break one cell (e.g. flip a sign in mp-tcp-bufsync codegen); CI catches it.
- [ ] #6 Implementation notes record any cells skipped/excluded with reason.
- [ ] #7 Implementation notes record honest limitations (still sync only; async + buffered comes at M4; reuse and distributed come at M5).
<!-- AC:END -->
