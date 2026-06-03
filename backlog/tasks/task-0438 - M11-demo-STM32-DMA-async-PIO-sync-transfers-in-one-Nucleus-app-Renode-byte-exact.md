---
id: TASK-0438
title: >-
  M11 demo: STM32 DMA-async + PIO-sync transfers in one Nucleus app (Renode
  byte-exact)
status: To Do
assignee: []
created_date: '2026-06-03 21:55'
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
