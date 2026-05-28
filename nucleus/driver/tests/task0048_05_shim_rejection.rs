//! Integration test for TASK-0048.05: pin the driver's two `--shim`
//! fail-fast rejection arms (M10, TASK-0048.01 cycle-238b architect
//! P3.3). Both guards live in `nucleus/driver/src/main.rs` (grep
//! `--shim` for the current sites; symbol grep, not line numbers, so
//! this docstring does not rot on the next refactor):
//!
//!   1. `--shim VALUE` on a NON-embedded backend  -> typed error
//!      ("--shim is only meaningful for the embedded-pattern backend"),
//!      placed before the backend dispatch match.
//!   2. unknown `--shim VALUE` on `embedded-pattern` -> typed error
//!      ("unknown --shim `VALUE` for backend embedded-pattern"),
//!      inside the embedded-pattern dispatch arm's `match shim`.
//!
//! `cmd_build` has no in-crate test module, so before this test the two
//! guards were covered only by manual runs — a future refactor could
//! silently drop a guard (the silent-sibling failure class). These
//! subprocess tests exercise the REAL binary end to end: each asserts a
//! non-zero process exit AND the variant-specific typed-error text, so
//! dropping or weakening either guard fails the gate LOUD. Both guards
//! reject before any generated-project codegen, so the tests are cheap
//! (no cargo build of an emitted project).
//!
//! Subprocess pattern mirrors `nucleus/driver/tests/cli_reuse_strict.rs`.

use std::path::PathBuf;
use std::process::Command;

/// Walk up from `CARGO_MANIFEST_DIR` until we find the repo root.
/// Same idiom as `cli_reuse_strict.rs` / `emit_pn.rs`.
fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if p.join("nucleus").join("Cargo.toml").exists()
            && p.join("nuc-nucleus").join("PRD.md").exists()
        {
            return p;
        }
        if !p.pop() {
            panic!("could not locate repo root from CARGO_MANIFEST_DIR");
        }
    }
}

fn nucleus_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_nucleus"))
}

/// A real, maintained single-worker example. `01-elementwise-add/naive`
/// is what `just check-embedded` builds for the embedded-pattern
/// backend, so it is guaranteed to drive the pipeline up to the backend
/// dispatch (where both guards sit) for both the embedded and the
/// tier-1 case.
fn example_files() -> (PathBuf, PathBuf, PathBuf) {
    let ex = repo_root()
        .join("nuc-nucleus")
        .join("examples")
        .join("01-elementwise-add");
    (
        ex.join("prog.algo.nuc"),
        ex.join("schedules").join("naive.sched.nuc"),
        ex.join("kernels.rs"),
    )
}

fn fresh_outdir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nucleus-shim-reject-{}-{}", tag, std::process::id()));
    if p.exists() {
        let _ = std::fs::remove_dir_all(&p);
    }
    std::fs::create_dir_all(&p).expect("create tempdir");
    p
}

/// AC#1: an unrecognised `--shim` value on `--backend embedded-pattern`
/// must produce the typed unknown-shim error and a failure exit — NOT a
/// panic, NOT a silent ignore that falls through to the M9 lib emit.
#[test]
fn unknown_shim_on_embedded_pattern_rejects_typed_nonzero() {
    let (algo, sched, kernels) = example_files();
    let out = fresh_outdir("unknown-shim");

    let result = Command::new(nucleus_bin())
        .arg("build")
        .arg("--algo")
        .arg(&algo)
        .arg("--sched")
        .arg(&sched)
        .arg("--kernels")
        .arg(&kernels)
        .arg("--backend")
        .arg("embedded-pattern")
        .arg("--shim")
        .arg("bogus")
        .arg("--out")
        .arg(&out)
        .output()
        .expect("spawn nucleus");

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !result.status.success(),
        "expected `--shim bogus` on embedded-pattern to FAIL, but it \
         succeeded (silent-ignore regression).\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        combined.contains("unknown --shim `bogus`"),
        "expected the typed unknown-shim diagnostic naming the bad value \
         in CLI output.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        combined.contains("backend embedded-pattern"),
        "expected the unknown-shim diagnostic to name the embedded-pattern \
         backend.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // Must be a typed error, not a panic (PRD fail-fast rule + the
    // panic-not-diagnostic recurring defect class).
    assert!(
        !combined.contains("panicked"),
        "rejection must be a typed error, not a panic.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// AC#2: `--shim stm32h7` on a NON-embedded backend (here
/// `pthreads-sync`) must produce the typed wrong-backend error and a
/// failure exit. The guard sits before the backend dispatch match, so
/// it must fire rather than silently ignore the irrelevant shim.
#[test]
fn stm32h7_shim_on_non_embedded_backend_rejects_typed_nonzero() {
    let (algo, sched, kernels) = example_files();
    let out = fresh_outdir("wrong-backend");

    let result = Command::new(nucleus_bin())
        .arg("build")
        .arg("--algo")
        .arg(&algo)
        .arg("--sched")
        .arg(&sched)
        .arg("--kernels")
        .arg(&kernels)
        .arg("--backend")
        .arg("pthreads-sync")
        .arg("--shim")
        .arg("stm32h7")
        .arg("--out")
        .arg(&out)
        .output()
        .expect("spawn nucleus");

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !result.status.success(),
        "expected `--shim stm32h7` on pthreads-sync to FAIL, but it \
         succeeded (silent-ignore regression).\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        combined.contains("--shim is only meaningful for the embedded-pattern backend"),
        "expected the typed wrong-backend shim diagnostic in CLI output.\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        combined.contains("pthreads-sync"),
        "expected the wrong-backend diagnostic to name the offending \
         backend `pthreads-sync`.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !combined.contains("panicked"),
        "rejection must be a typed error, not a panic.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
