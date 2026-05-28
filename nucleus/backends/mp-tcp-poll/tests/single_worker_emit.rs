//! Single-worker emit pins for mp-tcp-poll (TASK-0044.02 cycle 192).
//!
//! Two invariants pinned here:
//!
//! - Empty event-list (smallest legal input): both mp-tcp-poll and
//!   mp-tcp-bufsync must emit byte-identical Cargo.toml + binary +
//!   run.sh + kernels.rs + wire.rs, since mp-tcp-poll's single-process
//!   arm delegates to the SAME shared renderers mp-tcp-bufsync uses
//!   (`pthreads_sync::render_single_worker_main_with_kernels_attr` +
//!   `backend_common::project_skeleton::multi_binary`).
//!
//! - Real non-trivial witness (01-elementwise-add / naive): the same
//!   byte-identical assertion on a real fixture proves the invariant
//!   survives a kernel-call site + sidecar consumption, not just an
//!   empty scaffold. Mirrors openmp-rs/tests/single_worker_emit.rs's
//!   structural shape (cycle 191) and pthreads-async/tests/skeleton.rs's
//!   `single_worker_real_example_emits_byte_identical_to_pthreads_sync`
//!   (cycle 17). Drift detection: any mp-tcp-poll-specific wrapper
//!   around the delegated emitter would surface here as a diff.

use std::collections::BTreeMap;
use std::path::PathBuf;

use mp_tcp_poll::{emit, NameTables};
use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

#[test]
fn single_worker_empty_eventlist_emits_byte_identical_to_mp_tcp_bufsync() {
    let per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    let names = NameTables::default();
    let sidecar = NameSidecar::default();

    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("target"))
        .expect("workspace target/");
    let stem = target.join("mp-tcp-poll-test-scratch/single_worker_empty");
    let poll_out = stem.join("poll");
    let bufsync_out = stem.join("bufsync");
    let _ = std::fs::remove_dir_all(&poll_out);
    let _ = std::fs::remove_dir_all(&bufsync_out);

    let kernels = stem.join("empty_kernels.rs");
    std::fs::create_dir_all(&stem).expect("scratch dir");
    std::fs::write(
        &kernels,
        "// Empty kernels.rs for the empty-eventlist test.\n",
    )
    .expect("empty kernels.rs");

    let poll_res =
        emit(&per_worker, &names, &sidecar, &kernels, &poll_out).expect("mp-tcp-poll emit");
    let bufsync_res = mp_tcp_bufsync::emit(&per_worker, &names, &sidecar, &kernels, &bufsync_out)
        .expect("mp-tcp-bufsync emit");

    // ---- The invariant: byte-identical binary + Cargo.toml + run.sh +
    //                     wire.rs + kernels.rs ----
    assert_eq!(
        poll_res.worker_bins.len(),
        1,
        "single-worker mp-tcp-poll must emit exactly one binary, got {}",
        poll_res.worker_bins.len()
    );
    assert_eq!(
        bufsync_res.worker_bins.len(),
        1,
        "single-worker mp-tcp-bufsync must emit exactly one binary, got {}",
        bufsync_res.worker_bins.len()
    );

    let poll_bin = std::fs::read_to_string(&poll_res.worker_bins[0]).expect("poll binary");
    let bufsync_bin = std::fs::read_to_string(&bufsync_res.worker_bins[0]).expect("bufsync binary");
    assert_eq!(
        poll_bin, bufsync_bin,
        "mp-tcp-poll single-process binary must be byte-identical to \
         mp-tcp-bufsync's (the cross-backend differential invariant):\n\
         poll:\n{poll_bin}\n--- bufsync:\n{bufsync_bin}"
    );

    let poll_cargo = std::fs::read_to_string(&poll_res.cargo_toml).expect("poll Cargo.toml");
    let bufsync_cargo =
        std::fs::read_to_string(&bufsync_res.cargo_toml).expect("bufsync Cargo.toml");
    assert_eq!(
        poll_cargo, bufsync_cargo,
        "mp-tcp-poll Cargo.toml must be byte-identical to mp-tcp-bufsync's \
         (shared backend_common::project_skeleton::multi_binary::render_cargo_toml)"
    );

    let poll_run = std::fs::read_to_string(&poll_res.run_sh).expect("poll run.sh");
    let bufsync_run = std::fs::read_to_string(&bufsync_res.run_sh).expect("bufsync run.sh");
    assert_eq!(
        poll_run, bufsync_run,
        "mp-tcp-poll single-process run.sh must be byte-identical to \
         mp-tcp-bufsync's (shared render_run_sh_single)"
    );

    let poll_wire = std::fs::read_to_string(&poll_res.wire_rs).expect("poll wire.rs");
    let bufsync_wire = std::fs::read_to_string(&bufsync_res.wire_rs).expect("bufsync wire.rs");
    assert_eq!(
        poll_wire, bufsync_wire,
        "mp-tcp-poll wire.rs must be byte-identical to mp-tcp-bufsync's \
         (both emit mp_tcp_common::WIRE_RUNTIME_SRC verbatim)"
    );

    let poll_kernels = std::fs::read_to_string(&poll_res.kernels_rs).expect("poll kernels.rs");
    let bufsync_kernels =
        std::fs::read_to_string(&bufsync_res.kernels_rs).expect("bufsync kernels.rs");
    assert_eq!(
        poll_kernels, bufsync_kernels,
        "mp-tcp-poll kernels.rs must be a verbatim copy (same input file)"
    );
}

/// Non-trivial witness — proves the actual codegen (kernel call, loop
/// header, sidecar-driven pre-init) is byte-identical to mp-tcp-bufsync,
/// not just the empty scaffold. Mirrors cycle 191
/// (openmp-rs/tests/single_worker_emit.rs) and cycle 17
/// (pthreads-async/tests/skeleton.rs).
#[test]
fn single_worker_real_example_emits_byte_identical_to_mp_tcp_bufsync() {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/01-elementwise-add");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).expect("01 algo");
    let sched_src =
        std::fs::read_to_string(ex.join("schedules/naive.sched.nuc")).expect("01 sched");

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

    let scratch = root.join("nucleus/target/mp-tcp-poll-test-scratch/single_worker_01_naive");
    let poll_out = scratch.join("poll");
    let bufsync_out = scratch.join("bufsync");
    let _ = std::fs::remove_dir_all(&poll_out);
    let _ = std::fs::remove_dir_all(&bufsync_out);

    let poll_res = emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &poll_out)
        .expect("mp-tcp-poll emit (single-worker real example)");
    let bufsync_res =
        mp_tcp_bufsync::emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &bufsync_out)
            .expect("mp-tcp-bufsync emit (same input)");

    let poll_bin = std::fs::read_to_string(&poll_res.worker_bins[0]).expect("poll binary");
    let bufsync_bin = std::fs::read_to_string(&bufsync_res.worker_bins[0]).expect("bufsync binary");
    assert_eq!(
        poll_bin, bufsync_bin,
        "mp-tcp-poll single-process binary on 01-elementwise-add/naive MUST \
         be byte-identical to mp-tcp-bufsync's. A diff means the delegation \
         to render_single_worker_main_with_kernels_attr was bypassed or \
         wrapped:\n=== poll ===\n{poll_bin}\n=== bufsync ===\n{bufsync_bin}"
    );

    // Non-trivial witness check: the emitted binary MUST contain the
    // kernel call (so we are not vacuously identical).
    assert!(
        poll_bin.contains("kernels::add"),
        "01-elementwise-add witness must emit kernels::add — absence of it \
         means the test passed vacuously:\n{poll_bin}"
    );
}

fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .expect("three ancestors above mp-tcp-poll crate")
        .to_path_buf()
}
