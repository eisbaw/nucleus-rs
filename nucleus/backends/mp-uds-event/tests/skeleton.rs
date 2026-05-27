//! Smoke tests for the mp-uds-event backend.
//!
//! Cycle status (TASK-0044.03 cycle 194):
//! - Single-worker arm IMPLEMENTED (delegation to
//!   `pthreads_sync::render_single_worker_main_with_kernels_attr` +
//!   `backend_common::project_skeleton::multi_binary` — byte-identical
//!   to mp-tcp-event's single-process emit).
//! - Multi-worker arm: ContractGap forward-link to TASK-0044.03.01
//!   (UDS-reactor codegen pending).
//!
//! Tests pinned here:
//! - Multi-worker `emit()` returns `EmitError::ContractGap` naming
//!   `mp-uds-event` + forward-link to TASK-0044.03.
//! - `EmitResult` shape pin (compile-time via constructor).
//!
//! Bit-identical single-worker emit differential against mp-tcp-event
//! lives in `tests/single_worker_emit.rs`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use mp_uds_event::{emit, EmitError, EmitResult, NameTables};
use nucleus_compiler::event::{DataId, Event, IterTile, WorkerId};

#[test]
fn multi_worker_emit_returns_contract_gap_with_forward_link() {
    // Two non-empty worker lists -> used_workers.len() >= 2 -> multi-
    // worker arm -> ContractGap. Cheapest legal Event variant (`Free`)
    // so the test does not accidentally exercise downstream emit
    // machinery — dispatch happens before any per-event walk.
    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    let dummy = Event::Free {
        data: DataId(0),
        tile: IterTile::empty(),
    };
    per_worker.insert(WorkerId(0), vec![dummy.clone()]);
    per_worker.insert(WorkerId(1), vec![dummy]);

    let names = NameTables::default();
    let sidecar = nucleus_compiler::sidecar::NameSidecar::default();

    // mp-uds-event's emit() reads kernels.rs UPFRONT (same structure as
    // mp-tcp-event + mp-tcp-poll), so the ContractGap dispatch on the
    // multi-worker arm needs a real kernels file. Use the workspace
    // target/ scratch dir so the test is hermetic.
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("target"))
        .expect("workspace target/");
    let stem = target.join("mp-uds-event-test-scratch/skeleton_multi_worker");
    let _ = std::fs::remove_dir_all(&stem);
    std::fs::create_dir_all(&stem).expect("scratch dir");
    let kernels_path = stem.join("kernels.rs");
    std::fs::write(&kernels_path, "// stub for ContractGap test\n").expect("kernels.rs stub");
    let out_dir = stem.join("out");

    let result = emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir);

    let err = result.expect_err("multi-worker mp-uds-event must return ContractGap, not Ok");
    let msg = format!("{err}");
    assert!(
        matches!(err, EmitError::ContractGap(_)),
        "mp-uds-event multi-worker must return ContractGap variant, got: {msg}"
    );
    assert!(
        msg.contains("mp-uds-event"),
        "ContractGap message must name `mp-uds-event`, got: {msg}"
    );
    assert!(
        msg.contains("TASK-0044.03"),
        "ContractGap message must forward-link TASK-0044.03, got: {msg}"
    );
    assert!(
        msg.contains("multi-worker"),
        "ContractGap message must scope itself to the multi-worker arm, got: {msg}"
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
