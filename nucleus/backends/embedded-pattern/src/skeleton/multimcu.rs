//! M11 multi-MCU BIN templates (TASK-0049.05, slice B): the
//! Renode-runnable `no_std` firmware for ONE worker of a multi-worker
//! schedule, with a CONCRETE inter-MCU UART-hub transport shim.
//!
//! ADDITIVE: separate from the SINGLE-worker M10 templates in [`super`]
//! ([`super::render_bin_main`] / [`super::USART1_SHIM_SRC`] /
//! `Usart1Shim`), which are UNCHANGED — a single-worker schedule still
//! emits exactly that. The multi-MCU bin reuses the SAME trait surface
//! ([`super::NUCLEUS_SHIM_SRC`]) and the SAME lowered `run<S>` body
//! (`render::render_run_body`); only the concrete shim differs:
//!
//!   * `link_push(seq, ..)` -> TX the bytes over the USART that channel
//!     `seq` rides on (poll TXE, write TDR);
//!   * `link_recv(seq, dst, len)` -> BLOCKING RX poll-on-RXNE + read RDR,
//!     filling `dst` (the receive local) with exactly `len` bytes;
//!   * `dma_push(0, ..)` -> the effectful `save_output` raw USART1 stream
//!     (the output-capture worker only) — DISTINCT channel namespace from
//!     `link_push`, so peripheral IO and inter-MCU transport never collide
//!     on one channel id (TASK-0049.05 trap #1);
//!   * `alloc_in_region` -> the Renode-injected axiSram input region (the
//!     input-loading worker only);
//!   * `irq_barrier` -> a no-op (documented below).
//!
//! The per-`seq` -> USART base table and the set of USARTs the firmware
//! enables come from the [`crate::multimcu::WorkerPlan`], so the shim and
//! the generated `.resc` agree on the wiring by construction.

use crate::multimcu::{UsartSlot, WorkerPlan};

use super::{render_count_statics, CountSummary, NUCLEUS_SHIM_SRC};

/// Constant prelude shared by every multi-MCU worker bin: generic
/// (base-parameterised) USART helpers, the injected-input region, the
/// SysTick monotonic clock, and the diagnostic-line helpers. Only the
/// `MultiMcuShim` impl (its `link` USART table + `init` enables) is
/// generated per worker.
const MULTIMCU_SHIM_PRELUDE: &str = "\
// --- Generic STM32F7_USART register access (base-parameterised) --------
// Every modelled STM32H743 USART (usart1/2/3/6, uart4/5/7/8, lpuart1) has
// the same register layout: CR1 @ +0x00 (UE bit0, RE bit2, TE bit3),
// ISR @ +0x1C (RXNE bit5, TXE bit7), RDR @ +0x24, TDR @ +0x28.
const USART_CR1_OFF: usize = 0x00;
const USART_ISR_OFF: usize = 0x1C;
const USART_RDR_OFF: usize = 0x24;
const USART_TDR_OFF: usize = 0x28;
const USART_TXE: u32 = 1 << 7;
const USART_RXNE: u32 = 1 << 5;
const USART_CR1_UE: u32 = 1 << 0;
const USART_CR1_RE: u32 = 1 << 2;
const USART_CR1_TE: u32 = 1 << 3;

// USART1 (0x4001_1000) is the effectful-output capture stream (the M10
// raw-byte path the `renode-multimcu` recipe captures), reserved away from
// the cross-worker transport USART pool.
const USART1_BASE: usize = 0x4001_1000;

// The Renode-injected input region: axiSram @ 0x2400_0000 (mapped in the
// platform, NOT in memory.x). The `.resc` LoadBinary's the input fixture
// here BEFORE the CPU runs, so a loading worker reads it synchronously.
const NUC_INPUT_REGION: *const u8 = 0x2400_0000 as *const u8;

/// Enable a USART: write CR1 with the given enable bits.
fn usart_cr1(base: usize, val: u32) {
    unsafe { core::ptr::write_volatile((base + USART_CR1_OFF) as *mut u32, val); }
}
/// Blocking transmit of one byte (poll TXE, write TDR).
fn usart_putc(base: usize, b: u8) {
    unsafe {
        while core::ptr::read_volatile((base + USART_ISR_OFF) as *const u32) & USART_TXE == 0 {}
        core::ptr::write_volatile((base + USART_TDR_OFF) as *mut u32, b as u32);
    }
}
/// Blocking receive of one byte (poll RXNE, read RDR — reading dequeues).
fn usart_getc(base: usize) -> u8 {
    unsafe {
        while core::ptr::read_volatile((base + USART_ISR_OFF) as *const u32) & USART_RXNE == 0 {}
        core::ptr::read_volatile((base + USART_RDR_OFF) as *const u32) as u8
    }
}

// --- Tier-3 monotonic clock: Cortex-M SysTick (TASK-0048.04) -----------
// Same 24-bit down-counter the single-worker Usart1Shim uses; Renode
// models SysTick reliably (unlike DWT CYCCNT).
const SYST_CSR: *mut u32 = 0xE000_E010 as *mut u32;
const SYST_RVR: *mut u32 = 0xE000_E014 as *mut u32;
const SYST_CVR: *mut u32 = 0xE000_E018 as *mut u32;
const SYST_RELOAD: u32 = 0x00FF_FFFF;
const SYST_COUNTFLAG: u32 = 1 << 16;
const SYSTEM_CORE_CLOCK_HZ: u64 = 64_000_000;

fn systick_init() {
    unsafe {
        core::ptr::write_volatile(SYST_RVR, SYST_RELOAD);
        core::ptr::write_volatile(SYST_CVR, 0);
        core::ptr::write_volatile(SYST_CSR, (1 << 0) | (1 << 2));
    }
}

/// Write `n` as decimal ASCII over USART1 (no_std, alloc-free) — for the
/// on_violation=log/count diagnostic lines.
fn usart1_put_u64(mut n: u64) {
    if n == 0 { usart_putc(USART1_BASE, b'0'); return; }
    let mut buf = [0u8; 20];
    let mut len = 0usize;
    while n > 0 { buf[len] = b'0' + (n % 10) as u8; n /= 10; len += 1; }
    while len > 0 { len -= 1; usart_putc(USART1_BASE, buf[len]); }
}
fn usart1_puts(s: &[u8]) {
    let mut i = 0usize;
    while i < s.len() { usart_putc(USART1_BASE, s[i]); i += 1; }
}
";

/// Render the per-worker `MultiMcuShim` impl (the part that VARIES per
/// worker: the `seq` -> USART base table + which USARTs `init` enables).
fn render_multimcu_shim_impl(plan: &WorkerPlan) -> String {
    let mut s = String::new();

    // The seq -> USART base lookup (both link_push and link_recv consult it).
    s.push_str(
        "/// Map a cross-worker transport channel (`SeqTag`) to the USART it\n\
         /// rides on for THIS worker (one USART per peer; computed by the\n\
         /// host-side TransportPlan so it matches the generated .resc).\n\
         fn mmcu_link_base(seq: usize) -> usize {\n    match seq {\n",
    );
    for (seq, usart) in &plan.seq_usart {
        s.push_str(&format!(
            "        {seq} => 0x{:08X}, // {}\n",
            usart.base, usart.renode_name
        ));
    }
    // The `_` arm is UNREACHABLE by construction: every `seq` reaching
    // link_push/link_recv was registered in this table at codegen (the
    // host-side TransportPlan::map_seqs walks the SAME events that emit the
    // Push/Wait). A hit here is a nucleus codegen bug, never valid input —
    // so a panic (a brick on bare metal) is the correct invariant guard.
    s.push_str(
        "        // Unreachable: every link `seq` is registered above at codegen.\n\
         \x20       _ => panic!(\"nucleus codegen bug: no USART mapped for transport seq\"),\n    }\n}\n\n",
    );

    // The shim struct + impl.
    s.push_str(
        "/// Concrete inter-MCU UART-hub shim for one worker (TASK-0049.05).\n\
         struct MultiMcuShim {\n    \
             input_cursor: usize,\n    \
             clock_started: bool,\n    \
             accum_ticks: u64,\n    \
             last_cvr: u32,\n\
         }\n\
         impl MultiMcuShim {\n    \
             fn new() -> Self {\n        \
                 MultiMcuShim { input_cursor: 0, clock_started: false, accum_ticks: 0, last_cvr: 0 }\n    \
             }\n",
    );

    // init(): enable RX on every link USART FIRST (the start-gating
    // contract — RX must be on before any peer transmits to us), then the
    // output-capture USART1 (if this worker saves), then SysTick.
    s.push_str(
        "    /// Enable every USART this worker uses (RX-first so the\n    \
             /// .resc's receivers-first boot order makes RX-before-TX hold by\n    \
             /// construction), then start the SysTick clock.\n    \
             fn init(&mut self) {\n",
    );
    // Distinct link USART bases (deduped, deterministic order).
    let mut seen: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    let mut link_bases: Vec<UsartSlot> = Vec::new();
    for u in plan.peer_usart.values() {
        if seen.insert(u.base) {
            link_bases.push(*u);
        }
    }
    for u in &link_bases {
        s.push_str(&format!(
            "        usart_cr1(0x{:08X}, USART_CR1_UE | USART_CR1_RE | USART_CR1_TE); // {}\n",
            u.base, u.renode_name
        ));
    }
    if plan.saves_output {
        s.push_str(
            "        usart_cr1(USART1_BASE, USART_CR1_UE | USART_CR1_TE); // output capture\n",
        );
    }
    s.push_str("        systick_init();\n    }\n");

    // The NucleusShim impl.
    s.push_str(
        "}\n\
         impl NucleusShim for MultiMcuShim {\n    \
             fn alloc_in_region(&mut self, _region: usize, bytes: usize) -> *mut u8 {\n        \
                 // Effectful load: hand back the next slice of the injected\n        \
                 // axiSram input region + advance the cursor (the load lowering\n        \
                 // copies from it into the data array).\n        \
                 let p = unsafe { NUC_INPUT_REGION.add(self.input_cursor) } as *mut u8;\n        \
                 self.input_cursor += bytes;\n        \
                 p\n    \
             }\n    \
             fn dma_push(&mut self, _chan: usize, src: *const u8, len: usize) {\n        \
                 // Effectful save_output: stream the raw output bytes over\n        \
                 // USART1 (captured + diffed byte-exact vs reference.bin). This\n        \
                 // is the LOCAL-peripheral channel, never a cross-MCU link.\n        \
                 let mut i = 0usize;\n        \
                 while i < len {\n            \
                     let b = unsafe { core::ptr::read_volatile(src.add(i)) };\n            \
                     usart_putc(USART1_BASE, b);\n            \
                     i += 1;\n        \
                 }\n    \
             }\n    \
             fn dma_wait(&mut self, _chan: usize) {}\n    \
             fn link_push(&mut self, seq: usize, src: *const u8, len: usize) {\n        \
                 // Inter-MCU send: TX every byte over the channel's USART.\n        \
                 let base = mmcu_link_base(seq);\n        \
                 let mut i = 0usize;\n        \
                 while i < len {\n            \
                     let b = unsafe { core::ptr::read_volatile(src.add(i)) };\n            \
                     usart_putc(base, b);\n            \
                     i += 1;\n        \
                 }\n    \
             }\n    \
             fn link_recv(&mut self, seq: usize, dst: *mut u8, len: usize) {\n        \
                 // Inter-MCU receive: BLOCK until exactly `len` bytes have\n        \
                 // arrived on the channel's USART RX, filling `dst`. The\n        \
                 // blocking RX IS the data synchronisation (see irq_barrier).\n        \
                 let base = mmcu_link_base(seq);\n        \
                 let mut i = 0usize;\n        \
                 while i < len {\n            \
                     let b = usart_getc(base);\n            \
                     unsafe { core::ptr::write_volatile(dst.add(i), b); }\n            \
                     i += 1;\n        \
                 }\n    \
             }\n    \
             fn irq_barrier(&mut self, _tag: u64) {\n        \
                 // No-op: every cross-worker DATA dependency is already\n        \
                 // carried by a Push/Wait edge, and link_recv BLOCKS until the\n        \
                 // data arrives, so the data ordering is enforced by the\n        \
                 // transport itself. Event::Sync is a CONTROL-only barrier on\n        \
                 // these pipeline schedules; a no-op is correct here and is\n        \
                 // VERIFIED by the byte-exact reference.bin diff (not assumed).\n        \
                 // A schedule whose correctness needs a standalone control\n        \
                 // barrier (one ordering two workers' external IO with no\n        \
                 // subsuming data edge) is REJECTED LOUD at emit time by\n        \
                 // multimcu::verify_control_sync_subsumed (TASK-0049.05.01),\n        \
                 // so it can never reach this no-op silently; a real UART\n        \
                 // barrier protocol for that case is TASK-0049.05 follow-up.\n    \
             }\n",
    );
    // monotonic_ns + report_violation (SysTick; same logic as Usart1Shim).
    s.push_str(MULTIMCU_CLOCK_IMPL);
    s.push_str("}\n");
    s
}

/// The `monotonic_ns` + `report_violation` methods (identical SysTick
/// logic to the single-worker `Usart1Shim` — kept as a shared const so the
/// two cannot drift on the wrap-accounting arithmetic).
const MULTIMCU_CLOCK_IMPL: &str = "    \
    fn monotonic_ns(&mut self) -> u64 {\n        \
        let (csr, cvr) = unsafe {\n            \
            let csr = core::ptr::read_volatile(SYST_CSR);\n            \
            let cvr = core::ptr::read_volatile(SYST_CVR);\n            \
            (csr, cvr)\n        \
        };\n        \
        if !self.clock_started {\n            \
            self.clock_started = true;\n            \
            self.last_cvr = cvr;\n            \
            return 0;\n        \
        }\n        \
        let wrapped = (csr & SYST_COUNTFLAG) != 0;\n        \
        let delta = if wrapped {\n            \
            (self.last_cvr as u64) + ((SYST_RELOAD as u64) + 1 - (cvr as u64))\n        \
        } else if self.last_cvr >= cvr {\n            \
            (self.last_cvr - cvr) as u64\n        \
        } else {\n            \
            (self.last_cvr as u64) + ((SYST_RELOAD as u64) + 1 - (cvr as u64))\n        \
        };\n        \
        self.accum_ticks += delta;\n        \
        self.last_cvr = cvr;\n        \
        self.accum_ticks * 1_000_000_000 / SYSTEM_CORE_CLOCK_HZ\n    \
    }\n    \
    fn report_violation(&mut self, loop_var: &[u8], measured_ns: u64, threshold_ns: u64) {\n        \
        usart1_puts(b\"check loop `\");\n        \
        usart1_puts(loop_var);\n        \
        usart1_puts(b\"` violated latency_max=\");\n        \
        usart1_put_u64(threshold_ns);\n        \
        usart1_puts(b\" ns: iteration took \");\n        \
        usart1_put_u64(measured_ns);\n        \
        usart1_puts(b\" ns\\n\");\n    }\n";

/// Assemble one worker's complete multi-MCU `src/main.rs` (TASK-0049.05).
/// Same overall shape as [`super::render_bin_main`] (cortex-m-rt entry +
/// panic handler + trait + `mod kernels` + lowered `run`) but with the
/// concrete [`MultiMcuShim`] (real inter-MCU UART transport) instead of
/// the single-worker `Usart1Shim`.
pub fn render_multimcu_bin_main(
    plan: &WorkerPlan,
    kernel_defs: &str,
    run_body: &str,
    count_summaries: &[CountSummary],
) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "//! Generated by the nucleus pre-compiler (embedded-pattern, M11 multi-MCU BIN).\n\
         //! Do not edit; rerun `nucleus build --shim stm32h7` to regenerate.\n\
         //!\n\
         //! Worker `{}` of a multi-worker schedule: a Renode-runnable `no_std`\n\
         //! STM32H7 firmware that talks to its peer MCUs over UART-hub links\n\
         //! (link_push = TX, link_recv = blocking RX). Co-simulated with the\n\
         //! other workers' bins via the generated multi-machine `.resc`.\n\
         #![no_std]\n\
         #![no_main]\n\
         #![allow(unused_mut, dead_code, unused_variables, non_upper_case_globals)]\n\
         \n\
         use core::panic::PanicInfo;\n\
         use cortex_m_rt::entry;\n\
         \n",
        plan.name
    ));
    s.push_str(NUCLEUS_SHIM_SRC);
    s.push('\n');
    s.push_str(MULTIMCU_SHIM_PRELUDE);
    s.push('\n');
    s.push_str(&render_multimcu_shim_impl(plan));
    s.push('\n');

    let count_idents: Vec<&str> = count_summaries.iter().map(|c| c.ident.as_str()).collect();
    render_count_statics(&mut s, &count_idents);

    s.push_str("/// Pure compute kernels, copied verbatim from the source kernels.rs.\n");
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

    s.push_str(
        "/// Lower this worker's event list. Generic over any [`NucleusShim`];\n\
         /// instantiated with the concrete `MultiMcuShim` (real UART transport).\n\
         pub fn run<S: NucleusShim>(shim: &mut S) {\n",
    );
    s.push_str(run_body);
    s.push_str("}\n\n");

    s.push_str(
        "#[entry]\n\
         fn main() -> ! {\n    \
             let mut shim = MultiMcuShim::new();\n    \
             // Enable this worker's USARTs (RX-first) + SysTick BEFORE run, so\n    \
             // its receive channels are live before any peer transmits.\n    \
             shim.init();\n    \
             run(&mut shim);\n",
    );
    if !count_summaries.is_empty() {
        s.push_str(
            "    // TASK-0048.08: per-`check loop` Count summary over USART1.\n",
        );
        for cs in count_summaries {
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
