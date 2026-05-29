//! M10 (TASK-0048.01) bin-emit templates — the Renode-runnable no_std
//! BINARY shape. ADDITIVE: these are SEPARATE functions from the M9 lib
//! templates in the parent [`super`] module; the lib path
//! ([`super::render_cargo_toml`] / [`super::render_lib`]) is UNCHANGED.
//! The bin shape is the OPPOSITE of the lib in every honest respect (it
//! HAS a [[bin]], a panic profile, a panic_handler, a cortex-m-rt entry,
//! a linker script) — so the docs here state the bin facts, NOT the lib
//! facts (feedback-verbatim-copy-comment-doc-lie).
//!
//! These mirror the PROVEN hand-written template at
//! tests/renode/uart-smoke/ (commits b57d030 + 7de02f9), verified
//! end-to-end in Renode by the cycle-237b review gate. The register
//! addresses + bits below are the SAME ones the architect verified
//! against the actual Renode STM32F7_USART.cs model source.
//!
//! Re-exported via `pub use bin::*;` from [`super`] so the crate-root
//! call sites (`skeleton::render_bin_main`, `skeleton::USART1_SHIM_SRC`,
//! `skeleton::render_memory_x`, `skeleton::render_build_rs`,
//! `skeleton::render_cargo_config`, `skeleton::render_bin_cargo_toml`,
//! `skeleton::CountSummary`) resolve unchanged.

use super::{render_count_statics, NUCLEUS_SHIM_SRC};

/// The `Usart1Shim` impl emitted into the generated BIN's `main.rs`
/// (M10, TASK-0048.01 / TASK-0048.02). This is the CONCRETE
/// [`super::NucleusShim`] the M9 [`super::StubShim`] no-ops:
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
/// reuse of [`super::NUCLEUS_SHIM_SRC`] so the trait surface is IDENTICAL
/// to the lib path), `mod kernels` (the verbatim pure bodies), the
/// lowered `run<S>`, and the concrete [`USART1_SHIM_SRC`] `Usart1Shim`.
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
/// OPPOSITE of [`super::render_cargo_toml`]'s lib shape: it HAS a
/// `[[bin]]`, the `cortex-m-rt` dependency, and `panic = "abort"`
/// profiles (a runnable no_std bin needs all three — there is no unwinder
/// on bare metal). Standalone empty `[workspace]` so it is NOT pulled
/// into the nucleus/ cargo workspace (otherwise `just build` / `just ci`
/// would try to build ARM-only code on the host and fail). Mirrors
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
