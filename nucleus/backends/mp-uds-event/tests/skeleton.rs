//! Skeleton smoke tests for the mp-uds-event backend (TASK-0044.03 cycle 175).
//!
//! Cycle status: SKELETON. `emit()` returns `EmitError::ContractGap` for
//! ALL inputs. These tests pin three invariants that hold even in the
//! skeleton phase:
//!
//! 1. Calling `emit()` on the smallest legal input (empty per-worker map)
//!    returns `EmitError::ContractGap`, not a panic.
//! 2. The ContractGap message names `mp-uds-event` and references
//!    TASK-0044.03 (the precise forward-link to the work that lands the
//!    substantive emit).
//! 3. The `EmitResult` struct shape exists and matches the multi-binary
//!    seven-field convention with `runtime_rs: Option<PathBuf>`
//!    (compile-time test — if the struct changes, the driver dispatch
//!    arm must change in lockstep).
//!
//! When substantive emit lands in subsequent cycles of TASK-0044.03,
//! follow the mp-tcp-event precedent (tests/multi_worker_emit.rs) and
//! replace these aspirational tests with real emit-path smoke tests.

use std::collections::BTreeMap;
use std::path::PathBuf;

use mp_uds_event::{emit, EmitError, EmitResult, NameTables};

#[test]
fn skeleton_emit_returns_contract_gap_with_forward_link() {
    let per_worker = BTreeMap::new();
    let names = NameTables::default();
    let sidecar = nucleus_compiler::sidecar::NameSidecar::default();
    let kernels_path = PathBuf::from("/tmp/does-not-need-to-exist-skeleton-cycle.rs");
    let out_dir = PathBuf::from("/tmp/does-not-need-to-exist-skeleton-cycle/");

    let result = emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir);

    let err = result.expect_err("mp-uds-event skeleton must return ContractGap, not Ok");
    let msg = format!("{err}");
    assert!(
        matches!(err, EmitError::ContractGap(_)),
        "mp-uds-event skeleton must return ContractGap variant, got: {msg}"
    );
    assert!(
        msg.contains("mp-uds-event"),
        "ContractGap message must name `mp-uds-event`, got: {msg}"
    );
    assert!(
        msg.contains("TASK-0044.03"),
        "ContractGap message must forward-link TASK-0044.03, got: {msg}"
    );
}

#[test]
fn emit_result_shape_is_multi_binary_seven_field_with_optional_runtime() {
    // The CONSTRUCTOR is the pin. If a field is renamed/added/removed,
    // this fails to compile and the driver dispatch arm (driver/src/
    // main.rs match-arm "mp-uds-event" => { ... println!(...) ... })
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
