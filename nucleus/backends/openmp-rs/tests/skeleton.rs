//! Smoke tests for the openmp-rs backend.
//!
//! Cycle status (TASK-0044.01 cycle 191):
//! - Single-worker arm IMPLEMENTED (delegation to
//!   `pthreads_sync::render_single_worker_main` +
//!   `backend_common::project_skeleton::single_binary`).
//! - Multi-worker arm: ContractGap forward-link to the rayon-scope
//!   follow-up sub-task of TASK-0044.01.
//!
//! Tests pinned here:
//! 1. The `EmitResult` struct shape (compile-pin via constructor).
//! 2. Multi-worker `emit()` returns `EmitError::ContractGap` naming
//!    `openmp-rs` + forward-link to TASK-0044.01.
//! 3. Empty per-worker map returns Ok via single-worker arm (used == 0).
//!
//! Bit-identical single-worker emit-string differential against
//! pthreads-sync lives in `tests/single_worker_emit.rs` (a separate
//! file so the skeleton smoke tests stay small).

use std::collections::BTreeMap;
use std::path::PathBuf;

use nucleus_compiler::event::{DataId, Event, IterTile, WorkerId};
use openmp_rs::{emit, EmitError, EmitResult, NameTables};

#[test]
fn multi_worker_emit_returns_contract_gap_with_forward_link() {
    // Two non-empty worker lists -> used_workers.len() >= 2 -> multi-
    // worker arm -> ContractGap. The event content does not matter for
    // this pin; the dispatch happens before any per-event walk. Use the
    // cheapest legal Event variant (`Free`) so the test does not
    // accidentally exercise downstream emit machinery.
    let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    let dummy = Event::Free {
        data: DataId(0),
        tile: IterTile::empty(),
    };
    per_worker.insert(WorkerId(0), vec![dummy.clone()]);
    per_worker.insert(WorkerId(1), vec![dummy]);

    let names = NameTables::default();
    let sidecar = nucleus_compiler::sidecar::NameSidecar::default();
    let kernels_path = PathBuf::from("/tmp/openmp-rs-skeleton-cycle-191-kernels.rs");
    let out_dir = PathBuf::from("/tmp/openmp-rs-skeleton-cycle-191-out/");

    let result = emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir);

    let err = result.expect_err("multi-worker openmp-rs must return ContractGap, not Ok");
    let msg = format!("{err}");
    assert!(
        matches!(err, EmitError::ContractGap(_)),
        "openmp-rs multi-worker must return ContractGap variant, got: {msg}"
    );
    assert!(
        msg.contains("openmp-rs"),
        "ContractGap message must name `openmp-rs`, got: {msg}"
    );
    assert!(
        msg.contains("TASK-0044.01"),
        "ContractGap message must forward-link TASK-0044.01, got: {msg}"
    );
    assert!(
        msg.contains("multi-worker"),
        "ContractGap message must scope itself to the multi-worker arm, got: {msg}"
    );
}

#[test]
fn emit_result_shape_is_single_binary_five_field() {
    // Compile-time + minimal-runtime pin of the EmitResult shape. If a
    // later cycle adds / renames / removes a field, this test fails
    // and the driver dispatch arm (driver/src/main.rs match-arm
    // "openmp-rs" => { ... println!(...) ... }) MUST be updated in
    // lockstep — the same five println! lines (project_dir / cargo_toml
    // / main_rs / kernels_rs / run_sh) the dispatch site prints.
    // The CONSTRUCTOR is the pin. If a field is renamed/added/removed,
    // this fails to compile and the driver dispatch arm must be
    // updated in lockstep.
    let r = EmitResult {
        project_dir: PathBuf::from("/p"),
        cargo_toml: PathBuf::from("/p/Cargo.toml"),
        main_rs: PathBuf::from("/p/src/main.rs"),
        kernels_rs: PathBuf::from("/p/src/kernels.rs"),
        run_sh: PathBuf::from("/p/run.sh"),
    };
    let _ = (&r.project_dir, &r.cargo_toml, &r.main_rs, &r.kernels_rs, &r.run_sh);
}
