//! TASK-0405: per-backend bite tests for the three filesystem-I/O
//! `EmitError` variants on the embedded-pattern backend.
//!
//! Cross-backend silent-sibling sweep. TASK-0404 added these bite tests
//! to the CANONICAL backend (pthreads-sync) at VARIANT granularity; the
//! same three `fs::*().map_err(EmitError::X)` sites recur in all ten
//! backends. This file closes the SITE-granularity gap for
//! embedded-pattern so a future refactor that re-introduces an
//! `unwrap`/`expect`/`?`-on-raw-io in THIS backend's emit-I/O path is
//! caught by a permanent regression test instead of slipping through.
//!
//! NOTE: the embedded-pattern crate is a NORMAL host std crate — `just
//! ci` / `cargo test` build and run these tests WITHOUT any embedded
//! toolchain (the no_std-ness lives only in the GENERATED project, not in
//! this backend crate). So these bite tests run in the standard gate.
//!
//! LAYOUT DIFFERENCE vs the other nine backends: `emit` returns a
//! `MultiEmitResult` (one `EmitResult` per used worker) and emits a
//! `no_std` LIB project, not a bin/main project. For a SINGLE-worker
//! schedule the per-worker `project_dir` IS `out_dir`. The I/O sites:
//!   - `KernelsReadFailed`: `fs::read_to_string(kernels)` in `emit`
//!     (read once, shared across per-worker projects).
//!   - `OutputCreateFailed`: `create_dir_all(project_dir/src)` inside
//!     `emit_one_worker_lib` → `out_dir/src` for a single worker.
//!   - `WriteFailed`: the FIRST generated write is
//!     `project_dir/Cargo.toml` → `out_dir/Cargo.toml` (skeleton
//!     `render_cargo_toml`), written before `src/lib.rs`.
//!
//! `EmitError` derives only `Debug` (its `io::Error` payload is not
//! `PartialEq`), so each test matches on the variant pattern.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use embedded_pattern::{emit, EmitError, NameTables};
use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("three ancestors above embedded-pattern crate")
        .to_path_buf()
}

fn scratch_dir(name: &str) -> PathBuf {
    let target = repo_root().join("nucleus/target/embedded-pattern-test-scratch");
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

/// `KernelsReadFailed`: a missing kernels path fails the first I/O op
/// (`fs::read_to_string` is read ONCE in `emit`, before any per-worker
/// project is emitted).
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
/// `create_dir_all(project_dir/src)` (= `out_dir/src` for a single
/// worker) fail.
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

/// `WriteFailed`: pre-creating `out_dir/Cargo.toml` (embedded-pattern's
/// first generated write for a single-worker project) as a DIRECTORY
/// makes `fs::write` fail.
#[test]
fn write_failed_when_first_write_target_is_a_directory() {
    let (pw, names, sc, kernels) = lower_example_01_naive();
    let out = scratch_dir("emiterr_write_failed");

    let cargo_toml_as_dir = out.join("Cargo.toml");
    fs::create_dir_all(&cargo_toml_as_dir).expect("pre-create the Cargo.toml path as a directory");

    let err = emit(&pw, &names, &sc, &kernels, &out)
        .expect_err("writing onto a directory must fail with a typed error");
    match err {
        EmitError::WriteFailed { path, .. } => {
            assert_eq!(
                path, cargo_toml_as_dir,
                "WriteFailed must name the Cargo.toml path it could not write \
                 (embedded-pattern's first generated write)"
            );
        }
        other => panic!("expected WriteFailed, got {other:?}"),
    }
}
