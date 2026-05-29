//! End-to-end emit tests for the embedded-pattern backend (TASK-0047).
//!
//! These lower a real example (via `test_common::lower_for_test`, the
//! same IR-stage pipeline the driver runs) and assert the emitted
//! `no_std` lib SHAPE. They do NOT cross-compile — that is the dedicated
//! `just check-embedded` recipe's job (it needs the `.#embedded` dev
//! shell's thumbv7em-none-eabihf rust-std, which the default `just test`
//! shell does not have). The shape assertions here are the fast
//! drift-detection layer; the recipe is the genuine compile acceptance
//! (AC#4).

use std::path::PathBuf;

use crate::emit;

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = nucleus/backends/embedded-pattern. Three
    // ancestors up is the repo root (mirrors openmp-rs's test helper).
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .expect("three ancestors above embedded-pattern crate")
        .to_path_buf()
}

/// Lower `example`'s `naive` schedule and emit the no_std lib into a
/// scratch dir; return the emitted lib.rs source.
fn emit_example_naive(example: &str, scratch_leaf: &str) -> String {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples").join(example);
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).expect("algo source");
    let sched_src =
        std::fs::read_to_string(ex.join("schedules/naive.sched.nuc")).expect("sched source");

    // Naive single-worker: no block transforms, no partition, no check
    // frames (mirrors the driver's behaviour on a naive schedule and
    // openmp-rs's single-worker test).
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

    let out = root
        .join("nucleus/target/embedded-pattern-test-scratch")
        .join(scratch_leaf);
    let _ = std::fs::remove_dir_all(&out);

    let res = emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &out).expect("embedded emit");
    // A naive single-worker schedule emits EXACTLY ONE project, at the
    // out_dir root (worker_name None) — the M9 single-worker output is
    // unchanged by the TASK-0049.04 N-projects refactor.
    assert_eq!(
        res.workers.len(),
        1,
        "a naive single-worker schedule must emit exactly one lib project"
    );
    let w = &res.workers[0];
    assert!(
        w.worker_name.is_none(),
        "single-worker project must be emitted at the out_dir root (no worker subdir)"
    );
    assert_eq!(
        w.project_dir, out,
        "single-worker project_dir must equal out_dir (root), not a worker subdir"
    );
    // The lib path returned must exist on disk.
    assert!(w.lib_rs.exists(), "emitted lib.rs must exist on disk");
    assert!(w.cargo_toml.exists(), "emitted Cargo.toml must exist");
    std::fs::read_to_string(&w.lib_rs).expect("read emitted lib.rs")
}

/// Lower `example`'s `schedule_file` schedule (a MULTI-worker one) and
/// emit the per-worker no_std libs into a scratch dir; return the
/// [`MultiEmitResult`]. The TASK-0049.04 multi-worker LIB path
/// (one project per used worker under `out_dir/<worker>/`, with
/// Push/Wait/Sync lowered to stub-shim hooks). `apply_partition_workers`
/// stays false (the 02-split fixture splits across worker boundaries
/// only — no `partition=` directive), matching how the driver lowers a
/// non-partitioned multi-worker schedule.
fn emit_example_multi(example: &str, schedule_file: &str, scratch_leaf: &str) -> crate::MultiEmitResult {
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

    let out = root
        .join("nucleus/target/embedded-pattern-test-scratch")
        .join(scratch_leaf);
    let _ = std::fs::remove_dir_all(&out);

    emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &out).expect("embedded multi-worker emit")
}

/// Lower a MULTI-worker schedule against an EXPLICIT algorithm +
/// kernels file (NOT the default `prog.algo.nuc` / `kernels.rs`) and
/// emit the per-worker no_std libs. Used by the TASK-0049.06 real
/// example-14 test, where the embedded shape uses
/// `prog.embedded.algo.nuc` + the no_std-clean `kernels.embedded.rs`
/// (the tier-1 `kernels.rs` has `Vec<i32>` bodies that are not extracted
/// for the embedded backend). Mirrors `emit_example_multi` but with the
/// two filenames parameterised.
fn emit_example_multi_with_files(
    example: &str,
    algo_file: &str,
    schedule_file: &str,
    kernels_file: &str,
    scratch_leaf: &str,
) -> crate::MultiEmitResult {
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

    let out = root
        .join("nucleus/target/embedded-pattern-test-scratch")
        .join(scratch_leaf);
    let _ = std::fs::remove_dir_all(&out);

    emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &out).expect("embedded multi-worker emit")
}

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

    let out = root
        .join("nucleus/target/embedded-pattern-test-scratch")
        .join(scratch_leaf);
    let _ = std::fs::remove_dir_all(&out);

    let res =
        emit_bin(&r.per_worker, &r.names, &r.sidecar, &kernels, &out).expect("embedded bin emit");
    // Every bare-metal scaffolding file must land on disk.
    assert!(res.main_rs.exists(), "emitted src/main.rs must exist");
    assert!(res.cargo_toml.exists(), "emitted Cargo.toml must exist");
    assert!(res.memory_x.exists(), "emitted memory.x must exist");
    assert!(res.build_rs.exists(), "emitted build.rs must exist");
    assert!(res.cargo_config.exists(), "emitted .cargo/config.toml must exist");
    std::fs::read_to_string(&res.main_rs).expect("read emitted main.rs")
}

#[test]
fn ex01_emits_no_std_lib_with_shim_and_pure_add() {
    let lib = emit_example_naive("01-elementwise-add", "ex01_naive");

    // no_std + the six-method shim trait + stub (AC#1/#2/#3). The four
    // M9 methods plus the two M10 TASK-0048.04 tier-3 methods
    // (monotonic_ns + report_violation).
    assert!(lib.contains("#![no_std]"), "must be no_std:\n{lib}");
    assert!(lib.contains("pub trait NucleusShim"), "trait missing:\n{lib}");
    for m in [
        "fn alloc_in_region",
        "fn dma_push",
        "fn dma_wait",
        "fn irq_barrier",
        "fn monotonic_ns",
        "fn report_violation",
    ] {
        assert!(lib.contains(m), "shim method {m} missing:\n{lib}");
    }
    assert!(lib.contains("struct StubShim"), "stub shim missing:\n{lib}");

    // The PURE kernel `add` is extracted verbatim into mod kernels.
    assert!(lib.contains("mod kernels {"), "kernels mod missing:\n{lib}");
    assert!(
        lib.contains("pub fn add(a: i32, b: i32) -> i32"),
        "pure kernel `add` not extracted verbatim:\n{lib}"
    );
    assert!(
        lib.contains("a.wrapping_add(b)"),
        "pure kernel body not copied verbatim:\n{lib}"
    );

    // The EFFECTFUL kernels (load_input/load_input_b/save_output) MUST
    // NOT be emitted as kernel fns (they are std-bound). Their std
    // imports must be entirely absent.
    assert!(
        !lib.contains("std::fs"),
        "std::fs leaked into no_std lib:\n{lib}"
    );
    assert!(
        !lib.contains("fn load_input"),
        "effectful kernel body leaked into no_std lib:\n{lib}"
    );

    // The indexed compute Fire lowers to a kernels::add call.
    assert!(
        lib.contains("kernels::add("),
        "indexed compute Fire did not call kernels::add:\n{lib}"
    );
    // Data arrays are fixed [i32; 256] locals (alloc-free).
    assert!(
        lib.contains("[i32; 256] = [0; 256]"),
        "data arrays not fixed-size no_std arrays:\n{lib}"
    );
    // Effectful I/O mapped to shim hooks.
    assert!(
        lib.contains("shim.alloc_in_region("),
        "effectful input not mapped to shim alloc hook:\n{lib}"
    );
    assert!(
        lib.contains("shim.dma_push("),
        "effectful output not mapped to shim dma_push hook:\n{lib}"
    );
    // The run entry point.
    assert!(
        lib.contains("pub fn run<S: NucleusShim>(shim: &mut S) {"),
        "run entry point missing:\n{lib}"
    );
}

#[test]
fn ex01_bin_emits_renode_runnable_firmware_with_uart_streaming() {
    // M10 (TASK-0048.01): the --shim stm32h7 bin path for example 1.
    let main = emit_bin_example_naive("01-elementwise-add", "ex01_bin_naive");

    // no_std / no_main firmware with cortex-m-rt entry + panic handler.
    assert!(main.contains("#![no_std]"), "must be no_std:\n{main}");
    assert!(main.contains("#![no_main]"), "must be no_main:\n{main}");
    assert!(main.contains("use cortex_m_rt::entry;"), "missing cortex-m-rt:\n{main}");
    assert!(main.contains("#[entry]"), "missing #[entry]:\n{main}");
    assert!(main.contains("#[panic_handler]"), "missing #[panic_handler]:\n{main}");

    // Same trait surface as the lib (verbatim reuse) + the concrete shim.
    assert!(main.contains("pub trait NucleusShim"), "trait missing:\n{main}");
    assert!(
        main.contains("impl NucleusShim for Usart1Shim"),
        "Usart1Shim impl missing:\n{main}"
    );

    // The pure kernel is extracted verbatim (same as the lib path).
    assert!(
        main.contains("pub fn add(a: i32, b: i32) -> i32"),
        "pure kernel `add` not extracted:\n{main}"
    );
    assert!(main.contains("a.wrapping_add(b)"), "kernel body not verbatim:\n{main}");
    assert!(!main.contains("std::fs"), "std::fs leaked into no_std bin:\n{main}");

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

    // TASK-0048.02: dma_push streams the RAW output bytes verbatim (the
    // byte-exact reference.bin diff is the value-correctness bar); the
    // old ASCII summary line (NUC-EX1 / checksum) is GONE.
    assert!(
        main.contains("usart1_putc(byte);"),
        "dma_push must stream raw output bytes over USART1:\n{main}"
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
    assert!(main.contains("let mut shim = Usart1Shim::new();"), "main must build shim:\n{main}");
    assert!(main.contains("run(&mut shim);"), "main must call run:\n{main}");
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
    assert!(main.contains("#[panic_handler]"), "missing #[panic_handler]:\n{main}");

    // The PURE blur3 kernel extracted verbatim (9-param multiline sig + body).
    assert!(main.contains("pub fn blur3("), "pure kernel `blur3` not extracted:\n{main}");
    assert!(main.contains("sum / 9"), "blur3 body not copied verbatim:\n{main}");
    assert!(!main.contains("std::fs"), "std::fs leaked into no_std bin:\n{main}");

    // 2D flatten: img[y][x] -> img[y*16 + x] (same flatten as tier-1 / the lib).
    assert!(main.contains("* 16 +"), "2D index not flattened row-major:\n{main}");
    assert!(main.contains("kernels::blur3("), "compute Fire did not call kernels::blur3:\n{main}");
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
    assert!(main.contains("usart1_putc(byte);"), "dma_push must stream raw bytes:\n{main}");
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
    assert!(main.contains("pub fn produce(seed: i32) -> i32"), "produce not extracted:\n{main}");
    assert!(main.contains("seed.wrapping_mul(3)"), "produce body not verbatim:\n{main}");
    assert!(
        main.contains("pub fn transform(rec: i32) -> i32"),
        "transform not extracted:\n{main}"
    );
    assert!(
        main.contains("rec.wrapping_mul(7).wrapping_add(rec)"),
        "transform body not verbatim:\n{main}"
    );
    assert!(!main.contains("std::fs"), "std::fs leaked into no_std bin:\n{main}");

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
    assert!(main.contains("usart1_putc(byte);"), "dma_push must stream raw bytes:\n{main}");
}

#[test]
fn ex05_emits_no_std_lib_with_flattened_blur3() {
    let lib = emit_example_naive("05-stencil", "ex05_naive");

    assert!(lib.contains("#![no_std]"));
    assert!(lib.contains("pub trait NucleusShim"));

    // Pure blur3 extracted verbatim (9-param multiline signature + body).
    assert!(
        lib.contains("pub fn blur3("),
        "pure kernel `blur3` not extracted:\n{lib}"
    );
    assert!(
        lib.contains("sum / 9"),
        "blur3 body not copied verbatim:\n{lib}"
    );

    // No std leakage.
    assert!(!lib.contains("std::fs"), "std::fs leaked:\n{lib}");
    assert!(
        !lib.contains("fn load_image"),
        "effectful kernel leaked:\n{lib}"
    );

    // 2D flatten: img[y][x] -> img[y*16 + x] (same flatten as tier-1).
    assert!(
        lib.contains("* 16 +"),
        "2D index not flattened row-major:\n{lib}"
    );
    assert!(
        lib.contains("kernels::blur3("),
        "compute Fire did not call kernels::blur3:\n{lib}"
    );
    // 16*16 = 256-element fixed arrays.
    assert!(
        lib.contains("[i32; 256] = [0; 256]"),
        "stencil arrays not fixed-size [i32; 256]:\n{lib}"
    );
}

#[test]
fn multi_worker_lib_emits_one_project_per_worker_with_transport_hooks() {
    // TASK-0049.04 (M11 backend slice A): the formerly-rejecting
    // multi-worker case now SUCCEEDS on the LIB path. The 02-split-add
    // `split` schedule is a no_std-clean 2-worker SYNC fixture (host +
    // w0, `default` class, `heap` region, sync transfers a/b/c, no
    // block=/async/buffer/event, `add:(i32,i32)->i32` pure). It passes
    // the capability gate and exercises exactly the structural change.
    //
    // This REPLACES the former `rejects_multi_worker_with_m11_forward_link`
    // lib-path rejection test (the guard at lib.rs emit() is LIFTED). The
    // BIN-path sibling `bin_rejects_multi_worker_with_m11_forward_link`
    // still asserts rejection (the bin guard is retained — TASK-0049.05).
    let res = emit_example_multi("02-split-add", "split.sched.nuc", "split_multi");

    // Exactly two projects, one per used worker (host + w0).
    assert_eq!(
        res.workers.len(),
        2,
        "the 2-worker split schedule must emit exactly two lib projects"
    );
    // Each is emitted under out_dir/<worker_name>/ (NOT the root — that
    // is the single-worker shape). Names come from NameTables.
    let mut names: Vec<&str> = res
        .workers
        .iter()
        .map(|w| {
            w.worker_name
                .as_deref()
                .expect("a multi-worker project must carry its worker name")
        })
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec!["host", "w0"],
        "the two projects must be named for the two used workers (host, w0)"
    );
    for w in &res.workers {
        let name = w.worker_name.as_deref().unwrap();
        assert!(
            w.project_dir.ends_with(name),
            "worker `{name}` project must be nested under out_dir/{name}/: {}",
            w.project_dir.display()
        );
        assert!(w.lib_rs.exists(), "worker `{name}` lib.rs must exist on disk");
        assert!(w.cargo_toml.exists(), "worker `{name}` Cargo.toml must exist");
    }

    // The two libs' sources, by worker name.
    let read = |name: &str| -> String {
        let w = res
            .workers
            .iter()
            .find(|w| w.worker_name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("worker `{name}` project missing"));
        std::fs::read_to_string(&w.lib_rs).expect("read worker lib.rs")
    };
    let host = read("host");
    let w0 = read("w0");

    // --- AC#2: Push/Wait/Sync lower to the stub-shim hooks. ---
    // host loads a + b then PUSHES them to w0; the Push lowers to
    // dma_push with the data-ptr template (mirrors the save->dma_push).
    assert!(
        host.contains("shim.dma_push(0, a.as_ptr() as *const u8, core::mem::size_of_val(&a));"),
        "host must Push `a` via dma_push (data-ptr template):\n{host}"
    );
    assert!(
        host.contains("shim.dma_push(1, b.as_ptr() as *const u8, core::mem::size_of_val(&b));"),
        "host must Push `b` via dma_push:\n{host}"
    );
    // host WAITs for the computed `c` back from w0.
    assert!(
        host.contains("shim.dma_wait(2);"),
        "host must Wait (dma_wait) for `c` on its Push/Wait seq:\n{host}"
    );
    // w0 WAITs for a + b, computes, then PUSHES c back.
    assert!(
        w0.contains("shim.dma_wait(0);") && w0.contains("shim.dma_wait(1);"),
        "w0 must Wait (dma_wait) for both `a` and `b`:\n{w0}"
    );
    assert!(
        w0.contains("shim.dma_push(2, c.as_ptr() as *const u8, core::mem::size_of_val(&c));"),
        "w0 must Push the computed `c` back via dma_push:\n{w0}"
    );
    // The compute Fire still lowers on the worker that owns it (w0).
    assert!(
        w0.contains("c[(i) as usize] = kernels::add(a[(i) as usize], b[(i) as usize]);"),
        "w0 must lower the pure `add` compute Fire:\n{w0}"
    );
    // Sync barriers lower to irq_barrier on BOTH workers (matching tags
    // — they are the cross-worker join key).
    assert!(
        host.contains("shim.irq_barrier(") && w0.contains("shim.irq_barrier("),
        "Sync must lower to irq_barrier on both workers"
    );

    // --- per-worker receive locals (the Wait-side data must be in scope) ---
    // w0 never loads a/b itself (host does) — they arrive via Wait — so
    // w0 must still DECLARE a + b as zero-init fixed locals (the receive
    // buffers). Likewise host must declare c (the Wait receive local).
    assert!(
        w0.contains("let mut a: [i32; 256] = [0; 256];")
            && w0.contains("let mut b: [i32; 256] = [0; 256];"),
        "w0 must declare the Wait receive locals a + b as zero-init fixed arrays:\n{w0}"
    );
    assert!(
        host.contains("let mut c: [i32; 256] = [0; 256];"),
        "host must declare the Wait receive local c as a zero-init fixed array:\n{host}"
    );

    // --- both libs are no_std with the shim trait (compile-only shape) ---
    for (name, src) in [("host", &host), ("w0", &w0)] {
        assert!(src.contains("#![no_std]"), "worker `{name}` must be no_std:\n{src}");
        assert!(
            src.contains("pub trait NucleusShim"),
            "worker `{name}` must carry the NucleusShim trait:\n{src}"
        );
        assert!(
            !src.contains("std::fs"),
            "worker `{name}` must not leak std into the no_std lib:\n{src}"
        );
    }
}

#[test]
fn real_ex14_sync_emits_three_workers_with_array_typed_pure_kernels() {
    // TASK-0049.06 (M11 backend slice A follow-up): the REAL example-14
    // hearing-aid, via the SYNCHRONOUS sibling schedule
    // `embedded_multimcu_sync` (3 default-class workers fe/dsp/rf, sync
    // transfers, NO async/buffer/event/named-regions) + the no_std-clean
    // `kernels.embedded.rs` (mix2/denoise as fixed-array `[i32; 16]`
    // in/out). This is the literal 3-worker ex14 cross-compile that
    // TASK-0049.04 AC#3 demanded (the structural slice proved it with
    // the 02-split-add proxy; this closes it on the real algorithm).
    //
    // The KEY assertion: array-typed PURE kernel Fire args lower to the
    // no_std fixed-array form `.try_into().unwrap()` — NOT the tier-1
    // `.to_vec()`, which needs `alloc`/`Vec` and would not cross-compile
    // under no_std (the GAP-C lowering gap this task surfaced + fixed in
    // backend_common::render_fire_args_nostd / SubArrayForm::FixedArray).
    let res = emit_example_multi_with_files(
        "14-hearing-aid",
        "prog.embedded.algo.nuc",
        "embedded_multimcu_sync.sched.nuc",
        "kernels.embedded.rs",
        "ex14_sync_multi",
    );

    // Exactly three projects, one per used worker (fe, dsp, rf).
    let mut names: Vec<&str> = res
        .workers
        .iter()
        .map(|w| {
            w.worker_name
                .as_deref()
                .expect("a multi-worker project must carry its worker name")
        })
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec!["dsp", "fe", "rf"],
        "the real ex14 sync schedule must emit exactly three per-worker projects (fe/dsp/rf)"
    );

    let read = |name: &str| -> String {
        let w = res
            .workers
            .iter()
            .find(|w| w.worker_name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("worker `{name}` project missing"));
        std::fs::read_to_string(&w.lib_rs).expect("read worker lib.rs")
    };
    let dsp = read("dsp");

    // --- GAP C: array-typed pure-kernel Fire args use the no_std
    //     fixed-array form, NOT `.to_vec()`. The dsp worker runs both
    //     pure kernels (mix2 + the two denoise firings), each taking a
    //     `[i32; 16]` row of a 2D `i32[N_FRAMES][16]` datum. ---
    assert!(
        dsp.contains(".try_into().unwrap()"),
        "dsp must lower array-typed pure-kernel args as no_std fixed arrays \
         (.try_into().unwrap()):\n{dsp}"
    );
    assert!(
        !dsp.contains(".to_vec()"),
        "dsp must NOT use `.to_vec()` (needs alloc/Vec, not no_std-clean) for \
         array-typed pure-kernel args:\n{dsp}"
    );
    // The pure kernels are extracted verbatim into mod kernels with the
    // fixed-array (NOT Vec) signature from kernels.embedded.rs.
    assert!(
        dsp.contains("pub fn mix2(a: [i32; 16], b: [i32; 16]) -> [i32; 16]"),
        "mix2 not extracted with the no_std fixed-array signature:\n{dsp}"
    );
    assert!(
        dsp.contains("pub fn denoise(buf: [i32; 16]) -> [i32; 16]"),
        "denoise not extracted with the no_std fixed-array signature:\n{dsp}"
    );
    // The pure compute Fires call the extracted kernels.
    assert!(
        dsp.contains("kernels::mix2(") && dsp.contains("kernels::denoise("),
        "dsp must call the extracted pure kernels mix2 + denoise:\n{dsp}"
    );

    // --- cross-worker transport: dsp Waits its inputs and Pushes its
    //     outputs over the stub-shim hooks (the inter-MCU transport). ---
    assert!(
        dsp.contains("shim.dma_wait(") && dsp.contains("shim.dma_push("),
        "dsp must lower its cross-worker Wait/Push to the stub-shim hooks:\n{dsp}"
    );

    // --- producer-side transport: fe (mic_in) and rf (bt_in) must Push
    //     their captured frames to dsp — the transport is not dsp-only
    //     (TASK-0049.06 review P3.3: pin the producer side too). ---
    for name in ["fe", "rf"] {
        let src = read(name);
        assert!(
            src.contains("shim.dma_push("),
            "producer worker `{name}` must Push its captured data over the \
             stub-shim hook:\n{src}"
        );
    }

    // --- every worker is a no_std lib with the shim trait + no std leak,
    //     and lowers its Event::Sync barriers to shim.irq_barrier (the
    //     cross-MCU ordering primitive; TASK-0049.06 review P3.3). ---
    for name in ["fe", "dsp", "rf"] {
        let src = read(name);
        assert!(src.contains("#![no_std]"), "worker `{name}` must be no_std:\n{src}");
        assert!(
            src.contains("pub trait NucleusShim"),
            "worker `{name}` must carry the NucleusShim trait:\n{src}"
        );
        assert!(
            !src.contains("std::fs") && !src.contains(".to_vec()"),
            "worker `{name}` must not leak std / use Vec into the no_std lib:\n{src}"
        );
        assert!(
            src.contains("shim.irq_barrier("),
            "worker `{name}` must lower Event::Sync to shim.irq_barrier (the \
             cross-MCU ordering barrier):\n{src}"
        );
    }
}

#[test]
fn bin_rejects_multi_worker_with_m11_forward_link() {
    // The M10 bin path (emit_bin) must reject multi-worker IDENTICALLY
    // to the lib path — the single-worker guard is duplicated in
    // emit_bin (it cannot share emit's, having a different return type),
    // so pin both so a future edit to one cannot silently diverge
    // (feedback-silent-sibling-defect).
    use crate::emit_bin;
    use nucleus_compiler::event::{Event, FireBinding, IterTile, KernelId, WorkerId};
    use std::collections::BTreeMap;
    let bare_fire = || Event::Fire {
        kernel: KernelId(0),
        tile: IterTile::default(),
        bindings: FireBinding::default(),
    };
    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(WorkerId(0), vec![bare_fire()]);
    per_worker.insert(WorkerId(1), vec![bare_fire()]);
    let names = crate::NameTables::default();
    let sidecar = nucleus_compiler::sidecar::NameSidecar::default();
    let kernels = repo_root().join("nuc-nucleus/examples/01-elementwise-add/kernels.rs");
    let out = repo_root().join("nucleus/target/embedded-pattern-test-scratch/reject_multi_bin");
    let err = emit_bin(&per_worker, &names, &sidecar, &kernels, &out)
        .expect_err("multi-worker bin must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("single-worker") && msg.contains("TASK-0049"),
        "multi-worker bin rejection must forward-link M11 (TASK-0049): got {msg}"
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
    let out = root
        .join("nucleus/target/embedded-pattern-test-scratch")
        .join(scratch_leaf);
    let _ = std::fs::remove_dir_all(&out);

    emit_bin(&r.per_worker, &r.names, &r.sidecar, &kernels, &out)
        .map(|res| std::fs::read_to_string(&res.main_rs).expect("read emitted main.rs"))
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
        main.contains(
            "NUC_CHECK_COUNT_i.fetch_add(1, core::sync::atomic::Ordering::Relaxed);"
        ),
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
