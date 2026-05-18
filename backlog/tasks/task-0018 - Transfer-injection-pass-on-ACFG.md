---
id: TASK-0018
title: Transfer injection pass on ACFG
status: To Do
assignee: []
created_date: '2026-05-17 23:04'
labels:
  - M1
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Walk the ACFG and inject acfg::push / acfg::wait nodes for every dataflow edge that crosses workers. Apply transfer policy (sync/async/buffer/notify) from the schedule.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 compiler exposes inject_transfers(ACFG, SchedIR) -> ACFG.
- [ ] #2 Each cross-worker data dependency yields a matched push/wait pair with a unique SeqTag.
- [ ] #3 Transfer policy (sync vs async, buffer depth, notify mode) attaches to the push/wait pair.
- [ ] #4 Schedule capability check: if backend lacks a capability (e.g. async on pthreads-sync), the pass errors before codegen.
- [ ] #5 Test: synthetic schedules covering each (sync/async × buffered/unbuffered × event/poll) combination.
- [ ] #6 Implementation notes record design questions (e.g. coalescing per-element pushes into per-tile bulk transfers).
- [ ] #7 Implementation notes record honest limitations (e.g. transfer-aggregation may be naive at M1).
<!-- AC:END -->
