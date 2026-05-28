//! String templates for the generated `no_std` lib project (TASK-0047).
//!
//! These are EMBEDDED-SPECIFIC and deliberately NOT shared with
//! `backend_common::project_skeleton` (which renders std single/multi-
//! BINARY projects with `panic = "abort"` and a `[[bin]]` named
//! `nuc-generated`). A compile-only `no_std` LIB has a genuinely
//! different shape — `[lib]`, no `[[bin]]`, no `panic` profile (a lib
//! `cargo check` needs no panic strategy), no `run.sh` (nothing to run).
//! Forcing it through the shared single-binary template would be the
//! WRONG abstraction, not a reuse win.

/// The `NucleusShim` trait + `StubShim` impl emitted verbatim into
/// every generated lib (AC#2 / AC#3). Held as a constant so the test
/// suite can pin its exact shape and the M10 shim author has one
/// canonical reference for the trait surface.
///
/// The SIX methods: `alloc_in_region` (reserve backing storage
/// in a named region — a TCM / shared-SRAM / SDRAM address on real
/// hardware), `dma_push` (enqueue a DMA descriptor draining a buffer to
/// a peripheral / peer), `dma_wait` (block until a DMA channel
/// completes), `irq_barrier` (an IRQ-completion control barrier),
/// `monotonic_ns` (the PRD §6.3.5 tier-3 backend-specified monotonic
/// clock that `check loop V : latency_max=T` measures against — TASK-
/// 0048.04), and `report_violation` (the `on_violation=log` sink). The
/// `StubShim` no-ops all six — `monotonic_ns` returns 0 and
/// `report_violation` does nothing (M9 AC#6 "no real timing"; a
/// generated lib that carries a check_frame compiles but does not
/// measure or report), compile-only (AC#3 / AC#6).
pub const NUCLEUS_SHIM_SRC: &str = "\
/// The hardware-abstraction trait the generated `no_std` code lowers
/// against. A per-MCU SHIM crate (M10+, TASK-0048) implements this with
/// real DMA descriptors, IRQ vector binding, and memory-region
/// addresses; the M9 `StubShim` below no-ops every method (compile-only,
/// no real hardware — PRD \u{00A7}7.3 / \u{00A7}11 M9).
///
/// Design question (recorded for M10): the methods are presented as
/// SYNCHRONOUS (`dma_push` enqueues, `dma_wait` blocks). A real
/// async/IRQ-driven shim may prefer a completion-future or a callback;
/// that is an M10 design decision once a concrete MCU shim exists. M9
/// fixes the minimal blocking surface so the trait shape is stable.
pub trait NucleusShim {
    /// Reserve `bytes` of backing storage in memory `region` (a
    /// backend-opaque region handle). Returns a raw pointer to the
    /// region; the stub returns null (compile-only).
    fn alloc_in_region(&mut self, region: usize, bytes: usize) -> *mut u8;
    /// Enqueue a DMA transfer of `len` bytes from `src` on channel
    /// `chan` (drain a buffer to a peripheral / peer).
    fn dma_push(&mut self, chan: usize, src: *const u8, len: usize);
    /// Block until DMA channel `chan` has completed.
    fn dma_wait(&mut self, chan: usize);
    /// An IRQ-completion control barrier identified by `tag`. Unused by
    /// the single-worker examples 1 + 5 (no `Event::Sync` in a naive
    /// schedule); declared for the M10/M11 multi-MCU barrier surface.
    fn irq_barrier(&mut self, tag: u32);
    /// The tier-3 backend-specified monotonic clock (PRD \u{00A7}6.3.5).
    /// Returns a nanosecond reading that increases monotonically across
    /// calls. `check loop V : latency_max=T` lowers to
    /// `let _start = shim.monotonic_ns(); ...body...; let _elapsed =
    /// shim.monotonic_ns().wrapping_sub(_start);` (TASK-0048.04). The
    /// `StubShim` returns 0 (M9 compile-only, no real timing). The
    /// concrete `Usart1Shim` reads the Cortex-M SysTick down-counter (see
    /// `Usart1Shim::monotonic_ns`); under Renode SysTick advances
    /// reliably, unlike DWT CYCCNT.
    fn monotonic_ns(&mut self) -> u64;
    /// The tier-3 `on_violation=log` sink (TASK-0048.04). Called once per
    /// violating loop iteration with the source loop-var name, the
    /// measured ns, and the threshold ns. The tier-1 analogue is
    /// `eprintln!` — but no_std has no stderr, so the concrete shim writes
    /// a line over its diagnostic channel (USART1). The `StubShim` no-ops
    /// it (M9 compile-only, no real reporting). Panic is rejected at
    /// codegen (it bricks the device, PRD \u{00A7}6.3.5); Count lowers WITHOUT
    /// this method (it increments a module-scope `AtomicU32` static and is
    /// summarised over USART1 at program exit — TASK-0048.08), so `log` is
    /// the only on-violation action that reaches this method.
    fn report_violation(&mut self, loop_var: &[u8], measured_ns: u64, threshold_ns: u64);
}

/// Do-nothing shim satisfying [`NucleusShim`] — the M9 compile-only
/// target (AC#3). No DMA, no IRQ, no real timing (AC#6): every method
/// is a no-op, so the generated `run` compiles and executes on
/// zero-filled input arrays. `monotonic_ns` returns 0, so a generated
/// lib that DOES carry a `check loop` frame still compiles (the
/// _elapsed always reads 0, i.e. never exceeds a positive latency_max —
/// honest: M9 is compile-only, no real timing).
pub struct StubShim;

impl NucleusShim for StubShim {
    fn alloc_in_region(&mut self, _region: usize, _bytes: usize) -> *mut u8 {
        core::ptr::null_mut()
    }
    fn dma_push(&mut self, _chan: usize, _src: *const u8, _len: usize) {}
    fn dma_wait(&mut self, _chan: usize) {}
    fn irq_barrier(&mut self, _tag: u32) {}
    fn monotonic_ns(&mut self) -> u64 {
        0
    }
    fn report_violation(&mut self, _loop_var: &[u8], _measured_ns: u64, _threshold_ns: u64) {}
}
";

/// Emit the module-scope `static NUC_CHECK_COUNT_<ident>: AtomicU32`
/// declarations for every `on_violation=count` check loop (TASK-0048.08,
/// PART 1). Shared by [`render_lib`] and [`render_bin_main`] so both emit
/// the IDENTICAL static (the Count arm in `render_event` references it on
/// both paths). `AtomicU32` (not the tier-1 `AtomicU64`) because
/// `AtomicU64` is unavailable on `thumbv7em-none-eabihf`. Writes nothing
/// when there are no count loops, so the no-check-loop firmware is
/// byte-unchanged.
fn render_count_statics(s: &mut String, count_idents: &[&str]) {
    if count_idents.is_empty() {
        return;
    }
    s.push_str(
        "// TASK-0048.08: per-`check loop` Count violation counters. A\n\
         // bare-metal firmware spins forever in `loop {}`, so the tier-1\n\
         // Drop-guard summary never fires; the bin's `#[entry]` flushes a\n\
         // summary over USART1 after `run` returns instead. AtomicU32 (the\n\
         // only width with hardware atomics on thumbv7em); Relaxed is\n\
         // sufficient on a single-core MCU read only after `run`.\n",
    );
    for ident in count_idents {
        s.push_str(&format!(
            "static NUC_CHECK_COUNT_{ident}: core::sync::atomic::AtomicU32 = \
             core::sync::atomic::AtomicU32::new(0);\n",
        ));
    }
    s.push('\n');
}

/// Assemble the full `src/lib.rs`: header + trait/stub + `mod kernels`
/// (the verbatim pure bodies) + `run<S>`.
///
/// `kernel_defs` is the already-extracted pure kernel fn text (may be
/// empty for a program with no pure compute kernel). `run_body` is the
/// rendered body of `fn run<S: NucleusShim>(shim: &mut S)`.
/// `count_idents` are the sanitised idents of every `on_violation=count`
/// check loop — module-scope `AtomicU32` statics are emitted for them so
/// the lib cross-compiles when the schedule carries a count frame (the
/// lib has no `main`, so they are never flushed — fine for compile-only;
/// TASK-0048.08).
pub fn render_lib(kernel_defs: &str, run_body: &str, count_idents: &[&str]) -> String {
    let mut s = String::new();
    s.push_str(
        "//! Generated by the nucleus pre-compiler (embedded-pattern, M9).\n\
         //! Do not edit; rerun `nucleus build` to regenerate.\n\
         //!\n\
         //! A `no_std` compile-only lib (PRD \u{00A7}7.3 / \u{00A7}11 M9). The pure\n\
         //! compute kernels are copied verbatim from the source kernels.rs;\n\
         //! the effectful I/O kernels are lowered to `NucleusShim` hooks.\n\
         #![no_std]\n\
         // non_upper_case_globals: the Count counter statics preserve the\n\
         // source loop-var spelling (e.g. lowercase `i`), matching the\n\
         // tier-1 convention and keeping the name greppable back to the\n\
         // directive (TASK-0048.08).\n\
         #![allow(unused_mut, dead_code, unused_variables, non_upper_case_globals)]\n\
         \n",
    );
    s.push_str(NUCLEUS_SHIM_SRC);
    s.push('\n');
    render_count_statics(&mut s, count_idents);
    // The pure kernel bodies live in a private `mod kernels` — the same
    // `kernels::<name>(..)` call spelling the run body emits.
    s.push_str("/// Pure compute kernels, copied verbatim from the source\n");
    s.push_str("/// kernels.rs (PRD \u{00A7}6.2.2: Nucleus does not interpolate kernel\n");
    s.push_str("/// bodies). The effectful I/O kernels are NOT here — they map to\n");
    s.push_str("/// `NucleusShim` hooks in `run` below.\n");
    s.push_str("mod kernels {\n");
    // Indent the extracted defs by 4 spaces so they sit inside the mod.
    for line in kernel_defs.lines() {
        if line.is_empty() {
            s.push('\n');
        } else {
            s.push_str("    ");
            s.push_str(line);
            s.push('\n');
        }
    }
    s.push_str("}\n\n");
    // The entry point: a generic `run` over any `NucleusShim`.
    s.push_str(
        "/// Lower the single-worker event list. Generic over any\n\
         /// [`NucleusShim`]; M9 ships the do-nothing [`StubShim`] so\n\
         /// `cargo check --target thumbv7em-none-eabihf` validates the\n\
         /// shape without real hardware.\n\
         pub fn run<S: NucleusShim>(shim: &mut S) {\n",
    );
    s.push_str(run_body);
    s.push_str("}\n");
    s
}

/// The generated project's `Cargo.toml`: a standalone `no_std` LIB
/// crate. No `[[bin]]`, no `panic` profile (a lib needs neither for
/// `cargo check`), no external deps. `edition = "2021"` matches the
/// pinned 1.83.0 toolchain (flake.nix `rustChannel`).
pub fn render_cargo_toml() -> String {
    String::from(
        "# Generated by the nucleus pre-compiler (embedded-pattern, M9). Do not edit; \
         rerun `nucleus build` to regenerate.\n\
         [package]\n\
         name        = \"nuc-embedded-generated\"\n\
         version     = \"0.0.0\"\n\
         edition     = \"2021\"\n\
         publish     = false\n\
         \n\
         [workspace]\n\
         # Empty: this crate is standalone, not part of any parent workspace.\n\
         \n\
         [lib]\n\
         name = \"nuc_embedded_generated\"\n\
         path = \"src/lib.rs\"\n",
    )
}

// --------------------------------------------------------------------
// M10 (TASK-0048.01) bin-emit templates — the Renode-runnable no_std
// BINARY shape. ADDITIVE: these are SEPARATE functions from the M9 lib
// templates above; the lib path (render_cargo_toml / render_lib) is
// UNCHANGED. The bin shape is the OPPOSITE of the lib in every honest
// respect (it HAS a [[bin]], a panic profile, a panic_handler, a
// cortex-m-rt entry, a linker script) — so the docs here state the bin
// facts, NOT the lib facts (feedback-verbatim-copy-comment-doc-lie).
//
// These mirror the PROVEN hand-written template at
// tests/renode/uart-smoke/ (commits b57d030 + 7de02f9), verified
// end-to-end in Renode by the cycle-237b review gate. The register
// addresses + bits below are the SAME ones the architect verified
// against the actual Renode STM32F7_USART.cs model source.

/// The `Usart1Shim` impl emitted into the generated BIN's `main.rs`
/// (M10, TASK-0048.01 / TASK-0048.02). This is the CONCRETE
/// [`NucleusShim`] the M9 `StubShim` no-ops:
///
/// - `alloc_in_region` IS the REAL input path (TASK-0048.02): it hands
///   back a pointer into the Renode-injected input region (axiSram @
///   0x2400_0000, where the `.resc` `sysbus LoadBinary @input.bin`s the
///   fixture) and advances an internal byte cursor by `bytes`. The load
///   lowering then copies those `bytes` into the data array (see lib.rs
///   `render_fire`'s effectful-input arm). Sequential loads (`a <--
///   load_input(); b <-- load_input_b()`) consume the region in order,
///   matching `input.bin`'s layout (a's N words then b's N words —
///   exactly the advancing offsets `kernels.rs::read_i32_le_slice`
///   uses, WITHOUT this backend parsing kernel bodies).
/// - `dma_push` IS the UART emission: it streams the `len` RAW bytes of
///   the drained output region (the `save_output(c)` effectful Fire
///   lowers to `shim.dma_push(0, c.as_ptr() as *const u8,
///   core::mem::size_of_val(&c)); shim.dma_wait(0)` — see lib.rs
///   `render_fire`) verbatim over USART1. The `renode-embedded`
///   recipe captures those bytes and `cmp`s them BYTE-EXACT against the
///   example's `reference.bin` (PRD §10.3 point 3). Raw (not ASCII): a byte-exact
///   reference diff is the value-correctness bar, and Renode's USART
///   file backend captures raw bytes faithfully (proven in the
///   TASK-0048.02 de-risk: a hand firmware streamed c and the capture
///   was `cmp -s`-identical to reference.bin).
/// - `dma_wait` / `irq_barrier` are no-ops: the injection is synchronous
///   (the region is populated before the CPU starts), so there is
///   nothing to block on. Real async DMA/IRQ is parent TASK-0048 AC#1.
/// - `monotonic_ns` (TASK-0048.04) IS the tier-3 monotonic clock (PRD
///   §6.3.5): it reads the Cortex-M SysTick 24-bit down-counter and
///   accumulates elapsed ticks across calls into a monotonically
///   increasing ns reading. SysTick (not DWT CYCCNT) because Renode
///   models SysTick reliably whereas DWT CYCCNT may not advance under
///   Renode's non-cycle-accurate timing (docs/check-loop-latency-max.md
///   §3). cycles→ns uses `SYSTEM_CORE_CLOCK_HZ`, a CALIBRATION ASSUMPTION
///   pinned to the STM32H7 HSI reset default (64 MHz); under Renode the
///   ns figure is NOT physically meaningful (Renode is not cycle-
///   accurate) — what tier-3 verifies is the lowering correctness + that
///   the clock ADVANCES, not timing fidelity.
///
/// REGISTERS (STM32H7 USART1 @ 0x4001_1000; Renode UART.STM32F7_USART):
/// CR1 @ +0x00 (UE bit0, TE bit3), ISR @ +0x1C (TXE bit7), TDR @ +0x28.
/// Renode hardwires TXE=true (the TX poll never waits there; back-
/// pressure is UNVALIDATED — but the poll is the correct pattern, so we
/// emit it). CR1 = UE|TE enable IS load-bearing (the model DROPS bytes
/// if the transmitter is not enabled).
///
/// INPUT REGION: axiSram @ 0x2400_0000 (512K, mapped in the platform but
/// NOT in `memory.x`, so the firmware's stack/.bss never collide with
/// the injected fixture). The `.resc` injects the fixture there before
/// the CPU runs.
pub const USART1_SHIM_SRC: &str = "\
// STM32H7 USART1 — Renode models it as UART.STM32F7_USART @ 0x4001_1000
// (platforms/cpus/stm32h743.repl). Register layout (STM32F7/H7):
//   CR1 @ +0x00  (UE = bit 0, TE = bit 3)
//   ISR @ +0x1C  (TXE = bit 7: transmit data register empty)
//   TDR @ +0x28  (transmit data register)
const USART1_CR1: *mut u32 = 0x4001_1000 as *mut u32;
const USART1_ISR: *const u32 = 0x4001_101C as *const u32;
const USART1_TDR: *mut u32 = 0x4001_1028 as *mut u32;
const USART1_TXE: u32 = 1 << 7;

// The Renode-injected input region: axiSram @ 0x2400_0000 (mapped in the
// stm32h743 platform, NOT in memory.x — so the linker never places the
// stack/.bss here and the injected fixture is safe). The `.resc` does
// `sysbus LoadBinary @input.bin 0x2400_0000` BEFORE the CPU starts, so by
// the time `run` executes the bytes are already present (no DMA wait).
const NUC_INPUT_REGION: *const u8 = 0x2400_0000 as *const u8;

// --- Tier-3 monotonic clock: Cortex-M SysTick (TASK-0048.04) ---------
// SysTick is the ARMv7-M architectural 24-bit DOWN-counter in the System
// Control Space (same on every Cortex-M; Renode models it reliably,
// unlike DWT CYCCNT under its non-cycle-accurate timing).
//   SYST_CSR @ 0xE000_E010  (ENABLE bit0, TICKINT bit1, CLKSOURCE bit2,
//                            COUNTFLAG bit16: set when the counter reached
//                            0 since last read; reading CSR clears it)
//   SYST_RVR @ 0xE000_E014  (reload value, 24-bit)
//   SYST_CVR @ 0xE000_E018  (current value, 24-bit; writing clears it +
//                            COUNTFLAG)
const SYST_CSR: *mut u32 = 0xE000_E010 as *mut u32;
const SYST_RVR: *mut u32 = 0xE000_E014 as *mut u32;
const SYST_CVR: *mut u32 = 0xE000_E018 as *mut u32;
const SYST_RELOAD: u32 = 0x00FF_FFFF; // full 24-bit span
const SYST_COUNTFLAG: u32 = 1 << 16;
// CALIBRATION ASSUMPTION (TASK-0048.04): cycles->ns needs a core-clock
// frequency. Pinned to the STM32H7 HSI reset default (64 MHz). Under
// Renode this ns figure is NOT physically meaningful (Renode is not
// cycle-accurate); tier-3 verifies lowering correctness + that the clock
// ADVANCES, not timing fidelity. On real silicon, recalibrate to the
// configured SystemCoreClock.
const SYSTEM_CORE_CLOCK_HZ: u64 = 64_000_000;

/// Push one byte out over USART1. Renode's STM32F7_USART hardwires
/// TXE=true so this poll never actually waits there; on real silicon it
/// blocks until the transmit register drains. The poll is the correct
/// pattern regardless.
fn usart1_putc(b: u8) {
    unsafe {
        while core::ptr::read_volatile(USART1_ISR) & USART1_TXE == 0 {}
        core::ptr::write_volatile(USART1_TDR, b as u32);
    }
}

/// Push a byte slice out over USART1 (the `on_violation=log` sink — the
/// tier-3 analogue of tier-1's `eprintln!`, since no_std has no stderr).
fn usart1_puts(s: &[u8]) {
    let mut i = 0usize;
    while i < s.len() {
        usart1_putc(s[i]);
        i += 1;
    }
}

/// Write `n` as decimal ASCII over USART1 (no_std, alloc-free) — used by
/// the `on_violation=log` violation line to report the measured ns.
fn usart1_put_u64(mut n: u64) {
    if n == 0 {
        usart1_putc(b'0');
        return;
    }
    // Up to 20 decimal digits for a u64.
    let mut buf = [0u8; 20];
    let mut len = 0usize;
    while n > 0 {
        buf[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    // Digits were produced least-significant first; emit reversed.
    while len > 0 {
        len -= 1;
        usart1_putc(buf[len]);
    }
}

/// Enable SysTick as a free-running monotonic source: reload = full
/// 24-bit, processor clock, counting, no interrupt. Called once from
/// `main` before `run`. Idempotent.
fn systick_init() {
    unsafe {
        core::ptr::write_volatile(SYST_RVR, SYST_RELOAD);
        core::ptr::write_volatile(SYST_CVR, 0); // clears CVR + COUNTFLAG
        // ENABLE (bit0) | CLKSOURCE=processor (bit2). TICKINT stays 0.
        core::ptr::write_volatile(SYST_CSR, (1 << 0) | (1 << 2));
    }
}

/// The CONCRETE shim for the STM32H7 Renode target.
///
/// - `alloc_in_region` is the REAL input path: it returns a pointer into
///   the injected input region (`NUC_INPUT_REGION`) at the current cursor
///   and advances the cursor by `bytes`. Sequential loads consume the
///   region in order (matching `input.bin`'s array-concatenation layout).
/// - `dma_push` is the UART emission: it streams the `len` RAW bytes of
///   the drained region verbatim over USART1 (captured + `cmp`'d
///   byte-exact vs reference.bin by `just renode-embedded <example>`).
/// - `dma_wait` / `irq_barrier` are no-ops (synchronous injection; real
///   DMA/IRQ is parent TASK-0048 AC#1).
/// - `monotonic_ns` accumulates SysTick down-counter ticks into a
///   monotonic ns reading (TASK-0048.04).
struct Usart1Shim {
    // Byte offset into NUC_INPUT_REGION consumed so far. Each effectful
    // load advances it by the loaded array's byte length.
    input_cursor: usize,
    // --- SysTick monotonic-clock accumulator state (TASK-0048.04) ---
    // SysTick counts DOWN and reloads; to expose a monotonic UP clock we
    // accumulate ticks elapsed since the previous `monotonic_ns` call.
    // `clock_started` is false until the first call (which seeds last_cvr
    // without counting a bogus initial delta); `accum_ticks` is the
    // running total elapsed tick count; `last_cvr` is the CVR at the
    // previous call.
    clock_started: bool,
    accum_ticks: u64,
    last_cvr: u32,
}

impl Usart1Shim {
    fn new() -> Self {
        Usart1Shim {
            input_cursor: 0,
            clock_started: false,
            accum_ticks: 0,
            last_cvr: 0,
        }
    }
}

impl NucleusShim for Usart1Shim {
    fn alloc_in_region(&mut self, _region: usize, bytes: usize) -> *mut u8 {
        // Hand back the next `bytes`-sized slice of the injected input
        // region and advance the cursor. The load lowering copies from
        // this pointer into the data array. (Cast away const: the load
        // lowering only READS through it; mut is the trait's contract.)
        // ASSUMES input.bin == the effectful-load arrays concatenated in
        // EventList load order. Exact for ex1 (two loads: a then b).
        // CONFIRMED single-load for ex5 (load_image -> img_in) and ex9
        // (load_input -> seeds) in TASK-0048.03: exactly one effectful
        // load, so the cursor starts at 0 and consumes the whole injected
        // region trivially — the concatenation-order assumption is only
        // exercised by ex1's two loads (TASK-0048.06).
        let p = unsafe { NUC_INPUT_REGION.add(self.input_cursor) } as *mut u8;
        self.input_cursor += bytes;
        p
    }
    fn dma_push(&mut self, _chan: usize, src: *const u8, len: usize) {
        // Stream the drained output region's RAW bytes verbatim over
        // USART1. The `renode-embedded` recipe captures these and
        // `cmp`s them BYTE-EXACT against reference.bin (PRD §10.3 point 3
        // value-correctness). `read_volatile` so the byte loads are not
        // reordered/elided across the MMIO writes in `usart1_putc`.
        let mut i = 0usize;
        while i < len {
            let byte = unsafe { core::ptr::read_volatile(src.add(i)) };
            usart1_putc(byte);
            i += 1;
        }
    }
    fn dma_wait(&mut self, _chan: usize) {}
    fn irq_barrier(&mut self, _tag: u32) {}
    fn monotonic_ns(&mut self) -> u64 {
        // Read SysTick's CVR (down-counter) + COUNTFLAG, accumulate the
        // ticks elapsed since the previous call, and convert to ns.
        // SysTick reloads from SYST_RELOAD when it hits 0; COUNTFLAG (set
        // on the wrap, cleared by reading CSR) tells us whether AT LEAST
        // one reload happened since the previous read.
        let (csr, cvr) = unsafe {
            let csr = core::ptr::read_volatile(SYST_CSR); // reading clears COUNTFLAG
            let cvr = core::ptr::read_volatile(SYST_CVR);
            (csr, cvr)
        };
        if !self.clock_started {
            // First call: seed last_cvr; no delta yet (avoids a bogus
            // initial span). Returns 0 — the loop's `_start` reading.
            self.clock_started = true;
            self.last_cvr = cvr;
            return 0;
        }
        let wrapped = (csr & SYST_COUNTFLAG) != 0;
        // Ticks elapsed since last_cvr. Counter goes DOWN, so without a
        // wrap the delta is last_cvr - cvr. With a wrap the counter went
        // last_cvr -> 0 (last_cvr ticks) then RELOAD -> cvr ((RELOAD+1) -
        // cvr ticks). LIMIT: COUNTFLAG only says >=1 wrap; if >1 reload
        // occurred between calls (a span longer than ~0.26s at 64 MHz)
        // this UNDER-counts. The check-loop bodies are microsecond-class,
        // so a single span never spans >1 reload — documented limit.
        let delta = if wrapped {
            (self.last_cvr as u64) + ((SYST_RELOAD as u64) + 1 - (cvr as u64))
        } else if self.last_cvr >= cvr {
            (self.last_cvr - cvr) as u64
        } else {
            // CVR went UP without COUNTFLAG: should not happen on real
            // SysTick (monotonic down between reloads). Treat as a missed
            // wrap rather than a negative delta.
            (self.last_cvr as u64) + ((SYST_RELOAD as u64) + 1 - (cvr as u64))
        };
        self.accum_ticks += delta;
        self.last_cvr = cvr;
        // ticks -> ns. Multiply first (u64 headroom: accum_ticks * 1e9
        // overflows only past ~1.8e10 ticks ~= 287s at 64 MHz; bounded
        // runs stay well under). On real silicon recalibrate the clock.
        self.accum_ticks * 1_000_000_000 / SYSTEM_CORE_CLOCK_HZ
    }
    fn report_violation(&mut self, loop_var: &[u8], measured_ns: u64, threshold_ns: u64) {
        // The tier-3 `on_violation=log` sink: a one-line UART message, the
        // no_std analogue of tier-1's `eprintln!`. Captured by the same
        // Renode USART1 file backend the firmware's output uses. NOTE: if
        // a schedule ALSO streams raw output bytes over the SAME USART1
        // (e.g. ex1's save_output), the captured stream interleaves the
        // ASCII violation lines with the raw output bytes — distinguishable
        // by the `check loop ` ASCII prefix, but a real deployment would
        // route diagnostics to a separate channel (a 2nd UART / RTT). That
        // separation is a TASK-0048.08 follow-up; M10 streams both on USART1.
        usart1_puts(b\"check loop `\");
        usart1_puts(loop_var);
        usart1_puts(b\"` violated latency_max=\");
        usart1_put_u64(threshold_ns);
        usart1_puts(b\" ns: iteration took \");
        usart1_put_u64(measured_ns);
        usart1_puts(b\" ns\\n\");
    }
}
";

/// Assemble the full Renode-runnable `src/main.rs` for the BIN target
/// (M10, TASK-0048.01). Mirrors the proven template at
/// tests/renode/uart-smoke/src/main.rs: a single self-contained
/// `#![no_std]` / `#![no_main]` file with a cortex-m-rt `#[entry]`, a
/// `#[panic_handler]`, the `NucleusShim` trait + `StubShim` (verbatim
/// reuse of [`NUCLEUS_SHIM_SRC`] so the trait surface is IDENTICAL to
/// the lib path), `mod kernels` (the verbatim pure bodies), the lowered
/// `run<S>`, and the concrete [`USART1_SHIM_SRC`] `Usart1Shim`.
///
/// One `on_violation=count` check loop's summary descriptor (TASK-0048.08,
/// PART 1). [`render_bin_main`] emits, per descriptor, both the
/// module-scope `AtomicU32` static (via the ident) and a one-line USART1
/// summary in the `#[entry]` after `run` returns (using `loop_var` +
/// `latency_max_ns`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountSummary {
    /// Sanitised ident — names the `NUC_CHECK_COUNT_<ident>` static.
    pub ident: String,
    /// Original loop-variable name, printed verbatim in the summary line.
    pub loop_var: String,
    /// Threshold in nanoseconds, printed in the summary line.
    pub latency_max_ns: u64,
}

/// `kernel_defs` is the already-extracted pure kernel fn text. `run_body`
/// is the rendered body of `fn run<S: NucleusShim>(shim: &mut S)` — the
/// SAME body the lib path emits (the only difference between lib and bin
/// is the surrounding scaffolding, not the lowering). `count_summaries`
/// are the `on_violation=count` check loops: each contributes a
/// module-scope `AtomicU32` static AND a one-line USART1 summary emitted
/// AFTER `run(&mut shim)` returns and BEFORE the `loop {}` spin — the
/// bare-metal program-exit equivalent of the tier-1 Drop-guard summary
/// (TASK-0048.08, PART 1).
pub fn render_bin_main(
    kernel_defs: &str,
    run_body: &str,
    count_summaries: &[CountSummary],
) -> String {
    let mut s = String::new();
    s.push_str(
        "//! Generated by the nucleus pre-compiler (embedded-pattern, M10 BIN).\n\
         //! Do not edit; rerun `nucleus build --shim stm32h7` to regenerate.\n\
         //!\n\
         //! A Renode-runnable `no_std` BIN for the STM32H7 (Cortex-M7).\n\
         //! Boots via cortex-m-rt, loads REAL input from the Renode-injected\n\
         //! region (axiSram @ 0x2400_0000), calls the lowered `run`, and the\n\
         //! effectful save Fire's `dma_push` streams the RAW output bytes\n\
         //! over USART1 (captured + diffed BYTE-EXACT vs reference.bin in\n\
         //! Renode). Mirrors tests/renode/uart-smoke/ (the proven M10 template).\n\
         #![no_std]\n\
         #![no_main]\n\
         // non_upper_case_globals: the Count counter statics preserve the\n\
         // source loop-var spelling (e.g. lowercase `i`), matching the\n\
         // tier-1 convention and keeping the name greppable back to the\n\
         // directive (TASK-0048.08).\n\
         #![allow(unused_mut, dead_code, unused_variables, non_upper_case_globals)]\n\
         \n\
         use core::panic::PanicInfo;\n\
         use cortex_m_rt::entry;\n\
         \n",
    );
    s.push_str(NUCLEUS_SHIM_SRC);
    s.push('\n');
    s.push_str(USART1_SHIM_SRC);
    s.push('\n');
    // The Count violation counters — IDENTICAL statics to the lib path
    // (the Count arm in render_event references NUC_CHECK_COUNT_<ident> on
    // both paths). On the bin path they ARE flushed: the `#[entry]` below
    // emits a USART1 summary after `run` returns (TASK-0048.08).
    let count_idents: Vec<&str> = count_summaries.iter().map(|c| c.ident.as_str()).collect();
    render_count_statics(&mut s, &count_idents);
    // The pure kernel bodies live in a private `mod kernels` — the same
    // `kernels::<name>(..)` call spelling the run body emits (identical
    // to the lib path).
    s.push_str("/// Pure compute kernels, copied verbatim from the source\n");
    s.push_str("/// kernels.rs (PRD \u{00A7}6.2.2: Nucleus does not interpolate kernel\n");
    s.push_str("/// bodies). The effectful I/O kernels are NOT here — they map to\n");
    s.push_str("/// `NucleusShim` hooks in `run` below.\n");
    s.push_str("mod kernels {\n");
    for line in kernel_defs.lines() {
        if line.is_empty() {
            s.push('\n');
        } else {
            s.push_str("    ");
            s.push_str(line);
            s.push('\n');
        }
    }
    s.push_str("}\n\n");
    // The lowered run — identical body to the lib path.
    s.push_str(
        "/// Lower the single-worker event list. Generic over any\n\
         /// [`NucleusShim`]; the BIN instantiates it with the concrete\n\
         /// `Usart1Shim` (whose `dma_push` IS the UART emission).\n\
         pub fn run<S: NucleusShim>(shim: &mut S) {\n",
    );
    s.push_str(run_body);
    s.push_str("}\n\n");
    // The cortex-m-rt entry point: enable USART1, build the concrete
    // shim, run, then spin. Inside `run` the load Fires fill the arrays
    // from the injected input region (via the shim) and the save Fire's
    // dma_push streams the RAW computed output bytes over USART1.
    s.push_str(
        "#[entry]\n\
         fn main() -> ! {\n    \
             unsafe {\n        \
                 // Enable the USART (UE = bit 0) and its transmitter (TE = bit 3).\n        \
                 // This enable IS load-bearing under Renode (the model drops bytes\n        \
                 // if the transmitter is not enabled).\n        \
                 core::ptr::write_volatile(USART1_CR1, (1 << 0) | (1 << 3));\n    \
             }\n    \
             // Start the SysTick monotonic clock (TASK-0048.04) before run so a\n    \
             // `check loop` frame's `shim.monotonic_ns()` reads a running counter.\n    \
             // Harmless no-op cost when the schedule carries no check_frame.\n    \
             systick_init();\n    \
             let mut shim = Usart1Shim::new();\n    \
             run(&mut shim);\n",
    );
    // PROGRAM-EXIT SINK for on_violation=count (TASK-0048.08, PART 1): the
    // bare-metal equivalent of the tier-1 Drop-guard summary, emitted HERE
    // (after `run` returns, before `loop {}`) because a firmware spins
    // forever so a Rust `Drop` at `main` return never fires. One USART1
    // line PER count check loop, reporting the EXACT violation count. The
    // summary shares USART1 with raw output (same documented wart as the
    // log sink): these lines come AFTER `run` has streamed all raw output
    // bytes, distinguishable by the `check loop ` ASCII prefix. A SEPARATE
    // physical diagnostic channel (2nd UART / RTT / SWO) is the deferred
    // PART-2 follow-up (TASK-0048.09).
    if !count_summaries.is_empty() {
        s.push_str(
            "    // TASK-0048.08: per-`check loop` Count summary over USART1 (the\n    \
                 // bare-metal program-exit sink; firmware spins forever so a Drop\n    \
                 // at `main` return never fires).\n",
        );
        for cs in count_summaries {
            // The loop_var is a parsed identifier ([A-Za-z0-9_]) so it is
            // already a valid byte-string-literal body; no escaping needed.
            s.push_str(&format!(
                "    usart1_puts(b\"check loop `{lv}` violated latency_max=\");\n    \
                 usart1_put_u64({ns});\n    \
                 usart1_puts(b\" ns: \");\n    \
                 usart1_put_u64(\
                   NUC_CHECK_COUNT_{id}.load(core::sync::atomic::Ordering::Relaxed) as u64);\n    \
                 usart1_puts(b\" occurrence(s)\\n\");\n",
                lv = cs.loop_var,
                ns = cs.latency_max_ns,
                id = cs.ident,
            ));
        }
    }
    s.push_str(
        "    loop {}\n\
         }\n\
         \n\
         #[panic_handler]\n\
         fn panic(_: &PanicInfo) -> ! {\n    \
             loop {}\n\
         }\n",
    );
    s
}

/// The generated BIN project's `Cargo.toml` (M10, TASK-0048.01). The
/// OPPOSITE of [`render_cargo_toml`]'s lib shape: it HAS a `[[bin]]`,
/// the `cortex-m-rt` dependency, and `panic = "abort"` profiles (a
/// runnable no_std bin needs all three — there is no unwinder on bare
/// metal). Standalone empty `[workspace]` so it is NOT pulled into the
/// nucleus/ cargo workspace (otherwise `just build` / `just ci` would
/// try to build ARM-only code on the host and fail). Mirrors
/// tests/renode/uart-smoke/Cargo.toml.
pub fn render_bin_cargo_toml() -> String {
    String::from(
        "# Generated by the nucleus pre-compiler (embedded-pattern, M10 BIN). Do not edit; \
         rerun `nucleus build --shim stm32h7` to regenerate.\n\
         [package]\n\
         name        = \"nuc-embedded-generated\"\n\
         version     = \"0.0.0\"\n\
         edition     = \"2021\"\n\
         publish     = false\n\
         \n\
         [[bin]]\n\
         name = \"nuc-embedded-generated\"\n\
         path = \"src/main.rs\"\n\
         \n\
         [dependencies]\n\
         cortex-m-rt = \"0.7\"\n\
         \n\
         [profile.dev]\n\
         panic = \"abort\"\n\
         \n\
         [profile.release]\n\
         panic = \"abort\"\n\
         \n\
         [workspace]\n\
         # Empty: this crate is standalone, not part of any parent workspace\n\
         # (it cross-compiles only for thumbv7em-none-eabihf).\n",
    )
}

/// The STM32H743 `memory.x` linker fragment (M10, TASK-0048.01).
/// Mirrors tests/renode/uart-smoke/memory.x EXACTLY: 128K FLASH @
/// 0x08000000, 128K RAM @ 0x20000000 (the FULL DTCM). DO NOT raise RAM
/// past 128K without mapping a larger region (axiSram @ 0x24000000), or
/// the stack would silently overflow DTCM with no linker error.
pub fn render_memory_x() -> String {
    String::from(
        "/* STM32H743 memory map (from Renode platforms/cpus/stm32h743.repl):\n\
        \x20  flashBank1 @ 0x08000000 (size 0x100000 = 1024K) and DTCM @ 0x20000000\n\
        \x20  (size 0x20000 = 128K). FLASH here is a strict subset (128K of 1024K).\n\
        \x20  RAM here is the FULL DTCM (128K == 0x20000) — do NOT raise RAM LENGTH\n\
        \x20  past 128K without mapping a larger region (e.g. axiSram @ 0x24000000),\n\
        \x20  or the stack would silently overflow DTCM with no linker error. */\n\
        MEMORY\n\
        {\n\
        \x20 FLASH : ORIGIN = 0x08000000, LENGTH = 128K\n\
        \x20 RAM   : ORIGIN = 0x20000000, LENGTH = 128K\n\
        }\n",
    )
}

/// The generated BIN's `build.rs` (M10, TASK-0048.01). Puts `memory.x`
/// on the linker search path so cortex-m-rt's `link.x` finds it.
/// Mirrors tests/renode/uart-smoke/build.rs.
pub fn render_build_rs() -> String {
    String::from(
        "// Put memory.x on the linker search path so cortex-m-rt's link.x finds it.\n\
        use std::env;\n\
        use std::fs;\n\
        use std::path::PathBuf;\n\
        \n\
        fn main() {\n    \
            let out = PathBuf::from(env::var(\"OUT_DIR\").unwrap());\n    \
            fs::write(out.join(\"memory.x\"), include_bytes!(\"memory.x\")).unwrap();\n    \
            println!(\"cargo:rustc-link-search={}\", out.display());\n    \
            println!(\"cargo:rerun-if-changed=memory.x\");\n    \
            println!(\"cargo:rerun-if-changed=build.rs\");\n\
        }\n",
    )
}

/// The generated BIN's `.cargo/config.toml` (M10, TASK-0048.01).
/// Defaults the target to thumbv7em-none-eabihf and links via
/// cortex-m-rt's bundled `link.x`. Mirrors
/// tests/renode/uart-smoke/.cargo/config.toml.
pub fn render_cargo_config() -> String {
    String::from(
        "# cortex-m-rt links via its bundled link.x (which pulls in memory.x from\n\
        # the build-script search path). Default the target so a bare `cargo\n\
        # build` cross-compiles for the Cortex-M7.\n\
        [target.thumbv7em-none-eabihf]\n\
        rustflags = [\"-C\", \"link-arg=-Tlink.x\"]\n\
        \n\
        [build]\n\
        target = \"thumbv7em-none-eabihf\"\n",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_toml_is_a_lib_not_a_bin() {
        let s = render_cargo_toml();
        assert!(s.contains("[lib]"), "embedded Cargo.toml must declare [lib]");
        assert!(
            !s.contains("[[bin]]"),
            "embedded Cargo.toml must NOT declare a [[bin]] — compile-only no_std lib"
        );
        assert!(
            !s.contains("panic"),
            "a no_std lib needs no panic profile for cargo check"
        );
    }

    #[test]
    fn shim_trait_declares_all_six_methods() {
        // The six shim methods are present in the canonical trait source:
        // the four M9 methods + the two M10 TASK-0048.04 tier-3 methods
        // (monotonic_ns clock + report_violation log sink).
        for m in [
            "fn alloc_in_region",
            "fn dma_push",
            "fn dma_wait",
            "fn irq_barrier",
            "fn monotonic_ns",
            "fn report_violation",
        ] {
            assert!(
                NUCLEUS_SHIM_SRC.contains(m),
                "NucleusShim trait missing method {m}"
            );
        }
        assert!(
            NUCLEUS_SHIM_SRC.contains("struct StubShim"),
            "StubShim impl missing"
        );
        // The StubShim's clock is honestly inert (M9 AC#6 "no real timing"):
        // monotonic_ns returns 0 so a lib carrying a check_frame still
        // compiles + never spuriously reports.
        assert!(
            NUCLEUS_SHIM_SRC.contains("fn monotonic_ns(&mut self) -> u64 {\n        0\n    }"),
            "StubShim monotonic_ns must return 0 (compile-only, no real timing)"
        );
    }

    #[test]
    fn render_lib_emits_no_std_and_run() {
        // No count check loops → no AtomicU32 statics (byte-unchanged shape).
        let s = render_lib("pub fn add(a: i32, b: i32) -> i32 { a }\n", "    // body\n", &[]);
        assert!(s.starts_with("//! Generated by the nucleus pre-compiler"));
        assert!(s.contains("#![no_std]"));
        assert!(s.contains("pub trait NucleusShim"));
        assert!(s.contains("mod kernels {"));
        // The kernel def is indented inside the mod.
        assert!(s.contains("    pub fn add(a: i32, b: i32) -> i32 { a }"));
        assert!(s.contains("pub fn run<S: NucleusShim>(shim: &mut S) {"));
        // With no count loop, NO counter static is emitted.
        assert!(
            !s.contains("NUC_CHECK_COUNT_"),
            "no count check loop must emit no AtomicU32 static"
        );
    }

    #[test]
    fn render_lib_emits_atomic_u32_static_for_count_loop() {
        // TASK-0048.08: the lib path emits the module-scope AtomicU32
        // counter so the lib cross-compiles when a count check frame is
        // present (no `main`, so it is never flushed — compile-only).
        let s = render_lib("pub fn add(a: i32, b: i32) -> i32 { a }\n", "    // body\n", &["i"]);
        assert!(
            s.contains(
                "static NUC_CHECK_COUNT_i: core::sync::atomic::AtomicU32 = \
                 core::sync::atomic::AtomicU32::new(0);"
            ),
            "lib path must emit the AtomicU32 counter static for a count loop:\n{s}"
        );
        // AtomicU64 is unavailable on thumbv7em — must NOT appear.
        assert!(
            !s.contains("AtomicU64"),
            "AtomicU64 (absent on thumbv7em) must NOT appear in the lib:\n{s}"
        );
    }

    // ---- M10 (TASK-0048.01) bin-emit template tests ----

    #[test]
    fn render_bin_cargo_toml_is_a_runnable_bin() {
        // The bin shape is the OPPOSITE of the lib: it HAS a [[bin]], a
        // cortex-m-rt dep, and panic=abort profiles, and is its own
        // empty [workspace].
        let s = render_bin_cargo_toml();
        assert!(s.contains("[[bin]]"), "bin Cargo.toml must declare [[bin]]");
        assert!(
            s.contains("cortex-m-rt = \"0.7\""),
            "bin Cargo.toml must depend on cortex-m-rt 0.7"
        );
        assert!(
            s.contains("[profile.dev]") && s.contains("[profile.release]"),
            "bin Cargo.toml must set panic=abort on both profiles"
        );
        assert!(
            s.matches("panic = \"abort\"").count() == 2,
            "panic=abort must appear on both profiles"
        );
        assert!(
            s.contains("[workspace]"),
            "bin Cargo.toml must be its own empty [workspace] (isolation)"
        );
        assert!(
            !s.contains("[lib]"),
            "bin Cargo.toml must NOT declare [lib]"
        );
    }

    #[test]
    fn render_bin_main_is_a_runnable_firmware() {
        let s = render_bin_main(
            "pub fn add(a: i32, b: i32) -> i32 { a.wrapping_add(b) }\n",
            "    let mut c: [i32; 256] = [0; 256];\n",
            &[],
        );
        // no_std / no_main firmware header.
        assert!(s.contains("#![no_std]"), "bin main must be no_std");
        assert!(s.contains("#![no_main]"), "bin main must be no_main");
        // cortex-m-rt entry + panic handler (a runnable bin needs both;
        // the lib has NEITHER — keep the docs honest per class).
        assert!(s.contains("use cortex_m_rt::entry;"), "missing cortex-m-rt import");
        assert!(s.contains("#[entry]"), "missing #[entry]");
        assert!(s.contains("#[panic_handler]"), "missing #[panic_handler]");
        // The SAME trait surface as the lib (verbatim reuse).
        assert!(s.contains("pub trait NucleusShim"), "trait missing");
        assert!(s.contains("struct StubShim"), "StubShim missing (verbatim reuse)");
        // The concrete USART1 shim — its dma_push IS the UART emission.
        assert!(s.contains("struct Usart1Shim"), "Usart1Shim missing");
        assert!(
            s.contains("impl NucleusShim for Usart1Shim"),
            "Usart1Shim must impl NucleusShim"
        );
        // TASK-0048.02: dma_push streams RAW output bytes (the byte-exact
        // reference.bin diff is the value-correctness bar); the old ASCII
        // summary framing (NUC-EX1 / checksum) is GONE.
        assert!(
            s.contains("usart1_putc(byte);"),
            "dma_push must stream raw output bytes over USART1"
        );
        assert!(!s.contains("NUC-EX1"), "ASCII summary framing must be GONE");
        assert!(!s.contains("checksum="), "ASCII checksum framing must be GONE");
        // TASK-0048.02: the shim reads REAL input from the injected region.
        assert!(
            s.contains("const NUC_INPUT_REGION: *const u8 = 0x2400_0000 as *const u8;"),
            "shim must read injected input from axiSram @ 0x2400_0000"
        );
        assert!(
            s.contains("input_cursor"),
            "shim must track an input cursor across sequential loads"
        );
        // USART1 registers exactly as the proven template.
        assert!(s.contains("0x4001_1000"), "USART1_CR1 address");
        assert!(s.contains("0x4001_101C"), "USART1_ISR address");
        assert!(s.contains("0x4001_1028"), "USART1_TDR address");
        // main enables the USART then calls run.
        assert!(s.contains("fn main() -> !"), "missing entry main");
        assert!(s.contains("let mut shim = Usart1Shim::new();"), "main must build the shim");
        assert!(s.contains("run(&mut shim);"), "main must call run");
        // The kernel def is indented inside mod kernels (same as lib).
        assert!(s.contains("    pub fn add(a: i32, b: i32) -> i32 { a.wrapping_add(b) }"));
        assert!(s.contains("pub fn run<S: NucleusShim>(shim: &mut S) {"));
        // With no count loop, NO counter static and NO summary line.
        assert!(
            !s.contains("NUC_CHECK_COUNT_"),
            "no count check loop must emit no AtomicU32 static / summary"
        );
        // run -> spin, with nothing emitted between them.
        assert!(
            s.contains("run(&mut shim);\n    loop {}"),
            "no-count firmware must spin immediately after run:\n{s}"
        );
    }

    #[test]
    fn render_bin_main_emits_count_static_and_program_exit_summary() {
        // TASK-0048.08, PART 1: a count check loop drives (a) the
        // module-scope AtomicU32 static and (b) a USART1 summary emitted
        // AFTER run returns and BEFORE the loop {} spin (the bare-metal
        // program-exit sink).
        let s = render_bin_main(
            "pub fn add(a: i32, b: i32) -> i32 { a.wrapping_add(b) }\n",
            "    let mut c: [i32; 256] = [0; 256];\n",
            &[CountSummary {
                ident: "i".to_string(),
                loop_var: "i".to_string(),
                latency_max_ns: 1,
            }],
        );
        // (a) module-scope AtomicU32 static.
        assert!(
            s.contains(
                "static NUC_CHECK_COUNT_i: core::sync::atomic::AtomicU32 = \
                 core::sync::atomic::AtomicU32::new(0);"
            ),
            "bin path must emit the AtomicU32 counter static:\n{s}"
        );
        assert!(!s.contains("AtomicU64"), "AtomicU64 must NOT appear:\n{s}");
        // (b) the summary reads the counter and prints over USART1.
        assert!(
            s.contains(
                "usart1_put_u64(NUC_CHECK_COUNT_i.load(core::sync::atomic::Ordering::Relaxed) \
                 as u64);"
            ),
            "summary must load the AtomicU32 counter and print it:\n{s}"
        );
        assert!(
            s.contains("usart1_puts(b\"check loop `i` violated latency_max=\");"),
            "summary line must name the loop var + threshold:\n{s}"
        );
        assert!(
            s.contains("usart1_puts(b\" occurrence(s)\\n\");"),
            "summary line must report occurrence count:\n{s}"
        );
        // CRITICAL ordering: the summary is AFTER run and BEFORE loop {}.
        let run_at = s.find("run(&mut shim);").expect("run call present");
        let summary_at = s
            .find("NUC_CHECK_COUNT_i.load")
            .expect("summary present");
        let spin_at = s.rfind("loop {}").expect("spin present");
        assert!(
            run_at < summary_at && summary_at < spin_at,
            "summary must be emitted AFTER run and BEFORE the loop {{}} spin"
        );
        // No Drop guard / std atomic machinery (the tier-1 sink does NOT port).
        assert!(
            !s.contains("Drop for") && !s.contains("std::sync::atomic"),
            "the tier-1 Drop-guard / std-atomic count sink must NOT appear:\n{s}"
        );
    }

    #[test]
    fn memory_x_pins_128k_flash_and_ram() {
        let s = render_memory_x();
        assert!(s.contains("ORIGIN = 0x08000000, LENGTH = 128K"), "FLASH region");
        assert!(s.contains("ORIGIN = 0x20000000, LENGTH = 128K"), "RAM region");
    }

    #[test]
    fn build_rs_and_cargo_config_target_thumbv7em() {
        let b = render_build_rs();
        assert!(b.contains("memory.x"), "build.rs must wire memory.x");
        assert!(b.contains("rustc-link-search"), "build.rs must add link search");
        let c = render_cargo_config();
        assert!(
            c.contains("[target.thumbv7em-none-eabihf]"),
            "config must target thumbv7em-none-eabihf"
        );
        assert!(c.contains("-Tlink.x"), "config must link via link.x");
    }
}
