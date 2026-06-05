//! TASK-0405: per-backend bite tests for the three filesystem-I/O
//! `EmitError` variants on the mpi-nonblocking backend.
//!
//! Cross-backend silent-sibling sweep. TASK-0404 added these bite tests
//! to the CANONICAL backend (pthreads-sync) at VARIANT granularity; the
//! same three `fs::*().map_err(EmitError::X)` sites recur verbatim in all
//! ten backends. This file closes the SITE-granularity gap for
//! mpi-nonblocking so a future refactor that re-introduces an
//! `unwrap`/`expect`/`?`-on-raw-io in THIS backend's emit-I/O path is
//! caught by a permanent regression test instead of slipping through.
//!
//! NOTE: the mpi-nonblocking crate is a NORMAL host std crate — `just ci` /
//! `cargo test` build and run these tests WITHOUT any MPI toolchain (the
//! `mpi` crate dependency lives only in the GENERATED project, not in
//! this backend crate). So these bite tests run in the standard gate.
//!
//! `EmitError` derives only `Debug` (its `io::Error` payload is not
//! `PartialEq`), so each test matches on the variant pattern.
//!
//! mpi-nonblocking layout: the first generated write (the `WriteFailed`
//! target) is `out_dir/src/kernels.rs` (in BOTH the single- and
//! multi-worker SPMD arms); its `OutputCreateFailed` directory is
//! `out_dir/src`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use mpi_nonblocking::{emit, EmitError, NameTables};
use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("three ancestors above mpi-nonblocking crate")
        .to_path_buf()
}

fn scratch_dir(name: &str) -> PathBuf {
    let target = repo_root().join("nucleus/target/mpi-nonblocking-test-scratch");
    test_common::unique_scratch_dir(&target, name)
}

fn lower_example_01_naive() -> (
    BTreeMap<WorkerId, Vec<Event>>,
    NameTables,
    NameSidecar,
    PathBuf,
) {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/01-elementwise-add");
    let algo_src = fs::read_to_string(ex.join("prog.algo.nuc")).expect("01 algo");
    let sched_src = fs::read_to_string(ex.join("schedules/naive.sched.nuc")).expect("01 sched");
    let r = test_common::lower_for_test(
        &algo_src,
        &sched_src,
        &test_common::LowerForTestOpts::default(),
    );
    (r.per_worker, r.names, r.sidecar, ex.join("kernels.rs"))
}

/// `KernelsReadFailed`: a missing kernels path fails the first I/O op.
#[test]
fn kernels_read_failed_on_missing_kernels_rs() {
    let (pw, names, sc, _kernels) = lower_example_01_naive();
    let out = scratch_dir("emiterr_kernels_read_failed");

    let missing = out.join("does-not-exist-kernels.rs");
    assert!(
        !missing.exists(),
        "precondition: the kernels path must not exist"
    );

    let err = emit(&pw, &names, &sc, &missing, &out)
        .expect_err("a missing kernels.rs must fail with a typed error, not panic");
    match err {
        EmitError::KernelsReadFailed { path, source } => {
            assert_eq!(path, missing, "KernelsReadFailed must name the missing path");
            assert_eq!(
                source.kind(),
                std::io::ErrorKind::NotFound,
                "the wrapped io::Error must be NotFound for a missing file"
            );
        }
        other => panic!("expected KernelsReadFailed, got {other:?}"),
    }
}

/// `OutputCreateFailed`: rooting `out_dir` under a regular file makes
/// `create_dir_all(out_dir/src)` fail.
#[test]
fn output_create_failed_when_a_path_component_is_a_file() {
    let (pw, names, sc, kernels) = lower_example_01_naive();
    let scratch = scratch_dir("emiterr_output_create_failed");

    let blocker = scratch.join("blocker");
    fs::write(&blocker, b"i am a regular file, not a directory").unwrap();
    let out = blocker.join("nested-out");

    let err = emit(&pw, &names, &sc, &kernels, &out)
        .expect_err("out_dir under a regular file must fail with a typed error");
    match err {
        EmitError::OutputCreateFailed { path, .. } => {
            assert!(
                path.starts_with(&out),
                "OutputCreateFailed must name a path under out_dir; got {path:?}"
            );
        }
        other => panic!("expected OutputCreateFailed, got {other:?}"),
    }
}

/// `WriteFailed`: pre-creating `out_dir/src/kernels.rs` (mpi-nonblocking's
/// first generated write) as a DIRECTORY makes `fs::write` fail.
#[test]
fn write_failed_when_first_write_target_is_a_directory() {
    let (pw, names, sc, kernels) = lower_example_01_naive();
    let out = scratch_dir("emiterr_write_failed");

    let kernels_rs_as_dir = out.join("src").join("kernels.rs");
    fs::create_dir_all(&kernels_rs_as_dir)
        .expect("pre-create the src/kernels.rs path as a directory");

    let err = emit(&pw, &names, &sc, &kernels, &out)
        .expect_err("writing onto a directory must fail with a typed error");
    match err {
        EmitError::WriteFailed { path, .. } => {
            assert_eq!(
                path, kernels_rs_as_dir,
                "WriteFailed must name the src/kernels.rs path it could not write \
                 (mpi-nonblocking's first generated write)"
            );
        }
        other => panic!("expected WriteFailed, got {other:?}"),
    }
}
