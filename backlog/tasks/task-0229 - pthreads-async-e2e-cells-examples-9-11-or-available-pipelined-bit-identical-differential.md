---
id: TASK-0229
title: >-
  pthreads-async e2e cells: examples 9 + 11 (or available pipelined) +
  bit-identical differential
status: To Do
assignee: []
created_date: '2026-05-21 21:49'
labels:
  - M4
  - backend
  - e2e
dependencies:
  - TASK-0226
  - TASK-0227
  - TASK-0228
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
AC#4 of the parent TASK-0042.01: examples 9 (producer/consumer pipe) and 11 (Game of Life multi-iter) on pthreads-async × pipelined.sched.nuc bit-identical to their reference.bin.

Note examples 9 and 11 may not yet exist as runnable directories under nuc-nucleus/examples/. As of cycle 16 the runnable_examples list is [01-elementwise-add, 02-split-add, 03-reduction, 04-prefix-sum, 05-stencil, 06-separable-filter, 07-matmul, 13-cnn-inference]. If examples 9 / 11 are not yet authored, this task EITHER:
(a) Adds the missing example dirs (algo.nuc + sched + reference + kernels.rs + input.bin + reference.bin) as part of the e2e cell wiring, OR
(b) Targets the existing pipeline_parallel schedule in 13-cnn-inference (line 466 of nuc-nucleus/e2e-matrix.toml) and converts it from SKIPPED to required pthreads-async cells.

Either path makes the cross-backend differential gate THREE-WAY: pthreads-sync, mp-tcp-bufsync, pthreads-async all bit-identical for cells whose capability surface ALL three satisfy. The async/buffered/pipelined cells become the headline pthreads-async-only column.

Add 'pthreads-async' to nuc-nucleus/e2e-matrix.toml backends list ONLY when this task is ready to land — adding it sooner produces N cells × ContractGap = N false-fails.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 nuc-nucleus/e2e-matrix.toml backends list includes 'pthreads-async'.
- [ ] #2 Cells for 13-cnn-inference/pipeline_parallel/pthreads-async (and any new examples 9/11 if authored) are listed as required + pass bit-identical.
- [ ] #3 Determinism gate (PRD §10.1): the bit-identical output reproduces under a 2x run.
- [ ] #4 The two SKIPPED 13-cnn-inference/pipeline_parallel entries (pthreads-sync + mp-tcp-bufsync) STAY SKIPPED because those backends genuinely lack the capability — they are not converted; pthreads-async is the new column carrying that schedule.
<!-- AC:END -->
