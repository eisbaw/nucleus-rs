//! Smoke tests for the mp-tcp-poll backend.
//!
//! Cycle status (TASK-0044.02.02 cycle 195):
//! - Single-worker arm IMPLEMENTED (cycle 192): delegation to
//!   `pthreads_sync::render_single_worker_main_with_kernels_attr` +
//!   `backend_common::project_skeleton::multi_binary`; byte-identical
//!   to mp-tcp-bufsync's single-process emit.
//! - Multi-worker arm IMPLEMENTED (cycle 195): Plan-shaped codegen
//!   consuming the nonblocking-poll wire primitives.
//!
//! Tests pinned here:
//! - Multi-worker `emit()` on a minimal 2-worker fixture returns Ok +
//!   produces the expected per-worker binaries (cycle-195 promotion;
//!   replaces the cycle-192 ContractGap pin).
//! - `EmitResult` shape pin (compile-time via constructor).
//!
//! Cross-backend bit-identicality vs reference.bin and emit-string
//! parity vs mp-tcp-bufsync live in `tests/multi_worker_emit.rs`,
//! `tests/pingpong.rs`, and the e2e matrix.

use std::path::PathBuf;

use mp_tcp_poll::{emit, EmitResult};

#[test]
fn multi_worker_emit_smoke_produces_per_worker_binaries() {
    // Drive a real (parser → lower → link → ACFG → inject → project)
    // pipeline on the smallest in-tree multi-worker example
    // (02-split-add / split, 2 used workers — host + w0). The cycle-192
    // ContractGap pin is gone; we now assert that emit() returns Ok and
    // produces 2 per-worker binaries with the expected names, AND that
    // those binaries contain the cycle-195 poll-variant call sites.
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/02-split-add");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).expect("02 algo");
    let sched_src =
        std::fs::read_to_string(ex.join("schedules/split.sched.nuc")).expect("02 split sched");
    let r = test_common::lower_for_test(
        &algo_src,
        &sched_src,
        &test_common::LowerForTestOpts::default(),
    );

    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("target"))
        .expect("workspace target/");
    let stem = target.join("mp-tcp-poll-test-scratch/skeleton_multi_worker_02_split");
    let _ = std::fs::remove_dir_all(&stem);
    std::fs::create_dir_all(&stem).expect("scratch dir");
    let kernels_path = ex.join("kernels.rs");
    let out_dir = stem.join("out");

    let result = emit(&r.per_worker, &r.names, &r.sidecar, &kernels_path, &out_dir)
        .expect("mp-tcp-poll multi-worker emit must succeed (cycle 195, TASK-0044.02.02)");

    // 2 used workers in the split schedule => 2 per-worker binaries.
    assert_eq!(
        result.worker_bins.len(),
        2,
        "02-split-add/split must emit 2 per-worker binaries; got {}",
        result.worker_bins.len()
    );

    // Poll-variant call sites must appear in the generated code (else
    // the cycle-195 swap was lost). Cross-grep both bins so a future
    // single-sided regression (e.g. one worker missing the swap)
    // surfaces here.
    let mut saw_read_poll = false;
    let mut saw_write_poll = false;
    let mut saw_nonblocking = false;
    for bin_path in &result.worker_bins {
        let src = std::fs::read_to_string(bin_path).expect("read bin");
        if src.contains("wire::read_msg_expect_poll") {
            saw_read_poll = true;
        }
        if src.contains("wire::write_msg_poll") {
            saw_write_poll = true;
        }
        if src.contains("wire::apply_nonblocking") {
            saw_nonblocking = true;
        }
        // Anti-needles: the blocking primitives must NOT appear in
        // mp-tcp-poll's emit (they would mean the cycle-195 swap was
        // partial — silent-sibling defect class).
        assert!(
            !src.contains("wire::read_msg_expect(&mut "),
            "mp-tcp-poll bin {bin_path:?} contains blocking read_msg_expect — \
             cycle-195 swap regressed:\n{src}"
        );
        assert!(
            !src.contains("wire::barrier_cross(&mut "),
            "mp-tcp-poll bin {bin_path:?} contains blocking barrier_cross — \
             cycle-195 swap regressed:\n{src}"
        );
        // cycle-195 (review-gate P2.2): the swap covers 3 wire
        // primitives; without a per-bin anti-needle on write_msg, a
        // regression that swapped write only in some bins would slip
        // past the OR-style saw_write_poll positive-needle below.
        assert!(
            !src.contains("wire::write_msg(&mut "),
            "mp-tcp-poll bin {bin_path:?} contains blocking write_msg — \
             cycle-195 swap regressed:\n{src}"
        );
    }
    assert!(
        saw_read_poll,
        "no per-worker bin contains wire::read_msg_expect_poll — codegen swap missing"
    );
    assert!(
        saw_write_poll,
        "no per-worker bin contains wire::write_msg_poll — codegen swap missing"
    );
    assert!(
        saw_nonblocking,
        "no per-worker bin contains wire::apply_nonblocking — setup line missing"
    );
}

#[test]
fn emit_result_shape_is_multi_binary_six_field() {
    // The CONSTRUCTOR is the pin. If a field is renamed/added/removed,
    // this fails to compile and the driver dispatch arm
    // (driver/src/main.rs match-arm "mp-tcp-poll" => { ... println!(...) ... })
    // must be updated in lockstep — six println! lines: project_dir,
    // cargo_toml, worker_bin0..N, kernels_rs, wire_rs, run_sh.
    let r = EmitResult {
        project_dir: PathBuf::from("/p"),
        cargo_toml: PathBuf::from("/p/Cargo.toml"),
        worker_bins: vec![PathBuf::from("/p/src/bin/host.rs")],
        kernels_rs: PathBuf::from("/p/src/kernels.rs"),
        wire_rs: PathBuf::from("/p/src/wire.rs"),
        run_sh: PathBuf::from("/p/run.sh"),
    };
    let _ = (
        &r.project_dir,
        &r.cargo_toml,
        &r.worker_bins,
        &r.kernels_rs,
        &r.wire_rs,
        &r.run_sh,
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
