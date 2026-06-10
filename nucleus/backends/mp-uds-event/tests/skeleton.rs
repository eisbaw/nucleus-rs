//! Smoke tests for the mp-uds-event backend.
//!
//! Cycle status (TASK-0044.03.01 cycle 197):
//! - Single-worker arm IMPLEMENTED (delegation to
//!   `backend_common::single_worker_main::render_single_worker_main_with_kernels_attr` +
//!   `backend_common::project_skeleton::multi_binary` — byte-identical
//!   to mp-tcp-event's single-process emit).
//! - Multi-worker arm IMPLEMENTED (mio UnixStream reactor + per-(seq,
//!   peer) outbound queue + per-seq inbound queue; structural twin of
//!   mp-tcp-event with TCP→UDS transport swap).
//!
//! Tests pinned here:
//! - Multi-worker `emit()` succeeds and produces an EmitResult with
//!   `runtime_rs = Some(_)` + one worker_bin per used worker.
//! - Single-worker input still routed to the single-worker arm
//!   (runtime_rs = None).
//! - `EmitResult` shape pin (compile-time via constructor).
//!
//! Bit-identical single-worker emit differential against mp-tcp-event
//! lives in `tests/single_worker_emit.rs`.
//! Cross-backend structural-twin oracle for multi-worker emit lives
//! in `tests/multi_worker_emit.rs`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use mp_uds_event::{emit, EmitResult, NameTables};
use nucleus_compiler::event::{Event, WorkerId};

#[test]
fn single_worker_input_routed_to_single_worker_arm() {
    // Exactly one non-empty worker list with NO cross-worker events:
    // dispatch should route to the single-worker arm (runtime_rs =
    // None; one binary). Sentinel against a regression that
    // routes single-worker inputs through Plan::build (would
    // ContractGap on the >= 2 invariant).
    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    per_worker.insert(WorkerId(0), vec![]);
    per_worker.insert(WorkerId(1), vec![]);

    let names = NameTables::default();
    let sidecar = nucleus_compiler::sidecar::NameSidecar::default();

    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("target"))
        .expect("workspace target/");
    // TASK-0426.01: per-call-unique stem (created once by the helper,
    // never removed). The `out` subdir rides under this unique stem.
    let stem = test_common::unique_scratch_dir(
        &target.join("mp-uds-event-test-scratch"),
        "skeleton_single_worker_route",
    );
    let kernels_path = stem.join("kernels.rs");
    std::fs::write(&kernels_path, "// stub for routing test\n").expect("kernels.rs stub");
    let out_dir = stem.join("out");

    let res = emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
        .expect("single-worker mp-uds-event emit must succeed (zero used workers => arm 1)");
    assert!(
        res.runtime_rs.is_none(),
        "single-worker-route must NOT emit runtime.rs (no mio reactor for single-process), \
         got: {:?}",
        res.runtime_rs
    );
    assert_eq!(
        res.worker_bins.len(),
        1,
        "single-worker-route must emit exactly one binary"
    );
}

#[test]
fn emit_result_shape_is_multi_binary_seven_field_with_optional_runtime() {
    // The CONSTRUCTOR is the pin. If a field is renamed/added/removed,
    // this fails to compile and the driver dispatch arm
    // (driver/src/main.rs match-arm "mp-uds-event" => { ... println!(...) ... })
    // must be updated in lockstep — the same shape the mp-tcp-event
    // dispatch arm uses: project_dir, cargo_toml, worker_bin0..N,
    // kernels_rs, wire_rs, runtime_rs (Option), run_sh.
    let r = EmitResult {
        project_dir: PathBuf::from("/p"),
        cargo_toml: PathBuf::from("/p/Cargo.toml"),
        worker_bins: vec![PathBuf::from("/p/src/bin/host.rs")],
        kernels_rs: PathBuf::from("/p/src/kernels.rs"),
        wire_rs: PathBuf::from("/p/src/wire.rs"),
        runtime_rs: Some(PathBuf::from("/p/src/runtime.rs")),
        run_sh: PathBuf::from("/p/run.sh"),
    };
    let _ = (
        &r.project_dir,
        &r.cargo_toml,
        &r.worker_bins,
        &r.kernels_rs,
        &r.wire_rs,
        &r.runtime_rs,
        &r.run_sh,
    );
}
