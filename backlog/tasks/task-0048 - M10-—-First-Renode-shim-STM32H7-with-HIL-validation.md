---
id: TASK-0048
title: M10 — First Renode shim (STM32H7) with HIL validation
status: To Do
assignee: []
created_date: '2026-05-17 23:08'
labels:
  - M10
  - backend
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Tier-3 milestone: reference shim for STM32H7 (Cortex-M7). Renode in CI. Examples 1, 5, 9 validated via Renode simulation. PRD §11. Placeholder.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 backends/embedded-pattern/shims/stm32h7/ crate provides DMA controller, IRQ bindings, memory layout.
- [ ] #2 Renode .resc scripts committed under examples/NN/renode/.
- [ ] #3 CI job spins up Renode and runs examples 1, 5, 9 single-MCU; captures UART output; diffs against reference.bin.
- [ ] #4 Test: 'just e2e --milestone M10' includes Renode runs.
- [ ] #5 Implementation notes record design questions (DMA configuration choices; IRQ priorities; memory-region mapping decisions).
- [ ] #6 Implementation notes record honest limitations (single-MCU only; multi-MCU at M11; HIL hardware not required).
<!-- AC:END -->
