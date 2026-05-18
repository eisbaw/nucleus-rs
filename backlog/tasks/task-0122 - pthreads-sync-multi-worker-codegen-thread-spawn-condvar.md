---
id: TASK-0122
title: 'pthreads-sync: multi-worker codegen (thread spawn + condvar)'
status: To Do
assignee: []
created_date: '2026-05-18 02:13'
labels:
  - M1
  - backend
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
At TASK-0020 the pthreads-sync backend rejects schedules that use more than one worker because multi-worker codegen is not implemented. Implement std::thread::spawn for the multi-worker case, with std::sync::Condvar-based barriers for ACFG::Sync nodes and shared-memory channels (Mutex<Option<T>> + Condvar) for ACFG::Xfer pairs (Push/Wait). The synthetic AC #5 ping-pong test in the original task description (a two-worker pingpong EventList producing compilable Rust that runs correctly) belongs here.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Two-worker synthetic pingpong (producer on w0, consumer on w1, three Push/Wait pairs) compiles and runs correctly.
- [ ] #2 Each declared worker becomes its own std::thread::spawn block; main joins them all.
- [ ] #3 Sync nodes lower to std::sync::Barrier (or equivalent Condvar dance) across the participating worker threads.
- [ ] #4 Push/Wait pairs share a typed Arc<(Mutex<Option<T>>, Condvar)> slot; producer sets + notifies, consumer waits + takes.
- [ ] #5 Example 2 (split element-wise add, TASK-0021) works end-to-end on this path.
<!-- AC:END -->
