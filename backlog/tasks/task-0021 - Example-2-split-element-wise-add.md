---
id: TASK-0021
title: 'Example 2: split element-wise add'
status: To Do
assignee: []
created_date: '2026-05-17 23:04'
labels:
  - M1
  - examples
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Two-worker version of example 1: host loads input, worker w0 processes, host writes output. Smallest example with a real cross-worker transfer.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 examples/02-split-add/prog.algo.nuc declares the add and the iteration.
- [ ] #2 examples/02-split-add/schedules/naive.sched.nuc places host + w0 with appropriate transfers.
- [ ] #3 examples/02-split-add/kernels.rs implements add as plain Rust.
- [ ] #4 examples/02-split-add/reference/ contains the independent hand-written reference.
- [ ] #5 examples/02-split-add/input.bin + reference.bin committed.
- [ ] #6 Test: e2e harness runs this example through naive sched + pthreads-sync; bit-identical output.
- [ ] #7 Implementation notes record design questions (e.g. one-shot transfer vs streamed; v2 picks one-shot at this stage).
- [ ] #8 Implementation notes record honest limitations (no blocking yet; whole input transferred once).
<!-- AC:END -->
