//! End-to-end emit tests for the embedded-pattern backend (TASK-0047).
//!
//! These lower a real example (via `test_common::lower_for_test`, the
//! same IR-stage pipeline the driver runs) and assert the emitted
//! `no_std` SHAPE. They do NOT cross-compile — that is the dedicated
//! `just check-embedded` recipe's job (it needs the `.#embedded` dev
//! shell's thumbv7em-none-eabihf rust-std, which the default `just test`
//! shell does not have). The shape assertions here are the fast
//! drift-detection layer; the recipe is the genuine compile acceptance
//! (AC#4).
//!
//! This file holds the LIB-shape tests (`emit` path) and the shared
//! lowering helpers. The BIN-shape tests (M10 `emit_bin` firmware path)
//! live in the [`bin_shape`] child module, split out per TASK-0383 to
//! keep both files under the 1000-LoC mega-file fence; that child reuses
//! the [`repo_root`] helper here via `super::repo_root`.

use std::path::PathBuf;

use crate::emit;

mod bin_shape;
mod boot_order;
mod input_offsets;
mod output_capture;
mod sync_guard;
mod transport_mode_render;
mod transport_per_seq;

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

    // TASK-0426.01: per-call-unique scratch (created once, never removed).
    let out = test_common::unique_scratch_dir(
        &root.join("nucleus/target/embedded-pattern-test-scratch"),
        scratch_leaf,
    );

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
fn emit_example_multi(
    example: &str,
    schedule_file: &str,
    scratch_leaf: &str,
) -> crate::MultiEmitResult {
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

    // TASK-0426.01: per-call-unique scratch (created once, never removed).
    let out = test_common::unique_scratch_dir(
        &root.join("nucleus/target/embedded-pattern-test-scratch"),
        scratch_leaf,
    );

    emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &out).expect("embedded multi-worker emit")
}
#[test]
fn ex01_emits_no_std_lib_with_shim_and_pure_add() {
    let lib = emit_example_naive("01-elementwise-add", "ex01_naive");

    // no_std + the six-method shim trait + stub (AC#1/#2/#3). The four
    // M9 methods plus the two M10 TASK-0048.04 tier-3 methods
    // (monotonic_ns + report_violation).
    assert!(lib.contains("#![no_std]"), "must be no_std:\n{lib}");
    assert!(
        lib.contains("pub trait NucleusShim"),
        "trait missing:\n{lib}"
    );
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
        assert!(
            w.lib_rs.exists(),
            "worker `{name}` lib.rs must exist on disk"
        );
        assert!(
            w.cargo_toml.exists(),
            "worker `{name}` Cargo.toml must exist"
        );
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

    // --- AC#2: Push/Wait/Sync lower to the cross-worker transport hooks. ---
    // host loads a + b then PUSHES them to w0; the Push lowers to the
    // DEDICATED `link_push` transport hook (TASK-0049.05 trap #1: distinct
    // from the effectful `dma_push(0)` save so a real shim routes them to
    // different USARTs).
    assert!(
        host.contains("shim.link_push(0, a.as_ptr() as *const u8, core::mem::size_of_val(&a));"),
        "host must Push `a` via link_push (transport hook):\n{host}"
    );
    assert!(
        host.contains("shim.link_push(1, b.as_ptr() as *const u8, core::mem::size_of_val(&b));"),
        "host must Push `b` via link_push:\n{host}"
    );
    // host WAITs for the computed `c` back from w0 — `link_recv` FILLS the
    // receive local (mut-ptr + byte length).
    assert!(
        host.contains(
            "shim.link_recv(2, c.as_mut_ptr() as *mut u8, core::mem::size_of_val(&c));"
        ),
        "host must Wait (link_recv, filling `c`) on its transport seq:\n{host}"
    );
    // w0 WAITs for a + b, computes, then PUSHES c back.
    assert!(
        w0.contains("shim.link_recv(0, a.as_mut_ptr() as *mut u8, core::mem::size_of_val(&a));")
            && w0.contains(
                "shim.link_recv(1, b.as_mut_ptr() as *mut u8, core::mem::size_of_val(&b));"
            ),
        "w0 must Wait (link_recv, filling `a` + `b`) for both inputs:\n{w0}"
    );
    assert!(
        w0.contains("shim.link_push(2, c.as_ptr() as *const u8, core::mem::size_of_val(&c));"),
        "w0 must Push the computed `c` back via link_push:\n{w0}"
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
        assert!(
            src.contains("#![no_std]"),
            "worker `{name}` must be no_std:\n{src}"
        );
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
    //     outputs over the DEDICATED transport hooks (link_recv/link_push,
    //     distinct from the effectful dma_* — TASK-0049.05 trap #1). ---
    assert!(
        dsp.contains("shim.link_recv(") && dsp.contains("shim.link_push("),
        "dsp must lower its cross-worker Wait/Push to the link_recv/link_push hooks:\n{dsp}"
    );

    // --- producer-side transport: fe (mic_in) and rf (bt_in) must Push
    //     their captured frames to dsp — the transport is not dsp-only
    //     (TASK-0049.06 review P3.3: pin the producer side too). ---
    for name in ["fe", "rf"] {
        let src = read(name);
        assert!(
            src.contains("shim.link_push("),
            "producer worker `{name}` must Push its captured data over the \
             link_push transport hook:\n{src}"
        );
    }

    // --- every worker is a no_std lib with the shim trait + no std leak,
    //     and lowers its Event::Sync barriers to shim.irq_barrier (the
    //     cross-MCU ordering primitive; TASK-0049.06 review P3.3). ---
    for name in ["fe", "dsp", "rf"] {
        let src = read(name);
        assert!(
            src.contains("#![no_std]"),
            "worker `{name}` must be no_std:\n{src}"
        );
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
fn ex14_effectful_indexed_input_lowers_to_per_frame_region_read_not_stub() {
    // TASK-0049.10.01 (BLOCKER 1 root fix, slice A): an EFFECTFUL kernel
    // with an INDEXED output AND no inputs (`mic_in[frame] <-- fe_capture()`,
    // declared `kernel fe_capture : () -> i32[16] effectful`) is
    // STRUCTURALLY indistinguishable from a pure indexed compute, so
    // before the purity bit reached the codegen contract it lowered to
    // the verbatim-extracted `kernels::fe_capture()` STUB returning
    // `[0i32; 16]` — firmware mic_in/bt_in were all zeros and could never
    // match the real reference output. With purity mirrored into
    // `KernelSig`, the effectful zero-input indexed firing now lowers to a
    // PER-FRAME shim region-read into the indexed slice instead.
    let res = emit_example_multi_with_files(
        "14-hearing-aid",
        "prog.embedded.algo.nuc",
        "embedded_multimcu_sync.sched.nuc",
        "kernels.embedded.rs",
        "ex14_effectful_indexed_input",
    );

    let read = |name: &str| -> String {
        let w = res
            .workers
            .iter()
            .find(|w| w.worker_name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("worker `{name}` project missing"));
        std::fs::read_to_string(&w.lib_rs).expect("read worker lib.rs")
    };

    // fe runs `fe_capture` (mic_in[frame]); rf runs `rf_receive`
    // (bt_in[frame]). Both are effectful, indexed-output, zero-input ->
    // both must lower to the per-frame region read, NOT the stub call.
    for (worker, kernel, datum) in [("fe", "fe_capture", "mic_in"), ("rf", "rf_receive", "bt_in")] {
        let src = read(worker);

        // The NEW per-frame effectful-input lowering: a shim region read
        // (alloc_in_region + dma_wait + a null-guarded copy_nonoverlapping)
        // into the indexed slice of the datum, sized by the indexed row.
        assert!(
            src.contains("shim.alloc_in_region(0, core::mem::size_of_val("),
            "worker `{worker}`: effectful per-frame input `{kernel}` must read a \
             shim region (alloc_in_region):\n{src}"
        );
        assert!(
            src.contains("core::ptr::copy_nonoverlapping("),
            "worker `{worker}`: effectful per-frame input `{kernel}` must copy the \
             region into the indexed slice (copy_nonoverlapping):\n{src}"
        );
        // The read fills the INDEXED slice of the datum as a SUB-ARRAY
        // place `datum[start..start + 16usize]` — the per-frame row, NOT
        // the whole array and NOT a scalar slot. ANCHORED to the actual
        // fill line (`copy_nonoverlapping(__src, datum[...`) rather than a
        // file-wide `datum[` + ` + 16usize]` pair: the worker file ALSO
        // contains `mic_in[` from the pure-compute input reads
        // (`mix2(mic_in[frame], ..)`, `denoise(mic_in[frame])`) and other
        // ` + 16usize]` sub-array slices, so an unanchored pair would NOT
        // bite a whole-array mis-fill regression (architect review
        // TASK-0049.10.02 P2-1; the same hardening as the slice-B drain
        // test below). The ` + 16usize]` row-sizing also guards a wrong
        // element-count and a scalar `datum[idx]` mis-lowering.
        assert!(
            src.contains(&format!("core::ptr::copy_nonoverlapping(__src, {datum}["))
                && src.contains(" + 16usize]"),
            "worker `{worker}`: the per-frame read must fill the indexed datum \
             as a sub-array row (`copy_nonoverlapping(__src, {datum}[start..start \
             + 16usize]...`), not a scalar slot or the whole array:\n{src}"
        );
        // CRITICAL regression assertion: the zero-returning extracted stub
        // call must NOT be emitted for the effectful capture (the whole
        // point of BLOCKER 1).
        assert!(
            !src.contains(&format!("kernels::{kernel}(")),
            "worker `{worker}`: effectful per-frame input `{kernel}` must NOT lower \
             to the zero-returning stub `kernels::{kernel}(`:\n{src}"
        );
    }

    // ADDITIVITY pin: the PURE indexed-output kernels (mix2, denoise on
    // dsp) MUST still emit the extracted stub call — the new arm is
    // additive and only diverts the effectful zero-input shape. This is
    // the same positive assertion the TASK-0049.06 test makes; repeated
    // here so the additivity is pinned in the SAME test that asserts the
    // effectful divergence.
    let dsp = read("dsp");
    assert!(
        dsp.contains("kernels::mix2(") && dsp.contains("kernels::denoise("),
        "PURE indexed kernels (mix2/denoise) must STILL emit the extracted stub \
         call — the effectful-input arm must be purely additive:\n{dsp}"
    );
}

#[test]
fn ex14_effectful_indexed_output_drains_per_frame_row_not_whole_array() {
    // TASK-0049.10.02 (BLOCKER 1/3, slice B): the structural mirror of
    // slice A for the OUTPUT (effectful save) side. ex14 fires
    // `fe_emit(spk_out[frame])` / `rf_transmit(bt_out[frame])` — effectful
    // kernels with NO output (`() ` return => FireBinding.output == None)
    // and an INDEXED one-frame data input, once PER frame inside the
    // for-frame loop. The `None` (save) arm previously drained the BARE
    // array name `spk_out` => `size_of_val(&spk_out)` = the WHOLE array,
    // every frame (N_FRAMES x over-drain). With `effect_drain_place` the
    // indexed-input drain now targets the per-frame row place
    // `spk_out[start..start + 16usize]` instead.
    let res = emit_example_multi_with_files(
        "14-hearing-aid",
        "prog.embedded.algo.nuc",
        "embedded_multimcu_sync.sched.nuc",
        "kernels.embedded.rs",
        "ex14_effectful_indexed_output",
    );

    let read = |name: &str| -> String {
        let w = res
            .workers
            .iter()
            .find(|w| w.worker_name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("worker `{name}` project missing"));
        std::fs::read_to_string(&w.lib_rs).expect("read worker lib.rs")
    };

    // fe runs `fe_emit(spk_out[frame])`; rf runs `rf_transmit(bt_out[frame])`.
    // Both are effectful, output=None, single INDEXED data input -> both
    // must drain the indexed FRAME row, NOT the whole array.
    for (worker, kernel, datum) in [
        ("fe", "fe_emit", "spk_out"),
        ("rf", "rf_transmit", "bt_out"),
    ] {
        let src = read(worker);

        // The effectful-save drain shape is UNCHANGED (dma_push(0, ...
        // .as_ptr() + dma_wait) — only the drained PLACE changes.
        assert!(
            src.contains("shim.dma_push(0, ") && src.contains(".as_ptr() as *const u8"),
            "worker `{worker}`: effectful per-frame output `{kernel}` must still \
             drain via shim.dma_push(0, <place>.as_ptr()):\n{src}"
        );
        assert!(
            src.contains("shim.dma_wait(0);"),
            "worker `{worker}`: effectful per-frame output `{kernel}` must wait the \
             drain DMA (dma_wait):\n{src}"
        );
        // The drained place is the INDEXED datum as a SUB-ARRAY row
        // `datum[start..start + 16usize]` — the per-frame frame slice, NOT
        // the whole array. ANCHORED to the actual drain line
        // (`dma_push(0, datum[...`) rather than a file-wide `datum[` +
        // ` + 16usize]` pair: the fe worker file ALSO contains `spk_out[`
        // from the indexed write `spk_out[frame] <-- denoise(..)` and
        // ` + 16usize]` from the slice-A `mixed[frame]` input read, so an
        // unanchored pair would STILL pass if the drain reverted to the
        // whole-array `dma_push(0, spk_out.as_ptr()..)` shape (architect
        // review TASK-0049.10.02 P2-1). The ` + 16usize]` row-sizing also
        // guards a wrong element-count regression.
        assert!(
            src.contains(&format!("shim.dma_push(0, {datum}[")) && src.contains(" + 16usize]"),
            "worker `{worker}`: the per-frame drain must target the indexed datum \
             as a sub-array row (`dma_push(0, {datum}[start..start + 16usize]...`), \
             not the whole array `{datum}`:\n{src}"
        );
        // The CRITICAL anti-regression: the whole-array drain shape
        // `dma_push(0, {datum}.as_ptr()` must NOT appear — if the indexed
        // branch were lost, the bare-array drain would re-emit exactly this
        // (a regression the anchored positive assertion alone could miss if
        // both shapes coexisted).
        assert!(
            !src.contains(&format!("shim.dma_push(0, {datum}.as_ptr()")),
            "worker `{worker}`: effectful per-frame output `{kernel}` must NOT drain \
             the WHOLE array (`dma_push(0, {datum}.as_ptr()`) — only the per-frame \
             row:\n{src}"
        );
    }

    // ADDITIVITY pin: a WHOLE-ARRAY save (empty-indices input, the tier-1
    // `save_output(c)` shape) must STILL drain the BARE array name with NO
    // sub-array ` + 16usize]` suffix on the drain. Example 1's naive
    // single-worker schedule fires `save_output(c)` on the whole array
    // `c` (`kernel save_output : (i32[N]) -> () effectful`). This is the
    // regression guard for the 7 M6 + M10 ex1/5/9 + 02-split-add cells:
    // the empty-indices drain path is byte-unchanged.
    let ex1 = emit_example_naive("01-elementwise-add", "ex01_naive_drain_pin");
    assert!(
        ex1.contains("shim.dma_push(0, c.as_ptr() as *const u8"),
        "WHOLE-ARRAY save (save_output(c)) must drain the BARE array name `c` \
         (empty-indices path byte-unchanged):\n{ex1}"
    );
    // The whole-array drain line must size by `size_of_val(&c)` (the whole
    // array) — and crucially must NOT carry the ` + 16usize]` sub-array
    // row-sizing suffix that the indexed per-frame drain uses. Example 1
    // has no SAMPLES_PER_FRAME (16-wide) sub-array anywhere, so its
    // absence pins that the additive indexed arm did not leak into the
    // whole-array path.
    assert!(
        !ex1.contains(" + 16usize]"),
        "WHOLE-ARRAY save must NOT emit a `[start..start + 16usize]` sub-array \
         drain — the indexed per-frame arm must be purely additive:\n{ex1}"
    );
}
