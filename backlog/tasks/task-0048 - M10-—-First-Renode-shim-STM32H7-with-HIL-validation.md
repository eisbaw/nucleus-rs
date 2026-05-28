---
id: TASK-0048
title: M10 — First Renode shim (STM32H7) with HIL validation
status: To Do
assignee: []
created_date: '2026-05-17 23:08'
updated_date: '2026-05-28 11:30'
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

## Prereq unblocked: 'Renode in flake' (TASK-0064 AC#1+AC#2 — commit 632d98c)

TASK-0064 (Add Renode to flake) AC#1+AC#2 landed in commit 632d98c:
- 'nix develop .#renode -c which renode' resolves to /nix/store/.../renode-1.16.1/bin/renode.
- 'nix develop .#renode -c renode --version' returns 'Renode v1.16.1.0' (.NET 9.0.15), exit 0.

The 'is Renode available at all?' prerequisite for this M10 task is satisfied. AC#3 of TASK-0064 (example .resc + UART capture harness) was scope-split to TASK-0223, which IS load-bearing for this task (the shim development needs a harness to validate against). TASK-0223 + TASK-0062 (cross-compile Rust target for embedded) remain the open prereqs before M10 implementation can start.

Toolchain prereq satisfied by TASK-0062 (commit 9787412 / 2026-05-21): `nix develop .#embedded` provides thumbv7em-none-eabihf rust-std + probe-rs 0.31.0 on the pinned 1.83.0 toolchain. Renode-side (.resc + UART) tracked by TASK-0223.

== Forward-carried from TASK-0047 (M9 embedded-pattern landing) ==
M9 landed the GENERIC embedded-pattern backend: a COMPILE-ONLY no_std
LIB (backends/embedded-pattern/). M10's job is the no_std-LIB -> no_std-
runnable-BIN transition + a real STM32H7 shim. Concrete carry-overs:

1. NucleusShim trait shape (STABLE M9 surface, implement in the M10 shim):
     fn alloc_in_region(&mut self, region: usize, bytes: usize) -> *mut u8;
     fn dma_push(&mut self, chan: usize, src: *const u8, len: usize);
     fn dma_wait(&mut self, chan: usize);
     fn irq_barrier(&mut self, tag: u32);
   Canonical source: backends/embedded-pattern/src/skeleton.rs
   NUCLEUS_SHIM_SRC. M9 ships a do-nothing StubShim; M10 replaces it with
   real DMA/IRQ. DESIGN QUESTION recorded (AC#5): methods are SYNCHRONOUS
   (dma_push enqueues, dma_wait blocks). A real async/IRQ shim may prefer
   a completion-future/callback — M10 decides once a concrete MCU exists.

2. LIB -> BIN transition. M9 emits a no_std LIB (no panic_handler, no
   entry point, no linker script — a lib `cargo check` needs none). M10's
   runnable Renode bin needs ALL of: #[panic_handler], a cortex-m-rt
   entry (#[entry]), a memory.x/linker script, and the .resc Renode
   script. That boilerplate was DELIBERATELY deferred from M9 (the M9 bar
   is compile-only, PRD §10.3 point 2). The emitted Cargo.toml is
   currently `[lib]` only (backends/embedded-pattern/src/skeleton.rs
   render_cargo_toml) — M10 adds the bin target + cortex-m deps.

3. PURE vs EFFECTFUL kernel split (structural, NOT purity lookup — the
   Event/sidecar contract drops purity). PURE = kernel called by an
   indexed-output Fire (extracted verbatim into mod kernels). EFFECTFUL =
   top-level output-less Fire (save -> dma_push hook) or top-level whole-
   array-output Fire with no inputs (load -> alloc_in_region+dma_wait
   hook). On M9 the effectful hooks are stub no-ops; M10's shim wires
   them to real DMA from the input.bin / to a UART or memory region the
   Renode .resc dumps + diffs against reference.bin (PRD §10.3 point 3).

4. The tier-3 acceptance lives OUTSIDE default `just ci` (TASK-0223 rule):
   `just check-embedded` runs under `nix develop .#embedded`. M10's Renode
   run-and-diff is likewise a dedicated recipe under `.#renode`, NOT
   wired into the tier-1 e2e matrix. embedded-pattern is NOT in
   e2e-matrix.toml's `backends` list (compile-only != run-and-diff).

5. Capability surface: M9 capabilities.toml declares supports_async=false,
   supports_buffer=false, notify=["barrier","blocking"], memory_regions=
   ["heap"] (minimal — exactly what ex1/ex5 naive demand). M10 flips these
   to the PRD §7.3 `embedded-cortexm-dma-irq` surface (irq notify, async,
   buffer ring, tcm_per_core/shared_sram regions) when the real shim can
   honour them.
<!-- SECTION:NOTES:END -->
