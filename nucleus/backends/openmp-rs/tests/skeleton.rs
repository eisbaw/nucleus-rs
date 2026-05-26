//! Skeleton smoke tests for the openmp-rs backend (TASK-0044.01 cycle 173).
//!
//! Cycle status: SKELETON. `emit()` returns `EmitError::ContractGap` for
//! ALL inputs. These tests pin three invariants that hold even in the
//! skeleton phase:
//!
//! 1. The `EmitResult` struct shape exists and matches the single-binary
//!    five-field convention (compile-time test — if the struct changes,
//!    the driver dispatch arm must change in lockstep).
//! 2. Calling `emit()` on the smallest legal input (empty per-worker map)
//!    returns `EmitError::ContractGap`, not a panic.
//! 3. The ContractGap message names `openmp-rs` and references
//!    TASK-0044.01, so the user-facing error carries the precise
//!    forward-link to the work that lands the substantive emit.
//!
//! When substantive emit lands in subsequent cycles of TASK-0044.01,
//! follow the pthreads-async precedent (tests/skeleton.rs) and replace
//! these aspirational tests with real emit-path smoke tests.

use std::collections::BTreeMap;
use std::path::PathBuf;

use openmp_rs::{emit, EmitError, EmitResult, NameTables};

#[test]
fn skeleton_emit_returns_contract_gap_with_forward_link() {
    let per_worker = BTreeMap::new();
    let names = NameTables::default();
    let sidecar = nucleus_compiler::sidecar::NameSidecar::default();
    let kernels_path = PathBuf::from("/tmp/does-not-need-to-exist-skeleton-cycle.rs");
    let out_dir = PathBuf::from("/tmp/does-not-need-to-exist-skeleton-cycle/");

    let result = emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir);

    let err = result.expect_err("openmp-rs skeleton must return ContractGap, not Ok");
    let msg = format!("{err}");
    assert!(
        matches!(err, EmitError::ContractGap(_)),
        "openmp-rs skeleton must return ContractGap variant, got: {msg}"
    );
    assert!(
        msg.contains("openmp-rs"),
        "ContractGap message must name `openmp-rs`, got: {msg}"
    );
    assert!(
        msg.contains("TASK-0044.01"),
        "ContractGap message must forward-link TASK-0044.01, got: {msg}"
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
    // updated in lockstep. Runtime asserts on the just-written values
    // pin nothing beyond derive(Eq); cycle 173 architect P3.2 dropped
    // them as noise.
    let r = EmitResult {
        project_dir: PathBuf::from("/p"),
        cargo_toml: PathBuf::from("/p/Cargo.toml"),
        main_rs: PathBuf::from("/p/src/main.rs"),
        kernels_rs: PathBuf::from("/p/src/kernels.rs"),
        run_sh: PathBuf::from("/p/run.sh"),
    };
    // Read each field once so the compiler doesn't warn about an
    // unused binding; this is the use-site that links the names to
    // the dispatch contract.
    let _ = (&r.project_dir, &r.cargo_toml, &r.main_rs, &r.kernels_rs, &r.run_sh);
}
