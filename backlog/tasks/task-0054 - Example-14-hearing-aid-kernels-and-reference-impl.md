---
id: TASK-0054
title: Example 14 (hearing aid) kernels and reference impl
status: To Do
assignee: []
created_date: '2026-05-17 23:09'
labels:
  - examples
  - M6
  - M11
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Complete example 14: kernels.rs with denoise (FFT-based), mix2, peripheral-IO stubs that read from canned binary files in test build. reference/ for hand-rolled verification. Required for M6 and M11.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 examples/14-hearing-aid/kernels.rs implements denoise and mix2 deterministically (integer fixed-point or fixed-order FFT).
- [ ] #2 fe_capture / rf_receive read from canned input bins in tier-1; in Renode they read from simulated peripherals.
- [ ] #3 fe_emit / rf_transmit write to canned output bins in tier-1; in Renode they write to simulated peripherals.
- [ ] #4 examples/14-hearing-aid/reference/ provides hand-rolled reference.
- [ ] #5 Test: naive and embedded_multimcu both reference-match under tier-1 and (at M11) under Renode multi-MCU.
- [ ] #6 Implementation notes record design questions (e.g. choice of FFT impl for determinism; whether to use rustfft, microfft, or hand-rolled fixed-point).
- [ ] #7 Implementation notes record honest limitations (denoise is a toy implementation; not deployable; v2 is about the dataflow shape, not the audio quality).
<!-- AC:END -->
