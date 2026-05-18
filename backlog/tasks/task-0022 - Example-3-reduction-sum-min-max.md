---
id: TASK-0022
title: 'Example 3: reduction (sum / min / max)'
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
Tree-reduction example. Multiple workers compute partial reductions, host combines. Stresses sync barrier semantics and the Barrier SyncKind.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 examples/03-reduction/prog.algo.nuc declares the input and a kernel for the per-element accumulation; the reduction is expressed as a for-loop pattern.
- [ ] #2 examples/03-reduction/schedules/naive.sched.nuc places everything on host (smoke test).
- [ ] #3 examples/03-reduction/kernels.rs implements the accumulator function.
- [ ] #4 examples/03-reduction/reference/ provides the hand-written reference.
- [ ] #5 Test: e2e harness runs this through naive + pthreads-sync; bit-identical output.
- [ ] #6 Implementation notes record design questions (e.g. how to express tree-reduction as a Nuc pattern when v2 has no built-in reduce primitive).
- [ ] #7 Implementation notes record honest limitations (integer reductions only at M1; float reductions reorder and break bit-identity).
<!-- AC:END -->
