---
id: TASK-0031
title: 'Example 5: 3x3 stencil — algorithm + naive + blocked + reference'
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
The PRD's canonical stencil example. Already sketched under examples/05-stencil/. Add kernels.rs, blocked.sched.nuc, reference impl, input/reference binaries.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 examples/05-stencil/kernels.rs implements blur3.
- [ ] #2 examples/05-stencil/schedules/blocked.sched.nuc exists with loop y : block=64.
- [ ] #3 examples/05-stencil/reference/ contains hand-written stencil reference impl.
- [ ] #4 input.bin and reference.bin committed; test images small enough (~100x100) for inspection.
- [ ] #5 Test: naive and blocked schedules both produce bit-identical output under pthreads-sync at M2.
- [ ] #6 Implementation notes record any design questions discovered when implementing the reference and the kernels.rs body.
- [ ] #7 Implementation notes record honest limitations (e.g. boundary rows currently handled by clamping; reuse-with-shift not yet wired).
<!-- AC:END -->
