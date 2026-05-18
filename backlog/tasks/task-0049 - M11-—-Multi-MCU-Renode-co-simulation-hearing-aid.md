---
id: TASK-0049
title: M11 — Multi-MCU Renode co-simulation (hearing aid)
status: To Do
assignee: []
created_date: '2026-05-17 23:08'
labels:
  - M11
  - backend
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Final tier-3 milestone: example 14 (hearing aid) compiles and runs in Renode multi-MCU co-simulation (e.g. master + sensor MCUs over SPI). PRD §11. Placeholder.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Renode multi-MCU script for example 14 commits; CI runs it.
- [ ] #2 FE-class, DSP-class, RF-class workers map to three co-simulated MCUs over a shared interconnect.
- [ ] #3 Example 14 produces bit-identical output to its tier-1 reference under embedded_multimcu.sched.nuc.
- [ ] #4 check loop frame : latency_max=10ms produces a measured value in Renode output.
- [ ] #5 Test: 'just e2e --milestone M11' is green; the multi-MCU runs without deadlock.
- [ ] #6 Implementation notes record design questions (interconnect choice for the co-simulation, shared-memory vs message-passing across MCUs).
- [ ] #7 Implementation notes record honest limitations (Renode timing not cycle-accurate; latency assertion verified within Renode's accuracy budget, not against real silicon).
<!-- AC:END -->
