---
id: TASK-0013
title: 'Example 1: element-wise add — algorithm + naive schedule + reference'
status: To Do
assignee: []
created_date: '2026-05-17 23:03'
labels:
  - M0
  - examples
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Smallest possible end-to-end example. Algorithm: c[i] <-- add(a[i], b[i]) over a 1D iteration. Naive schedule places everything on host. Reference impl is a hand-written Rust function.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 examples/01-elementwise-add/prog.algo.nuc declares input arrays a, b, output c, kernel add, dataflow.
- [ ] #2 examples/01-elementwise-add/schedules/naive.sched.nuc places everything on host.
- [ ] #3 examples/01-elementwise-add/kernels.rs implements add as a plain Rust function.
- [ ] #4 examples/01-elementwise-add/reference/ contains an independent hand-written Rust implementation.
- [ ] #5 examples/01-elementwise-add/input.bin and reference.bin committed; small enough (<10KB) to be inspected.
- [ ] #6 examples/01-elementwise-add/README.md describes what the example stresses.
- [ ] #7 Test: reference impl run on input.bin produces reference.bin (CI check).
- [ ] #8 Implementation notes record any decisions about input format and size.
- [ ] #9 Implementation notes record honest limitations (e.g. integer-only at this point).
<!-- AC:END -->
