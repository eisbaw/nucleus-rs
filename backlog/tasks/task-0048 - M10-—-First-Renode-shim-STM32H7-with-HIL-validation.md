---
id: TASK-0048
title: M10 — First Renode shim (STM32H7) with HIL validation
status: To Do
assignee: []
created_date: '2026-05-17 23:08'
updated_date: '2026-05-21 16:19'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Forward-carried lesson from TASK-0052.02 (real-time codegen)

The tier-1 backends (TASK-0052.02 commit `d2bbf76` + review-gate
hardening) emit `std::time::Instant::now()` for the latency_max
measurement. PRD §6.3.5 says "tier 3 backend-specified monotonic
clock" — `std::time::Instant` does NOT exist on bare-metal Cortex-M
(no_std + no allocator + no OS).

When this Renode STM32H7 shim lands, the `Event::Loop.check_frame`
codegen path needs a DIFFERENT clock primitive. Candidate sources:
- DWT cycle counter (`CYCCNT` register) — Cortex-M7 has it; convert
  cycles to ns using the configured SystemCoreClock.
- SysTick down-counter — fixed-tick; precision depends on tick rate.
- Renode's `Machine.GetTimeSourceCurrentTime` — exposed via UART
  trace; useful for HIL but not embedded production.

The tier-3 backend's `Event::Loop` arm consumes the same
`Option<CheckFrame>` field as tier 1; only the clock-source rendering
differs. Sketch:

```text
let _check_start = <clock>::now();
... body ...
let _check_elapsed = <clock>::now().sub_ns(_check_start);
if _check_elapsed > {latency_max_ns} { <on_violation panics or logs> }
```

PRD §6.3.5: `on_violation=panic` on tier-3 BRICKS the device — for
embedded targets, `log` or `count` is preferred. TASK-0052.04 wires
log/count for tier-1 first; the tier-3 shim should follow that
contract.
<!-- SECTION:NOTES:END -->
