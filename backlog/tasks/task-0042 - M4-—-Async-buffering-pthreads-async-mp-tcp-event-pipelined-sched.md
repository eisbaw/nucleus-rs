---
id: TASK-0042
title: 'M4 — Async + buffering (pthreads-async, mp-tcp-event, pipelined sched)'
status: To Do
assignee: []
created_date: '2026-05-17 23:08'
labels:
  - M4
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Tier-1 milestone: add async + buffered backends and the pipelined schedule pattern. Examples 9 and 11 land. PRD §11. This task is a placeholder; refine into sub-tasks before starting M4.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 pthreads-async backend lands (std::thread + condvar + ring buffer).
- [ ] #2 mp-tcp-event backend lands (mio for epoll-based readiness).
- [ ] #3 pipelined.sched.nuc pattern works for examples 9 and 11.
- [ ] #4 buffer=N is validated against Petri net place capacity end-to-end.
- [ ] #5 Test: M4 differential matrix is green.
- [ ] #6 Implementation notes record design questions discovered during async-codegen work.
- [ ] #7 Implementation notes record honest limitations (e.g. mio's polling overhead; whether to also offer tokio variant).
<!-- AC:END -->
