---
id: TASK-0053
title: Example 13 (CNN inference) kernels and reference impl
status: To Do
assignee: []
created_date: '2026-05-17 23:09'
labels:
  - examples
  - M6
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Complete example 13: kernels.rs implementing conv_block_1, conv_block_2, classifier; reference/ Rust impl; input.bin (canned input + canned weights); reference.bin. Required for M6 (full tier-1) and M7 (MPI). Algorithm and schedules already sketched.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 examples/13-cnn-inference/kernels.rs implements all four pure kernels and the two effectful ones.
- [ ] #2 Weights are deterministic — either baked into kernels.rs as const arrays or loaded from a committed binary.
- [ ] #3 examples/13-cnn-inference/reference/ contains an independent reference impl.
- [ ] #4 Required schedules: naive, batch_parallel, pipeline_parallel — all listed in README under M6 are present and reference-matching.
- [ ] #5 Test: all three schedules × all tier-1 backends produce reference-matching output.
- [ ] #6 Implementation notes record design questions (e.g. precision: f32 vs integer scaling for determinism; what fixed-input/fixed-weights mean for the differential test).
- [ ] #7 Implementation notes record honest limitations (no training; small network; no quantisation).
<!-- AC:END -->
