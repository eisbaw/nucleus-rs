//! BIN-shape emit tests for the SECOND MCU family (nRF52840, P10
//! TASK-0453.10), split out of `bin_shape.rs` to keep that file under the
//! 1000-LoC mega-file fence (`just check-mega-files`). A child of the
//! `tests` module (reaches `super::repo_root`). Like the STM32 bin-shape
//! tests, these do NOT cross-compile — that is `just check-embedded` /
//! `just renode-embedded-nrf`; these are the fast drift detector.

use super::repo_root;

/// Lower `example`'s naive schedule and emit the BIN for `target`; return
/// `(main.rs, memory.x, .cargo/config.toml)` sources. Parameterised over
/// the shim family so the same helper drives both the nRF assertions and
/// the STM32-vs-nRF seam comparison without reaching across test modules.
fn emit_bin_files(
    target: crate::ShimTarget,
    example: &str,
    scratch_leaf: &str,
) -> (String, String, String) {
    use crate::emit_bin;
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples").join(example);
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).expect("algo source");
    let sched_src =
        std::fs::read_to_string(ex.join("schedules/naive.sched.nuc")).expect("sched source");
    let r = test_common::lower_for_test(
        &algo_src,
        &sched_src,
        &test_common::LowerForTestOpts {
            apply_block_transforms: false,
            apply_partition_workers: false,
            inject_check_frames: false,
        },
    );
    let kernels = ex.join("kernels.rs");
    let out = test_common::unique_scratch_dir(
        &root.join("nucleus/target/embedded-pattern-test-scratch"),
        scratch_leaf,
    );
    let res =
        emit_bin(&r.per_worker, &r.names, &r.sidecar, &kernels, &out, target).expect("embedded bin emit");
    assert_eq!(res.workers.len(), 1, "single-worker example must emit one bin");
    assert!(res.resc.is_none(), "single-worker bin must NOT emit a .resc");
    let one = &res.workers[0];
    let main = std::fs::read_to_string(&one.main_rs).expect("read emitted main.rs");
    let mem = std::fs::read_to_string(&one.memory_x).expect("read emitted memory.x");
    let cfg = std::fs::read_to_string(&one.cargo_config).expect("read emitted config.toml");
    (main, mem, cfg)
}

#[test]
fn nrf52840_bin_is_a_runnable_easydma_firmware() {
    // P10 (TASK-0453.10): the --shim nrf52840 bin path for example 1 — the
    // SECOND MCU family. Same generic scaffolding as the STM32 bin, with the
    // nRF UARTE EasyDMA concrete shim swapped in.
    let (main, _mem, _cfg) =
        emit_bin_files(crate::ShimTarget::Nrf52840, "01-elementwise-add", "ex01_nrf_naive");

    // no_std / no_main cortex-m firmware (same class as the STM32 bin).
    assert!(main.contains("#![no_std]"), "must be no_std:\n{main}");
    assert!(main.contains("#![no_main]"), "must be no_main:\n{main}");
    assert!(main.contains("#[entry]"), "missing #[entry]:\n{main}");
    assert!(
        main.contains("#[panic_handler]"),
        "missing #[panic_handler]:\n{main}"
    );

    // The SHARED portability seam: the SAME trait + the SAME generic run<S>
    // as the STM32 bin (verbatim reuse — this is the whole P10 claim).
    assert!(
        main.contains("pub trait NucleusShim"),
        "trait missing (shared seam):\n{main}"
    );
    assert!(
        main.contains("pub fn run<S: NucleusShim>(shim: &mut S) {"),
        "generic run<S> missing (shared seam):\n{main}"
    );

    // The CONCRETE nRF shim — NrfUarteShim, NOT the STM32 Usart1Shim.
    assert!(
        main.contains("impl NucleusShim for NrfUarteShim"),
        "NrfUarteShim impl missing:\n{main}"
    );
    // Structural (not bare-word): the nRF shim's doc comment cross-references
    // `Usart1Shim` by name, so assert the STM32 shim IMPL/struct + DMA1
    // registers are absent, not the mere string.
    assert!(
        !main.contains("impl NucleusShim for Usart1Shim")
            && !main.contains("struct Usart1Shim")
            && !main.contains("DMA1_S0CR"),
        "STM32 USART/DMA1 shim must NOT appear in the nRF bin:\n{main}"
    );

    // The nRF UARTE EasyDMA register interface (genuinely different from the
    // STM32 USART DR/ISR): a RAM source POINTER + byte COUNT + STARTTX/ENDTX.
    assert!(
        main.contains("UARTE0_TXD_PTR") && main.contains("UARTE0_TXD_MAXCNT"),
        "EasyDMA TXD.PTR/MAXCNT registers missing:\n{main}"
    );
    assert!(
        main.contains("UARTE0_STARTTX") && main.contains("UARTE0_ENDTX"),
        "EasyDMA STARTTX/ENDTX task/event missing:\n{main}"
    );
    assert!(
        main.contains("0x4000_2000") || main.contains("0x4000_2544"),
        "nRF UARTE0 base / TXD.PTR address missing:\n{main}"
    );
    assert!(
        main.contains("const NUC_INPUT_REGION: *const u8 = 0x2002_0000 as *const u8;"),
        "nRF input region must be the injected RAM window @ 0x2002_0000:\n{main}"
    );

    // The pure kernel is extracted verbatim (same generic path as STM32).
    assert!(
        main.contains("pub fn add(a: i32, b: i32) -> i32") && main.contains("a.wrapping_add(b)"),
        "pure kernel not extracted verbatim:\n{main}"
    );
    // The save Fire lowers to the SAME trait hook (dma_push) — shim-agnostic.
    assert!(
        main.contains("shim.dma_push(0, c.as_ptr() as *const u8"),
        "save Fire did not lower to dma_push (the shared UART hook):\n{main}"
    );
    assert!(
        !main.contains("std::fs"),
        "std::fs leaked into no_std bin:\n{main}"
    );
}

#[test]
fn nrf52840_memory_map_and_linker_config() {
    let (_main, mem, cfg) =
        emit_bin_files(crate::ShimTarget::Nrf52840, "01-elementwise-add", "ex01_nrf_mem");
    // nRF FLASH @ 0x0 (1024K), RAM @ 0x20000000 (lower 128K of the 256K
    // block; upper half is the injected input window).
    assert!(
        mem.contains("FLASH : ORIGIN = 0x00000000, LENGTH = 1024K"),
        "nRF FLASH region:\n{mem}"
    );
    assert!(
        mem.contains("RAM   : ORIGIN = 0x20000000, LENGTH = 128K"),
        "nRF RAM region:\n{mem}"
    );
    // NO STM32 AXISRAM staging section (nRF EasyDMA reaches RAM directly).
    // Structural: the comment explains WHY there is no AXISRAM, so assert the
    // region declaration + the NOLOAD section are absent, not the bare word.
    assert!(
        !mem.contains("AXISRAM :") && !mem.contains(".axisram_tx (NOLOAD)"),
        "nRF map must NOT carry the STM32 AXI-SRAM staging section:\n{mem}"
    );
    // FLASH @ 0x0 needs --nmagic so the vector table stays at 0x0.
    assert!(
        cfg.contains("link-arg=--nmagic"),
        "nRF config must add --nmagic (FLASH @ 0x0 page-align fix):\n{cfg}"
    );
    assert!(
        cfg.contains("link-arg=-Tlink.x"),
        "cortex-m-rt link.x must still be wired:\n{cfg}"
    );
}

#[test]
fn stm32_and_nrf_share_the_trait_seam_but_differ_in_concrete_shim() {
    // The P10 portability claim, asserted directly: BOTH families emit the
    // byte-identical `NucleusShim` trait + generic `run<S>` body, and differ
    // ONLY in the concrete shim type. The generic backend is written once.
    let (stm, _sm, _sc) =
        emit_bin_files(crate::ShimTarget::Stm32h7, "01-elementwise-add", "ex01_seam_stm");
    let (nrf, _nm, _nc) =
        emit_bin_files(crate::ShimTarget::Nrf52840, "01-elementwise-add", "ex01_seam_nrf");

    // Shared seam: the trait declaration + the run signature line + the
    // shim-agnostic save hook appear identically in both bins.
    for shared in [
        "pub trait NucleusShim",
        "fn dma_push(&mut self, chan: usize, src: *const u8, len: usize);",
        "pub fn run<S: NucleusShim>(shim: &mut S) {",
        "shim.dma_push(0, c.as_ptr() as *const u8",
    ] {
        assert!(stm.contains(shared), "STM32 bin missing shared seam `{shared}`");
        assert!(nrf.contains(shared), "nRF bin missing shared seam `{shared}`");
    }
    // Divergent concrete shim: each emits its OWN impl, not the other's
    // (the nRF shim's doc comment cross-references Usart1Shim by name, so
    // assert on the `impl NucleusShim for <X>` not the bare type string).
    assert!(
        stm.contains("impl NucleusShim for Usart1Shim")
            && !stm.contains("impl NucleusShim for NrfUarteShim")
    );
    assert!(
        nrf.contains("impl NucleusShim for NrfUarteShim")
            && !nrf.contains("impl NucleusShim for Usart1Shim")
    );
}
