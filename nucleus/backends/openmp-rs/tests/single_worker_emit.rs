//! Single-worker emit pins for openmp-rs (TASK-0044.01 cycle 191).
//!
//! Two invariants pinned here:
//!
//! - Empty event-list (smallest legal input): both openmp-rs and
//!   pthreads-sync must emit byte-identical Cargo.toml + main.rs +
//!   run.sh + kernels.rs, since openmp-rs's single-worker arm
//!   delegates to pthreads-sync's `render_single_worker_main` plus
//!   backend-common's project skeleton.
//!
//! - Real non-trivial witness (01-elementwise-add / naive): the same
//!   byte-identical assertion on a real fixture proves the invariant
//!   survives a kernel-call site + sidecar consumption, not just an
//!   empty scaffold. Mirrors pthreads-async/tests/skeleton.rs's
//!   `single_worker_real_example_emits_byte_identical_to_pthreads_sync`
//!   (same drift-detection rationale: any openmp-rs-specific wrapper
//!   around the delegated emitter would surface here as a diff).

use std::collections::BTreeMap;
use std::path::PathBuf;

use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;
use openmp_rs::{emit, NameTables};

#[test]
fn single_worker_empty_eventlist_emits_byte_identical_to_pthreads_sync() {
    let per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    let names = NameTables::default();
    let sidecar = NameSidecar::default();

    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("target"))
        .expect("workspace target/");
    let stem = target.join("openmp-rs-test-scratch/single_worker_empty");
    let openmp_out = stem.join("openmp");
    let sync_out = stem.join("sync");
    let _ = std::fs::remove_dir_all(&openmp_out);
    let _ = std::fs::remove_dir_all(&sync_out);

    let kernels = stem.join("empty_kernels.rs");
    std::fs::create_dir_all(&stem).expect("scratch dir");
    std::fs::write(
        &kernels,
        "// Empty kernels.rs for the empty-eventlist test.\n",
    )
    .expect("empty kernels.rs");

    let openmp_res =
        emit(&per_worker, &names, &sidecar, &kernels, &openmp_out).expect("openmp-rs emit");
    let sync_res = pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels, &sync_out)
        .expect("pthreads-sync emit");

    let openmp_main = std::fs::read_to_string(&openmp_res.main_rs).expect("openmp main.rs");
    let sync_main = std::fs::read_to_string(&sync_res.main_rs).expect("sync main.rs");
    assert_eq!(
        openmp_main, sync_main,
        "openmp-rs single-worker main.rs must be byte-identical to \
         pthreads-sync's (the cross-backend differential invariant):\n\
         openmp:\n{openmp_main}\n--- sync:\n{sync_main}"
    );

    let openmp_cargo = std::fs::read_to_string(&openmp_res.cargo_toml).expect("openmp Cargo.toml");
    let sync_cargo = std::fs::read_to_string(&sync_res.cargo_toml).expect("sync Cargo.toml");
    assert_eq!(
        openmp_cargo, sync_cargo,
        "openmp-rs single-worker Cargo.toml must be byte-identical to \
         pthreads-sync's (shared backend_common::project_skeleton::\
         single_binary::render_cargo_toml)"
    );

    let openmp_run = std::fs::read_to_string(&openmp_res.run_sh).expect("openmp run.sh");
    let sync_run = std::fs::read_to_string(&sync_res.run_sh).expect("sync run.sh");
    assert_eq!(
        openmp_run, sync_run,
        "openmp-rs single-worker run.sh must be byte-identical to \
         pthreads-sync's (shared render_run_sh)"
    );

    let openmp_kernels =
        std::fs::read_to_string(&openmp_res.kernels_rs).expect("openmp kernels.rs");
    let sync_kernels = std::fs::read_to_string(&sync_res.kernels_rs).expect("sync kernels.rs");
    assert_eq!(
        openmp_kernels, sync_kernels,
        "openmp-rs single-worker kernels.rs must be a verbatim copy \
         (same input file, same copy)"
    );
}

/// Non-empty witness — proves the actual codegen (kernel calls, loop
/// headers, sidecar-driven pre-init) is byte-identical to
/// pthreads-sync, not just the empty scaffold. Mirrors
/// pthreads-async/tests/skeleton.rs::single_worker_real_example_
/// emits_byte_identical_to_pthreads_sync. If a future drift adds an
/// openmp-rs-specific wrapper around the delegated emitter, this test
/// fails: the wrapper would appear in openmp's output but not sync's.
#[test]
fn single_worker_real_example_emits_byte_identical_to_pthreads_sync() {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/01-elementwise-add");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).expect("01 algo");
    let sched_src =
        std::fs::read_to_string(ex.join("schedules/naive.sched.nuc")).expect("01 sched");

    // 01-elementwise-add/naive is single-worker. Mirror
    // pthreads-async's helper exactly (apply_block_transforms=false,
    // apply_partition_workers=false, inject_check_frames=false) to
    // pin the historical contract.
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

    let scratch = root.join("nucleus/target/openmp-rs-test-scratch/single_worker_01_naive");
    let openmp_out = scratch.join("openmp");
    let sync_out = scratch.join("sync");
    let _ = std::fs::remove_dir_all(&openmp_out);
    let _ = std::fs::remove_dir_all(&sync_out);

    let openmp_res = emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &openmp_out)
        .expect("openmp-rs emit (single-worker real example)");
    let sync_res = pthreads_sync::emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &sync_out)
        .expect("pthreads-sync emit (same input)");

    let openmp_main = std::fs::read_to_string(&openmp_res.main_rs).expect("openmp main.rs");
    let sync_main = std::fs::read_to_string(&sync_res.main_rs).expect("sync main.rs");
    assert_eq!(
        openmp_main, sync_main,
        "openmp-rs single-worker main.rs on 01-elementwise-add/naive MUST \
         be byte-identical to pthreads-sync's. A diff means the delegation \
         to pthreads_sync::render_single_worker_main was bypassed or \
         wrapped:\n=== openmp ===\n{openmp_main}\n=== sync ===\n{sync_main}"
    );

    // Non-trivial witness check: the emitted main.rs MUST contain the
    // kernel call (so we are not vacuously identical).
    assert!(
        openmp_main.contains("kernels::add"),
        "01-elementwise-add witness must emit kernels::add — absence of it \
         means the test passed vacuously:\n{openmp_main}"
    );
}

fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .expect("three ancestors above openmp-rs crate")
        .to_path_buf()
}
