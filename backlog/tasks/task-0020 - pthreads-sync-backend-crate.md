---
id: TASK-0020
title: pthreads-sync backend crate
status: To Do
assignee: []
created_date: '2026-05-17 23:04'
labels:
  - M1
  - backend
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
First backend, the test harness foundation. Emits std::thread + std::sync::Condvar based Rust code from an EventList. Tier-1, shared memory, sync only.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 backends/pthreads-sync/ is a crate with a sibling capabilities.toml.
- [ ] #2 Backend exposes emit(per_worker_event_lists: Map<WorkerId, EventList>) -> CodegenOutput.
- [ ] #3 Generated code links only against std; no external runtime crates.
- [ ] #4 Push/Wait pairs lower to writes-to-shared-memory + condvar signal/wait.
- [ ] #5 Test: a synthetic two-worker pingpong EventList produces compilable Rust that runs correctly.
- [ ] #6 Implementation notes record design questions (e.g. whether to use std::sync::Mutex or hand-rolled spinlocks for very small transfers).
- [ ] #7 Implementation notes record honest limitations (sync only; no buffering; no async; no error recovery).
<!-- AC:END -->
