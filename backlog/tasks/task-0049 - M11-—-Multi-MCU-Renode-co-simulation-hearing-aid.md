---
id: TASK-0049
title: M11 — Multi-MCU Renode co-simulation (hearing aid)
status: To Do
assignee: []
created_date: '2026-05-17 23:08'
updated_date: '2026-05-29 05:28'
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

forward-carried from TASK-0048.04: the tier-3 monotonic clock for check_frame is SysTick (NucleusShim::monotonic_ns), NOT DWT CYCCNT (DWT may not advance under Renode; SysTick does — empirically confirmed). When M11 multi-MCU schedules carry a check loop (e.g. example 14 'check loop frame : latency_max=10ms'), each MCU's shim provides its own monotonic_ns. CAVEAT inherited: a multi-MCU/pipelined check loop measures per-STAGE latency on the worker that runs the loop body, NOT end-to-end frame latency (docs/check-loop-latency-max.md §2; end-to-end correlation is TASK-0106). on_violation=panic is rejected on tier-3 (bricks the MCU); use log. on_violation=count is rejected pending a bare-metal sink (TASK-0048.08). NucleusShim is now SIX methods — a multi-MCU shim must impl monotonic_ns + report_violation too.

=== Forward-carried from TASK-0048.08 (tier-3 on_violation=count sink) ===

When M11 multi-MCU embedded check loops land: the tier-3 count sink pattern is established — a MODULE-scope AtomicU32 counter per count check loop (AtomicU64 is absent on thumbv7em) + a program-exit USART1 summary emitted in the cortex-m-rt #[entry] AFTER run() returns and BEFORE loop {} (a spinning firmware never fires a Rust Drop, so the tier-1 Drop-guard summary does NOT port). Counter is the shared lib+bin seam; summary is bin-only inline code; NucleusShim stays 6 methods. PER-MCU caveat for M11: each MCU's firmware has its OWN program-exit sink, so a multi-MCU count check loop would summarise PER-MCU (no cross-MCU aggregation) unless a coordinator collects them — mirrors the tier-1 'each process gets its own counter + Drop summary' note (docs/check-loop-latency-max.md §count). RENODE TIMING: do not assert exact timing-derived counts (Renode is not cycle-accurate; the clock-seeding iteration may resolve to 0 ns — count was 255 not 256 for the single-MCU ex1 fixture); assert a band or a structural invariant.

CORRECTION (TASK-0048.10, cycle docs-sweep) — SUPERSEDES the 'RENODE TIMING' clock-seeding framing in the M11 forward-carry note. A future M11 multi-MCU implementer must NOT reason from the seeding model. The single-MCU ex1 count was 255 (not 256) NOT because the clock-seeding iteration resolves to 0 ns. `_check_elapsed = monotonic_ns().wrapping_sub(_check_start)` CANCELS iteration 0's seeded `_check_start = 0`, so seeding contributes nothing. The real cause is Renode's coarse, non-cycle-accurate SysTick stepping: an iteration whose two clock reads fall within one un-stepped SysTick counter quantum (CVR unchanged -> delta 0) measures elapsed 0 ns and is not counted; which iteration (if any) is instruction-layout-dependent, hence 255 OR 256. For M11: per-MCU count check loops summarise PER-MCU and EACH is subject to the same SysTick quantization band — assert a 255-or-256-style band or a structural invariant (loop ran N iters / counter flushed), never an exact timing-derived count.

=== Forward-carried from M10 AC#1 de-risk (TASK-0048.11 cycle): Renode multi-MCU interconnect scouting ===
Verified from bundled Renode 1.16.1 source/scripts (see TASK-0049.01 for detail):
- Multi-MCU co-sim IS supported (multiple `mach create`; bundled scripts/multi-node/ co-simulate even heterogeneous MCU families over a wired UART hub).
- Wired cross-machine interconnects that EXIST: UART hub (byte/word/dword), CAN hub, LIN hub, GPIO connector, USB connector (+ BLE / 802.15.4 wireless).
- CRITICAL: there is NO MCU-to-MCU SPI link in Renode (no SPIHub/SPIConnector anywhere). SPI is intra-machine only. So AC#2 / PRD 'over SPI' is NOT directly modellable — the interconnect must be re-decided (UART hub is the natural supported choice, matching AC#2's flexible 'shared interconnect' wording). Filed as TASK-0049.01 (medium); this is a PRD-level decision to make BEFORE the deep M11 codegen.

=== Forward-carried from TASK-0049.01 (inter-MCU transport de-risk, DONE): startup-ordering discipline the M11 codegen MUST emit ===
Interconnect DECIDED = UART hub (Renode has no MCU-to-MCU SPI link; user choice). The empirical 2x-STM32H743 + UARTHub smoke (tests/renode/multimcu-uart-smoke/, just renode-multimcu-uart-smoke) proved wired cross-MCU UART transport works end-to-end.
DURABLE GOTCHA the generated multi-MCU firmware/.resc MUST handle: Renode's UARTBase.WriteChar DROPS a received char when the receiver's RX is not yet enabled (IsReceiveEnabled = RE && UE) — pre-enable arrivals are NOT queued. So a generated sender that transmits before the generated receiver has enabled its USART RX loses the opening bytes (a silent, scheduler-luck-dependent corruption). The de-risk harness fixes this in run.resc by start-gating the sender (`cpu IsHalted true` until the receiver boots) + a fine SetGlobalQuantum, mirroring the bundled nrf52840-ble-hci-uart reference. When M11 codegen emits the multi-MCU project, the generated .resc (or the firmware's own handshake) MUST guarantee every receiver has RX-enabled before any sender transmits to it — e.g. start-gate non-host workers' transmit, or have the host/coordinator barrier on receiver-ready. This is the cross-MCU analogue of the Event::Sync barrier; Sync->irq_barrier wiring should subsume it once real.
TRANSPORT SHAPE for the lowering: USART1 on a CreateUARTHub bus (byte hub); TX via TDR poll-on-TXE (Renode hardwires TXE=true), RX via poll-on-RXNE + read RDR. Push -> transmit over the hub; Wait -> poll RX until the expected bytes arrive; Sync -> irq_barrier (cross-MCU). See tests/renode/multimcu-uart-smoke/src/lib.rs for the reference register helpers.
<!-- SECTION:NOTES:END -->
