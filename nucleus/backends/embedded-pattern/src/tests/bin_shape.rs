//! BIN-shape (M10 `--shim stm32h7` / TASK-0048) emit tests for the
//! embedded-pattern backend, split out of `tests.rs` per TASK-0383 to
//! keep that file under the 1000-LoC mega-file fence.
//!
//! These assert the emitted `no_std` BIN (firmware) SHAPE — the
//! `emit_bin` path with the cortex-m-rt entry + USART1 shim. They are a
//! child of the `tests` module, so they reach its private `repo_root`
//! helper via `super::repo_root`. Like the LIB-shape tests in the
//! parent, they do NOT cross-compile — that is the dedicated
//! `just check-embedded` / `just renode-*` recipes' job. The shape
//! assertions here are the fast drift-detection layer.

use super::repo_root;

/// Lower `example`'s `naive` schedule and emit the no_std BIN (M10
/// `--shim stm32h7`) into a scratch dir; return the emitted main.rs
/// source. The bin path is the SAME lowering as the lib, with the
/// bare-metal scaffolding (cortex-m-rt entry + USART1 shim) added — so
/// the cross-compile / Renode run is the genuine acceptance (the
/// `renode-embedded` just recipe, parameterised over the example dir as
/// a positional arg). This is the fast shape-drift detector
/// (TASK-0048.01 / .03).
fn emit_bin_example_naive(example: &str, scratch_leaf: &str) -> String {
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

    // TASK-0426.01: per-call-unique scratch (created once, never removed).
    let out = test_common::unique_scratch_dir(
        &root.join("nucleus/target/embedded-pattern-test-scratch"),
        scratch_leaf,
    );

    let res =
        emit_bin(&r.per_worker, &r.names, &r.sidecar, &kernels, &out).expect("embedded bin emit");
    // A single-worker example emits exactly ONE bin at the root (no worker
    // sub-dir) and NO multi-machine .resc (unchanged M10 shape).
    assert_eq!(res.workers.len(), 1, "single-worker example must emit one bin");
    assert!(
        res.resc.is_none(),
        "single-worker example must NOT emit a multi-machine .resc"
    );
    let one = &res.workers[0];
    assert!(
        one.worker_name.is_none(),
        "single-worker bin must be emitted at root (worker_name None)"
    );
    // Every bare-metal scaffolding file must land on disk.
    assert!(one.main_rs.exists(), "emitted src/main.rs must exist");
    assert!(one.cargo_toml.exists(), "emitted Cargo.toml must exist");
    assert!(one.memory_x.exists(), "emitted memory.x must exist");
    assert!(one.build_rs.exists(), "emitted build.rs must exist");
    assert!(
        one.cargo_config.exists(),
        "emitted .cargo/config.toml must exist"
    );
    std::fs::read_to_string(&one.main_rs).expect("read emitted main.rs")
}

#[test]
fn ex01_bin_emits_renode_runnable_firmware_with_uart_streaming() {
    // M10 (TASK-0048.01): the --shim stm32h7 bin path for example 1.
    let main = emit_bin_example_naive("01-elementwise-add", "ex01_bin_naive");

    // no_std / no_main firmware with cortex-m-rt entry + panic handler.
    assert!(main.contains("#![no_std]"), "must be no_std:\n{main}");
    assert!(main.contains("#![no_main]"), "must be no_main:\n{main}");
    assert!(
        main.contains("use cortex_m_rt::entry;"),
        "missing cortex-m-rt:\n{main}"
    );
    assert!(main.contains("#[entry]"), "missing #[entry]:\n{main}");
    assert!(
        main.contains("#[panic_handler]"),
        "missing #[panic_handler]:\n{main}"
    );

    // Same trait surface as the lib (verbatim reuse) + the concrete shim.
    assert!(
        main.contains("pub trait NucleusShim"),
        "trait missing:\n{main}"
    );
    assert!(
        main.contains("impl NucleusShim for Usart1Shim"),
        "Usart1Shim impl missing:\n{main}"
    );

    // The pure kernel is extracted verbatim (same as the lib path).
    assert!(
        main.contains("pub fn add(a: i32, b: i32) -> i32"),
        "pure kernel `add` not extracted:\n{main}"
    );
    assert!(
        main.contains("a.wrapping_add(b)"),
        "kernel body not verbatim:\n{main}"
    );
    assert!(
        !main.contains("std::fs"),
        "std::fs leaked into no_std bin:\n{main}"
    );

    // The lowered run + the save Fire -> dma_push (the UART hook).
    assert!(
        main.contains("pub fn run<S: NucleusShim>(shim: &mut S) {"),
        "run entry missing:\n{main}"
    );
    assert!(
        main.contains("shim.dma_push(0, c.as_ptr() as *const u8"),
        "save Fire did not lower to dma_push (the UART hook):\n{main}"
    );

    // TASK-0048.02: the load Fire fills the array from the shim's input
    // source (alloc_in_region returns a pointer into the Renode-injected
    // region; the lowering copies into the array under a null-guard so
    // the stub-shim lib path is unaffected). BOTH inputs a + b are loaded.
    assert!(
        main.contains("let __src = shim.alloc_in_region(0, core::mem::size_of_val(&a));"),
        "load `a` did not request its input slice from the shim:\n{main}"
    );
    assert!(
        main.contains("let __src = shim.alloc_in_region(0, core::mem::size_of_val(&b));"),
        "load `b` did not request its input slice from the shim:\n{main}"
    );
    assert!(
        main.contains("core::ptr::copy_nonoverlapping(__src,"),
        "load lowering must copy the injected input bytes into the array:\n{main}"
    );
    assert!(
        main.contains("if !__src.is_null() {"),
        "load copy must be null-guarded (stub shim returns null => no copy):\n{main}"
    );

    // TASK-0048.12: dma_push streams the RAW output bytes via a REAL DMA1
    // MemoryToPeripheral transfer into USART1's TDR (the byte-exact
    // reference.bin diff is the value-correctness bar); the old CPU
    // usart1_putc loop AND the older ASCII summary (NUC-EX1 / checksum)
    // are GONE.
    assert!(
        main.contains("DMA_TX_STAGING") && main.contains("DMA1_S0CR"),
        "dma_push must drive a real DMA1 MemoryToPeripheral USART1 TX:\n{main}"
    );
    assert!(
        !main.contains("NUC-EX1"),
        "the ASCII summary framing must be GONE (raw-byte stream now):\n{main}"
    );
    assert!(
        !main.contains("checksum="),
        "the ASCII checksum framing must be GONE (raw-byte stream now):\n{main}"
    );

    // The concrete shim reads from the injected input region (axiSram).
    assert!(
        main.contains("const NUC_INPUT_REGION: *const u8 = 0x2400_0000 as *const u8;"),
        "shim must read from the injected input region axiSram @ 0x2400_0000:\n{main}"
    );
    assert!(
        main.contains("input_cursor"),
        "shim must track an input cursor across sequential loads:\n{main}"
    );

    // main enables USART then runs.
    assert!(
        main.contains("let mut shim = Usart1Shim::new();"),
        "main must build shim:\n{main}"
    );
    assert!(
        main.contains("run(&mut shim);"),
        "main must call run:\n{main}"
    );
}

#[test]
fn ex05_bin_emits_renode_runnable_firmware_with_flattened_blur3() {
    // M10 (TASK-0048.03): the --shim stm32h7 bin path for example 5
    // (05-stencil). The firm bar of TASK-0048.03 — ex5's lib already
    // cross-compiles (check-embedded), so the bin scaffolding transfers.
    let main = emit_bin_example_naive("05-stencil", "ex05_bin_naive");

    // no_std / no_main firmware with cortex-m-rt entry + panic handler.
    assert!(main.contains("#![no_std]"), "must be no_std:\n{main}");
    assert!(main.contains("#![no_main]"), "must be no_main:\n{main}");
    assert!(main.contains("#[entry]"), "missing #[entry]:\n{main}");
    assert!(
        main.contains("#[panic_handler]"),
        "missing #[panic_handler]:\n{main}"
    );

    // The PURE blur3 kernel extracted verbatim (9-param multiline sig + body).
    assert!(
        main.contains("pub fn blur3("),
        "pure kernel `blur3` not extracted:\n{main}"
    );
    assert!(
        main.contains("sum / 9"),
        "blur3 body not copied verbatim:\n{main}"
    );
    assert!(
        !main.contains("std::fs"),
        "std::fs leaked into no_std bin:\n{main}"
    );

    // 2D flatten: img[y][x] -> img[y*16 + x] (same flatten as tier-1 / the lib).
    assert!(
        main.contains("* 16 +"),
        "2D index not flattened row-major:\n{main}"
    );
    assert!(
        main.contains("kernels::blur3("),
        "compute Fire did not call kernels::blur3:\n{main}"
    );
    // 16*16 = 256-element fixed arrays.
    assert!(
        main.contains("[i32; 256] = [0; 256]"),
        "stencil arrays not fixed-size [i32; 256]:\n{main}"
    );

    // The single effectful load (load_image -> img_in) fills from the
    // injected region; the single effectful save (save_image) streams
    // raw bytes. Exactly ONE load => cursor trivially starts at 0
    // (TASK-0048.06 load-order confirmation).
    assert!(
        main.contains("let __src = shim.alloc_in_region(0, core::mem::size_of_val(&img_in));"),
        "load `img_in` did not request its input slice from the shim:\n{main}"
    );
    assert!(
        main.contains("shim.dma_push(0, img_out.as_ptr() as *const u8"),
        "save Fire did not lower to dma_push streaming img_out:\n{main}"
    );
    assert!(
        main.contains("DMA_TX_STAGING") && main.contains("DMA1_S0CR"),
        "dma_push must drive a real DMA1 MemoryToPeripheral USART1 TX:\n{main}"
    );
}

#[test]
fn ex09_bin_emits_renode_runnable_firmware_with_two_stage_pipe() {
    // M10 (TASK-0048.03): the --shim stm32h7 bin path for example 9
    // (09-producer-consumer). This was the RISK arm — ex9 had never been
    // through embedded codegen. It lowers cleanly: a two-stage pipe
    // (produce -> stream -> transform -> result) where `stream` is an
    // intermediate data array written by produce and read by transform in
    // the SAME loop. Both produce + transform are indexed-output PURE
    // compute Fires (extracted verbatim); `stream` is collected as a
    // fixed-size local. This test pins that two-stage shape.
    let main = emit_bin_example_naive("09-producer-consumer", "ex09_bin_naive");

    assert!(main.contains("#![no_std]"), "must be no_std:\n{main}");
    assert!(main.contains("#![no_main]"), "must be no_main:\n{main}");
    assert!(main.contains("#[entry]"), "missing #[entry]:\n{main}");

    // BOTH pure compute kernels extracted verbatim (the two-op transform
    // body is the load-bearing one — a bug that drops `rec` shows here).
    assert!(
        main.contains("pub fn produce(seed: i32) -> i32"),
        "produce not extracted:\n{main}"
    );
    assert!(
        main.contains("seed.wrapping_mul(3)"),
        "produce body not verbatim:\n{main}"
    );
    assert!(
        main.contains("pub fn transform(rec: i32) -> i32"),
        "transform not extracted:\n{main}"
    );
    assert!(
        main.contains("rec.wrapping_mul(7).wrapping_add(rec)"),
        "transform body not verbatim:\n{main}"
    );
    assert!(
        !main.contains("std::fs"),
        "std::fs leaked into no_std bin:\n{main}"
    );

    // The intermediate `stream` array is a fixed [i32; 16] local — NOT a
    // shim hook (it is internal dataflow, neither loaded nor saved).
    assert!(
        main.contains("let mut stream: [i32; 16] = [0; 16];"),
        "intermediate `stream` not laid out as a fixed-size local:\n{main}"
    );
    // The loop fires BOTH stages: produce writes stream[n], transform
    // reads stream[n] and writes result[n] — in the same iteration.
    assert!(
        main.contains("stream[(n) as usize] = kernels::produce(seeds[(n) as usize]);"),
        "stage-1 produce Fire missing / mis-lowered:\n{main}"
    );
    assert!(
        main.contains("result[(n) as usize] = kernels::transform(stream[(n) as usize]);"),
        "stage-2 transform Fire missing / mis-lowered:\n{main}"
    );

    // Single effectful load (load_input -> seeds), single effectful save
    // (save_output -> result). Exactly ONE load => cursor trivially
    // starts at 0 (TASK-0048.06 load-order confirmation).
    assert!(
        main.contains("let __src = shim.alloc_in_region(0, core::mem::size_of_val(&seeds));"),
        "load `seeds` did not request its input slice from the shim:\n{main}"
    );
    assert!(
        main.contains("shim.dma_push(0, result.as_ptr() as *const u8"),
        "save Fire did not lower to dma_push streaming result:\n{main}"
    );
    assert!(
        main.contains("DMA_TX_STAGING") && main.contains("DMA1_S0CR"),
        "dma_push must drive a real DMA1 MemoryToPeripheral USART1 TX:\n{main}"
    );
}

/// Lower the 02-split-add `split` (2-worker) schedule and emit the
/// multi-MCU BINs into a scratch dir; return the [`crate::MultiBinEmitResult`].
/// Mirrors `super::emit_example_multi` (same lowering opts) but drives the
/// BIN path (`emit_bin`) — the M11 multi-MCU slice (TASK-0049.05).
fn emit_bin_example_multi(
    example: &str,
    schedule_file: &str,
    scratch_leaf: &str,
) -> crate::MultiBinEmitResult {
    use crate::emit_bin;
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples").join(example);
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).expect("algo source");
    let sched_src =
        std::fs::read_to_string(ex.join("schedules").join(schedule_file)).expect("sched source");
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
    // TASK-0426.01: per-call-unique scratch (created once, never removed).
    let out = test_common::unique_scratch_dir(
        &root.join("nucleus/target/embedded-pattern-test-scratch"),
        scratch_leaf,
    );
    emit_bin(&r.per_worker, &r.names, &r.sidecar, &kernels, &out).expect("embedded multi-MCU bin emit")
}

#[test]
fn multi_worker_bin_emits_one_firmware_per_mcu_with_real_uart_transport() {
    // TASK-0049.05 (M11 BIN slice B): the formerly-rejecting multi-worker
    // bin case now SUCCEEDS — one Renode-runnable firmware per worker with
    // a CONCRETE inter-MCU UART-hub shim, plus a generated multi-machine
    // .resc. This REPLACES the former rejection pin.
    let res = emit_bin_example_multi("02-split-add", "split.sched.nuc", "split_multi_bin");

    // Two bins, one per worker (host + w0), each nested under out_dir/<name>/.
    assert_eq!(res.workers.len(), 2, "the 2-worker split schedule must emit two bins");
    let mut names: Vec<&str> = res
        .workers
        .iter()
        .map(|w| w.worker_name.as_deref().expect("multi-worker bin carries its name"))
        .collect();
    names.sort_unstable();
    assert_eq!(names, vec!["host", "w0"], "two bins named for the two workers");
    for w in &res.workers {
        let name = w.worker_name.as_deref().unwrap();
        assert!(
            w.project_dir.ends_with(name),
            "worker `{name}` bin must be nested under out_dir/{name}/"
        );
        assert!(w.main_rs.exists() && w.cargo_toml.exists(), "scaffolding must exist");
    }
    let read = |name: &str| -> String {
        let w = res
            .workers
            .iter()
            .find(|w| w.worker_name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("worker `{name}` bin missing"));
        std::fs::read_to_string(&w.main_rs).expect("read worker main.rs")
    };
    let host = read("host");
    let w0 = read("w0");

    // Both are Renode-runnable no_std firmwares with the MultiMcuShim.
    for (name, src) in [("host", &host), ("w0", &w0)] {
        assert!(src.contains("#![no_std]") && src.contains("#![no_main]"), "{name} no_std/no_main");
        assert!(src.contains("#[entry]") && src.contains("#[panic_handler]"), "{name} entry/panic");
        assert!(src.contains("struct MultiMcuShim"), "{name} must use MultiMcuShim:\n{src}");
        assert!(src.contains("fn mmcu_link_base(seq: usize)"), "{name} must map seq->USART:\n{src}");
        assert!(!src.contains("std::fs"), "{name} must not leak std");
    }

    // host: link_push a (seq0) + b (seq1) over the REAL transport hook
    // (NOT dma_push — trap #1), link_recv c, dma_push(0) for the effectful
    // save over USART1, alloc_in_region for the effectful load.
    assert!(
        host.contains("shim.link_push(0, a.as_ptr() as *const u8, core::mem::size_of_val(&a));"),
        "host must link_push `a`:\n{host}"
    );
    assert!(
        host.contains("shim.link_push(1, b.as_ptr() as *const u8, core::mem::size_of_val(&b));"),
        "host must link_push `b`:\n{host}"
    );
    assert!(
        host.contains("shim.link_recv(2, c.as_mut_ptr() as *mut u8, core::mem::size_of_val(&c));"),
        "host must link_recv `c`:\n{host}"
    );
    assert!(
        host.contains("shim.dma_push(0, c.as_ptr() as *const u8"),
        "host effectful save must stay on the dma_push(0) USART1 stream:\n{host}"
    );
    assert!(
        host.contains("shim.alloc_in_region("),
        "host effectful load must use alloc_in_region:\n{host}"
    );
    // TASK-0049.10.04 regression: `host` is the SINGLE loader (loads BOTH
    // a + b, contiguous from byte 0), so its per-worker input base offset
    // MUST stay 0 — Mechanism A leaves single-loader schedules byte-identical
    // (the 02-split-add renode byte-exact path depends on this).
    assert!(
        host.contains("const NUC_INPUT_BASE: usize = 0;")
            && host.contains("input_cursor: NUC_INPUT_BASE,"),
        "single-loader host must keep input base offset 0 (regression-safe):\n{host}"
    );

    // w0: link_recv a + b, computes, link_push c.
    assert!(
        w0.contains("shim.link_recv(0, a.as_mut_ptr() as *mut u8, core::mem::size_of_val(&a));")
            && w0.contains("shim.link_recv(1, b.as_mut_ptr() as *mut u8, core::mem::size_of_val(&b));"),
        "w0 must link_recv `a` + `b`:\n{w0}"
    );
    assert!(
        w0.contains("shim.link_push(2, c.as_ptr() as *const u8, core::mem::size_of_val(&c));"),
        "w0 must link_push `c`:\n{w0}"
    );
    assert!(
        w0.contains("c[(i) as usize] = kernels::add(a[(i) as usize], b[(i) as usize]);"),
        "w0 must lower the pure `add` compute:\n{w0}"
    );

    // PER-SEQ TRANSPORT (TASK-0049.05.02): one UARTHub per CHANNEL, NOT one
    // per worker-pair. host->w0 carries TWO same-direction channels (seq0=a,
    // seq1=b); each MUST get its OWN hub + USART so they cannot cross on a
    // shared byte FIFO. seq2 (w0->host, c) is the third channel. So three
    // distinct hubs, and seq0/seq1 ride DISTINCT USARTs in the link table.
    let resc_path = res.resc.expect("multi-worker emit must generate a .resc");
    let resc = std::fs::read_to_string(&resc_path).expect("read .resc");
    for hub in ["link_host_w0_s0", "link_host_w0_s1", "link_w0_host_s2"] {
        assert!(
            resc.contains(&format!("CreateUARTHub \"{hub}\"")),
            "must create the per-channel hub `{hub}`:\n{resc}"
        );
    }
    // The BITE for the ex14 deadlock fix: the two SAME-DIRECTION channels
    // (seq0=a, seq1=b, both host->w0) land on DISTINCT USARTs (usart2 vs
    // usart3) in BOTH firmwares' mmcu_link_base tables — not the pre-fix
    // single shared usart2 that let same-direction streams interleave.
    for (name, src) in [("host", &host), ("w0", &w0)] {
        assert!(
            src.contains("0 => 0x40004400") && src.contains("1 => 0x40004800"),
            "{name}: same-direction seq0 (a) and seq1 (b) must ride DISTINCT \
             USARTs (usart2 0x40004400 vs usart3 0x40004800):\n{src}"
        );
    }
    // Each channel's hub is Connected by BOTH endpoints (seq0 on usart2).
    assert!(
        resc.matches("connector Connect usart2").count() == 2,
        "both workers must Connect their usart2 to the seq0 hub:\n{resc}"
    );
    assert!(resc.contains("usart1 CreateFileBackend $uartFile true"), "host USART1 must be captured:\n{resc}");
    assert!(resc.contains("sysbus LoadBinary $input 0x24000000"), "host input must inject to axiSram:\n{resc}");
    assert!(resc.contains("sysbus LoadELF $hostBin") && resc.contains("sysbus LoadELF $w0Bin"), "both ELFs loaded:\n{resc}");
    // Receivers-first: w0 (receives a/b before sending c) is released
    // BEFORE host (the early sender) — the start-gating discipline.
    let w0_release = resc.find("mach set \"w0\"\ncpu IsHalted false").expect("w0 release present");
    let host_release = resc.find("mach set \"host\"\ncpu IsHalted false").expect("host release present");
    assert!(
        w0_release < host_release,
        "receivers-first boot: w0 must be released before host:\n{resc}"
    );
}

/// Lower a multi-worker schedule from EXPLICIT algo/kernels files (the
/// ex14 `prog.embedded.algo.nuc` + `kernels.embedded.rs` per-frame variant)
/// and ATTEMPT the multi-MCU BIN emit; return the `emit_bin` Result so a
/// caller can assert either the emitted bins OR a typed rejection. Mirrors
/// `emit_bin_example_multi` but parameterises the algorithm + kernels file
/// names (TASK-0049.10.04).
fn try_emit_bin_example_multi_with_files(
    example: &str,
    algo_file: &str,
    schedule_file: &str,
    kernels_file: &str,
    scratch_leaf: &str,
) -> Result<crate::MultiBinEmitResult, crate::EmitError> {
    use crate::emit_bin;
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples").join(example);
    let algo_src = std::fs::read_to_string(ex.join(algo_file)).expect("algo source");
    let sched_src =
        std::fs::read_to_string(ex.join("schedules").join(schedule_file)).expect("sched source");
    let r = test_common::lower_for_test(
        &algo_src,
        &sched_src,
        &test_common::LowerForTestOpts {
            apply_block_transforms: false,
            apply_partition_workers: false,
            inject_check_frames: false,
        },
    );
    let kernels = ex.join(kernels_file);
    let out = test_common::unique_scratch_dir(
        &root.join("nucleus/target/embedded-pattern-test-scratch"),
        scratch_leaf,
    );
    emit_bin(&r.per_worker, &r.names, &r.sidecar, &kernels, &out)
}

#[test]
fn ex14_multimcu_cross_worker_input_partition_emits_decl_order_offsets() {
    // TASK-0049.10.06 (BLOCKER 2 ROOT): per-worker INPUT partition, now with
    // declaration order threaded into the codegen contract. This test was
    // previously `..._fails_loud_pending_decl_order` and pinned the OLD
    // BLOCKED state; it now pins the EMIT progressing PAST the rejection with
    // the CORRECT declaration-order byte offsets (AC#2 + AC#4 at the emit
    // layer, on the REAL ex14 sources).
    //
    // Two coupled results are asserted:
    //
    // (1) CLASSIFICATION (silent-sibling of slice A, TASK-0049.10.04) — ex14
    //     `fe`/`rf` perform INDEXED effectful loads (`mic_in[frame] <--
    //     fe_capture()`, `bt_in[frame] <-- rf_receive()`). The purity-gated
    //     indexed arm recognises BOTH as loaders, so `compute_input_offsets`
    //     sees TWO loader workers and takes the cross-worker branch.
    //
    // (2) OFFSET — the reference input.bin is DECLARATION order (mic_in@0 then
    //     bt_in@256) but DataId is ALPHABETICAL (bt_in=0 < mic_in=2), the
    //     REVERSE. The emit now orders the global layout by
    //     `NameSidecar.data_decl_order` (threaded AST->IR->ACFG->sidecar by
    //     this task), so `fe`(mic_in) gets base offset 0 and `rf`(bt_in) gets
    //     base offset 256 — matching the reference generator's hand-written
    //     layout, NOT the byte-reversed DataId order.
    let res = try_emit_bin_example_multi_with_files(
        "14-hearing-aid",
        "prog.embedded.algo.nuc",
        "embedded_multimcu_sync.sched.nuc",
        "kernels.embedded.rs",
        "ex14_multimcu_input_partition",
    )
    .expect(
        "ex14's cross-worker input partition must now EMIT (declaration order \
         is threaded into the contract, TASK-0049.10.06) — past the old \
         fail-loud rejection",
    );

    // The per-worker shim seeds its input cursor from
    // `const NUC_INPUT_BASE: usize = <offset>;` (skeleton/multimcu.rs). Read
    // fe's and rf's emitted main.rs and assert the declaration-order offsets.
    // ex14 frame = N_FRAMES(4) * SAMPLES_PER_FRAME(16) = 64 i32 = 256 bytes.
    let read_base = |worker: &str| -> String {
        let w = res
            .workers
            .iter()
            .find(|b| b.worker_name.as_deref() == Some(worker))
            .unwrap_or_else(|| panic!("emit produced no `{worker}` worker bin"));
        std::fs::read_to_string(&w.main_rs).expect("read worker main.rs")
    };
    let fe_main = read_base("fe");
    let rf_main = read_base("rf");
    assert!(
        fe_main.contains("const NUC_INPUT_BASE: usize = 0;"),
        "fe loads mic_in (declared FIRST) -> NUC_INPUT_BASE 0; emitted main.rs \
         did not carry it"
    );
    assert!(
        rf_main.contains("const NUC_INPUT_BASE: usize = 256;"),
        "rf loads bt_in (declared SECOND, 256 bytes after mic_in) -> \
         NUC_INPUT_BASE 256; emitted main.rs did not carry it. A byte-WRONG \
         DataId-order layout would have put rf at 0 and fe at 256."
    );
}

/// Read the `output_captures.txt` capture manifest beside the emitted
/// `.resc` (TASK-0049.10.08, BLOCKER 3 slice D). The manifest lives at
/// `out_dir/output_captures.txt`; `res.resc` is `out_dir/multimcu.resc`,
/// so the manifest is its sibling.
fn read_capture_manifest(res: &crate::MultiBinEmitResult) -> String {
    let resc = res
        .resc
        .as_ref()
        .expect("multi-worker emit must produce a .resc (and a manifest beside it)");
    let manifest = resc
        .parent()
        .expect("resc has a parent out_dir")
        .join("output_captures.txt");
    std::fs::read_to_string(&manifest)
        .unwrap_or_else(|e| panic!("read capture manifest {}: {e}", manifest.display()))
}

#[test]
fn ex14_capture_manifest_is_decl_order_fe_then_rf() {
    // TASK-0049.10.08 (BLOCKER 3 slice D): the multi-SAVER manifest lists one
    // `file_var` per saver in `TransportPlan.output_captures` order, which is
    // DECL order (spk_out@fe before bt_out@rf), NOT WorkerId / DataId order.
    // This is the single source of truth the `renode-multimcu` recipe reads
    // for BOTH var-injection and concat order, so the captured files
    // reconstruct the reference layout (spk_out@0 ++ bt_out@256).
    let res = try_emit_bin_example_multi_with_files(
        "14-hearing-aid",
        "prog.embedded.algo.nuc",
        "embedded_multimcu_sync.sched.nuc",
        "kernels.embedded.rs",
        "ex14_capture_manifest",
    )
    .expect("ex14 multi-MCU bin emit");
    let manifest = read_capture_manifest(&res);
    // Two savers, decl order: fe (spk_out, declared first) then rf (bt_out).
    let lines: Vec<&str> = manifest.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(
        lines,
        vec!["feUart", "rfUart"],
        "ex14 capture manifest must be decl-order [feUart, rfUart] \
         (spk_out before bt_out), not WorkerId/DataId order; got:\n{manifest}"
    );
}

#[test]
fn single_saver_capture_manifest_is_lone_uartfile() {
    // TASK-0049.10.08 (BLOCKER 3 slice D): the SINGLE-saver schedule
    // (02-split-add, host drains `c`) keeps the recipe-compatible `uartFile`
    // var and the manifest is exactly that one line — concat-of-one is
    // byte-identical to the captured file, so the 1024B reference diff is
    // unchanged.
    let res = emit_bin_example_multi("02-split-add", "split.sched.nuc", "split_manifest");
    let manifest = read_capture_manifest(&res);
    let lines: Vec<&str> = manifest.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(
        lines,
        vec!["uartFile"],
        "single-saver 02-split-add manifest must be the lone `uartFile` line; got:\n{manifest}"
    );
}

// ---- TASK-0048.04: no_std monotonic clock for Event::Loop check_frame ----

/// Lower example 1 with `schedule_file` (a schedule under
/// 01-elementwise-add/schedules/) WITH check frames injected, and emit
/// the no_std BIN. Returns the `emit_bin` Result so a caller can assert
/// either the emitted source (Log path) or a typed rejection (Panic /
/// Count). `inject_check_frames: true` so the `check loop` directive in
/// the schedule projects into `Event::Loop.check_frame` exactly as the
/// driver does (driver main.rs runs inject_check_frames unconditionally).
fn try_emit_bin_ex1_with_check(
    schedule_file: &str,
    scratch_leaf: &str,
) -> Result<String, crate::EmitError> {
    use crate::emit_bin;
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/01-elementwise-add");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).expect("algo source");
    let sched_src =
        std::fs::read_to_string(ex.join("schedules").join(schedule_file)).expect("sched source");

    let r = test_common::lower_for_test(
        &algo_src,
        &sched_src,
        &test_common::LowerForTestOpts {
            apply_block_transforms: false,
            apply_partition_workers: false,
            inject_check_frames: true,
        },
    );
    let kernels = ex.join("kernels.rs");
    // TASK-0426.01: per-call-unique scratch (created once, never removed).
    let out = test_common::unique_scratch_dir(
        &root.join("nucleus/target/embedded-pattern-test-scratch"),
        scratch_leaf,
    );

    emit_bin(&r.per_worker, &r.names, &r.sidecar, &kernels, &out).map(|res| {
        // ex1 is single-worker: exactly one bin at the root.
        std::fs::read_to_string(&res.workers[0].main_rs).expect("read emitted main.rs")
    })
}

#[test]
fn check_loop_log_lowers_to_systick_clock_and_uart_report() {
    // AC#1 + AC#2: a `check loop i : latency_max=…, on_violation=log`
    // frame lowers using the no_std SysTick clock (via the trait method
    // shim.monotonic_ns) and the report_violation UART sink — NOT
    // std::time::Instant and NOT a Drop-guard AtomicU64 reporter.
    let main = try_emit_bin_ex1_with_check("embedded_check.sched.nuc", "ex01_check_log")
        .expect("check-loop log frame must lower on the embedded bin path");

    // The per-iteration wall-clock wrapping uses the trait clock method.
    assert!(
        main.contains("let _check_start = shim.monotonic_ns();"),
        "check frame must start the per-iteration clock via shim.monotonic_ns():\n{main}"
    );
    assert!(
        main.contains("let _check_elapsed = shim.monotonic_ns().wrapping_sub(_check_start);"),
        "check frame must compute elapsed via shim.monotonic_ns():\n{main}"
    );
    // The on-violation branch routes to the report_violation UART sink,
    // carrying the loop_var name as a byte-string literal.
    assert!(
        main.contains("shim.report_violation(b\"i\", _check_elapsed,"),
        "log on-violation branch must call shim.report_violation(b\"i\", ...):\n{main}"
    );
    // The clock is SysTick (the chosen tier-3 source), wired by main.
    assert!(
        main.contains("systick_init();"),
        "main must start the SysTick monotonic clock before run:\n{main}"
    );
    assert!(
        main.contains("const SYST_CVR:"),
        "the SysTick current-value register must be defined:\n{main}"
    );
    // CRITICAL no_std invariants: the tier-1 std clock + atomic must be
    // ABSENT — they do not exist / compile on no_std thumbv7em.
    assert!(
        !main.contains("std::time::Instant"),
        "std::time::Instant must NOT appear in a no_std firmware:\n{main}"
    );
    assert!(
        !main.contains("AtomicU64"),
        "AtomicU64 (unavailable on thumbv7em) must NOT appear:\n{main}"
    );
    // The eprintln! MACRO CALL must not appear (no stderr on no_std). The
    // `eprintln!` token does appear in docstrings as the named tier-1
    // analogue we are NOT using; match the call form `eprintln!(` to
    // exclude those backtick-quoted comment references.
    assert!(
        !main.contains("eprintln!("),
        "an eprintln! macro call (no stderr on no_std) must NOT appear:\n{main}"
    );
    // The compute still lowers (the check wrapping is additive).
    assert!(
        main.contains("c[(i) as usize] = kernels::add("),
        "the wrapped loop body must still lower the compute Fire:\n{main}"
    );
}

#[test]
fn check_loop_panic_is_rejected_with_brick_warning() {
    // AC#2 + AC#4: on_violation=panic (explicit OR defaulted) bricks the
    // device on tier-3; the embedded backend REJECTS it with a typed
    // EmitError directing the user to on_violation=log — it must NOT
    // silently remap panic->log. naive.sched.nuc carries no check frame,
    // so a panic frame is exercised via a dedicated fixture schedule.
    let err = try_emit_bin_ex1_with_check("embedded_check_panic.sched.nuc", "ex01_check_panic")
        .expect_err("on_violation=panic must be rejected on tier-3");
    let msg = format!("{err}");
    assert!(
        msg.contains("panic") && msg.contains("brick"),
        "panic rejection must explain that panic bricks the device: got {msg}"
    );
    assert!(
        msg.contains("on_violation = log"),
        "panic rejection must direct the user to on_violation=log: got {msg}"
    );
}

#[test]
fn check_loop_count_lowers_to_atomic_u32_static_and_program_exit_summary() {
    // TASK-0048.08, PART 1 (flipped from the former negative rejection
    // test): on_violation=count now LOWERS on tier-3. The bin emits (a) a
    // module-scope AtomicU32 counter, (b) a per-iteration fetch_add on
    // violation (same SysTick timing as the log path), and (c) a USART1
    // summary after run() returns and before the loop {} spin (the
    // bare-metal program-exit sink — a firmware spins forever so the
    // tier-1 Drop-guard summary never fires).
    let main = try_emit_bin_ex1_with_check("embedded_check_count.sched.nuc", "ex01_check_count")
        .expect("on_violation=count must LOWER on the embedded bin path (TASK-0048.08)");

    // Same SysTick per-iteration wall-clock as the log path.
    assert!(
        main.contains("let _check_start = shim.monotonic_ns();"),
        "count frame must start the per-iteration clock via shim.monotonic_ns():\n{main}"
    );
    assert!(
        main.contains("let _check_elapsed = shim.monotonic_ns().wrapping_sub(_check_start);"),
        "count frame must compute elapsed via shim.monotonic_ns():\n{main}"
    );
    // (a) module-scope AtomicU32 counter — AtomicU64 is absent on thumbv7em.
    assert!(
        main.contains(
            "static NUC_CHECK_COUNT_i: core::sync::atomic::AtomicU32 = \
             core::sync::atomic::AtomicU32::new(0);"
        ),
        "count frame must emit a module-scope AtomicU32 counter:\n{main}"
    );
    assert!(
        !main.contains("AtomicU64"),
        "AtomicU64 (unavailable on thumbv7em) must NOT appear:\n{main}"
    );
    // (b) the on-violation branch increments the counter (NOT report_violation).
    assert!(
        main.contains("NUC_CHECK_COUNT_i.fetch_add(1, core::sync::atomic::Ordering::Relaxed);"),
        "count on-violation branch must fetch_add the AtomicU32 counter:\n{main}"
    );
    assert!(
        !main.contains("shim.report_violation"),
        "count must NOT route through the log report_violation sink:\n{main}"
    );
    // (c) the program-exit summary over USART1 reads the counter.
    assert!(
        main.contains(
            "usart1_put_u64(NUC_CHECK_COUNT_i.load(core::sync::atomic::Ordering::Relaxed) \
             as u64);"
        ),
        "count must flush a USART1 summary reading the counter at program exit:\n{main}"
    );
    // No tier-1 Drop-guard / std atomic machinery (does not port to no_std).
    assert!(
        !main.contains("Drop for") && !main.contains("std::sync::atomic"),
        "the tier-1 Drop-guard / std-atomic count sink must NOT appear:\n{main}"
    );
    // The compute still lowers (the count wrapping is additive).
    assert!(
        main.contains("c[(i) as usize] = kernels::add("),
        "the wrapped loop body must still lower the compute Fire:\n{main}"
    );
}
