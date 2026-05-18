---
id: TASK-0032
title: 'Example 7: blocked matrix multiply — algorithm + naive + blocked + reference'
status: To Do
assignee: []
created_date: '2026-05-17 23:06'
labels:
  - M2
  - examples
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Blocked-matmul example. Stresses 2D blocking and all-to-all communication when distributed later. At M2, naive + blocked on pthreads-sync.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 examples/07-matmul/prog.algo.nuc declares A, B, C matrices and a per-block multiply kernel; the iteration is the canonical i/j/k nest.
- [ ] #2 examples/07-matmul/schedules/{naive,blocked}.sched.nuc exist.
- [ ] #3 examples/07-matmul/kernels.rs implements the block-multiply.
- [ ] #4 examples/07-matmul/reference/ provides the hand-written reference.
- [ ] #5 Test: naive and blocked schedules produce bit-identical output under pthreads-sync.
- [ ] #6 Implementation notes record design questions (e.g. block dimensions chosen vs alternatives, whether to expose B as a schedule parameter).
- [ ] #7 Implementation notes record honest limitations (e.g. integer matmul to avoid float-assoc reordering; small matrix size for fast CI).
<!-- AC:END -->
