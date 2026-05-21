//! Skeleton smoke test for the pthreads-async backend.
//!
//! TASK-0042.01 cycle 16 (2026-05-21): the crate ships as a skeleton
//! with capabilities.toml + driver wiring + a stub `emit()` returning
//! `EmitError::ContractGap`. These tests pin the foundation so the
//! later codegen cycles (TASK-0226+) can replace `emit`'s body
//! without bikeshedding the wire shape.
//!
//! Two asserts:
//!
//! 1. `emit()` returns the SKELETON `ContractGap` whose message contains
//!    the precise forward-link to TASK-0226 (the codegen sub-task) so a
//!    future implementer is steered correctly the moment they see this
//!    error path. The message text is the documented contract; later
//!    cycles delete this test as part of TASK-0226.
//!
//! 2. The `EmitResult` shape is the single-binary five-field shape
//!    (mirrors pthreads-sync). Compile-time only: if a later cycle
//!    changes the struct, this `let _r = EmitResult { ... }` won't
//!    compile and the driver dispatch arm has to be updated in lockstep.

use std::collections::BTreeMap;
use std::path::PathBuf;

use compiler::event::Event;
use compiler::sidecar::NameSidecar;
use pthreads_async::{emit, EmitError, EmitResult, NameTables};

#[test]
fn skeleton_emit_returns_contract_gap_with_task_0226_forward_link() {
    let per_worker = BTreeMap::new();
    let names = NameTables::default();
    let sidecar = NameSidecar::default();
    // Paths are never read by the skeleton — it short-circuits before
    // any I/O. Use unmistakably bogus paths so a future drift (the
    // body being filled in but the early-return left behind) surfaces
    // as an I/O error rather than a silent success.
    let kernels = PathBuf::from("/does-not-exist/kernels.rs");
    let out = PathBuf::from("/does-not-exist/out");

    let res = emit(&per_worker, &names, &sidecar, &kernels, &out);

    match res {
        Err(EmitError::ContractGap(msg)) => {
            assert!(
                msg.contains("pthreads-async"),
                "skeleton ContractGap message must name the backend: {msg}"
            );
            assert!(
                msg.contains("TASK-0226"),
                "skeleton ContractGap message must forward-link TASK-0226 (the \
                 codegen sub-task) so the next implementer steps directly into \
                 the right place: {msg}"
            );
            assert!(
                msg.contains("not yet implemented")
                    || msg.contains("skeleton"),
                "skeleton ContractGap message must declare its own status: {msg}"
            );
        }
        Err(other) => panic!(
            "expected EmitError::ContractGap from the skeleton, got: {other:?}"
        ),
        Ok(result) => panic!(
            "skeleton must return Err; got Ok({result:?}). Did TASK-0226 \
             land without removing this skeleton test? Delete this test \
             as part of that cycle."
        ),
    }
}

#[test]
fn emit_result_shape_is_single_binary_five_fields() {
    // Compile-time pin: if the EmitResult struct gains/loses a field
    // this test stops compiling. The driver dispatch arm
    // (`nucleus/driver/src/main.rs`, the `"pthreads-async"` match arm)
    // must update in lockstep — pinning the shape here keeps the two
    // in sync.
    let _r = EmitResult {
        project_dir: PathBuf::from("/p"),
        cargo_toml: PathBuf::from("/p/Cargo.toml"),
        main_rs: PathBuf::from("/p/src/main.rs"),
        kernels_rs: PathBuf::from("/p/src/kernels.rs"),
        run_sh: PathBuf::from("/p/run.sh"),
    };
    // Silence the unused-binding lint via an explicit ack — we want
    // the binding to live so the compile-time check matters.
    let _ = _r;
}

// Suppress the "unused import" warning that fires until later cycles
// give these types a real test consumer. They are listed here on
// purpose: they form the documentary `use ...` contract that the
// driver consumes (Event + NameSidecar via the inert compiler exports;
// emit + EmitError + EmitResult + NameTables via this crate). Removing
// the `use` would let the next implementer think the skeleton's
// dependencies were narrower than they really are.
#[allow(dead_code)]
fn _doc_use_sanity(
    _e: Vec<Event>,
    _s: NameSidecar,
) {
}
