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
    // The lib path returned must exist on disk.
    assert!(res.lib_rs.exists(), "emitted lib.rs must exist on disk");
    assert!(res.cargo_toml.exists(), "emitted Cargo.toml must exist");
    std::fs::read_to_string(&res.lib_rs).expect("read emitted lib.rs")
}

/// Lower `example`'s `naive` schedule and emit the no_std BIN (M10
/// `--shim stm32h7`) into a scratch dir; return the emitted main.rs
/// source. The bin path is the SAME lowering as the lib, with the
/// bare-metal scaffolding (cortex-m-rt entry + USART1 shim) added — so
/// the cross-compile / Renode run is the genuine acceptance (the
/// `renode-embedded-ex1` just recipe). This is the fast shape-drift
/// detector (TASK-0048.01).
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

    // no_std + the four-method shim trait + stub (AC#1/#2/#3).
    assert!(lib.contains("#![no_std]"), "must be no_std:\n{lib}");
    assert!(lib.contains("pub trait NucleusShim"), "trait missing:\n{lib}");
    for m in [
        "fn alloc_in_region",
        "fn dma_push",
        "fn dma_wait",
        "fn irq_barrier",
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
fn rejects_multi_worker_with_m11_forward_link() {
    // A 2-worker schedule must be rejected with a precise M11 forward
    // link rather than mis-lowered (M9 is single-worker only). Build a
    // minimal 2-used-worker per_worker map directly: two workers each
    // carrying one bare Fire. (We avoid the full pipeline here — the
    // point is the used_workers > 1 guard.)
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
    let out = repo_root().join("nucleus/target/embedded-pattern-test-scratch/reject_multi");
    let err = emit(&per_worker, &names, &sidecar, &kernels, &out)
        .expect_err("multi-worker must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("single-worker") && msg.contains("TASK-0049"),
        "multi-worker rejection must forward-link M11 (TASK-0049): got {msg}"
    );
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
