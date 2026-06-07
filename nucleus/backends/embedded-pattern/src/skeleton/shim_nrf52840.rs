//! The SECOND-MCU-family concrete shim source (P10, TASK-0453.10), split
//! out of [`super::bin`] to keep that file under the 1000-LoC mega-file
//! fence (`just check-mega-files`). Holds only the `NrfUarteShim` source
//! string; the family-selection plumbing (`ShimTarget` / `bin_spec` /
//! `render_bin_main`) stays in [`super::bin`], which references this const
//! via `bin_spec(ShimTarget::Nrf52840)`.

/// The CONCRETE shim for the nRF52840 (Cortex-M4F) Renode target — the
/// SECOND MCU family (P10, TASK-0453.10). It implements the SAME
/// `NucleusShim` trait (`NUCLEUS_SHIM_SRC`) as the STM32 `Usart1Shim`
/// (`USART1_SHIM_SRC`), but the bodies drive nRF UARTE EasyDMA instead of
/// the STM32 USART/DMA1 — the genuine portability test of the trait.
///
/// - `alloc_in_region` IS the REAL input path: it returns a cursor into
///   the Renode-injected input region (a RAM window @ 0x2002_0000), same
///   contract as the STM32 shim.
/// - `dma_push` is the UART emission via a DIRECT UARTE0 EasyDMA transmit
///   of the computed output array. Unlike the STM32 shim there is NO
///   stage-copy: nRF EasyDMA reaches the whole RAM block, and the codegen
///   output array is already RAM-resident, so `src` is the DMA source
///   directly (the nRF single-RAM simplification over STM32's
///   DTCM->AXI-SRAM bridge).
/// - `dma_wait` / `irq_barrier` / `link_*` are no-ops (single-MCU; the
///   EasyDMA TX already blocks on EVENTS_ENDTX inside `dma_push`).
/// - `monotonic_ns` reads the architectural Cortex-M SysTick (IDENTICAL
///   mechanism to the STM32 shim — SysTick is the same System-Control-
///   Space peripheral on every Cortex-M); `report_violation` is the
///   on_violation=log UART sink.
///
/// VERIFICATION SCOPE (honest): the VALUE-transport path
/// (`alloc_in_region` / `dma_push` / `dma_wait` / `run`) is runtime-
/// verified byte-exact on this family (examples 1/5/9 under
/// `just renode-embedded-nrf`). The TIMING/DIAGNOSTIC path
/// (`systick_init` / `monotonic_ns` / `report_violation` /
/// `nrf_uarte_puts`) is structurally MIRRORED from the STM32 shim but NOT
/// separately runtime-exercised on nRF: examples 1/5/9 carry no
/// `check_frame`, so these are dead code under `#![allow(dead_code)]` here.
/// The STM32 shim's equivalent IS runtime-proven (`renode-embedded-check`);
/// an nRF analogue is the follow-up TASK-0453.10.01. (`SYSTEM_CORE_CLOCK_HZ`
/// is the nRF52840's 64 MHz core clock, which coincides with the STM32 HSI
/// reset default — under Renode the ns figure is not physically meaningful
/// regardless, only lowering correctness + clock advance are at stake.)
pub const NRF_UARTE_SHIM_SRC: &str = "\
// nRF52840 UARTE0 — Renode models it as UART.NRF52840_UART @ 0x4000_2000
// (platforms/cpus/nrf52840.repl, easyDMA: true). The transmit path is
// EasyDMA: a RAM source POINTER + byte COUNT, NOT the single-byte data
// register the STM32 USART uses — a genuinely DIFFERENT register interface,
// which is the point: the SAME generic `run<S>` + `NucleusShim` trait lower
// against a second family by swapping ONLY this concrete shim.
//   TASKS_STARTTX @ +0x008 (begin the DMA transmit)
//   EVENTS_ENDTX  @ +0x120 (set when the DMA transfer completes)
//   ENABLE        @ +0x500 (8 = UARTE / EasyDMA mode)
//   PSEL_TXD      @ +0x50C (TX pin select; Renode ignores routing)
//   BAUDRATE      @ +0x524
//   TXD_PTR       @ +0x544 (EasyDMA source: a RAM address)
//   TXD_MAXCNT    @ +0x548 (EasyDMA byte count)
const UARTE0_STARTTX: *mut u32 = 0x4000_2008 as *mut u32;
const UARTE0_ENDTX: *mut u32 = 0x4000_2120 as *mut u32;
const UARTE0_ENABLE: *mut u32 = 0x4000_2500 as *mut u32;
const UARTE0_PSEL_TXD: *mut u32 = 0x4000_250C as *mut u32;
const UARTE0_BAUDRATE: *mut u32 = 0x4000_2524 as *mut u32;
const UARTE0_TXD_PTR: *mut u32 = 0x4000_2544 as *mut u32;
const UARTE0_TXD_MAXCNT: *mut u32 = 0x4000_2548 as *mut u32;

// The Renode-injected input region: a RAM window @ 0x2002_0000, ABOVE the
// firmware's 128K RAM (memory.x RAM = 0x2000_0000 LENGTH 128K) but still
// inside the platform's 256K RAM block (0x2000_0000..0x2004_0000). The
// `.resc` does `sysbus LoadBinary @input.bin 0x2002_0000` BEFORE the CPU
// starts, so by the time `run` executes the bytes are already present (no
// DMA wait). The firmware stack grows DOWN from 0x2002_0000 (top of the
// firmware RAM), so it never collides with the injected fixture above it
// — the nRF single-RAM-block analogue of the STM32 axiSram arrangement.
const NUC_INPUT_REGION: *const u8 = 0x2002_0000 as *const u8;

// Diagnostic TX scratch (the on_violation=log / count-summary sink). nRF
// EasyDMA can only read RAM, but the diagnostic byte-string literals live
// in .rodata (FLASH @ 0x0), which EasyDMA cannot reach — so `nrf_uarte_puts`
// CPU-copies each chunk into this RAM staging buffer, then EasyDMA-streams
// it. (The MAIN output drain in `dma_push` needs no staging: the computed
// output array is already RAM-resident, directly EasyDMA-reachable.)
const NRF_TX_SCRATCH_CAP: usize = 64;
static mut NRF_TX_SCRATCH: [u8; NRF_TX_SCRATCH_CAP] = [0u8; NRF_TX_SCRATCH_CAP];

// --- Tier-3 monotonic clock: Cortex-M SysTick ------------------------
// Architectural ARMv7-M SysTick — the SAME System-Control-Space peripheral
// as the block in USART1_SHIM_SRC (every Cortex-M has it), so the second
// family reuses the exact mechanism. Kept duplicated rather than shared
// because each concrete shim is emitted as one self-contained firmware
// source. See the STM32 shim for the per-register-field commentary.
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

/// EasyDMA one-shot transmit of `len` bytes from the RAM address `src`.
/// `src` MUST be RAM-resident (EasyDMA cannot read FLASH). Blocks on
/// EVENTS_ENDTX (synchronous under Renode; real silicon completes the DMA
/// asynchronously and would IRQ on ENDTX — unverifiable on a non-cycle-
/// accurate emulator, the same fidelity caveat as the STM32 dma_wait).
fn nrf_uarte_tx(src: *const u8, len: usize) {
    unsafe {
        core::ptr::write_volatile(UARTE0_ENDTX, 0);
        core::ptr::write_volatile(UARTE0_TXD_PTR, src as u32);
        core::ptr::write_volatile(UARTE0_TXD_MAXCNT, len as u32);
        core::ptr::write_volatile(UARTE0_STARTTX, 1);
        while core::ptr::read_volatile(UARTE0_ENDTX) == 0 {}
    }
}

/// Emit a byte slice over UARTE0. Stages through NRF_TX_SCRATCH because the
/// slice may point into FLASH .rodata (EasyDMA reads RAM only); chunks
/// longer than the scratch are sent in pieces.
fn nrf_uarte_puts(s: &[u8]) {
    let mut off = 0usize;
    while off < s.len() {
        let n = core::cmp::min(NRF_TX_SCRATCH_CAP, s.len() - off);
        unsafe {
            let scratch = core::ptr::addr_of_mut!(NRF_TX_SCRATCH) as *mut u8;
            let mut i = 0usize;
            while i < n {
                core::ptr::write_volatile(scratch.add(i), s[off + i]);
                i += 1;
            }
            nrf_uarte_tx(scratch as *const u8, n);
        }
        off += n;
    }
}

/// Write `n` as decimal ASCII over UARTE0 (no_std, alloc-free).
fn nrf_uarte_put_u64(mut n: u64) {
    if n == 0 {
        nrf_uarte_puts(b\"0\");
        return;
    }
    let mut buf = [0u8; 20];
    let mut len = 0usize;
    while n > 0 {
        buf[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    // Digits were produced least-significant first; reverse in place.
    let mut i = 0usize;
    while i < len / 2 {
        buf.swap(i, len - 1 - i);
        i += 1;
    }
    nrf_uarte_puts(&buf[..len]);
}

/// The CONCRETE shim for the nRF52840 Renode target (P10, TASK-0453.10) —
/// the SECOND MCU family. Same `NucleusShim` trait as `Usart1Shim`; the
/// bodies drive nRF UARTE EasyDMA instead of the STM32 USART/DMA1.
struct NrfUarteShim {
    // Byte offset into NUC_INPUT_REGION consumed so far. Each effectful
    // load advances it by the loaded array's byte length.
    input_cursor: usize,
    // SysTick monotonic-clock accumulator state (mirrors Usart1Shim).
    clock_started: bool,
    accum_ticks: u64,
    last_cvr: u32,
}

impl NrfUarteShim {
    fn new() -> Self {
        NrfUarteShim {
            input_cursor: 0,
            clock_started: false,
            accum_ticks: 0,
            last_cvr: 0,
        }
    }
}

impl NucleusShim for NrfUarteShim {
    fn alloc_in_region(&mut self, _region: usize, bytes: usize) -> *mut u8 {
        // Hand back the next `bytes`-sized slice of the injected input
        // region and advance the cursor (same contract + concatenation-
        // order assumption as the STM32 shim).
        let p = unsafe { NUC_INPUT_REGION.add(self.input_cursor) } as *mut u8;
        self.input_cursor += bytes;
        p
    }
    fn dma_push(&mut self, _chan: usize, src: *const u8, len: usize) {
        // DIRECT EasyDMA TX of the computed output (RAM-resident). nRF
        // EasyDMA reaches the whole RAM block, so unlike the STM32 shim
        // there is no DTCM->AXI-SRAM stage-copy — `src` is the DMA source.
        nrf_uarte_tx(src, len);
    }
    fn dma_wait(&mut self, _chan: usize) {
        // No-op: nrf_uarte_tx already blocks on EVENTS_ENDTX (synchronous
        // under Renode). Real silicon would complete the DMA via the ENDTX
        // IRQ; that async overlap is unverifiable on the non-cycle-accurate
        // emulator (mirrors the STM32 dma_wait fidelity note).
    }
    // Cross-worker transport (M11 multi-MCU) is unreachable on the
    // SINGLE-worker BIN path (examples 1/5/9 emit no Push/Wait), so the
    // single-worker nRF shim no-ops link_push/link_recv (mirrors the STM32
    // single-worker Usart1Shim; nRF multi-MCU is future work).
    fn link_push(&mut self, _seq: usize, _src: *const u8, _len: usize) {}
    fn link_recv(&mut self, _seq: usize, _dst: *mut u8, _len: usize) {}
    fn irq_barrier(&mut self, _tag: u64) {}
    fn monotonic_ns(&mut self) -> u64 {
        // IDENTICAL accumulation logic to Usart1Shim::monotonic_ns (SysTick
        // is architectural). See the STM32 shim for the wrap/COUNTFLAG
        // reasoning + the single-reload-span limit.
        let (csr, cvr) = unsafe {
            let csr = core::ptr::read_volatile(SYST_CSR);
            let cvr = core::ptr::read_volatile(SYST_CVR);
            (csr, cvr)
        };
        if !self.clock_started {
            self.clock_started = true;
            self.last_cvr = cvr;
            return 0;
        }
        let wrapped = (csr & SYST_COUNTFLAG) != 0;
        let delta = if wrapped {
            (self.last_cvr as u64) + ((SYST_RELOAD as u64) + 1 - (cvr as u64))
        } else if self.last_cvr >= cvr {
            (self.last_cvr - cvr) as u64
        } else {
            (self.last_cvr as u64) + ((SYST_RELOAD as u64) + 1 - (cvr as u64))
        };
        self.accum_ticks += delta;
        self.last_cvr = cvr;
        self.accum_ticks * 1_000_000_000 / SYSTEM_CORE_CLOCK_HZ
    }
    fn report_violation(&mut self, loop_var: &[u8], measured_ns: u64, threshold_ns: u64) {
        // The tier-3 on_violation=log sink, over UARTE0 (same one-line ASCII
        // shape + `check loop ` prefix as the STM32 shim; same shared-channel
        // wart).
        nrf_uarte_puts(b\"check loop `\");
        nrf_uarte_puts(loop_var);
        nrf_uarte_puts(b\"` violated latency_max=\");
        nrf_uarte_put_u64(threshold_ns);
        nrf_uarte_puts(b\" ns: iteration took \");
        nrf_uarte_put_u64(measured_ns);
        nrf_uarte_puts(b\" ns\\n\");
    }
}
";
