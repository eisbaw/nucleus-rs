//! Unit tests for the pthreads-sync backend `emit(...)` function.
//!
//! These exercise the renderer in isolation: parse + lower + link
//! example 01, build the ACFG, run sync + transfer injection, then
//! call `emit(...)` into a tempdir. We assert:
//!
//! - Every expected file appears.
//! - Cargo.toml is a parseable standalone manifest.
//! - main.rs mentions the expected kernel names (a smoke test that
//!   the dataflow walk actually emitted calls).
//!
//! The actual bit-identical end-to-end test (compile + run + diff
//! reference.bin) lives at `nucleus/compiler/tests/e2e_example_01.rs`
//! to keep this crate's test load light.
//!
//! Multi-worker codegen (TASK-0122) is now in place. A synthetic
//! two-worker pingpong test lives in `tests/multi_worker.rs`. The
//! load-bearing end-to-end test (example 02 split.sched.nuc against
//! `reference.bin`) lives at
//! `nucleus/compiler/tests/e2e_example_02.rs`.
//!
//! At this layer (the unit-test surface of the backend) we keep:
//! - The single-worker happy-path tests below (example 01 × naive).
//! - A small synthetic test that proves a two-worker ACFG produces
//!   compilable Rust without a runtime check (the runtime is in the
//!   multi_worker.rs harness).
//! - A test that rejects an unsupported distributed placement
//!   (`place k on {w0,w1,...}`).

use std::fs;
use std::path::{Path, PathBuf};

use std::collections::BTreeMap;

use compiler::{
    acfg_to_events, build_acfg, build_sidecar,
    algo::{lower_algo, parse_algo},
    inject_syncs, inject_transfers, link,
    sched::{lower_sched, parse_sched},
};
use pthreads_sync::{emit, NameTables};

/// Build the EventList + sidecar + reverse name tables from a
/// post-pass ACFG + LinkedIR — exactly what the driver does. Tests
/// that previously called `emit(&acfg, &linked, ...)` now go through
/// this so they exercise the real TASK-0124 contract path.
fn contract_inputs(
    linked: &compiler::link::LinkedIR,
    acfg: &compiler::ACFG,
) -> (
    BTreeMap<compiler::WorkerId, Vec<compiler::event::Event>>,
    NameTables,
    compiler::NameSidecar,
) {
    let per_worker = acfg_to_events(acfg);
    let sidecar = build_sidecar(linked, acfg).expect("build_sidecar");
    let names = NameTables {
        data: acfg.name_data.iter().map(|(n, i)| (*i, n.clone())).collect(),
        kernel: acfg
            .name_kernels
            .iter()
            .map(|(n, i)| (*i, n.clone()))
            .collect(),
        worker: acfg
            .name_workers
            .iter()
            .map(|(n, i)| (*i, n.clone()))
            .collect(),
        iter_var: acfg
            .name_iter_vars
            .iter()
            .map(|(n, i)| (*i, n.clone()))
            .collect(),
        inner_block_iter_vars: acfg.inner_block_iter_vars.clone(),
    };
    (per_worker, names, sidecar)
}

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR -> nucleus/backends/pthreads-sync
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("three ancestors above pthreads-sync crate")
        .to_path_buf()
}

fn scratch_dir(name: &str) -> PathBuf {
    let target = repo_root().join("nucleus/target/pthreads-sync-test-scratch");
    let _ = fs::create_dir_all(&target);
    let dir = target.join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Load + link example 01 with the naive schedule. Shared helper.
fn link_example_01_naive() -> (compiler::link::LinkedIR, compiler::ACFG, PathBuf) {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/01-elementwise-add");
    let algo_src = fs::read_to_string(ex.join("prog.algo.nuc")).unwrap();
    let sched_src = fs::read_to_string(ex.join("schedules/naive.sched.nuc")).unwrap();

    let algo_ast = parse_algo(&algo_src).unwrap();
    let sched_ast = parse_sched(&sched_src).unwrap();
    let algo_ir = lower_algo(&algo_ast).unwrap();
    let sched_ir = lower_sched(&sched_ast).unwrap();
    let linked = link(algo_ir, sched_ir).unwrap();
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg);
    (linked, acfg, ex.join("kernels.rs"))
}

/// Load + link an arbitrary example/schedule through the FULL driver
/// pipeline (incl. `apply_block_transforms`) so a blocked schedule
/// gets the tile rewrite. Returns `(linked, post-pass acfg, kernels
/// path)`.
fn link_example(ex_rel: &str, sched_rel: &str) -> (compiler::link::LinkedIR, compiler::ACFG, PathBuf) {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples").join(ex_rel);
    let algo_src = fs::read_to_string(ex.join("prog.algo.nuc")).unwrap();
    let sched_src = fs::read_to_string(ex.join(sched_rel)).unwrap();
    let algo_ir = lower_algo(&parse_algo(&algo_src).unwrap()).unwrap();
    let sched_ir = lower_sched(&parse_sched(&sched_src).unwrap()).unwrap();
    let linked = link(algo_ir, sched_ir).unwrap();
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = compiler::apply_block_transforms(&linked, acfg).unwrap();
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg);
    (linked, acfg, ex.join("kernels.rs"))
}

/// Finding (7) reconciliation (TASK-0124): the sidecar sufficiency
/// tests in `compiler/tests/petri_to_events.rs` HAND-MIRROR the
/// backend's bound/type/zero spelling (they cannot call the real
/// backend — `compiler` must not depend on `pthreads-sync`). Now
/// that the backend actually consumes the sidecar, pin the EXACT
/// load-bearing strings via the REAL `emit()` codegen here, at the
/// backend layer, so any spelling drift fails loudly against real
/// output rather than only against a mirror that drifts with it.
#[test]
fn golden_real_codegen_strings_pin_sidecar_consumption() {
    // 05-stencil naive: the symbolic loop bound rendered from
    // sidecar.loop_bounds + sidecar.consts, and the sidecar-sized
    // pre-init. Exactly the strings petri_to_events.rs's
    // `sidecar_renders_stencil_symbolic_loop_bound_in_source_form`
    // / `sidecar_alone_sizes_preinit_*` assert via mirrors.
    let (linked, acfg, kernels) = link_example("05-stencil", "schedules/naive.sched.nuc");
    let out = scratch_dir("golden_05_naive");
    let (pw, names, sc) = contract_inputs(&linked, &acfg);
    let main_rs = fs::read_to_string(emit(&pw, &names, &sc, &kernels, &out).unwrap().main_rs).unwrap();
    assert!(
        main_rs.contains("for y in (1_i64)..((16_i64 - 1_i64)) {"),
        "05-stencil symbolic bound from sidecar drifted:\n{main_rs}"
    );
    assert!(
        main_rs.contains("for x in (1_i64)..((16_i64 - 1_i64)) {"),
        "05-stencil inner symbolic bound drifted:\n{main_rs}"
    );
    assert!(
        main_rs.contains("let mut img_out = vec![0; 256];"),
        "05-stencil sidecar-sized pre-init drifted:\n{main_rs}"
    );

    // 07-matmul blocked: the absolute-index rebinding the backend
    // MUST do because block_transform defers it (TASK-0124). Pin the
    // exact rebinding spelling so it cannot silently regress.
    let (linked, acfg, kernels) = link_example("07-matmul", "schedules/blocked.sched.nuc");
    let out = scratch_dir("golden_07_blocked");
    let (pw, names, sc) = contract_inputs(&linked, &acfg);
    let main_rs = fs::read_to_string(emit(&pw, &names, &sc, &kernels, &out).unwrap().main_rs).unwrap();
    assert!(
        main_rs.contains("for i__tile in (0_i64)..(2_i64) {")
            && main_rs.contains("for i in (0_i64)..(8_i64) {"),
        "07 blocked tile/inner loop headers drifted:\n{main_rs}"
    );
    assert!(
        main_rs.contains("(0_i64 + (i__tile * 8_i64) + i)")
            && main_rs.contains("(0_i64 + (j__tile * 8_i64) + j)"),
        "07 blocked absolute-index rebinding drifted (accumulator would \
         double-count without it):\n{main_rs}"
    );
}

#[test]
fn emit_writes_all_files() {
    let (linked, acfg, kernels) = link_example_01_naive();
    let out = scratch_dir("emit_writes_all_files");
    let (pw, names, sc) = contract_inputs(&linked, &acfg);
    let result = emit(&pw, &names, &sc, &kernels, &out).expect("emit succeeded");

    // All four artefacts present.
    assert!(result.cargo_toml.exists(), "Cargo.toml not written");
    assert!(result.main_rs.exists(), "main.rs not written");
    assert!(result.kernels_rs.exists(), "kernels.rs not copied");
    assert!(result.run_sh.exists(), "run.sh not written");

    // run.sh on unix is executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&result.run_sh).unwrap().permissions().mode();
        // 0o111 is "any execute bit set". Don't pin the exact 0o755
        // because umask could legitimately strip the group/other.
        assert!(
            mode & 0o111 != 0,
            "run.sh is not executable (mode={mode:o})"
        );
    }
}

#[test]
fn main_rs_calls_every_kernel() {
    let (linked, acfg, kernels) = link_example_01_naive();
    let out = scratch_dir("main_rs_calls_every_kernel");
    let (pw, names, sc) = contract_inputs(&linked, &acfg);
    let result = emit(&pw, &names, &sc, &kernels, &out).unwrap();
    let main_rs = fs::read_to_string(&result.main_rs).unwrap();

    // Smoke test: every kernel from example 01 appears as a call.
    for kernel in &["load_input", "load_input_b", "add", "save_output"] {
        let needle = format!("kernels::{kernel}");
        assert!(
            main_rs.contains(&needle),
            "main.rs is missing `{needle}`:\n---\n{main_rs}\n---"
        );
    }

    // The for-loop bound for example 01 is N=256.
    assert!(
        main_rs.contains("256_i64"),
        "main.rs is missing the N=256 loop bound:\n---\n{main_rs}\n---"
    );
}

#[test]
fn kernels_rs_is_copied_verbatim() {
    let (linked, acfg, kernels) = link_example_01_naive();
    let out = scratch_dir("kernels_rs_is_copied_verbatim");
    let (pw, names, sc) = contract_inputs(&linked, &acfg);
    let result = emit(&pw, &names, &sc, &kernels, &out).unwrap();
    let src = fs::read_to_string(&kernels).unwrap();
    let dst = fs::read_to_string(&result.kernels_rs).unwrap();
    assert_eq!(src, dst, "kernels.rs was not copied byte-for-byte");
}

// REMOVED (TASK-0124): `distributed_placement_is_rejected`.
//
// That test hand-built a `LinkedIR` with a `place dist on {w0,w1}`
// distributed placement and asserted that the *backend*
// (`multi_worker::validate_placements`) rejected it. TASK-0124 moved
// the backend off `&ACFG`/`&LinkedIR` entirely (AC#2): the backend
// now consumes only the per-worker EventList + NameSidecar + name
// tables and has no `placements`/`kernel_workers` to validate — that
// is no longer a backend responsibility. Distributed-placement
// rejection now lives upstream of the projection (the capability
// check + the deliberate e2e SKIP of every `distributed` cell,
// tracked by TASK-0117). Re-introducing this assertion at the
// backend layer would require putting `LinkedIR` back into the
// `emit()` signature, directly contradicting AC#2. Keeping a
// synthetic ACFG/LinkedIR test here would therefore test a
// responsibility the backend no longer has. The two-worker codegen
// path itself stays covered end-to-end by `tests/multi_worker.rs`
// (synthetic pingpong) and `compiler/tests/e2e_example_02.rs`
// (example 02 `split.sched.nuc`).
