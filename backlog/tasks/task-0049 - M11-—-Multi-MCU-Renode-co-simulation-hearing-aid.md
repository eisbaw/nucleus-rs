---
id: TASK-0049
title: M11 — Multi-MCU Renode co-simulation (hearing aid)
status: To Do
assignee: []
created_date: '2026-05-17 23:08'
updated_date: '2026-05-28 11:30'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
== Forward-carried from TASK-0047 (M9 embedded-pattern landing) ==
M9's embedded-pattern backend is SINGLE-WORKER compile-only. Multi-MCU
(M11) is the explicit OUT-OF-SCOPE boundary. Concrete carry-overs:

1. The M9 backend REJECTS used_workers > 1 with a precise forward link to
   THIS task (TASK-0049): "embedded-pattern (M9) is single-worker
   compile-only; ... Multi-MCU embedded codegen (workers on co-simulated
   MCUs over SPI / Ethernet) is M11 — TASK-0049." Site:
   backends/embedded-pattern/src/lib.rs emit(). M11 lifts that guard and
   lowers Push/Wait/Sync events (cross-MCU transport).

2. NucleusShim.irq_barrier(tag: u32) is DEFINED but UNEXERCISED on M9
   (the naive single-worker schedules carry no Event::Sync barrier). It
   was declared early precisely so M11's partitioned multi-MCU schedules
   (which DO emit barriers) implement a stable surface. M11 wires
   Event::Sync -> shim.irq_barrier(sync_tag). Likewise Event::Push/Wait
   on M11 map to dma_push/dma_wait across the inter-MCU transport (SPI /
   Ethernet), which M9 currently rejects (single-worker carries none).

3. The PURE/EFFECTFUL structural split + verbatim kernel extraction +
   fixed-[T;N] no_std layout all carry forward unchanged; M11 adds the
   cross-worker transport lowering on top. See TASK-0048 notes for the
   full NucleusShim trait shape + lib->bin transition (M11 builds on M10).

4. 14-hearing-aid/embedded_multimcu is the canonical M11 multi-MCU
   example (TASK-0192 brings it into the lower/link/ACFG matrix;
   TASK-0054.01 reinstates per-frame peripheral kernels). When M11 runs
   it through embedded-pattern, the multi-worker guard above is the first
   thing to lift.
<!-- SECTION:NOTES:END -->
