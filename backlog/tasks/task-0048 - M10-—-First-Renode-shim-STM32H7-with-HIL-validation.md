---
id: TASK-0048
title: M10 — First Renode shim (STM32H7) with HIL validation
status: To Do
assignee: []
created_date: '2026-05-17 23:08'
updated_date: '2026-05-28 22:34'
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

=== Cycle 237 Renode harness de-risk (orchestrator, this session 2026-05-28) ===
Proved the M10 Renode loop is viable IN THIS SANDBOX before building the codegen (analog of the M9 no_std cross-compile smoke). Headless smoke: `renode --disable-xwt --console --plain <script.resc>` on the bundled stm32f746.resc platform (STM32F746 = Cortex-M7, same core family as the STM32H7 target).
VERIFIED:
- Renode 1.16.1 runs HEADLESS here (no X/GUI), exit 0, clean quit via `emulation RunFor "0.2"; quit`.
- Network fetch works: the script's `@https://dl.antmicro.com/.../dartino-lines.elf` downloaded + loaded.
- Real Cortex-M7 EMULATION: CPU executed instructions (trace shows `[cpu: 0x801C8E4]` PCs + firmware driving rcc/gpioPortH/DCMI/ethernet.phy peripherals).
- UART file backend creatable: `usart1 CreateFileBackend @/tmp/uart.txt true` accepted; file created (0 bytes here only because the dartino LCD demo doesn't write usart1).
- Bundled platforms live under <renode-store>/lib/renode/{scripts,platforms}; stm32f7_discovery-bb.repl + many single-node .resc (stm32f746, stm32f103, nrf52840, miv, ...).
STILL TO DO for M10 (the real codegen work, NOT yet done):
1. lib->bin: emit embedded-pattern as a `#![no_main]` cortex-m bin (#[entry] via cortex-m-rt, #[panic_handler], STM32H7 memory.x linker). M9 emits a no_std LIB only.
2. STM32H7 NucleusShim impl (DMA/IRQ/memory map) under backends/embedded-pattern/shims/stm32h7/.
3. UART OUTPUT path: the generated firmware must write its result over USART1 so `reference.bin` can be diffed against captured UART. (UART *capture* mechanics proven; UART *emission* from our firmware is unproven — needs the bin.)
4. A committed .resc + a `just` recipe (under .#renode) that loads our firmware, RunFor a bounded time, captures UART to a file, diffs vs reference, fails LOUD on mismatch. (This is the TASK-0223 harness that was closed deferred.)
Recommended first M10 slice: a minimal hand-written STM32H7/F7 no_std bin that prints a known byte sequence over USART1, run in Renode, UART captured + asserted — proving emission+capture end-to-end and establishing the bin template — BEFORE wiring embedded-pattern's emit to it.

=== Cycle 237b: M10 firmware->Renode->UART TEMPLATE landed (commits b57d030 + 7de02f9) ===
First tier-3 RUNTIME slice proven end-to-end IN-SANDBOX (the user chose the bounded UART-firmware slice). Hand-written minimal STM32H743 (Cortex-M7) no_std firmware at tests/renode/uart-smoke/ (cortex-m-rt vector table + #[entry] + panic handler; memory.x FLASH@0x08000000 / DTCM RAM@0x20000000) emits sentinel "NUCLEUS-M10-OK" over USART1 (raw regs: CR1=UE|TE @0x40011000, poll ISR.TXE @+0x1C, write TDR @+0x28). New `just renode-uart-smoke` recipe: cross-compiles under .#embedded, runs headless in Renode (--disable-xwt --console) on bundled platforms/cpus/stm32h743.repl under .#renode, captures USART1 to a file backend, asserts the sentinel (fail-loud grep||exit1 + dumps renode log on failure).
PROVEN (both reviews GO; architect verified every register/mem claim vs the actual Renode STM32F7_USART.cs model source): lib->bin transition, Renode Cortex-M7 emulation, USART1 emission + deterministic file capture, bounded RunFor + clean quit, fail-loud assertion. Host gate unaffected (firmware crate is its own [workspace], outside nucleus/; recipe NOT in just ci; target/ gitignored). e2e still 280/246/0/34/0.
KEY FACT for real M10: Renode STM32F7_USART hardwires TXE=true (the TX poll never waits in Renode; back-pressure unvalidated) BUT CR1=UE|TE enable IS load-bearing (model drops bytes if TX disabled).
STILL TO DO for M10 proper (this task stays To Do/In Progress — the TEMPLATE is a prerequisite, not the milestone):
1. embedded-pattern backend emits a no_std BIN (this firmware shape) instead of the M9 LIB — wire #[entry]/panic_handler/memory.x into the generated project + a USART-streaming output path.
2. Stream the COMPUTED result (not a constant sentinel) over USART1 + diff captured bytes vs reference.bin (replace the grep with a binary diff).
3. STM32H7 NucleusShim impl (DMA/IRQ/memory map) under backends/embedded-pattern/shims/stm32h7/.
4. Generalise the recipe to examples 1/5/9 (PRD §11 M10 set) via the embedded-pattern emit, parameterised over example.

=== Forward-carried from TASK-0048.01 (cycle 238, commit 42685fd): lib->bin transition for example 1 LANDED ===

The M10 lib->bin transition (STILL-TO-DO item 1 from the cycle-237b list above) is DONE for example 1. The embedded-pattern backend now has an ADDITIVE bin-emit mode selected by driver flag --shim stm32h7 (PRD §10.3 quad's target-shim). emit_bin() in backends/embedded-pattern/src/lib.rs produces the full Renode-runnable no_std bin project; emit() (no --shim) is the UNCHANGED M9 lib. just renode-embedded-ex1 is the run-and-assert recipe (mirrors renode-uart-smoke). Captured line: 'NUC-EX1 len=1024 checksum=0'.

Lessons the NEXT M10 slice (TASK-0048.02/.03 or the real STM32H7 shim, AC#1) inherits:

1. THE UART HOOK IS dma_push, NOT new lowering. The save_output(c) effectful Fire already lowers (in render_fire) to shim.dma_push(0, c.as_ptr() as *const u8, size_of_val(&c)); shim.dma_wait(0). To stream the output you ONLY supply a concrete shim whose dma_push emits — no codegen change needed. The real STM32H7 shim (AC#1) replaces Usart1Shim's body, NOT the lowering. Same for the load hooks: alloc_in_region + the load dma_wait are where real input-fill (DMA from a sensor / Renode-injected region) plugs in.

2. STILL-STUB INPUTS => zero output. Until alloc_in_region/load-dma_wait actually fill the arrays, the compute runs on zeros. The deterministic checksum is 0 BECAUSE of this, not despite it. TASK-0048.02 must wire a real input path (Renode sysbus WriteBytes / memory file backend / embedded input.bin bytes) BEFORE a reference.bin binary diff is meaningful.

3. FRAMING CHOICE: deterministic ASCII line + grep-q, not raw bytes. Raw null bytes are fragile to assert from a shell recipe; the no_std u32->ASCII-decimal writer + wrapping checksum gives a robust, corruption-sensitive assertion. When TASK-0048.02 switches to a byte-exact reference.bin diff, the raw-byte path (stream c.as_ptr()[0..len] verbatim) is the natural replacement — keep the ASCII summary as a human-readable sentinel alongside.

4. RENODE FACTS (re-confirmed, now from GENERATED firmware not just the hand template): STM32F7_USART hardwires TXE=true (TX poll never waits; back-pressure UNVALIDATED) BUT CR1=UE|TE enable IS load-bearing (drops bytes if TX disabled). RunFor 0.05 + quit; --disable-xwt --console --plain; platforms/cpus/stm32h743.repl bundled.

5. memory.x: RAM<=128K (full DTCM); over-raising silently overflows DTCM (no linker error) unless axiSram@0x24000000 mapped. The generated bin MUST be its own empty [workspace] (else just build compiles ARM code on host -> fail).

6. CHECK-FRAME on bin path is REJECTED with a typed EmitError forward-linking TASK-0048.04 (no_std clock). The reject is shared lib+bin (render_run_body). Naive schedules carry no check frames so it is latent; a real-time embedded schedule needs DWT CYCCNT (PRD §6.3.5: on_violation=panic BRICKS the device, prefer log/count).

7. SHARED LOWERING: lib and bin both go through lower_kernels_and_run (single source of truth). emit_bin's single-worker guard is DUPLICATED (different return type than emit); both have sibling pin tests (rejects_multi_worker_* / bin_rejects_multi_worker_*) — keep them in lockstep.

Remaining STILL-TO-DO for M10 proper (parent ACs): real STM32H7 NucleusShim DMA/IRQ/memory-map (AC#1, shims/stm32h7/ crate); computed-result + reference.bin diff (TASK-0048.02); examples 5+9 (TASK-0048.03); .resc under examples/NN/renode/ + tier-3 CI matrix row (AC#2/AC#3, TASK-0165); just e2e --milestone M10 (AC#4).

=== Forward-carried from TASK-0048.04 (no_std monotonic clock for check_frame; commit 2abcc56) ===

The check_frame REJECT (forward-carry note 6 from TASK-0048.01) is now LOWERED. embedded-pattern lowers Event::Loop check_frame on both lib + bin paths.

CLOCK CHOICE that the parent + M11 inherit: Cortex-M SysTick (24-bit down-counter @ 0xE000_E010/14/18), exposed as NucleusShim::monotonic_ns(). NOT DWT CYCCNT — DWT CYCCNT may not advance under Renode's non-cycle-accurate timing (docs §3); SysTick advances reliably (EMPIRICALLY confirmed in Renode: ~2828-3562 ns/iter, varying nonzero). PRD §6.3.5 permits either. The shim IS the 'backend-specified monotonic clock' seam: keep Cortex-M register details in the shim, render the lowering against the trait method.

on_violation tier-3 policy: log fully lowered (per-violation UART line via NucleusShim::report_violation); panic + count REJECTED (typed EmitError). panic bricks the MCU (PRD §6.3.5). count has no bare-metal Drop summary sink (firmware spins forever) — TASK-0048.08.

NucleusShim is now SIX methods (was four): added monotonic_ns + report_violation. The real STM32H7 shim (AC#1, shims/stm32h7/ crate) must implement all six; the SysTick monotonic_ns + UART report_violation in Usart1Shim are the reference impls.

e2e: the 3 embedded-only check fixtures (01-elementwise-add/schedules/embedded_check{,_panic,_count}) are auto-discovered by the schedule walk and declared [[skip]] M10 ×7 backends (mirrors embedded_multimcu / example 14).
<!-- SECTION:NOTES:END -->
