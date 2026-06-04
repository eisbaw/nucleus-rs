---
id: TASK-0438
title: >-
  M11 demo: STM32 DMA-async + PIO-sync transfers in one Nucleus app (Renode
  byte-exact)
status: To Do
assignee: []
created_date: '2026-06-03 21:55'
updated_date: '2026-06-04 07:01'
labels: []
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Motivation: counter the 'this is just toy memcpy' framing by showing the embedded-pattern backend can emit two structurally different transport paths from one nucleus DSL source — DMA-driven async (bulk/streaming) AND CPU-driven PIO sync (control/low-latency) — within ONE Nuc application, verified byte-exact under Renode.

Natural example shape (audio-style, mirrors ex14 hearing-aid reality):
- DMA-async edge: bulk audio frame transfer (peripheral → buffer or buffer → DAC) — Nuc emits DMA arm + IRQ wait
- PIO-sync edge: control register write or status poll — Nuc emits direct memory-mapped store/load
- Both within one schedule, one prog.algo.nuc

Open design questions (decide in task — empirical pre-brief):
1. WHERE does the DMA-vs-PIO choice live? Likely a backend HINT on a transport edge (not algo IR — DMA is a scheduling/codegen choice, not algorithmic correctness). Maybe schedule-side: 'place k on w mode=dma-async'.
2. WHICH peripheral pair gives the clean demo? Candidates: GPDMA + USART/UART (already wired in current .repl); MDMA + memory-to-memory; BDMA + ADC. UART path is the smallest extension (current renode-multimcu uses USART1 for UARTHub).
3. WHICH cortex-m HAL? embedded-pattern is no_std thumbv7em-none-eabihf; stm32h7xx-hal exposes DMA streams. Need to confirm it's already pulled in (likely yes — check Cargo.toml of the embedded-pattern generated cargo template).
4. IRQ wait strategy: wfi-loop vs spin on transfer-complete flag. wfi is honest-embedded; spin is simpler. Either works for byte-exact differential.

Acceptance:
- AC#1: example added (likely '22-dma-pio-demo' or '23-...' depending on numbering after 21-jacobi-converge) with at least one DMA-async transport edge and one PIO-sync transport edge in the same Nuc app
- AC#2: embedded-pattern backend emits a structurally distinguishable code path for the two (DMA: HAL call + IRQ-wait stub; PIO: memcpy/volatile-store)
- AC#3: Renode .repl wires the DMA peripheral correctly (or the existing platform already has it); 'just renode-multimcu <example>' runs BYTE-EXACT vs reference.bin
- AC#4: HONEST header note in the example explaining Renode DMA timing is a model (not cycle-accurate); the proof is value-correctness, not timing
- AC#5: tier-1 unaffected (e2e baseline preserved; the new example is multi-MCU-only or skipped on tier-1 like 14-hearing-aid is today)

Dependencies / context:
- Pick AFTER TASK-0049.10's slice cluster (A/B/C/D) lands — that establishes the effectful-kernel pipeline this demo will need for IO
- Reuse the renode-multimcu recipe (justfile:1578-ish range) and the proven UARTHub co-sim wiring from TASK-0049.05.01 (commit 3315247)
- Reference memories: project-grammar-deferred-cluster (for any schedule-DSL extensions), feedback-workaround-before-root-cause (decide HINT-on-edge before falling back to per-Fire flags)

Honest scope: this is comparable in scope to a single .10 sibling — a real codegen + DSL-or-hint + example + Renode-platform task. Decompose to sub-slices if the schedule-DSL change turns out non-trivial. Not a quick demo.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 example added (likely '22-dma-pio-demo') with >=1 DMA-async transport edge AND >=1 PIO-sync transport edge in the same Nuc app
- [ ] #2 embedded-pattern backend emits a structurally distinguishable code path for the two (DMA: HAL/descriptor arm + IRQ-wait stub; PIO: volatile byte loop)
- [ ] #3 Renode .repl wires the DMA peripheral correctly (or existing platform already has it); 'just renode-multimcu <example>' runs BYTE-EXACT vs reference.bin
- [ ] #4 HONEST header note in the example: Renode DMA timing is a model (not cycle-accurate); proof is value-correctness not timing
- [ ] #5 tier-1 unaffected (e2e baseline preserved; new example is multi-MCU-only or tier-1-skipped like 14-hearing-aid)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
PRE-BRIEF (orchestrator, cycle start 2026-06-04): empirical machinery map complete. Threading path for the transport-mode hint, exact current pointers:
- AST: nucleus/nucleus-compiler/src/sched/ast.rs:307 `TransferOption` enum (Sync/Async/Buffer/Notify)
- Parser: nucleus/nucleus-compiler/src/sched/parser.rs:607 `transfer_option()` (model the new rule on the `notify=event|poll` choice arm)
- Lower: nucleus/nucleus-compiler/src/sched/lower.rs:1119-1206 (raw->Resolved + dup-check)
- IR: nucleus/nucleus-compiler/src/sched/ir.rs:283-299 `ResolvedTransferOption`
- Capabilities: nucleus/nucleus-compiler/src/capabilities.rs:347-394 (per-option capability check)
- TransferPolicy consumer: nucleus/nucleus-compiler/src/passes/transfer_inject/ordering.rs:776-786 `policy_from_directive`
- Backend transport lowering (where .02 will diverge): nucleus/backends/embedded-pattern/src/render.rs:348-391 (link_push/link_recv) + skeleton/multimcu.rs:238-258 (concrete UART byte loop) + multimcu.rs:202-335 `TransportPlan::build`
- Renode platform/.resc gen: embedded-pattern/src/multimcu.rs:1171-1297 `render_multimachine_resc`; existing .repl = platforms/cpus/stm32h743.repl (USART-only today, NO DMA engine wired)
- Examples: examples 02-split-add/schedules/split.sched.nuc + 14-hearing-aid/schedules/embedded_multimcu_sync.sched.nuc (both all-`sync`, byte-exact under renode-multimcu-gate)

DESIGN DECISION (open Q#1 RESOLVED): DMA-vs-PIO lives as a new `TransferOption` HINT on the `transfer` edge (`mode=pio|dma`), parallel to `notify=`. NOT in algo IR (transport is codegen concern). NOT per-Fire flag. Default = PIO (=current behavior) so 02/ex14 stay byte-exact.

DECOMPOSED into 3 sub-slices: .01 DSL+IR+policy threading (no codegen divergence, regression-safe); .02 backend divergent codegen (DMA arm+IRQ-wait stub vs PIO volatile loop); .03 new demo example + Renode platform + byte-exact gate. Open Q#2 (peripheral) leaning GPDMA-over-USART to reuse UARTHub fabric; Q#3 (HAL) current template is cortex-m-rt only + bare volatile regs (no HAL); Q#4 (IRQ wait) deferred to .02. Starting .01.
<!-- SECTION:NOTES:END -->
