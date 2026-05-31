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
//!
//! # Module layout (TASK-0340.10 file-hygiene split)
//!
//! This module holds the M9 LIB templates (the `NucleusShim` trait +
//! `StubShim`, [`render_lib`], [`render_cargo_toml`]) plus the shared
//! [`render_count_statics`] helper. The M10 Renode-runnable BIN
//! templates (the concrete `Usart1Shim`, [`render_bin_main`],
//! [`render_bin_cargo_toml`], [`render_memory_x`], [`render_build_rs`],
//! [`render_cargo_config`]) live in the sibling [`bin`] module and are
//! re-exported here via `pub use bin::*;` so every `skeleton::*` call
//! site (in `lib.rs` + the inline tests) resolves UNCHANGED. The seam is
//! the LIB-vs-BIN(M10) boundary; the two share only the trait source
//! ([`NUCLEUS_SHIM_SRC`]) and the count-statics helper.

mod bin;
pub use bin::*;

mod multimcu;
pub use multimcu::*;

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
    /// Send `len` bytes from `src` to the PEER worker on cross-worker
    /// transport channel `seq` (M11 multi-MCU, TASK-0049.05). DISTINCT
    /// from [`Self::dma_push`]: `dma_push` drains a buffer to a local
    /// PERIPHERAL (the effectful `save_output` USART stream, channel 0),
    /// whereas `link_push` crosses the inter-MCU link to another worker
    /// keyed by the schedule's `SeqTag`. Keeping the two namespaces
    /// separate is load-bearing: on the host, `save_output` (`dma_push(0)`)
    /// and `Push a` (`link_push(0)`) would otherwise collide on a single
    /// channel id and a real shim could not route them to different USARTs.
    fn link_push(&mut self, seq: usize, src: *const u8, len: usize);
    /// Receive `len` bytes from the peer worker on cross-worker transport
    /// channel `seq` into `dst` (M11 multi-MCU, TASK-0049.05). Blocks
    /// until the peer's matching [`Self::link_push`] (same `seq`) has
    /// delivered the bytes. The `StubShim` no-ops it (the receive buffer
    /// stays zero-filled — honest for the compile-only LIB path); the
    /// concrete multi-MCU shim polls the channel's USART RX and fills
    /// `dst`.
    fn link_recv(&mut self, seq: usize, dst: *mut u8, len: usize);
    /// An IRQ-completion control barrier identified by `tag`. Unused by
    /// the single-worker examples 1 + 5 (no `Event::Sync` in a naive
    /// schedule); declared for the M10/M11 multi-MCU barrier surface. The
    /// tag is `u64` (the full `SyncTag` width) — NOT `u32` — so a large
    /// barrier tag cannot silently truncate at the lowering boundary
    /// (TASK-0049.05 trap #2).
    fn irq_barrier(&mut self, tag: u64);
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
    fn link_push(&mut self, _seq: usize, _src: *const u8, _len: usize) {}
    fn link_recv(&mut self, _seq: usize, _dst: *mut u8, _len: usize) {}
    fn irq_barrier(&mut self, _tag: u64) {}
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
///
/// `pub(super)` (TASK-0340.10 split) so the sibling [`bin`] module's
/// `render_bin_main` can emit the identical static; not exported beyond
/// the crate.
pub(super) fn render_count_statics(s: &mut String, count_idents: &[&str]) {
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
// M10 (TASK-0048.01) bin-emit templates moved to the sibling `bin`
// module (TASK-0340.10 file-hygiene split) and re-exported above via
// `pub use bin::*;`. The lib path (`render_cargo_toml` / `render_lib`)
// stays here. See `skeleton/bin.rs`.
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_toml_is_a_lib_not_a_bin() {
        let s = render_cargo_toml();
        assert!(
            s.contains("[lib]"),
            "embedded Cargo.toml must declare [lib]"
        );
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
    fn shim_trait_declares_all_methods() {
        // The canonical trait source declares: the four M9 methods, the two
        // M10 TASK-0048.04 tier-3 methods (monotonic_ns clock +
        // report_violation log sink), and the two M11 TASK-0049.05
        // cross-worker transport methods (link_push / link_recv — distinct
        // from the effectful dma_push/dma_wait so a real multi-MCU shim can
        // route peripheral IO and inter-MCU transport to different USARTs).
        for m in [
            "fn alloc_in_region",
            "fn dma_push",
            "fn dma_wait",
            "fn link_push",
            "fn link_recv",
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
        let s = render_lib(
            "pub fn add(a: i32, b: i32) -> i32 { a }\n",
            "    // body\n",
            &[],
        );
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
        let s = render_lib(
            "pub fn add(a: i32, b: i32) -> i32 { a }\n",
            "    // body\n",
            &["i"],
        );
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
        assert!(
            s.contains("use cortex_m_rt::entry;"),
            "missing cortex-m-rt import"
        );
        assert!(s.contains("#[entry]"), "missing #[entry]");
        assert!(s.contains("#[panic_handler]"), "missing #[panic_handler]");
        // The SAME trait surface as the lib (verbatim reuse).
        assert!(s.contains("pub trait NucleusShim"), "trait missing");
        assert!(
            s.contains("struct StubShim"),
            "StubShim missing (verbatim reuse)"
        );
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
        assert!(
            !s.contains("checksum="),
            "ASCII checksum framing must be GONE"
        );
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
        assert!(
            s.contains("let mut shim = Usart1Shim::new();"),
            "main must build the shim"
        );
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
        let summary_at = s.find("NUC_CHECK_COUNT_i.load").expect("summary present");
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
        assert!(
            s.contains("ORIGIN = 0x08000000, LENGTH = 128K"),
            "FLASH region"
        );
        assert!(
            s.contains("ORIGIN = 0x20000000, LENGTH = 128K"),
            "RAM region"
        );
    }

    #[test]
    fn build_rs_and_cargo_config_target_thumbv7em() {
        let b = render_build_rs();
        assert!(b.contains("memory.x"), "build.rs must wire memory.x");
        assert!(
            b.contains("rustc-link-search"),
            "build.rs must add link search"
        );
        let c = render_cargo_config();
        assert!(
            c.contains("[target.thumbv7em-none-eabihf]"),
            "config must target thumbv7em-none-eabihf"
        );
        assert!(c.contains("-Tlink.x"), "config must link via link.x");
    }
}
